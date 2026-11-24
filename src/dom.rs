use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{
    AddEventListenerOptions, Document, Element, Event, FocusOptions, HtmlCanvasElement,
    HtmlElement, HtmlInputElement, HtmlTextAreaElement, KeyboardEvent, ShadowRoot, Window,
};

use egui::accesskit::{Action, ActionRequest, Live, NodeId, Rect, Toggled};
use crate::mapper::{
    DomNodeSpec, DomOp, DomTag, TreeBridge, click_action, decrement_action, focus_action,
    increment_action, replace_selected_text_action, scroll_action, scroll_into_view_action,
    set_numeric_value_action, set_value_action,
};

pub(crate) enum PendingDomEvent {
    Action(ActionRequest),
    TextSelection {
        target_node: NodeId,
        anchor_utf16: usize,
        focus_utf16: usize,
    },
}

impl PendingDomEvent {
    fn target_node(&self) -> NodeId {
        match self {
            Self::Action(req) => req.target_node,
            Self::TextSelection { target_node, .. } => *target_node,
        }
    }

    fn is_text_mutation(&self) -> bool {
        matches!(
            self,
            Self::Action(req)
                if matches!(req.action, Action::SetValue | Action::ReplaceSelectedText)
        )
    }

    fn targets_text_state(&self, node: NodeId) -> bool {
        if self.target_node() != node {
            return false;
        }
        match self {
            Self::TextSelection { .. } => true,
            Self::Action(req) => matches!(
                req.action,
                Action::SetValue | Action::ReplaceSelectedText | Action::SetTextSelection
            ),
        }
    }
}

pub(crate) type ActionQueue = Arc<Mutex<Vec<PendingDomEvent>>>;
type RepaintWakeup = Rc<RefCell<Option<Box<dyn Fn()>>>>;

thread_local! {
    static BRIDGES: RefCell<HashMap<u64, WebDomBridge>> = RefCell::new(HashMap::new());
    static PENDING_CANVASES: RefCell<HashMap<u64, HtmlCanvasElement>> = RefCell::new(HashMap::new());
}

pub(crate) fn stash_pending_canvas(id: u64, canvas: HtmlCanvasElement) {
    PENDING_CANVASES.with(|pending| {
        pending.borrow_mut().insert(id, canvas);
    });
}

pub(crate) fn insert_bridge(id: u64, bridge: WebDomBridge) {
    BRIDGES.with(|bridges| {
        bridges.borrow_mut().insert(id, bridge);
    });
}

pub(crate) fn with_bridge_mut<R>(id: u64, f: impl FnOnce(&mut WebDomBridge) -> R) -> Option<R> {
    BRIDGES.with(|bridges| bridges.borrow_mut().get_mut(&id).map(f))
}

pub(crate) fn install_viewport_listeners(id: u64) {
    with_bridge_mut(id, |bridge| bridge.install_viewport_listeners(id));
}

pub(crate) fn drain_pending_events(id: u64, queue: &ActionQueue, out: &mut Vec<egui::Event>) {
    let drained = {
        let Ok(mut pending) = queue.lock() else {
            return;
        };
        if pending.is_empty() {
            return;
        }
        pending.drain(..).collect::<Vec<_>>()
    };

    for event in drained {
        match event {
            PendingDomEvent::Action(req) => out.push(egui::Event::AccessKitActionRequest(req)),
            PendingDomEvent::TextSelection {
                target_node,
                anchor_utf16,
                focus_utf16,
            } => {
                if let Some(req) = with_bridge_mut(id, |bridge| {
                    bridge
                        .tree
                        .set_text_selection_action(target_node, anchor_utf16, focus_utf16)
                })
                .flatten()
                {
                    out.push(egui::Event::AccessKitActionRequest(req));
                }
            }
        }
    }
}

pub(crate) fn has_dom_focus(id: u64) -> bool {
    BRIDGES.with(|bridges| {
        bridges
            .borrow()
            .get(&id)
            .is_some_and(WebDomBridge::has_dom_focus)
    })
}

pub(crate) fn remove_bridge(id: u64) {
    PENDING_CANVASES.with(|pending| {
        pending.borrow_mut().remove(&id);
    });
    let bridge = BRIDGES.with(|bridges| bridges.borrow_mut().remove(&id));
    if let Some(mut bridge) = bridge {
        bridge.destroy();
    }
}

struct AttachedListener {
    target: web_sys::EventTarget,
    event_type: &'static str,
    capture: bool,
    closure: Closure<dyn FnMut(Event)>,
}

impl Drop for AttachedListener {
    fn drop(&mut self) {
        let _ = self.target.remove_event_listener_with_callback_and_bool(
            self.event_type,
            self.closure.as_ref().unchecked_ref(),
            self.capture,
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InteractionKind {
    None,
    Click,
    Text,
    Numeric { unstepped: bool },
}

impl InteractionKind {
    fn from_spec(spec: &DomNodeSpec) -> Self {
        if spec.disabled {
            return Self::None;
        }
        match spec.tag {
            DomTag::Input => match spec.input_type {
                Some("checkbox" | "radio") => Self::Click,
                Some("range") => Self::Numeric {
                    unstepped: spec.numeric_step.is_none(),
                },
                Some("number") if spec.aria_role == Some("spinbutton") => Self::Numeric {
                    unstepped: spec.numeric_step.is_none(),
                },
                _ => Self::Text,
            },
            DomTag::TextArea => Self::Text,
            DomTag::Button | DomTag::Anchor => Self::Click,
            _ if spec.clickable => Self::Click,
            _ => Self::None,
        }
    }
}

pub(crate) struct WebDomBridge {
    window: Window,
    document: Document,
    canvas: Option<Element>,
    container: Element,
    tree: TreeBridge,
    elements: HashMap<NodeId, Element>,
    specs: HashMap<NodeId, DomNodeSpec>,
    listeners: HashMap<NodeId, Vec<AttachedListener>>,
    viewport_listeners: Vec<AttachedListener>,
    queue: ActionQueue,
    repaint_wakeup: RepaintWakeup,
    syncing_focus: Rc<Cell<bool>>,
    syncing_selection: Rc<Cell<bool>>,
    dom_owned_selection: Rc<RefCell<HashSet<NodeId>>>,
    last_canvas_rect: Option<(f64, f64, f64, f64)>,
    last_dpr: Option<f64>,
}

impl WebDomBridge {
    pub(crate) fn mount(
        id: u64,
        canvas_id: &str,
        queue: ActionQueue,
        wakeup: Box<dyn Fn()>,
    ) -> Option<Self> {
        let window = web_sys::window()?;
        let document = window.document()?;

        let pending_canvas = PENDING_CANVASES.with(|pending| pending.borrow_mut().remove(&id));
        let canvas = pending_canvas.map(Element::from).or_else(|| {
            if canvas_id.is_empty() {
                document.query_selector("canvas").ok().flatten()
            } else {
                document
                    .get_element_by_id(canvas_id)
                    .or_else(|| document.query_selector("canvas").ok().flatten())
            }
        });

        let container = document.create_element("div").ok()?;
        let container_label = canvas
            .as_ref()
            .and_then(|c| c.get_attribute("aria-label"))
            .filter(|s| !s.is_empty());
        let _ = container.set_attribute("data-egui-accesskit", "true");
        let _ = container.set_attribute("role", "application");
        let _ = container.set_attribute(
            "aria-label",
            container_label.as_deref().unwrap_or("Application"),
        );
        if let Some(html) = container.dyn_ref::<HtmlElement>() {
            let style = html.style();
            let _ = style.set_property("position", "fixed");
            let _ = style.set_property("opacity", "0");
            let _ = style.set_property("pointer-events", "none");
            let _ = style.set_property("overflow", "hidden");
            let _ = style.set_property("z-index", "1");
        }

        let mounted = if let Some(parent) = canvas.as_ref().and_then(|c| c.parent_node()) {
            parent.append_child(&container).is_ok()
        } else if let Some(body) = document.body() {
            body.append_child(&container).is_ok()
        } else {
            false
        };

        if !mounted {
            return None;
        }

        let mut bridge = Self {
            window,
            document,
            canvas,
            container,
            tree: TreeBridge::new(),
            elements: HashMap::new(),
            specs: HashMap::new(),
            listeners: HashMap::new(),
            viewport_listeners: Vec::new(),
            queue,
            repaint_wakeup: Rc::new(RefCell::new(Some(wakeup))),
            syncing_focus: Rc::new(Cell::new(false)),
            syncing_selection: Rc::new(Cell::new(false)),
            dom_owned_selection: Rc::new(RefCell::new(HashSet::new())),
            last_canvas_rect: None,
            last_dpr: None,
        };
        let _ = bridge.sync_viewport();
        Some(bridge)
    }

    fn install_viewport_listeners(&mut self, id: u64) {
        self.viewport_listeners.clear();
        let window_target: web_sys::EventTarget = self.window.clone().into();
        if let Some(l) = add_listener_with_options(&window_target, "scroll", true, true, move |_| {
            with_bridge_mut(id, Self::realign_viewport);
        }) {
            self.viewport_listeners.push(l);
        }
        if let Some(l) =
            add_listener_with_options(&window_target, "resize", false, true, move |_| {
                with_bridge_mut(id, Self::realign_viewport);
            })
        {
            self.viewport_listeners.push(l);
        }

        let container_target: web_sys::EventTarget = self.container.clone().into();
        let queue = Arc::clone(&self.queue);
        let wakeup = Rc::clone(&self.repaint_wakeup);
        if let Some(l) = add_listener(&container_target, "keydown", move |event| {
            let Some(kb) = event.dyn_ref::<KeyboardEvent>() else {
                return;
            };
            let action = match kb.key().as_str() {
                "PageDown" => Action::ScrollDown,
                "PageUp" => Action::ScrollUp,
                _ => return,
            };
            let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
                return;
            };
            if target.tag_name().eq_ignore_ascii_case("TEXTAREA") {
                return;
            }
            let Some(raw_id) = target.get_attribute("data-accesskit-id") else {
                return;
            };
            let Ok(node_id) = raw_id.parse::<u64>().map(NodeId) else {
                return;
            };
            event.prevent_default();
            event.stop_propagation();
            push_action(&queue, &wakeup, scroll_action(node_id, action));
        }) {
            self.viewport_listeners.push(l);
        }
    }

    pub(crate) fn has_dom_focus(&self) -> bool {
        active_element_for(self.container.as_ref(), &self.document)
            .is_some_and(|active| self.container.contains(Some(active.as_ref())))
    }

    pub(crate) fn apply_update(&mut self, update: egui::accesskit::TreeUpdate) {
        let (dpr, canvas_origin, viewport_changed) = self.sync_viewport();
        let ops = self.tree.apply_update(update);

        for op in ops {
            match op {
                DomOp::Upsert(spec) => self.upsert_node(spec, dpr, canvas_origin),
                DomOp::SetChildren { parent, children } => self.set_children(parent, &children),
                DomOp::Remove(id) => self.remove_node(id),
                DomOp::SetFocus(target) => self.set_focus(target),
            }
        }

        if viewport_changed {
            self.reapply_all_bounds(dpr, canvas_origin);
        }
    }

    pub(crate) fn realign_viewport(&mut self) {
        let (dpr, canvas_origin, viewport_changed) = self.sync_viewport();
        if viewport_changed {
            self.reapply_all_bounds(dpr, canvas_origin);
        }
    }

    pub(crate) fn destroy(&mut self) {
        *self.repaint_wakeup.borrow_mut() = None;
        self.viewport_listeners.clear();
        self.listeners.clear();
        self.dom_owned_selection.borrow_mut().clear();
        self.elements.clear();
        self.specs.clear();
        self.container.remove();
    }

    fn has_pending_text_event(&self, id: NodeId) -> bool {
        self.queue
            .lock()
            .is_ok_and(|q| q.iter().any(|ev| ev.targets_text_state(id)))
    }

    fn reapply_all_bounds(&self, dpr: f64, canvas_origin: (f64, f64)) {
        for (id, element) in &self.elements {
            if let Some(spec) = self.specs.get(id) {
                apply_bounds(element, spec.bounds, dpr, canvas_origin);
            }
        }
    }

    fn sync_viewport(&mut self) -> (f64, (f64, f64), bool) {
        let raw_dpr = self.window.device_pixel_ratio();
        let dpr = if raw_dpr > 0.0 && raw_dpr.is_finite() {
            raw_dpr
        } else {
            1.0
        };
        let dpr_changed = self.last_dpr != Some(dpr);
        self.last_dpr = Some(dpr);

        let (origin, rect_changed) = if let Some(canvas) = &self.canvas {
            let rect = canvas.get_bounding_client_rect();
            let origin_x = rect.left() + f64::from(canvas.client_left());
            let origin_y = rect.top() + f64::from(canvas.client_top());
            let next = (origin_x, origin_y, rect.width(), rect.height());
            let changed = self.last_canvas_rect != Some(next);
            if changed {
                self.last_canvas_rect = Some(next);
                if let Some(html) = self.container.dyn_ref::<HtmlElement>() {
                    let style = html.style();
                    let _ = style.set_property("left", &format!("{}px", next.0));
                    let _ = style.set_property("top", &format!("{}px", next.1));
                    let _ = style.set_property("width", &format!("{}px", next.2));
                    let _ = style.set_property("height", &format!("{}px", next.3));
                }
            }
            ((origin_x, origin_y), changed)
        } else {
            ((0.0, 0.0), false)
        };

        (dpr, origin, dpr_changed || rect_changed)
    }

    fn upsert_node(&mut self, spec: DomNodeSpec, dpr: f64, canvas_origin: (f64, f64)) {
        let id = spec.id;
        let old_spec = self.specs.get(&id);
        let desired_tag = element_tag_name(&spec);
        let tag_changed = old_spec.is_some_and(|old| element_tag_name(old) != desired_tag);
        let needs_rebind = old_spec.is_none_or(|old| {
            element_tag_name(old) != desired_tag
                || old.input_type != spec.input_type
                || InteractionKind::from_spec(old) != InteractionKind::from_spec(&spec)
                || old.focusable != spec.focusable
                || old.disabled != spec.disabled
                || old.read_only != spec.read_only
        });

        let element = if tag_changed || !self.elements.contains_key(&id) {
            let Ok(new_el) = self.document.create_element(desired_tag) else {
                return;
            };
            let _ = new_el.set_attribute("data-accesskit-id", &id.0.to_string());
            if let Some(old_el) = self.elements.insert(id, new_el.clone()) {
                if let Some(parent) = old_el.parent_node() {
                    let _ = parent.replace_child(&new_el, &old_el);
                }
                while let Some(child) = old_el.first_child() {
                    let _ = new_el.append_child(&child);
                }
            }
            new_el
        } else {
            self.elements.get(&id).unwrap().clone()
        };

        let skip_text_sync = self.has_pending_text_event(id);
        apply_dom_attributes(&element, &spec, dpr, canvas_origin, skip_text_sync);

        let dom_owned = if !skip_text_sync {
            self.dom_owned_selection.borrow_mut().remove(&id)
        } else {
            false
        };
        if !skip_text_sync
            && supports_dom_selection(&spec)
            && active_element_for(self.container.as_ref(), &self.document)
                .is_some_and(|active| active == element)
        {
            if dom_owned {
                if let Some((anchor_utf16, focus_utf16)) = read_dom_selection(&element)
                    && spec.text_selection != Some((anchor_utf16, focus_utf16))
                {
                    queue_text_selection(
                        &self.queue,
                        &self.repaint_wakeup,
                        id,
                        anchor_utf16,
                        focus_utf16,
                    );
                }
            } else if let Some((anchor, focus)) = spec.text_selection {
                write_dom_selection(&element, anchor, focus, &self.syncing_selection);
            }
        }

        if Some(id) == self.tree.root_id() && element.parent_node().is_none() {
            let _ = self.container.append_child(&element);
        }

        if needs_rebind {
            self.listeners.remove(&id);
            let attached = self.attach_listeners(&element, &spec);
            if !attached.is_empty() {
                self.listeners.insert(id, attached);
            }
        }

        self.specs.insert(id, spec);
    }

    fn attach_listeners(&self, element: &Element, spec: &DomNodeSpec) -> Vec<AttachedListener> {
        let mut out = Vec::new();
        let target: web_sys::EventTarget = element.clone().into();
        let id = spec.id;

        match InteractionKind::from_spec(spec) {
            InteractionKind::Click => {
                let queue = Arc::clone(&self.queue);
                let wakeup = Rc::clone(&self.repaint_wakeup);
                let is_anchor = spec.tag == DomTag::Anchor;
                if let Some(l) = add_listener(&target, "click", move |event| {
                    if is_anchor {
                        event.prevent_default();
                    }
                    event.stop_propagation();
                    push_action(&queue, &wakeup, click_action(id));
                }) {
                    out.push(l);
                }
            }
            InteractionKind::Text => {
                if !spec.read_only {
                    let el_for_input = element.clone();
                    let queue = Arc::clone(&self.queue);
                    let wakeup = Rc::clone(&self.repaint_wakeup);
                    if spec.input_type == Some("password") {
                        let prev_val = Rc::new(RefCell::new(
                            read_text_value(element).unwrap_or_default(),
                        ));
                        let owned_selection = Rc::clone(&self.dom_owned_selection);
                        for event_name in ["focus", "beforeinput"] {
                            let el_snap = element.clone();
                            let prev_snap = Rc::clone(&prev_val);
                            if let Some(l) = add_listener(&target, event_name, move |_event| {
                                if let Some(val) = read_text_value(&el_snap) {
                                    *prev_snap.borrow_mut() = val;
                                }
                            }) {
                                out.push(l);
                            }
                        }
                        if let Some(l) = add_listener(&target, "input", move |event| {
                            event.stop_propagation();
                            let Some(new_val) = read_text_value(&el_for_input) else {
                                return;
                            };
                            let mut prev = prev_val.borrow_mut();
                            if new_val.chars().any(is_password_mask_char) {
                                let (anchor_utf16, focus_utf16, inserted) =
                                    password_diff_utf16(&prev, &new_val);
                                *prev = new_val;
                                if let Ok(mut guard) = queue.lock() {
                                    guard.retain(|ev| !ev.targets_text_state(id));
                                    guard.push(PendingDomEvent::TextSelection {
                                        target_node: id,
                                        anchor_utf16,
                                        focus_utf16,
                                    });
                                    guard.push(PendingDomEvent::Action(
                                        replace_selected_text_action(id, inserted),
                                    ));
                                }
                                if let Some(cb) = wakeup.borrow().as_ref() {
                                    cb();
                                }
                            } else {
                                *prev = new_val.clone();
                                owned_selection.borrow_mut().insert(id);
                                push_coalesced_value(&queue, &wakeup, id, set_value_action(id, new_val));
                            }
                        }) {
                            out.push(l);
                        }
                    } else {
                        let owned_selection = Rc::clone(&self.dom_owned_selection);
                        if let Some(l) = add_listener(&target, "input", move |event| {
                            event.stop_propagation();
                            if let Some(val) = read_text_value(&el_for_input) {
                                owned_selection.borrow_mut().insert(id);
                                push_coalesced_value(&queue, &wakeup, id, set_value_action(id, val));
                            }
                        }) {
                            out.push(l);
                        }
                    }
                }
                if let Some(l) = add_listener(&target, "keydown", |event| {
                    if let Some(kb) = event.dyn_ref::<KeyboardEvent>() {
                        let key = kb.key();
                        if key != "Tab" && key != "Escape" {
                            event.stop_propagation();
                        }
                    }
                }) {
                    out.push(l);
                }
                if supports_dom_selection(spec) {
                    let el_for_select = element.clone();
                    let queue = Arc::clone(&self.queue);
                    let wakeup = Rc::clone(&self.repaint_wakeup);
                    let syncing = Rc::clone(&self.syncing_selection);
                    if let Some(l) = add_listener(&target, "select", move |_event| {
                        emit_dom_selection(&el_for_select, id, &syncing, &queue, &wakeup);
                    }) {
                        out.push(l);
                    }

                    let el_for_keyup = element.clone();
                    let queue = Arc::clone(&self.queue);
                    let wakeup = Rc::clone(&self.repaint_wakeup);
                    let syncing = Rc::clone(&self.syncing_selection);
                    if let Some(l) = add_listener(&target, "keyup", move |event| {
                        if let Some(kb) = event.dyn_ref::<KeyboardEvent>()
                            && matches!(
                                kb.key().as_str(),
                                "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" | "Home" | "End"
                            )
                        {
                            emit_dom_selection(&el_for_keyup, id, &syncing, &queue, &wakeup);
                        }
                    }) {
                        out.push(l);
                    }
                }
            }
            InteractionKind::Numeric { unstepped } => {
                if let Some(input_el) = element.dyn_ref::<HtmlInputElement>().cloned() {
                    let queue = Arc::clone(&self.queue);
                    let wakeup = Rc::clone(&self.repaint_wakeup);
                    if let Some(l) = add_listener(&target, "input", move |event| {
                        event.stop_propagation();
                        if let Ok(val) = input_el.value().parse::<f64>() {
                            push_action(&queue, &wakeup, set_numeric_value_action(id, val));
                        }
                    }) {
                        out.push(l);
                    }
                }
                let queue = Arc::clone(&self.queue);
                let wakeup = Rc::clone(&self.repaint_wakeup);
                if let Some(l) = add_listener(&target, "keydown", move |event| {
                    if let Some(kb) = event.dyn_ref::<KeyboardEvent>() {
                        let key = kb.key();
                        if unstepped {
                            match key.as_str() {
                                "ArrowRight" | "ArrowUp" => {
                                    event.prevent_default();
                                    event.stop_propagation();
                                    push_action(&queue, &wakeup, increment_action(id));
                                    return;
                                }
                                "ArrowLeft" | "ArrowDown" => {
                                    event.prevent_default();
                                    event.stop_propagation();
                                    push_action(&queue, &wakeup, decrement_action(id));
                                    return;
                                }
                                _ => {}
                            }
                        }
                        if key.starts_with("Arrow") || key == "Home" || key == "End" {
                            event.stop_propagation();
                        }
                    }
                }) {
                    out.push(l);
                }
            }
            InteractionKind::None => {}
        }

        if spec.focusable && !spec.disabled && Some(id) != self.tree.root_id() {
            let queue = Arc::clone(&self.queue);
            let wakeup = Rc::clone(&self.repaint_wakeup);
            let syncing_focus = Rc::clone(&self.syncing_focus);
            if let Some(l) = add_listener(&target, "focus", move |_event| {
                if !syncing_focus.get() {
                    push_events(
                        &queue,
                        &wakeup,
                        [
                            PendingDomEvent::Action(focus_action(id)),
                            PendingDomEvent::Action(scroll_into_view_action(id)),
                        ],
                    );
                }
            }) {
                out.push(l);
            }
        }

        out
    }

    fn set_children(&self, parent: NodeId, children: &[NodeId]) {
        let Some(parent_el) = self.elements.get(&parent) else {
            return;
        };
        if Some(parent) == self.tree.root_id() && parent_el.parent_node().is_none() {
            let _ = self.container.append_child(parent_el);
        }
        if self
            .specs
            .get(&parent)
            .is_some_and(|s| matches!(s.tag, DomTag::Input | DomTag::TextArea))
        {
            return;
        }

        let parent_node: &web_sys::Node = parent_el.as_ref();
        let expected: Vec<&web_sys::Node> = children
            .iter()
            .filter_map(|id| self.elements.get(id).map(AsRef::as_ref))
            .collect();

        // Remove stale children first so surviving siblings stay attached without losing focus.
        let mut scan = parent_node.first_child();
        while let Some(node) = scan {
            scan = node.next_sibling();
            if !expected.contains(&&node) {
                let _ = parent_node.remove_child(&node);
            }
        }

        let mut cursor = parent_node.first_child();
        for child_node in expected {
            if cursor.as_ref() == Some(child_node) {
                cursor = cursor.and_then(|node| node.next_sibling());
            } else {
                let _ = parent_node.insert_before(child_node, cursor.as_ref());
            }
        }
    }

    fn remove_node(&mut self, id: NodeId) {
        self.listeners.remove(&id);
        self.specs.remove(&id);
        self.dom_owned_selection.borrow_mut().remove(&id);
        if let Some(el) = self.elements.remove(&id) {
            el.remove();
        }
    }

    fn set_focus(&self, target: Option<NodeId>) {
        let Some(target_id) = target else {
            return;
        };
        if Some(target_id) == self.tree.root_id() {
            return;
        }
        let Some(el) = self.elements.get(&target_id) else {
            return;
        };
        if active_element_for(self.container.as_ref(), &self.document)
            .is_some_and(|active| active == *el)
        {
            return;
        }
        if let Some(html) = el.dyn_ref::<HtmlElement>() {
            let options = FocusOptions::new();
            options.set_prevent_scroll(true);
            self.syncing_focus.set(true);
            let _ = html.focus_with_options(&options);
            self.syncing_focus.set(false);
        }
    }
}

fn active_element_for(node: &web_sys::Node, document: &Document) -> Option<Element> {
    let root = node.get_root_node();
    if let Some(doc) = root.dyn_ref::<Document>() {
        doc.active_element()
    } else if let Some(shadow) = root.dyn_ref::<ShadowRoot>() {
        shadow.active_element()
    } else {
        document.active_element()
    }
}

fn element_tag_name(spec: &DomNodeSpec) -> &'static str {
    if spec.tag == DomTag::Heading {
        match spec.heading_level {
            Some(1) => "h1",
            Some(2) | None => "h2",
            Some(3) => "h3",
            Some(4) => "h4",
            Some(5) => "h5",
            Some(6) => "h6",
            Some(_) => "div",
        }
    } else {
        spec.tag.as_str()
    }
}

fn supports_dom_selection(spec: &DomNodeSpec) -> bool {
    matches!(spec.tag, DomTag::TextArea)
        || (spec.tag == DomTag::Input
            && matches!(
                spec.input_type,
                None | Some("text" | "search" | "url" | "password")
            ))
}

fn read_text_value(element: &Element) -> Option<String> {
    if let Some(input) = element.dyn_ref::<HtmlInputElement>() {
        Some(input.value())
    } else {
        element.dyn_ref::<HtmlTextAreaElement>().map(|ta| ta.value())
    }
}

fn selection_fields(element: &Element) -> Option<(Option<u32>, Option<u32>, Option<String>)> {
    if let Some(input) = element.dyn_ref::<HtmlInputElement>() {
        Some((
            input.selection_start().ok()?,
            input.selection_end().ok()?,
            input.selection_direction().ok()?,
        ))
    } else if let Some(ta) = element.dyn_ref::<HtmlTextAreaElement>() {
        Some((
            ta.selection_start().ok()?,
            ta.selection_end().ok()?,
            ta.selection_direction().ok()?,
        ))
    } else {
        None
    }
}

fn read_dom_selection(element: &Element) -> Option<(usize, usize)> {
    let (Some(start), Some(end), dir) = selection_fields(element)? else {
        return None;
    };
    if dir.as_deref() == Some("backward") {
        Some((end as usize, start as usize))
    } else {
        Some((start as usize, end as usize))
    }
}

fn write_dom_selection(
    element: &Element,
    anchor: usize,
    focus: usize,
    syncing_selection: &Cell<bool>,
) {
    let (Ok(anchor_u32), Ok(focus_u32)) = (u32::try_from(anchor), u32::try_from(focus)) else {
        return;
    };
    let backward = anchor_u32 > focus_u32;
    let (start, end, dir) = if backward {
        (focus_u32, anchor_u32, "backward")
    } else {
        (anchor_u32, focus_u32, "forward")
    };

    if let Some((Some(cur_start), Some(cur_end), cur_dir)) = selection_fields(element)
        && cur_start == start
        && cur_end == end
        && cur_dir.as_deref().is_some_and(|d| (d == "backward") == backward)
    {
        return;
    }

    syncing_selection.set(true);
    if let Some(input) = element.dyn_ref::<HtmlInputElement>() {
        let _ = input.set_selection_range_with_direction(start, end, dir);
    } else if let Some(ta) = element.dyn_ref::<HtmlTextAreaElement>() {
        let _ = ta.set_selection_range_with_direction(start, end, dir);
    }
    syncing_selection.set(false);
}

fn emit_dom_selection(
    element: &Element,
    id: NodeId,
    syncing_selection: &Cell<bool>,
    queue: &ActionQueue,
    wakeup: &RepaintWakeup,
) {
    if syncing_selection.get() {
        return;
    }
    if let Some((anchor_utf16, focus_utf16)) = read_dom_selection(element) {
        queue_text_selection(queue, wakeup, id, anchor_utf16, focus_utf16);
    }
}

fn queue_text_selection(
    queue: &ActionQueue,
    wakeup: &RepaintWakeup,
    id: NodeId,
    anchor_utf16: usize,
    focus_utf16: usize,
) {
    let pushed = if let Ok(mut guard) = queue.lock() {
        if guard
            .iter()
            .any(|ev| ev.target_node() == id && ev.is_text_mutation())
        {
            false
        } else {
            guard.retain(|ev| {
                !(ev.target_node() == id && matches!(ev, PendingDomEvent::TextSelection { .. }))
            });
            guard.push(PendingDomEvent::TextSelection {
                target_node: id,
                anchor_utf16,
                focus_utf16,
            });
            true
        }
    } else {
        false
    };
    if pushed && let Some(cb) = wakeup.borrow().as_ref() {
        cb();
    }
}

fn push_coalesced_value(
    queue: &ActionQueue,
    wakeup: &RepaintWakeup,
    id: NodeId,
    request: ActionRequest,
) {
    if let Ok(mut guard) = queue.lock() {
        guard.retain(|ev| !ev.targets_text_state(id));
        guard.push(PendingDomEvent::Action(request));
    }
    if let Some(cb) = wakeup.borrow().as_ref() {
        cb();
    }
}

fn safe_href(url: Option<&str>) -> &str {
    match url {
        Some(u)
            if u.len() >= 8 && u[..8].eq_ignore_ascii_case("https://")
                || u.len() >= 7 && u[..7].eq_ignore_ascii_case("http://")
                || u.len() >= 7 && u[..7].eq_ignore_ascii_case("mailto:")
                || u.starts_with('#') =>
        {
            u
        }
        _ => "#",
    }
}

fn is_password_mask_char(c: char) -> bool {
    matches!(c, '\u{25CF}' | '\u{2022}')
}

fn password_diff_utf16(old: &str, new: &str) -> (usize, usize, String) {
    let old_u16: Vec<u16> = old.encode_utf16().collect();
    let new_u16: Vec<u16> = new.encode_utf16().collect();
    let min_len = old_u16.len().min(new_u16.len());
    let mut prefix = 0;
    while prefix < min_len && old_u16[prefix] == new_u16[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while prefix + suffix < min_len
        && old_u16[old_u16.len() - 1 - suffix] == new_u16[new_u16.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let inserted = String::from_utf16_lossy(&new_u16[prefix..new_u16.len() - suffix]);
    (prefix, old_u16.len() - suffix, inserted)
}

fn apply_dom_attributes(
    element: &Element,
    spec: &DomNodeSpec,
    dpr: f64,
    canvas_origin: (f64, f64),
    skip_value_sync: bool,
) {
    let base_role = match spec.aria_role {
        Some("meter" | "scrollbar") if spec.numeric_value.is_none() => None,
        other => other,
    };
    let effective_role = if base_role.is_none()
        && spec.tag == DomTag::Div
        && (spec.label.is_some() || spec.description.is_some())
    {
        Some("group")
    } else {
        base_role
    };
    let supports_range_aria = matches!(
        effective_role,
        Some("slider" | "spinbutton" | "progressbar" | "scrollbar" | "meter")
    ) || matches!(spec.input_type, Some("range" | "number"));

    set_optional_attr(element, "role", effective_role);
    set_optional_attr(element, "aria-label", spec.label.as_deref());
    set_optional_attr(element, "aria-description", spec.description.as_deref());
    set_optional_attr(element, "placeholder", spec.placeholder.as_deref());
    set_optional_attr(
        element,
        "aria-valuetext",
        supports_range_aria
            .then(|| spec.numeric_value.and(spec.value.as_deref()))
            .flatten(),
    );

    let heading_level = spec.heading_level.map(|n| n.to_string());
    set_optional_attr(element, "aria-level", heading_level.as_deref());
    set_optional_attr(element, "aria-modal", spec.modal.then_some("true"));
    let live = match spec.live {
        Some(Live::Polite) => Some("polite"),
        Some(Live::Assertive) => Some("assertive"),
        _ => None,
    };
    set_optional_attr(element, "aria-live", live);
    set_optional_attr(
        element,
        "aria-multiline",
        (spec.tag == DomTag::TextArea).then_some("true"),
    );

    if let Some(input_type) = spec.input_type {
        let _ = element.set_attribute("type", input_type);
    } else {
        let _ = element.remove_attribute("type");
    }

    if spec.tag == DomTag::Anchor {
        if spec.disabled {
            let _ = element.remove_attribute("href");
        } else {
            let _ = element.set_attribute("href", safe_href(spec.url.as_deref()));
        }
    }

    if spec.disabled {
        let _ = element.set_attribute("disabled", "");
        let _ = element.set_attribute("aria-disabled", "true");
    } else {
        let _ = element.remove_attribute("disabled");
        let _ = element.remove_attribute("aria-disabled");
    }

    if spec.read_only {
        let _ = element.set_attribute("readonly", "");
        let _ = element.set_attribute("aria-readonly", "true");
    } else {
        let _ = element.remove_attribute("readonly");
        let _ = element.remove_attribute("aria-readonly");
    }

    match spec.expanded {
        Some(true) => {
            let _ = element.set_attribute("aria-expanded", "true");
        }
        Some(false) => {
            let _ = element.set_attribute("aria-expanded", "false");
        }
        None => {
            let _ = element.remove_attribute("aria-expanded");
        }
    }

    if spec.tag == DomTag::Button {
        let _ = element.remove_attribute("aria-checked");
        match spec.toggled {
            Some(Toggled::True) => {
                let _ = element.set_attribute("aria-pressed", "true");
            }
            Some(Toggled::False) => {
                let _ = element.set_attribute("aria-pressed", "false");
            }
            Some(Toggled::Mixed) => {
                let _ = element.set_attribute("aria-pressed", "mixed");
            }
            None => {
                let _ = element.remove_attribute("aria-pressed");
            }
        }
    } else {
        let _ = element.remove_attribute("aria-pressed");
        match spec.toggled {
            Some(Toggled::True) => {
                let _ = element.set_attribute("aria-checked", "true");
            }
            Some(Toggled::False) => {
                let _ = element.set_attribute("aria-checked", "false");
            }
            Some(Toggled::Mixed) => {
                let _ = element.set_attribute("aria-checked", "mixed");
            }
            None => {
                let _ = element.remove_attribute("aria-checked");
            }
        }
    }

    if spec.focusable && !spec.disabled && !natively_focusable(spec.tag) {
        let _ = element.set_attribute("tabindex", "0");
    } else {
        let _ = element.remove_attribute("tabindex");
    }

    set_optional_attr(
        element,
        "aria-valuemin",
        supports_range_aria
            .then(|| spec.numeric_min.map(|v| v.to_string()))
            .flatten()
            .as_deref(),
    );
    set_optional_attr(
        element,
        "aria-valuemax",
        supports_range_aria
            .then(|| spec.numeric_max.map(|v| v.to_string()))
            .flatten()
            .as_deref(),
    );
    set_optional_attr(
        element,
        "aria-valuenow",
        supports_range_aria
            .then(|| spec.numeric_value.map(|v| v.to_string()))
            .flatten()
            .as_deref(),
    );

    if let Some(input_el) = element.dyn_ref::<HtmlInputElement>() {
        input_el.set_disabled(spec.disabled);
        input_el.set_read_only(spec.read_only);
        match spec.toggled {
            Some(Toggled::True) => {
                input_el.set_indeterminate(false);
                if !input_el.checked() {
                    input_el.set_checked(true);
                }
            }
            Some(Toggled::False) => {
                input_el.set_indeterminate(false);
                if input_el.checked() {
                    input_el.set_checked(false);
                }
            }
            Some(Toggled::Mixed) => {
                input_el.set_indeterminate(true);
                if input_el.checked() {
                    input_el.set_checked(false);
                }
            }
            None => {
                input_el.set_indeterminate(false);
            }
        }
        if spec.input_type == Some("range") || spec.aria_role == Some("spinbutton") {
            let step_str = spec
                .numeric_step
                .map(|s| s.to_string())
                .unwrap_or_else(|| String::from("any"));
            input_el.set_step(&step_str);
            set_optional_attr(
                element,
                "min",
                spec.numeric_min.map(|v| v.to_string()).as_deref(),
            );
            set_optional_attr(
                element,
                "max",
                spec.numeric_max.map(|v| v.to_string()).as_deref(),
            );
            if let Some(val) = spec.numeric_value {
                let s = val.to_string();
                if input_el.value() != s {
                    input_el.set_value(&s);
                }
            }
        } else if !matches!(spec.input_type, Some("checkbox" | "radio")) && !skip_value_sync {
            let val = spec.value.as_deref().unwrap_or("");
            if spec.input_type == Some("password") {
                let current = input_el.value();
                if current.chars().count() != val.chars().count()
                    || (val.is_empty() && !current.is_empty())
                {
                    input_el.set_value(val);
                }
            } else if input_el.value() != val {
                input_el.set_value(val);
            }
        }
    } else if let Some(textarea_el) = element.dyn_ref::<HtmlTextAreaElement>() {
        textarea_el.set_disabled(spec.disabled);
        textarea_el.set_read_only(spec.read_only);
        if !skip_value_sync {
            let val = spec.value.as_deref().unwrap_or("");
            if textarea_el.value() != val {
                textarea_el.set_value(val);
            }
        }
    } else if matches!(
        spec.tag,
        DomTag::Span | DomTag::Heading | DomTag::Anchor | DomTag::ListItem
    ) {
        let text = spec.value.as_deref().or(spec.label.as_deref()).unwrap_or("");
        if element.text_content().as_deref() != Some(text) {
            element.set_text_content(Some(text));
        }
    }

    apply_bounds(element, spec.bounds, dpr, canvas_origin);
}

fn apply_bounds(element: &Element, bounds: Option<Rect>, dpr: f64, canvas_origin: (f64, f64)) {
    let Some(html) = element.dyn_ref::<HtmlElement>() else {
        return;
    };
    let style = html.style();
    if let Some(bounds) = bounds {
        let left = canvas_origin.0 + bounds.x0 / dpr;
        let top = canvas_origin.1 + bounds.y0 / dpr;
        let width = (bounds.x1 - bounds.x0).max(0.0) / dpr;
        let height = (bounds.y1 - bounds.y0).max(0.0) / dpr;
        let _ = style.set_property("position", "fixed");
        let _ = style.set_property("left", &format!("{left}px"));
        let _ = style.set_property("top", &format!("{top}px"));
        let _ = style.set_property("width", &format!("{width}px"));
        let _ = style.set_property("height", &format!("{height}px"));
        let _ = style.set_property("margin", "0");
        let _ = style.set_property("padding", "0");
        let _ = style.set_property("border", "0");
        let _ = style.set_property("pointer-events", "none");
    } else {
        let _ = style.remove_property("position");
        let _ = style.remove_property("left");
        let _ = style.remove_property("top");
        let _ = style.remove_property("width");
        let _ = style.remove_property("height");
    }
}

fn natively_focusable(tag: DomTag) -> bool {
    matches!(
        tag,
        DomTag::Button | DomTag::Input | DomTag::TextArea | DomTag::Anchor
    )
}

fn set_optional_attr(element: &Element, name: &str, value: Option<&str>) {
    match value {
        Some(val) => {
            let _ = element.set_attribute(name, val);
        }
        None => {
            let _ = element.remove_attribute(name);
        }
    }
}

fn add_listener(
    target: &web_sys::EventTarget,
    event_type: &'static str,
    handler: impl FnMut(Event) + 'static,
) -> Option<AttachedListener> {
    let closure = Closure::wrap(Box::new(handler) as Box<dyn FnMut(Event)>);
    target
        .add_event_listener_with_callback(event_type, closure.as_ref().unchecked_ref())
        .ok()?;
    Some(AttachedListener {
        target: target.clone(),
        event_type,
        capture: false,
        closure,
    })
}

fn add_listener_with_options(
    target: &web_sys::EventTarget,
    event_type: &'static str,
    capture: bool,
    passive: bool,
    handler: impl FnMut(Event) + 'static,
) -> Option<AttachedListener> {
    let options = AddEventListenerOptions::new();
    options.set_capture(capture);
    options.set_passive(passive);
    let closure = Closure::wrap(Box::new(handler) as Box<dyn FnMut(Event)>);
    target
        .add_event_listener_with_callback_and_add_event_listener_options(
            event_type,
            closure.as_ref().unchecked_ref(),
            &options,
        )
        .ok()?;
    Some(AttachedListener {
        target: target.clone(),
        event_type,
        capture,
        closure,
    })
}

fn push_action(
    queue: &ActionQueue,
    wakeup: &RepaintWakeup,
    request: ActionRequest,
) {
    push_events(queue, wakeup, [PendingDomEvent::Action(request)]);
}

fn push_events(
    queue: &ActionQueue,
    wakeup: &RepaintWakeup,
    events: impl IntoIterator<Item = PendingDomEvent>,
) {
    if let Ok(mut guard) = queue.lock() {
        guard.extend(events);
    }
    if let Some(cb) = wakeup.borrow().as_ref() {
        cb();
    }
}

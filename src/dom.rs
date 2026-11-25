use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{
    AddEventListenerOptions, Document, Element, Event, FocusOptions, HtmlCanvasElement,
    HtmlElement, HtmlInputElement, HtmlTextAreaElement, InputEvent, KeyboardEvent, ResizeObserver,
    ShadowRoot, Window,
};

use crate::mapper::{
    DomNodeSpec, DomTag, TreeBridge, TreeDelta, click_action, decrement_action, focus_action,
    increment_action, replace_selected_text_action, scroll_action, scroll_into_view_action,
    set_numeric_value_action, set_value_action,
};
use egui::accesskit::{Action, ActionRequest, Invalid, Live, NodeId, Orientation, Rect, Toggled};

pub(crate) enum PendingDomEvent {
    Action(ActionRequest),
    KeyPress(egui::Key),
    SetValue {
        target_node: NodeId,
        value: String,
    },
    ReplaceRange {
        target_node: NodeId,
        anchor_utf16: usize,
        focus_utf16: usize,
        text: String,
    },
    TextSelection {
        target_node: NodeId,
        anchor_utf16: usize,
        focus_utf16: usize,
    },
}

impl PendingDomEvent {
    fn target_node(&self) -> Option<NodeId> {
        match self {
            Self::Action(req) => Some(req.target_node),
            Self::KeyPress(_) => None,
            Self::SetValue { target_node, .. }
            | Self::ReplaceRange { target_node, .. }
            | Self::TextSelection { target_node, .. } => Some(*target_node),
        }
    }

    fn is_text_mutation(&self) -> bool {
        match self {
            Self::SetValue { .. } | Self::ReplaceRange { .. } => true,
            Self::Action(req) => {
                matches!(req.action, Action::SetValue | Action::ReplaceSelectedText)
            }
            Self::KeyPress(_) | Self::TextSelection { .. } => false,
        }
    }

    fn targets_text_state(&self, node: NodeId) -> bool {
        if self.target_node() != Some(node) {
            return false;
        }
        match self {
            Self::SetValue { .. } | Self::ReplaceRange { .. } | Self::TextSelection { .. } => true,
            Self::Action(req) => matches!(
                req.action,
                Action::SetValue | Action::ReplaceSelectedText | Action::SetTextSelection
            ),
            Self::KeyPress(_) => false,
        }
    }
}

pub(crate) type ActionQueue = Arc<Mutex<Vec<PendingDomEvent>>>;
type RepaintWakeup = Rc<RefCell<Option<Box<dyn Fn()>>>>;
type ActivateCallback = Box<dyn FnMut()>;

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

pub(crate) fn try_with_bridge_mut<R>(id: u64, f: impl FnOnce(&mut WebDomBridge) -> R) -> Option<R> {
    BRIDGES.with(|bridges| {
        let Ok(mut map) = bridges.try_borrow_mut() else {
            return None;
        };
        map.get_mut(&id).map(f)
    })
}

pub(crate) fn activate_bridge(id: u64) {
    activate_bridge_inner(id, false);
}

fn activate_bridge_from_dom(id: u64) {
    activate_bridge_inner(id, true);
}

fn activate_bridge_inner(id: u64, from_dom: bool) {
    let cb = try_with_bridge_mut(id, |bridge| bridge.take_activation(from_dom)).flatten();
    if let Some(mut cb) = cb {
        cb();
    }
}

pub(crate) fn install_viewport_listeners(id: u64) {
    with_bridge_mut(id, |bridge| bridge.install_viewport_listeners(id));
}

pub(crate) fn drain_pending_events(
    id: u64,
    ctx: &egui::Context,
    queue: &ActionQueue,
    out: &mut Vec<egui::Event>,
) {
    let drained = {
        let Ok(mut pending) = queue.lock() else {
            return;
        };
        if pending.is_empty() {
            return;
        }
        pending.drain(..).collect::<Vec<_>>()
    };

    let deferred = with_bridge_mut(id, |bridge| {
        bridge.dom_action_in_flight = true;
        let current_focus = bridge.tree.focused_id();
        let mut focus_requested = HashSet::new();
        let mut deferred = Vec::new();

        for event in drained {
            if !deferred.is_empty() {
                deferred.push(event);
                continue;
            }
            match event {
                PendingDomEvent::Action(req) => {
                    if req.action == Action::Focus {
                        focus_requested.insert(req.target_node);
                    }
                    out.push(egui::Event::AccessKitActionRequest(req));
                }
                PendingDomEvent::KeyPress(key) => {
                    out.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                    out.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: false,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                PendingDomEvent::SetValue { target_node, value } => {
                    if bridge.tree.supports_text_edit_actions(target_node) {
                        bridge.pending_text_reconcile.insert(target_node);
                        out.push(egui::Event::AccessKitActionRequest(set_value_action(
                            target_node,
                            value,
                        )));
                    } else if should_defer_for_focus(
                        bridge,
                        current_focus,
                        target_node,
                        &mut focus_requested,
                        out,
                    ) {
                        deferred.push(PendingDomEvent::SetValue { target_node, value });
                    } else {
                        bridge.pending_text_reconcile.insert(target_node);
                        out.extend(TreeBridge::fallback_set_value_events(value));
                    }
                }
                PendingDomEvent::ReplaceRange {
                    target_node,
                    anchor_utf16,
                    focus_utf16,
                    text,
                } => {
                    if bridge.tree.supports_text_edit_actions(target_node) {
                        if let Some(req) = bridge.tree.set_text_selection_action(
                            target_node,
                            anchor_utf16,
                            focus_utf16,
                        ) {
                            out.push(egui::Event::AccessKitActionRequest(req));
                        }
                        bridge.pending_text_reconcile.insert(target_node);
                        out.push(egui::Event::AccessKitActionRequest(
                            replace_selected_text_action(target_node, text),
                        ));
                    } else if should_defer_for_focus(
                        bridge,
                        current_focus,
                        target_node,
                        &mut focus_requested,
                        out,
                    ) {
                        deferred.push(PendingDomEvent::ReplaceRange {
                            target_node,
                            anchor_utf16,
                            focus_utf16,
                            text,
                        });
                    } else {
                        bridge.pending_text_reconcile.insert(target_node);
                        out.extend(bridge.tree.fallback_replace_range_events(
                            target_node,
                            anchor_utf16,
                            focus_utf16,
                            text,
                        ));
                    }
                }
                PendingDomEvent::TextSelection {
                    target_node,
                    anchor_utf16,
                    focus_utf16,
                } => {
                    if !bridge.tree.supports_text_edit_actions(target_node)
                        && should_defer_for_focus(
                            bridge,
                            current_focus,
                            target_node,
                            &mut focus_requested,
                            out,
                        )
                    {
                        deferred.push(PendingDomEvent::TextSelection {
                            target_node,
                            anchor_utf16,
                            focus_utf16,
                        });
                    } else if let Some(req) = bridge.tree.set_text_selection_action(
                        target_node,
                        anchor_utf16,
                        focus_utf16,
                    ) {
                        out.push(egui::Event::AccessKitActionRequest(req));
                    }
                }
            }
        }

        if deferred.is_empty() {
            bridge.deferred_focus_target = None;
        }
        deferred
    })
    .unwrap_or_default();

    if !deferred.is_empty() {
        if let Ok(mut pending) = queue.lock() {
            let tail = pending.split_off(0);
            pending.extend(deferred);
            pending.extend(tail);
        }
        ctx.request_repaint();
    }
}

fn should_defer_for_focus(
    bridge: &mut WebDomBridge,
    current_focus: Option<NodeId>,
    target_node: NodeId,
    focus_requested: &mut HashSet<NodeId>,
    out: &mut Vec<egui::Event>,
) -> bool {
    if current_focus == Some(target_node) {
        return false;
    }
    let can_focus = bridge
        .tree
        .spec(target_node)
        .is_some_and(|s| s.focusable && !s.disabled);
    if !can_focus || bridge.deferred_focus_target == Some(target_node) {
        return false;
    }
    bridge.deferred_focus_target = Some(target_node);
    if focus_requested.insert(target_node) {
        out.push(egui::Event::AccessKitActionRequest(focus_action(
            target_node,
        )));
        out.push(egui::Event::AccessKitActionRequest(
            scroll_into_view_action(target_node),
        ));
    }
    true
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

struct ActiveResizeObserver {
    observer: ResizeObserver,
    _closure: Closure<dyn FnMut()>,
}

impl Drop for ActiveResizeObserver {
    fn drop(&mut self) {
        self.observer.disconnect();
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
    bridge_id: u64,
    window: Window,
    document: Document,
    canvas: Option<Element>,
    container: Element,
    activated: bool,
    activate_button: Option<Element>,
    activate_cb: Option<ActivateCallback>,
    tree: TreeBridge,
    elements: HashMap<NodeId, Element>,
    hoisted_children: HashMap<NodeId, Vec<NodeId>>,
    viewport_listeners: Vec<AttachedListener>,
    resize_observer: Option<ActiveResizeObserver>,
    modal_nodes: HashSet<NodeId>,
    modal_dirty: bool,
    /// Node that held DOM focus when a keyboard-driven modal opened; focus returns here on close.
    modal_return_focus: Option<NodeId>,
    inert_elements: Vec<Element>,
    queue: ActionQueue,
    repaint_wakeup: RepaintWakeup,
    syncing_focus: Rc<Cell<bool>>,
    syncing_selection: Rc<Cell<bool>>,
    dom_owned_selection: Rc<RefCell<HashSet<NodeId>>>,
    composing_nodes: Rc<RefCell<HashSet<NodeId>>>,
    password_prev: HashMap<NodeId, String>,
    pending_text_reconcile: HashSet<NodeId>,
    dom_action_in_flight: bool,
    deferred_focus_target: Option<NodeId>,
    viewport_dirty: bool,
    last_dpr: f64,
    last_buf_size: (u32, u32),
    last_canvas_rect: Option<(f64, f64, f64, f64)>,
    last_scale: Option<(f64, f64)>,
    bounds_buf: String,
}

impl WebDomBridge {
    pub(crate) fn mount(
        id: u64,
        canvas_id: &str,
        lazy: bool,
        queue: ActionQueue,
        wakeup: Box<dyn Fn()>,
        activate: ActivateCallback,
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
            let _ = style.set_property("z-index", "1");
            if canvas.is_some() {
                let _ = style.set_property("overflow", "hidden");
                let _ = style.set_property("contain", "strict");
            } else {
                let _ = style.set_property("left", "0px");
                let _ = style.set_property("top", "0px");
                let _ = style.set_property("width", "100%");
                let _ = style.set_property("height", "100%");
                let _ = style.set_property("contain", "layout style");
            }
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

        let activate_button = if lazy {
            let btn = document.create_element("button").ok()?;
            let _ = btn.set_attribute("type", "button");
            let _ = btn.set_attribute("data-egui-accesskit-activate", "true");
            let _ = btn.set_attribute("aria-label", "Enable screen reader accessibility");
            btn.set_text_content(Some("Enable screen reader accessibility"));
            if let Some(html) = btn.dyn_ref::<HtmlElement>() {
                let style = html.style();
                let _ = style.set_property("position", "absolute");
                let _ = style.set_property("left", "0px");
                let _ = style.set_property("top", "0px");
                let _ = style.set_property("width", "1px");
                let _ = style.set_property("height", "1px");
                let _ = style.set_property("margin", "0");
                let _ = style.set_property("padding", "0");
                let _ = style.set_property("border", "0");
                let _ = style.set_property("pointer-events", "none");
            }
            let _ = container.append_child(&btn);
            Some(btn)
        } else {
            None
        };

        let mut bridge = Self {
            bridge_id: id,
            window,
            document,
            canvas,
            container,
            activated: !lazy,
            activate_button,
            activate_cb: lazy.then_some(activate),
            tree: TreeBridge::new(),
            elements: HashMap::new(),
            hoisted_children: HashMap::new(),
            viewport_listeners: Vec::new(),
            resize_observer: None,
            modal_nodes: HashSet::new(),
            modal_dirty: false,
            modal_return_focus: None,
            inert_elements: Vec::new(),
            queue,
            repaint_wakeup: Rc::new(RefCell::new(Some(wakeup))),
            syncing_focus: Rc::new(Cell::new(false)),
            syncing_selection: Rc::new(Cell::new(false)),
            dom_owned_selection: Rc::new(RefCell::new(HashSet::new())),
            composing_nodes: Rc::new(RefCell::new(HashSet::new())),
            password_prev: HashMap::new(),
            pending_text_reconcile: HashSet::new(),
            dom_action_in_flight: false,
            deferred_focus_target: None,
            viewport_dirty: true,
            last_dpr: 0.0,
            last_buf_size: (0, 0),
            last_canvas_rect: None,
            last_scale: None,
            bounds_buf: String::with_capacity(32),
        };
        if bridge.activated {
            let _ = bridge.sync_viewport();
        }
        Some(bridge)
    }

    fn take_activation(&mut self, from_dom: bool) -> Option<ActivateCallback> {
        if let Some(btn) = self.activate_button.take() {
            btn.remove();
        }
        if self.activated {
            return None;
        }
        self.activated = true;
        self.viewport_dirty = true;
        if from_dom {
            self.dom_action_in_flight = true;
        }
        self.activate_cb.take()
    }

    fn install_viewport_listeners(&mut self, id: u64) {
        self.viewport_listeners.clear();
        self.resize_observer = None;

        let window_target: web_sys::EventTarget = self.window.clone().into();
        if let Some(l) = add_listener_with_options(&window_target, "scroll", true, true, move |_| {
            try_with_bridge_mut(id, Self::realign_viewport);
        }) {
            self.viewport_listeners.push(l);
        }
        if let Some(l) =
            add_listener_with_options(&window_target, "resize", false, true, move |_| {
                try_with_bridge_mut(id, Self::realign_viewport);
            })
        {
            self.viewport_listeners.push(l);
        }

        if let Some(canvas) = &self.canvas {
            let closure = Closure::wrap(Box::new(move || {
                try_with_bridge_mut(id, Self::realign_viewport);
            }) as Box<dyn FnMut()>);
            if let Ok(observer) = ResizeObserver::new(closure.as_ref().unchecked_ref()) {
                observer.observe(canvas);
                self.resize_observer = Some(ActiveResizeObserver {
                    observer,
                    _closure: closure,
                });
            }
        }

        let container_target: web_sys::EventTarget = self.container.clone().into();

        // 1. Focus (capture phase because focus does not bubble).
        let syncing_focus = Rc::clone(&self.syncing_focus);
        let queue = Arc::clone(&self.queue);
        let wakeup = Rc::clone(&self.repaint_wakeup);
        if let Some(l) =
            add_listener_with_options(&container_target, "focus", true, true, move |event| {
                if syncing_focus.get() {
                    return;
                }
                let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok())
                else {
                    return;
                };
                if target.has_attribute("data-egui-accesskit-activate") {
                    activate_bridge_from_dom(id);
                    return;
                }
                let Some(node_id) = parse_node_id(&target) else {
                    return;
                };
                let should_focus = try_with_bridge_mut(id, |bridge| {
                    let Some(spec) = bridge.tree.spec(node_id) else {
                        return false;
                    };
                    if InteractionKind::from_spec(spec) == InteractionKind::Text
                        && !spec.read_only
                        && spec.input_type == Some("password")
                        && let Some(val) = read_text_value(&target)
                    {
                        bridge.password_prev.insert(node_id, val);
                    }
                    spec.focusable && !spec.disabled && Some(node_id) != bridge.tree.root_id()
                })
                .unwrap_or(false);
                if should_focus {
                    push_events(
                        &queue,
                        &wakeup,
                        [
                            PendingDomEvent::Action(focus_action(node_id)),
                            PendingDomEvent::Action(scroll_into_view_action(node_id)),
                        ],
                    );
                }
            })
        {
            self.viewport_listeners.push(l);
        }

        // 2. Click delegation.
        let queue = Arc::clone(&self.queue);
        let wakeup = Rc::clone(&self.repaint_wakeup);
        if let Some(l) = add_listener(&container_target, "click", move |event| {
            let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
                return;
            };
            if target.has_attribute("data-egui-accesskit-activate") {
                event.prevent_default();
                event.stop_propagation();
                activate_bridge_from_dom(id);
                return;
            }
            let Some(node_id) = parse_node_id(&target) else {
                return;
            };
            let clickable_anchor = try_with_bridge_mut(id, |bridge| {
                let spec = bridge.tree.spec(node_id)?;
                (InteractionKind::from_spec(spec) == InteractionKind::Click)
                    .then_some(spec.tag == DomTag::Anchor)
            })
            .flatten();
            if let Some(is_anchor) = clickable_anchor {
                if is_anchor {
                    event.prevent_default();
                }
                event.stop_propagation();
                push_action(&queue, &wakeup, click_action(node_id));
            }
        }) {
            self.viewport_listeners.push(l);
        }

        // 3. BeforeInput delegation (password snapshot before edits).
        if let Some(l) =
            add_listener_with_options(&container_target, "beforeinput", false, true, move |event| {
                let Some((target, node_id)) = event_node_target(&event) else {
                    return;
                };
                try_with_bridge_mut(id, |bridge| {
                    if let Some(spec) = bridge.tree.spec(node_id)
                        && InteractionKind::from_spec(spec) == InteractionKind::Text
                        && !spec.read_only
                        && spec.input_type == Some("password")
                        && let Some(val) = read_text_value(&target)
                    {
                        bridge.password_prev.insert(node_id, val);
                    }
                });
            })
        {
            self.viewport_listeners.push(l);
        }

        // 4. CompositionStart delegation.
        if let Some(l) = add_listener_with_options(
            &container_target,
            "compositionstart",
            false,
            true,
            move |event| {
                let Some((_, node_id)) = event_node_target(&event) else {
                    return;
                };
                let hit = try_with_bridge_mut(id, |bridge| {
                    if let Some(spec) = bridge.tree.spec(node_id)
                        && InteractionKind::from_spec(spec) == InteractionKind::Text
                        && !spec.read_only
                    {
                        bridge.composing_nodes.borrow_mut().insert(node_id);
                        true
                    } else {
                        false
                    }
                })
                .unwrap_or(false);
                if hit {
                    event.stop_propagation();
                }
            },
        ) {
            self.viewport_listeners.push(l);
        }

        // 5. Input delegation (text + numeric sliders/spinbuttons).
        let queue = Arc::clone(&self.queue);
        let wakeup = Rc::clone(&self.repaint_wakeup);
        let dom_owned = Rc::clone(&self.dom_owned_selection);
        let composing = Rc::clone(&self.composing_nodes);
        if let Some(l) =
            add_listener_with_options(&container_target, "input", false, true, move |event| {
                let Some((target, node_id)) = event_node_target(&event) else {
                    return;
                };
                let is_composing = event
                    .dyn_ref::<InputEvent>()
                    .is_some_and(|ie| ie.is_composing());
                let action = try_with_bridge_mut(id, |bridge| {
                    let spec = bridge.tree.spec(node_id)?;
                    match InteractionKind::from_spec(spec) {
                        InteractionKind::Text if !spec.read_only => Some(Ok((
                            spec.input_type == Some("password"),
                            bridge.password_prev.get(&node_id).cloned(),
                        ))),
                        InteractionKind::Numeric { .. } => {
                            let val = target.dyn_ref::<HtmlInputElement>()?.value().parse::<f64>().ok()?;
                            Some(Err(val))
                        }
                        _ => None,
                    }
                })
                .flatten();

                match action {
                    Some(Ok((is_pw, prev_pw))) => {
                        event.stop_propagation();
                        if is_composing {
                            composing.borrow_mut().insert(node_id);
                            return;
                        }
                        composing.borrow_mut().remove(&node_id);
                        if let Some(new_val) = read_text_value(&target)
                            && let Some(pw) = commit_text_value(
                                &new_val,
                                node_id,
                                is_pw.then(|| prev_pw.as_deref().unwrap_or("")),
                                &dom_owned,
                                &queue,
                                &wakeup,
                            )
                        {
                            try_with_bridge_mut(id, |bridge| {
                                bridge.password_prev.insert(node_id, pw);
                            });
                        }
                    }
                    Some(Err(val)) => {
                        event.stop_propagation();
                        push_action(&queue, &wakeup, set_numeric_value_action(node_id, val));
                    }
                    None => {}
                }
            })
        {
            self.viewport_listeners.push(l);
        }

        // 6. CompositionEnd delegation.
        let queue = Arc::clone(&self.queue);
        let wakeup = Rc::clone(&self.repaint_wakeup);
        let dom_owned = Rc::clone(&self.dom_owned_selection);
        let composing = Rc::clone(&self.composing_nodes);
        if let Some(l) = add_listener_with_options(
            &container_target,
            "compositionend",
            false,
            true,
            move |event| {
                let Some((target, node_id)) = event_node_target(&event) else {
                    return;
                };
                let text_info = try_with_bridge_mut(id, |bridge| {
                    let spec = bridge.tree.spec(node_id)?;
                    if InteractionKind::from_spec(spec) == InteractionKind::Text && !spec.read_only
                    {
                        Some((
                            spec.input_type == Some("password"),
                            bridge.password_prev.get(&node_id).cloned(),
                        ))
                    } else {
                        None
                    }
                })
                .flatten();
                if let Some((is_pw, prev_pw)) = text_info {
                    event.stop_propagation();
                    composing.borrow_mut().remove(&node_id);
                    if let Some(new_val) = read_text_value(&target)
                        && let Some(pw) = commit_text_value(
                            &new_val,
                            node_id,
                            is_pw.then(|| prev_pw.as_deref().unwrap_or("")),
                            &dom_owned,
                            &queue,
                            &wakeup,
                        )
                    {
                        try_with_bridge_mut(id, |bridge| {
                            bridge.password_prev.insert(node_id, pw);
                        });
                    }
                }
            },
        ) {
            self.viewport_listeners.push(l);
        }

        // 7. Select delegation (capture catches non-bubbling synthetic events too).
        let syncing_selection = Rc::clone(&self.syncing_selection);
        let queue = Arc::clone(&self.queue);
        let wakeup = Rc::clone(&self.repaint_wakeup);
        if let Some(l) =
            add_listener_with_options(&container_target, "select", true, true, move |event| {
                if syncing_selection.get() {
                    return;
                }
                let Some((target, node_id)) = event_node_target(&event) else {
                    return;
                };
                let ok = try_with_bridge_mut(id, |bridge| {
                    bridge.tree.spec(node_id).is_some_and(|s| {
                        InteractionKind::from_spec(s) == InteractionKind::Text
                            && supports_dom_selection(s)
                    })
                })
                .unwrap_or(false);
                if ok {
                    emit_dom_selection(&target, node_id, &syncing_selection, &queue, &wakeup);
                }
            })
        {
            self.viewport_listeners.push(l);
        }

        // 8. Keyup delegation (cursor movement selection sync).
        let syncing_selection = Rc::clone(&self.syncing_selection);
        let queue = Arc::clone(&self.queue);
        let wakeup = Rc::clone(&self.repaint_wakeup);
        if let Some(l) =
            add_listener_with_options(&container_target, "keyup", false, true, move |event| {
                let Some(kb) = event.dyn_ref::<KeyboardEvent>() else {
                    return;
                };
                if kb.is_composing()
                    || !matches!(
                        kb.key().as_str(),
                        "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" | "Home" | "End"
                    )
                {
                    return;
                }
                let Some((target, node_id)) = event_node_target(&event) else {
                    return;
                };
                let ok = try_with_bridge_mut(id, |bridge| {
                    bridge.tree.spec(node_id).is_some_and(|s| {
                        InteractionKind::from_spec(s) == InteractionKind::Text
                            && supports_dom_selection(s)
                    })
                })
                .unwrap_or(false);
                if ok {
                    emit_dom_selection(&target, node_id, &syncing_selection, &queue, &wakeup);
                }
            })
        {
            self.viewport_listeners.push(l);
        }

        // 9. Keydown delegation (Escape/Enter forwarding, PageUp/PageDown scrolling, slider stepping, click keys).
        let queue = Arc::clone(&self.queue);
        let wakeup = Rc::clone(&self.repaint_wakeup);
        if let Some(l) = add_listener(&container_target, "keydown", move |event| {
            let Some(kb) = event.dyn_ref::<KeyboardEvent>() else {
                return;
            };
            let key = kb.key();
            if key == "Escape" && !kb.is_composing() {
                event.stop_propagation();
                push_events(
                    &queue,
                    &wakeup,
                    [PendingDomEvent::KeyPress(egui::Key::Escape)],
                );
                return;
            }
            if key == "Tab"
                && !kb.is_composing()
                && let Some(current) = event.target().and_then(|t| t.dyn_into::<Element>().ok())
            {
                let backwards = kb.shift_key();
                let next = try_with_bridge_mut(id, |bridge| {
                    bridge.modal_tab_target(&current, backwards)
                })
                .flatten();
                if let Some(next) = next {
                    event.prevent_default();
                    event.stop_propagation();
                    // Focus moves after the bridge borrow ends; the focus listener then reports
                    // the change to egui.
                    if let Some(html) = next.dyn_ref::<HtmlElement>() {
                        let options = FocusOptions::new();
                        options.set_prevent_scroll(true);
                        let _ = html.focus_with_options(&options);
                    }
                }
            }
            let Some((target, node_id)) = event_node_target(&event) else {
                return;
            };
            match key.as_str() {
                "PageDown" | "PageUp" if !target.tag_name().eq_ignore_ascii_case("TEXTAREA") => {
                    event.prevent_default();
                    event.stop_propagation();
                    let action = if key == "PageDown" {
                        Action::ScrollDown
                    } else {
                        Action::ScrollUp
                    };
                    push_action(&queue, &wakeup, scroll_action(node_id, action));
                    return;
                }
                _ => {}
            }
            let info = try_with_bridge_mut(id, |bridge| {
                bridge
                    .tree
                    .spec(node_id)
                    .map(|s| (InteractionKind::from_spec(s), s.tag))
            })
            .flatten();
            let Some((kind, tag)) = info else {
                return;
            };
            match kind {
                InteractionKind::Click => {
                    if !matches!(tag, DomTag::Button | DomTag::Anchor | DomTag::Input)
                        && matches!(key.as_str(), "Enter" | " ")
                    {
                        event.prevent_default();
                        event.stop_propagation();
                        push_action(&queue, &wakeup, click_action(node_id));
                    }
                }
                InteractionKind::Text => {
                    if key == "Enter"
                        && tag == DomTag::Input
                        && !kb.is_composing()
                        && !kb.shift_key()
                        && !kb.ctrl_key()
                        && !kb.alt_key()
                        && !kb.meta_key()
                    {
                        event.stop_propagation();
                        push_events(
                            &queue,
                            &wakeup,
                            [PendingDomEvent::KeyPress(egui::Key::Enter)],
                        );
                    } else if kb.is_composing() || key != "Tab" {
                        event.stop_propagation();
                    }
                }
                InteractionKind::Numeric { unstepped } => {
                    if unstepped {
                        match key.as_str() {
                            "ArrowRight" | "ArrowUp" => {
                                event.prevent_default();
                                event.stop_propagation();
                                push_action(&queue, &wakeup, increment_action(node_id));
                                return;
                            }
                            "ArrowLeft" | "ArrowDown" => {
                                event.prevent_default();
                                event.stop_propagation();
                                push_action(&queue, &wakeup, decrement_action(node_id));
                                return;
                            }
                            _ => {}
                        }
                    }
                    if key.starts_with("Arrow") || key == "Home" || key == "End" {
                        event.stop_propagation();
                    }
                }
                InteractionKind::None => {}
            }
        }) {
            self.viewport_listeners.push(l);
        }
    }

    pub(crate) fn has_dom_focus(&self) -> bool {
        active_element_for(self.container.as_ref(), &self.document)
            .is_some_and(|active| self.container.contains(Some(active.as_ref())))
    }

    pub(crate) fn apply_update(&mut self, update: egui::accesskit::TreeUpdate) {
        if !self.activated {
            return;
        }
        let prev_focused_id = active_element_for(self.container.as_ref(), &self.document)
            .filter(|active| self.container.contains(Some(active.as_ref())))
            .and_then(|el| parse_node_id(&el));
        let had_modal = !self.modal_nodes.is_empty();

        let (scale, scale_changed) = self.sync_viewport();
        let TreeDelta {
            upserts,
            children_ops,
            removals,
            focus_change,
        } = self.tree.apply_update_diff(update);

        for (old, spec) in upserts {
            self.upsert_node(old.as_ref(), spec, scale);
        }
        for (parent, children) in children_ops {
            self.set_children(parent, &children);
        }
        for id in removals {
            self.remove_node(id);
        }
        let saw_focus_op = focus_change.is_some();
        if let Some(target) = focus_change
            && (self.has_dom_focus() || self.dom_action_in_flight)
        {
            self.set_focus(target);
        }

        // Reparenting a focused element into a newly created ancestor <div> blurs it in
        // Blink without changing AccessKit's focused NodeId; restore focus & selection.
        if !saw_focus_op
            && let Some(focused_id) = prev_focused_id
            && self.tree.spec(focused_id).is_some_and(|s| !s.disabled)
            && let Some(el) = self.elements.get(&focused_id)
            && active_element_for(self.container.as_ref(), &self.document).as_ref() != Some(el)
        {
            let saved_sel = selection_fields(el);
            self.set_focus(Some(focused_id));
            if let Some((Some(start), Some(end), dir)) = saved_sel {
                let direction = dir.as_deref().unwrap_or("none");
                self.syncing_selection.set(true);
                if let Some(input) = el.dyn_ref::<HtmlInputElement>() {
                    let _ = input.set_selection_range_with_direction(start, end, direction);
                } else if let Some(ta) = el.dyn_ref::<HtmlTextAreaElement>() {
                    let _ = ta.set_selection_range_with_direction(start, end, direction);
                }
                self.syncing_selection.set(false);
            }
        }

        self.reconcile_text_inputs();
        self.sync_modal_inert();

        // `sync_modal_inert` marks the opener `inert`, which makes the browser drop DOM focus to
        // `document.body`. Keyboard users must therefore be moved into the dialog, and back out
        // to the opener when it closes (WAI-ARIA dialog pattern).
        let has_modal = !self.modal_nodes.is_empty();
        if !had_modal && has_modal {
            self.focus_modal_on_open(prev_focused_id);
        } else if had_modal && !has_modal {
            self.restore_focus_after_modal();
        }

        if scale_changed {
            self.reapply_all_bounds(scale);
        }
        self.dom_action_in_flight = false;
    }

    /// Moves focus into the top modal after it opens from a DOM-driven action (keyboard or a
    /// focused control). Pointer-opened modals leave focus on the `<canvas>`.
    fn focus_modal_on_open(&mut self, opener: Option<NodeId>) {
        if opener.is_none() && !self.dom_action_in_flight {
            return;
        }
        let Some(modal_el) = self.find_top_modal_element() else {
            return;
        };
        if active_element_for(self.container.as_ref(), &self.document)
            .is_some_and(|active| modal_el.contains(Some(active.as_ref())))
        {
            return;
        }
        let target = first_focusable_within(&modal_el).unwrap_or_else(|| {
            let _ = modal_el.set_attribute("tabindex", "-1");
            modal_el.clone()
        });
        self.modal_return_focus = opener;
        self.focus_element(&target);
        if let Some(id) = parse_node_id(&target) {
            push_action(&self.queue, &self.repaint_wakeup, focus_action(id));
        }
    }

    /// Returns focus to the control that opened a modal once the modal is gone, but only when
    /// focus fell back to `document.body`, so focus the user moved elsewhere is never stolen.
    fn restore_focus_after_modal(&mut self) {
        let Some(prev) = self.modal_return_focus.take() else {
            return;
        };
        let fell_to_body = active_element_for(self.container.as_ref(), &self.document)
            .is_none_or(|el| el.tag_name().eq_ignore_ascii_case("BODY"));
        if !fell_to_body {
            return;
        }
        let Some(el) = self.elements.get(&prev).cloned() else {
            return;
        };
        if !self
            .tree
            .spec(prev)
            .is_some_and(|s| s.focusable && !s.disabled)
        {
            return;
        }
        self.focus_element(&el);
        push_action(&self.queue, &self.repaint_wakeup, focus_action(prev));
    }

    pub(crate) fn realign_viewport(&mut self) {
        self.viewport_dirty = true;
        let (scale, scale_changed) = self.sync_viewport();
        if scale_changed {
            self.reapply_all_bounds(scale);
        }
    }

    pub(crate) fn destroy(&mut self) {
        self.activate_cb = None;
        if let Some(btn) = self.activate_button.take() {
            btn.remove();
        }
        *self.repaint_wakeup.borrow_mut() = None;
        self.resize_observer = None;
        self.viewport_listeners.clear();
        self.clear_modal_inert();
        self.modal_nodes.clear();
        self.modal_dirty = false;
        self.modal_return_focus = None;
        self.dom_owned_selection.borrow_mut().clear();
        self.composing_nodes.borrow_mut().clear();
        self.password_prev.clear();
        self.pending_text_reconcile.clear();
        self.hoisted_children.clear();
        self.elements.clear();
        self.tree = TreeBridge::new();
        self.container.remove();
    }

    fn has_pending_text_event(&self, id: NodeId) -> bool {
        self.queue
            .lock()
            .is_ok_and(|q| q.iter().any(|ev| ev.targets_text_state(id)))
    }

    fn reconcile_text_inputs(&mut self) {
        if self.pending_text_reconcile.is_empty() {
            return;
        }
        let ids: Vec<NodeId> = self.pending_text_reconcile.drain().collect();
        for id in ids {
            if self.composing_nodes.borrow().contains(&id) || self.has_pending_text_event(id) {
                self.pending_text_reconcile.insert(id);
                continue;
            }
            if let (Some(el), Some(spec)) = (self.elements.get(&id), self.tree.spec(id)) {
                sync_text_element_value(el, spec);
            }
        }
    }

    fn clear_modal_inert(&mut self) {
        for el in self.inert_elements.drain(..) {
            let _ = el.remove_attribute("inert");
        }
    }

    fn sync_modal_inert(&mut self) {
        if !self.modal_dirty {
            return;
        }
        self.modal_dirty = false;
        self.clear_modal_inert();
        let Some(modal_el) = self.find_top_modal_element() else {
            return;
        };

        let container_el = self.container.clone();
        let mut current = modal_el;
        while let Some(parent) = current.parent_element() {
            let mut sib = parent.first_element_child();
            while let Some(child) = sib {
                sib = child.next_element_sibling();
                if child != current && !child.has_attribute("inert") {
                    let _ = child.set_attribute("inert", "");
                    self.inert_elements.push(child);
                }
            }
            if parent == container_el {
                break;
            }
            current = parent;
        }
    }

    /// While a modal is open, keeps Tab / Shift+Tab focus inside it (focus trap).
    ///
    /// Returns the element that should take focus when `current` sits at the edge of the
    /// dialog. `None` leaves the browser's native Tab movement alone.
    fn modal_tab_target(&self, current: &Element, backwards: bool) -> Option<Element> {
        let modal = self.find_top_modal_element()?;
        let focusables: Vec<Element> = match modal.query_selector_all(FOCUSABLE_SELECTOR) {
            Ok(list) => (0..list.length())
                .filter_map(|i| list.item(i)?.dyn_into::<Element>().ok())
                .collect(),
            Err(_) => Vec::new(),
        };
        let (Some(first), Some(last)) = (focusables.first(), focusables.last()) else {
            // Nothing inside the dialog can take focus; keep it on the dialog itself.
            let _ = modal.set_attribute("tabindex", "-1");
            return Some(modal);
        };
        let index = focusables.iter().position(|el| el == current);
        if backwards {
            (index.is_none_or(|i| i == 0)).then(|| last.clone())
        } else {
            (index.is_none_or(|i| i + 1 == focusables.len())).then(|| first.clone())
        }
    }

    fn find_top_modal_element(&self) -> Option<Element> {
        if self.modal_nodes.is_empty() {
            return None;
        }
        if self.modal_nodes.len() == 1 {
            let modal_id = *self.modal_nodes.iter().next()?;
            return self
                .elements
                .get(&modal_id)
                .filter(|el| self.container.contains(Some(el.as_ref())))
                .cloned();
        }
        let mut stack = Vec::new();
        let mut child = self.container.first_element_child();
        while let Some(el) = child {
            child = el.next_element_sibling();
            stack.push(el);
        }
        while let Some(el) = stack.pop() {
            if parse_node_id(&el).is_some_and(|id| self.modal_nodes.contains(&id)) {
                return Some(el);
            }
            let mut c = el.first_element_child();
            while let Some(inner) = c {
                c = inner.next_element_sibling();
                stack.push(inner);
            }
        }
        None
    }

    fn reapply_all_bounds(&mut self, scale: (f64, f64)) {
        for (id, element) in &self.elements {
            if let Some(spec) = self.tree.spec(*id) {
                apply_bounds(element, None, spec.bounds, scale, &mut self.bounds_buf);
            }
        }
    }

    fn sync_viewport(&mut self) -> ((f64, f64), bool) {
        if !self.activated {
            return (self.last_scale.unwrap_or((1.0, 1.0)), false);
        }
        let raw_dpr = self.window.device_pixel_ratio();
        let dpr = if raw_dpr > 0.0 && raw_dpr.is_finite() {
            raw_dpr
        } else {
            1.0
        };
        let buf_size = self
            .canvas
            .as_ref()
            .and_then(|c| c.dyn_ref::<HtmlCanvasElement>())
            .map_or((0, 0), |c| (c.width(), c.height()));

        if !self.viewport_dirty
            && self.last_dpr == dpr
            && self.last_buf_size == buf_size
            && let Some(scale) = self.last_scale
        {
            return (scale, false);
        }
        self.viewport_dirty = false;
        self.last_dpr = dpr;
        self.last_buf_size = buf_size;

        let next_scale = if let Some(canvas) = &self.canvas {
            let rect = canvas.get_bounding_client_rect();
            let client_left = f64::from(canvas.client_left());
            let client_top = f64::from(canvas.client_top());
            let client_w = f64::from(canvas.client_width());
            let client_h = f64::from(canvas.client_height());
            let (offset_w, offset_h) = canvas
                .dyn_ref::<HtmlElement>()
                .map(|h| (f64::from(h.offset_width()), f64::from(h.offset_height())))
                .unwrap_or((0.0, 0.0));
            let ratio_x = if offset_w > 0.0 && rect.width() > 0.0 {
                rect.width() / offset_w
            } else {
                1.0
            };
            let ratio_y = if offset_h > 0.0 && rect.height() > 0.0 {
                rect.height() / offset_h
            } else {
                1.0
            };

            let origin_x = rect.left() + client_left * ratio_x;
            let origin_y = rect.top() + client_top * ratio_y;
            let content_w = if client_w > 0.0 {
                client_w * ratio_x
            } else {
                rect.width()
            };
            let content_h = if client_h > 0.0 {
                client_h * ratio_y
            } else {
                rect.height()
            };

            let prev = self.last_canvas_rect;
            let next_rect = (origin_x, origin_y, content_w, content_h);
            if prev != Some(next_rect) {
                self.last_canvas_rect = Some(next_rect);
                if let Some(html) = self.container.dyn_ref::<HtmlElement>() {
                    let style = html.style();
                    if prev.is_none_or(|p| p.0 != origin_x || p.1 != origin_y) {
                        let _ = set_px(&style, &mut self.bounds_buf, "left", origin_x);
                        let _ = set_px(&style, &mut self.bounds_buf, "top", origin_y);
                    }
                    if prev.is_none_or(|p| p.2 != content_w || p.3 != content_h) {
                        let _ = set_px(&style, &mut self.bounds_buf, "width", content_w);
                        let _ = set_px(&style, &mut self.bounds_buf, "height", content_h);
                    }
                }
            }

            let buf_w = f64::from(buf_size.0);
            let buf_h = f64::from(buf_size.1);
            let scale_x = if buf_w > 0.0 && content_w > 0.0 {
                content_w / buf_w
            } else {
                1.0 / dpr
            };
            let scale_y = if buf_h > 0.0 && content_h > 0.0 {
                content_h / buf_h
            } else {
                1.0 / dpr
            };
            (scale_x, scale_y)
        } else {
            (1.0 / dpr, 1.0 / dpr)
        };

        let scale_changed = self.last_scale != Some(next_scale);
        self.last_scale = Some(next_scale);
        (next_scale, scale_changed)
    }

    fn upsert_node(
        &mut self,
        old_spec: Option<&DomNodeSpec>,
        spec: DomNodeSpec,
        scale: (f64, f64),
    ) {
        let id = spec.id;
        let is_active_modal = spec.modal && !spec.disabled;
        if is_active_modal {
            if self.modal_nodes.insert(id) {
                self.modal_dirty = true;
            }
        } else if self.modal_nodes.remove(&id) {
            self.modal_dirty = true;
        }
        if spec.input_type != Some("password") {
            self.password_prev.remove(&id);
        }

        let desired_tag = element_tag_name(&spec);
        let tag_changed = old_spec.is_some_and(|old| element_tag_name(old) != desired_tag);

        let created = tag_changed || !self.elements.contains_key(&id);
        let element = if created {
            let Ok(new_el) = self.document.create_element(desired_tag) else {
                return;
            };
            let _ = new_el.set_attribute("id", &dom_element_id(self.bridge_id, id));
            let _ = new_el.set_attribute("data-accesskit-id", &id.0.to_string());
            if let Some(html) = new_el.dyn_ref::<HtmlElement>() {
                let style = html.style();
                let _ = style.set_property("margin", "0");
                let _ = style.set_property("padding", "0");
                let _ = style.set_property("border", "0");
                let _ = style.set_property("box-sizing", "border-box");
                let _ = style.set_property("pointer-events", "none");
            }
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

        let is_composing = self.composing_nodes.borrow().contains(&id);
        let skip_text_sync = is_composing || self.has_pending_text_event(id);
        let attr_old = if created { None } else { old_spec };
        apply_dom_attributes(
            &element,
            attr_old,
            &spec,
            skip_text_sync,
            self.hoisted_children.get(&id).map(Vec::as_slice),
            self.bridge_id,
        );
        if attr_old.is_none_or(|o| o.bounds != spec.bounds) {
            apply_bounds(
                &element,
                attr_old.and_then(|o| o.bounds),
                spec.bounds,
                scale,
                &mut self.bounds_buf,
            );
        }

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
    }

    fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        if !self.modal_nodes.is_empty() {
            self.modal_dirty = true;
        }
        let Some(parent_el) = self.elements.get(&parent).cloned() else {
            return;
        };
        if Some(parent) == self.tree.root_id() && parent_el.parent_node().is_none() {
            let _ = self.container.append_child(&parent_el);
        }
        if let Some(parent_spec) = self.tree.spec(parent).cloned()
            && is_leaf_tag(parent_spec.tag)
        {
            // Hoist popups/children attached to leaf elements (<button>, <input>, etc.)
            // out to the overlay root to avoid invalid nested interactive HTML.
            let valid_children: Vec<NodeId> = children
                .iter()
                .copied()
                .filter(|child_id| self.elements.contains_key(child_id))
                .collect();
            if let Some(prev_hoisted) = self.hoisted_children.get(&parent) {
                let container_node: &web_sys::Node = self.container.as_ref();
                for old_id in prev_hoisted {
                    if !valid_children.contains(old_id)
                        && let Some(old_el) = self.elements.get(old_id)
                        && old_el.parent_node().as_ref() == Some(container_node)
                    {
                        let _ = self.container.remove_child(old_el);
                    }
                }
            }
            let container_node: &web_sys::Node = self.container.as_ref();
            for child_id in &valid_children {
                if let Some(child_el) = self.elements.get(child_id)
                    && child_el.parent_node().as_ref() != Some(container_node)
                {
                    let _ = self.container.append_child(child_el);
                }
            }
            if valid_children.is_empty() {
                self.hoisted_children.remove(&parent);
            } else {
                self.hoisted_children.insert(parent, valid_children);
            }
            sync_disclosure_attrs(
                &parent_el,
                &parent_spec,
                self.hoisted_children.get(&parent).map(Vec::as_slice),
                self.bridge_id,
            );
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
        if self.modal_nodes.remove(&id) {
            self.modal_dirty = true;
        }
        self.password_prev.remove(&id);
        self.dom_owned_selection.borrow_mut().remove(&id);
        self.composing_nodes.borrow_mut().remove(&id);
        self.pending_text_reconcile.remove(&id);
        if self.deferred_focus_target == Some(id) {
            self.deferred_focus_target = None;
        }
        if self.modal_return_focus == Some(id) {
            self.modal_return_focus = None;
        }
        if let Some(old_hoisted) = self.hoisted_children.remove(&id) {
            let container_node: &web_sys::Node = self.container.as_ref();
            for child_id in old_hoisted {
                if let Some(child_el) = self.elements.get(&child_id)
                    && child_el.parent_node().as_ref() == Some(container_node)
                {
                    let _ = self.container.remove_child(child_el);
                }
            }
        }
        let mut emptied_parents = Vec::new();
        let mut touched_parents = Vec::new();
        for (&parent_id, list) in &mut self.hoisted_children {
            let before = list.len();
            list.retain(|&child_id| child_id != id);
            if list.len() != before {
                touched_parents.push(parent_id);
                if list.is_empty() {
                    emptied_parents.push(parent_id);
                }
            }
        }
        for parent_id in emptied_parents {
            self.hoisted_children.remove(&parent_id);
        }
        for parent_id in touched_parents {
            if let Some(parent_el) = self.elements.get(&parent_id)
                && let Some(parent_spec) = self.tree.spec(parent_id)
            {
                sync_disclosure_attrs(
                    parent_el,
                    parent_spec,
                    self.hoisted_children.get(&parent_id).map(Vec::as_slice),
                    self.bridge_id,
                );
            }
        }
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
        let Some(el) = self.elements.get(&target_id).cloned() else {
            return;
        };
        self.focus_element(&el);
    }

    /// Focuses `el` without echoing a redundant `Action::Focus` back to `egui`.
    fn focus_element(&self, el: &Element) {
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

/// Selector for keyboard-focusable mirrored controls, used for dialog focus entry and trapping.
const FOCUSABLE_SELECTOR: &str = "button:not([disabled]):not([aria-disabled='true']), \
     input:not([disabled]):not([type='hidden']):not([aria-disabled='true']), \
     select:not([disabled]), textarea:not([disabled]), a[href], \
     [tabindex]:not([tabindex='-1']):not([aria-disabled='true'])";

/// First keyboard-focusable descendant of a dialog, in document order.
fn first_focusable_within(root: &Element) -> Option<Element> {
    root.query_selector(FOCUSABLE_SELECTOR).ok().flatten()
}

fn commit_text_value(
    new_val: &str,
    id: NodeId,
    prev_pw: Option<&str>,
    owned_selection: &RefCell<HashSet<NodeId>>,
    queue: &ActionQueue,
    wakeup: &RepaintWakeup,
) -> Option<String> {
    if let Some(prev) = prev_pw
        && new_val.chars().any(is_password_mask_char)
    {
        let (anchor_utf16, focus_utf16, inserted) = password_diff_utf16(prev, new_val);
        push_text_event(
            queue,
            wakeup,
            id,
            PendingDomEvent::ReplaceRange {
                target_node: id,
                anchor_utf16,
                focus_utf16,
                text: inserted,
            },
            false,
        );
        return Some(new_val.to_owned());
    }
    owned_selection.borrow_mut().insert(id);
    push_text_event(
        queue,
        wakeup,
        id,
        PendingDomEvent::SetValue {
            target_node: id,
            value: new_val.to_owned(),
        },
        true,
    );
    prev_pw.map(|_| new_val.to_owned())
}

fn parse_node_id(element: &Element) -> Option<NodeId> {
    element
        .get_attribute("data-accesskit-id")
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(NodeId)
}

fn event_node_target(event: &Event) -> Option<(Element, NodeId)> {
    let target = event.target()?.dyn_into::<Element>().ok()?;
    let node_id = parse_node_id(&target)?;
    Some((target, node_id))
}

fn set_px(
    style: &web_sys::CssStyleDeclaration,
    buf: &mut String,
    prop: &str,
    val: f64,
) -> Result<(), wasm_bindgen::JsValue> {
    buf.clear();
    let _ = write!(buf, "{val}px");
    style.set_property(prop, buf)
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
            .any(|ev| ev.target_node() == Some(id) && ev.is_text_mutation())
        {
            false
        } else {
            guard.retain(|ev| {
                !(ev.target_node() == Some(id)
                    && matches!(ev, PendingDomEvent::TextSelection { .. }))
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

fn push_text_event(
    queue: &ActionQueue,
    wakeup: &RepaintWakeup,
    id: NodeId,
    event: PendingDomEvent,
    replace_all: bool,
) {
    if let Ok(mut guard) = queue.lock() {
        if replace_all {
            guard.retain(|ev| !ev.targets_text_state(id));
        } else {
            guard.retain(|ev| {
                !(ev.target_node() == Some(id)
                    && matches!(ev, PendingDomEvent::TextSelection { .. }))
            });
        }
        guard.push(event);
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

/// Single-byte glyph used for masked characters inside the DOM `<input type="password">`.
const DOM_PASSWORD_MASK: char = '*';

/// Characters egui uses to mask password text (`•` from `epaint`, plus `●` for compatibility).
fn is_egui_password_mask_char(c: char) -> bool {
    matches!(c, '\u{25CF}' | '\u{2022}')
}

/// True for characters that belong to the masked DOM representation of a password value.
fn is_password_mask_char(c: char) -> bool {
    c == DOM_PASSWORD_MASK || is_egui_password_mask_char(c)
}

/// Converts egui's masked password text into the DOM mask. Both representations are single
/// UTF-16 code units, so UTF-16 selection offsets are identical on either side.
fn dom_password_mask(val: &str) -> String {
    val.chars()
        .map(|c| {
            if is_egui_password_mask_char(c) {
                DOM_PASSWORD_MASK
            } else {
                c
            }
        })
        .collect()
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

fn dom_element_id(bridge_id: u64, node_id: NodeId) -> String {
    format!("egui-ak-{bridge_id}-{}", node_id.0)
}

fn is_leaf_tag(tag: DomTag) -> bool {
    matches!(
        tag,
        DomTag::Button | DomTag::Input | DomTag::TextArea | DomTag::Anchor
    )
}

fn effective_role_for(spec: &DomNodeSpec) -> Option<&'static str> {
    let base_role = match spec.aria_role {
        Some("meter" | "scrollbar") if spec.numeric_value.is_none() => None,
        other => other,
    };
    if base_role.is_none()
        && spec.tag == DomTag::Div
        && (spec.label.is_some() || spec.description.is_some())
    {
        Some("group")
    } else {
        base_role
    }
}

/// Accessible name written to `aria-label`: the explicit label, else a role-based fallback.
fn accessible_name(spec: &DomNodeSpec) -> Option<&str> {
    spec.label
        .as_deref()
        .or_else(|| match effective_role_for(spec) {
            Some("dialog") => Some("Dialog"),
            Some("alertdialog") => Some("Alert dialog"),
            Some("progressbar") => Some("Progress"),
            Some("listbox") => Some("Options"),
            Some("combobox") => spec
                .value
                .as_deref()
                .filter(|s| !s.is_empty())
                .or(Some("Select")),
            Some("spinbutton") => Some("Numeric input"),
            _ => None,
        })
}

fn sync_disclosure_attrs(
    element: &Element,
    spec: &DomNodeSpec,
    hoisted: Option<&[NodeId]>,
    bridge_id: u64,
) {
    let effective_role = effective_role_for(spec);
    let hoisted = hoisted.filter(|children| !children.is_empty());
    let expanded = if hoisted.is_some() {
        Some("true")
    } else if let Some(exp) = spec.expanded {
        Some(if exp { "true" } else { "false" })
    } else if effective_role == Some("combobox") {
        Some("false")
    } else {
        None
    };
    set_optional_attr(element, "aria-expanded", expanded);

    let controls = hoisted.map(|children| {
        children
            .iter()
            .map(|child_id| dom_element_id(bridge_id, *child_id))
            .collect::<Vec<_>>()
            .join(" ")
    });
    set_optional_attr(element, "aria-controls", controls.as_deref());
}

fn sync_text_element_value(element: &Element, spec: &DomNodeSpec) {
    if let Some(input_el) = element.dyn_ref::<HtmlInputElement>() {
        if !matches!(spec.input_type, Some("checkbox" | "radio" | "range"))
            && spec.aria_role != Some("spinbutton")
        {
            let val = spec.value.as_deref().unwrap_or("");
            if spec.input_type == Some("password") {
                // Chromium exposes a password field's value to assistive tech as one mask glyph
                // per UTF-8 byte, so egui's 3-byte bullets would be read as three glyphs each.
                let masked = dom_password_mask(val);
                let current = input_el.value();
                if current.chars().count() != masked.chars().count()
                    || (masked.is_empty() && !current.is_empty())
                {
                    input_el.set_value(&masked);
                }
            } else if input_el.value() != val {
                input_el.set_value(val);
            }
        }
    } else if let Some(textarea_el) = element.dyn_ref::<HtmlTextAreaElement>() {
        let val = spec.value.as_deref().unwrap_or("");
        if textarea_el.value() != val {
            textarea_el.set_value(val);
        }
    }
}

fn apply_dom_attributes(
    element: &Element,
    old: Option<&DomNodeSpec>,
    spec: &DomNodeSpec,
    skip_value_sync: bool,
    hoisted: Option<&[NodeId]>,
    bridge_id: u64,
) {
    let effective_role = effective_role_for(spec);
    let old_effective_role = old.and_then(effective_role_for);
    let role_changed = old.is_none_or(|_| old_effective_role != effective_role);
    let supports_range_aria = matches!(
        effective_role,
        Some("slider" | "spinbutton" | "progressbar" | "scrollbar" | "meter")
    ) || matches!(spec.input_type, Some("range" | "number"));

    if role_changed {
        set_optional_attr(element, "role", effective_role);
    }
    // Only rewrite `aria-label` when the accessible name really changes. Rewriting it on every
    // value change (e.g. each keystroke in a text field) makes Chromium report a name change,
    // which screen readers announce again, for example as "blank" after Backspace.
    if role_changed || old.is_none_or(|o| accessible_name(o) != accessible_name(spec)) {
        set_optional_attr(element, "aria-label", accessible_name(spec));
    }
    if old.is_none_or(|o| o.description != spec.description) {
        set_optional_attr(element, "aria-description", spec.description.as_deref());
    }
    if old.is_none_or(|o| o.placeholder != spec.placeholder) {
        set_optional_attr(element, "placeholder", spec.placeholder.as_deref());
    }
    if role_changed
        || old.is_none_or(|o| {
            o.input_type != spec.input_type
                || o.numeric_value != spec.numeric_value
                || o.value != spec.value
        })
    {
        set_optional_attr(
            element,
            "aria-valuetext",
            supports_range_aria
                .then(|| spec.numeric_value.and(spec.value.as_deref()))
                .flatten(),
        );
    }

    if old.is_none_or(|o| o.heading_level != spec.heading_level) {
        let heading_level = spec.heading_level.map(|n| n.to_string());
        set_optional_attr(element, "aria-level", heading_level.as_deref());
    }
    if old.is_none_or(|o| o.modal != spec.modal) {
        set_optional_attr(element, "aria-modal", spec.modal.then_some("true"));
    }
    if old.is_none_or(|o| o.live != spec.live) {
        let live = match spec.live {
            Some(Live::Polite) => Some("polite"),
            Some(Live::Assertive) => Some("assertive"),
            _ => None,
        };
        set_optional_attr(element, "aria-live", live);
    }
    if old.is_none_or(|o| o.tag != spec.tag) {
        set_optional_attr(
            element,
            "aria-multiline",
            (spec.tag == DomTag::TextArea).then_some("true"),
        );
    }
    if old.is_none_or(|o| o.required != spec.required) {
        set_optional_attr(element, "aria-required", spec.required.then_some("true"));
    }
    if old.is_none_or(|o| o.invalid != spec.invalid) {
        let invalid = spec.invalid.map(|inv| match inv {
            Invalid::True => "true",
            Invalid::Grammar => "grammar",
            Invalid::Spelling => "spelling",
        });
        set_optional_attr(element, "aria-invalid", invalid);
    }
    if old.is_none_or(|o| o.auto_complete != spec.auto_complete) {
        // Only emitted when the application explicitly sets an AccessKit autocomplete mode.
        let autocomplete = matches!(spec.tag, DomTag::Input | DomTag::TextArea)
            .then_some(spec.auto_complete)
            .flatten();
        set_optional_attr(element, "aria-autocomplete", autocomplete);
    }
    if old.is_none_or(|o| o.error_message != spec.error_message) {
        let err_id = spec.error_message.map(|id| dom_element_id(bridge_id, id));
        set_optional_attr(element, "aria-errormessage", err_id.as_deref());
    }
    if old.is_none_or(|o| o.described_by != spec.described_by) {
        if spec.described_by.is_empty() {
            let _ = element.remove_attribute("aria-describedby");
        } else {
            let mut ids = String::new();
            for (i, id) in spec.described_by.iter().enumerate() {
                if i > 0 {
                    ids.push(' ');
                }
                let _ = write!(ids, "{}", dom_element_id(bridge_id, *id));
            }
            let _ = element.set_attribute("aria-describedby", &ids);
        }
    }
    if old.is_none_or(|o| o.active_descendant != spec.active_descendant) {
        let active_id = spec
            .active_descendant
            .map(|id| dom_element_id(bridge_id, id));
        set_optional_attr(element, "aria-activedescendant", active_id.as_deref());
    }
    if old.is_none_or(|o| o.orientation != spec.orientation) {
        let orientation = spec.orientation.map(|o| match o {
            Orientation::Horizontal => "horizontal",
            Orientation::Vertical => "vertical",
        });
        set_optional_attr(element, "aria-orientation", orientation);
    }
    if old.is_none_or(|o| o.position_in_set != spec.position_in_set) {
        let pos = spec
            .position_in_set
            .and_then(|p| p.checked_add(1))
            .map(|p| p.to_string());
        set_optional_attr(element, "aria-posinset", pos.as_deref());
    }
    if old.is_none_or(|o| o.size_of_set != spec.size_of_set) {
        let size = spec.size_of_set.map(|s| s.to_string());
        set_optional_attr(element, "aria-setsize", size.as_deref());
    }
    if old.is_none_or(|o| o.row_count != spec.row_count) {
        let count = spec.row_count.map(|c| c.to_string());
        set_optional_attr(element, "aria-rowcount", count.as_deref());
    }
    if old.is_none_or(|o| o.column_count != spec.column_count) {
        let count = spec.column_count.map(|c| c.to_string());
        set_optional_attr(element, "aria-colcount", count.as_deref());
    }
    if old.is_none_or(|o| o.row_index != spec.row_index) {
        let idx = spec
            .row_index
            .and_then(|r| r.checked_add(1))
            .map(|r| r.to_string());
        set_optional_attr(element, "aria-rowindex", idx.as_deref());
    }
    if old.is_none_or(|o| o.column_index != spec.column_index) {
        let idx = spec
            .column_index
            .and_then(|c| c.checked_add(1))
            .map(|c| c.to_string());
        set_optional_attr(element, "aria-colindex", idx.as_deref());
    }

    if old.is_none_or(|o| o.input_type != spec.input_type || o.tag != spec.tag) {
        if let Some(input_type) = spec.input_type {
            let _ = element.set_attribute("type", input_type);
        } else if spec.tag == DomTag::Button {
            let _ = element.set_attribute("type", "button");
        } else {
            let _ = element.remove_attribute("type");
        }
    }

    if spec.tag == DomTag::Anchor
        && old.is_none_or(|o| o.tag != spec.tag || o.disabled != spec.disabled || o.url != spec.url)
    {
        if spec.disabled {
            let _ = element.remove_attribute("href");
        } else {
            let _ = element.set_attribute("href", safe_href(spec.url.as_deref()));
        }
    }

    if old.is_none_or(|o| o.disabled != spec.disabled || o.tag != spec.tag) {
        if spec.disabled {
            if matches!(spec.tag, DomTag::Button | DomTag::Input | DomTag::TextArea) {
                let _ = element.set_attribute("disabled", "");
            }
            let _ = element.set_attribute("aria-disabled", "true");
        } else {
            let _ = element.remove_attribute("disabled");
            let _ = element.remove_attribute("aria-disabled");
        }
    }

    if old.is_none_or(|o| o.read_only != spec.read_only) {
        if spec.read_only {
            let _ = element.set_attribute("readonly", "");
            let _ = element.set_attribute("aria-readonly", "true");
        } else {
            let _ = element.remove_attribute("readonly");
            let _ = element.remove_attribute("aria-readonly");
        }
    }

    if hoisted.is_some()
        || role_changed
        || old.is_none_or(|o| o.expanded != spec.expanded)
    {
        sync_disclosure_attrs(element, spec, hoisted, bridge_id);
    }

    if role_changed
        || old.is_none_or(|o| {
            o.toggled != spec.toggled || o.tag != spec.tag || o.input_type != spec.input_type
        })
    {
        let toggled_str = spec.toggled.map(|t| match t {
            Toggled::True => "true",
            Toggled::False => "false",
            Toggled::Mixed => "mixed",
        });
        if spec.tag == DomTag::Button && matches!(effective_role, None | Some("button")) {
            let _ = element.remove_attribute("aria-checked");
            let _ = element.remove_attribute("aria-selected");
            set_optional_attr(element, "aria-pressed", toggled_str);
        } else if matches!(
            effective_role,
            Some("option" | "tab" | "treeitem" | "gridcell" | "row")
        ) {
            let _ = element.remove_attribute("aria-pressed");
            let _ = element.remove_attribute("aria-checked");
            let selected = spec.toggled.map(|t| match t {
                Toggled::True => "true",
                Toggled::False | Toggled::Mixed => "false",
            });
            set_optional_attr(element, "aria-selected", selected);
        } else if matches!(
            effective_role,
            Some("checkbox" | "radio" | "switch" | "menuitemcheckbox" | "menuitemradio")
        ) || matches!(spec.input_type, Some("checkbox" | "radio"))
        {
            let _ = element.remove_attribute("aria-pressed");
            let _ = element.remove_attribute("aria-selected");
            set_optional_attr(element, "aria-checked", toggled_str);
        } else {
            let _ = element.remove_attribute("aria-pressed");
            let _ = element.remove_attribute("aria-checked");
            let _ = element.remove_attribute("aria-selected");
        }
    }

    if old.is_none_or(|o| {
        o.focusable != spec.focusable || o.disabled != spec.disabled || o.tag != spec.tag
    }) {
        if spec.focusable && !spec.disabled && !natively_focusable(spec.tag) {
            let _ = element.set_attribute("tabindex", "0");
        } else {
            let _ = element.remove_attribute("tabindex");
        }
    }

    if role_changed
        || old.is_none_or(|o| {
            o.input_type != spec.input_type
                || o.numeric_min != spec.numeric_min
                || o.numeric_max != spec.numeric_max
                || o.numeric_value != spec.numeric_value
        })
    {
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
    }

    if let Some(input_el) = element.dyn_ref::<HtmlInputElement>() {
        if old.is_none_or(|o| o.disabled != spec.disabled) {
            input_el.set_disabled(spec.disabled);
        }
        if old.is_none_or(|o| o.read_only != spec.read_only) {
            input_el.set_read_only(spec.read_only);
        }
        if old.is_none_or(|o| o.toggled != spec.toggled) {
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
        }
        if spec.input_type == Some("range") || spec.aria_role == Some("spinbutton") {
            if old.is_none_or(|o| o.numeric_step != spec.numeric_step) {
                let step_str = spec
                    .numeric_step
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| String::from("any"));
                input_el.set_step(&step_str);
            }
            if old.is_none_or(|o| o.numeric_min != spec.numeric_min) {
                set_optional_attr(
                    element,
                    "min",
                    spec.numeric_min.map(|v| v.to_string()).as_deref(),
                );
            }
            if old.is_none_or(|o| o.numeric_max != spec.numeric_max) {
                set_optional_attr(
                    element,
                    "max",
                    spec.numeric_max.map(|v| v.to_string()).as_deref(),
                );
            }
            if let Some(val) = spec.numeric_value {
                let s = val.to_string();
                if input_el.value() != s {
                    input_el.set_value(&s);
                }
            }
        } else if !skip_value_sync {
            sync_text_element_value(element, spec);
        }
    } else if let Some(textarea_el) = element.dyn_ref::<HtmlTextAreaElement>() {
        if old.is_none_or(|o| o.disabled != spec.disabled) {
            textarea_el.set_disabled(spec.disabled);
        }
        if old.is_none_or(|o| o.read_only != spec.read_only) {
            textarea_el.set_read_only(spec.read_only);
        }
        if !skip_value_sync {
            sync_text_element_value(element, spec);
        }
    } else if matches!(
        spec.tag,
        DomTag::Span | DomTag::Heading | DomTag::Anchor | DomTag::ListItem
    ) || (spec.tag == DomTag::Button && effective_role == Some("combobox"))
        || (spec.tag == DomTag::Div
            && matches!(
                effective_role,
                Some(
                    "option"
                        | "listitem"
                        | "treeitem"
                        | "cell"
                        | "gridcell"
                        | "columnheader"
                        | "rowheader"
                        | "tooltip"
                )
            )
            && element.first_element_child().is_none())
    {
        let text = spec.value.as_deref().or(spec.label.as_deref()).unwrap_or("");
        if element.text_content().as_deref() != Some(text) {
            element.set_text_content(Some(text));
        }
    }
}

fn apply_bounds(
    element: &Element,
    old_bounds: Option<Rect>,
    bounds: Option<Rect>,
    scale: (f64, f64),
    buf: &mut String,
) {
    let Some(html) = element.dyn_ref::<HtmlElement>() else {
        return;
    };
    let style = html.style();
    if let Some(bounds) = bounds {
        if old_bounds.is_none() {
            let _ = style.set_property("position", "absolute");
        }
        if old_bounds.is_none_or(|o| o.x0 != bounds.x0 || o.y0 != bounds.y0) {
            let _ = set_px(&style, buf, "left", bounds.x0 * scale.0);
            let _ = set_px(&style, buf, "top", bounds.y0 * scale.1);
        }
        let w = bounds.x1 - bounds.x0;
        let h = bounds.y1 - bounds.y0;
        if old_bounds.is_none_or(|o| (o.x1 - o.x0) != w || (o.y1 - o.y0) != h) {
            let _ = set_px(&style, buf, "width", w.max(0.0) * scale.0);
            let _ = set_px(&style, buf, "height", h.max(0.0) * scale.1);
        }
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

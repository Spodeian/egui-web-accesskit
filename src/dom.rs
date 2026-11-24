use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{
    Document, Element, Event, FocusOptions, HtmlCanvasElement, HtmlElement, HtmlInputElement,
    KeyboardEvent, ShadowRoot, Window,
};

use crate::egui;
use crate::egui::accesskit::{NodeId, Rect, Toggled};
use crate::mapper::{
    DomNodeSpec, DomOp, DomTag, TreeBridge, click_action, decrement_action, focus_action,
    increment_action, set_numeric_value_action, set_value_action,
};

pub(crate) type ActionQueue = Arc<Mutex<Vec<egui::Event>>>;

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
    closure: Closure<dyn FnMut(Event)>,
}

impl Drop for AttachedListener {
    fn drop(&mut self) {
        let _ = self.target.remove_event_listener_with_callback(
            self.event_type,
            self.closure.as_ref().unchecked_ref(),
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
                _ => Self::Text,
            },
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
    queue: ActionQueue,
    repaint_wakeup: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    syncing_focus: Rc<Cell<bool>>,
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
        let _ = container.set_attribute("data-egui-accesskit", "true");
        let _ = container.set_attribute("role", "application");
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
            queue,
            repaint_wakeup: Rc::new(RefCell::new(Some(wakeup))),
            syncing_focus: Rc::new(Cell::new(false)),
            last_canvas_rect: None,
            last_dpr: None,
        };
        let _ = bridge.sync_viewport();
        Some(bridge)
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
            for (id, element) in &self.elements {
                if let Some(spec) = self.specs.get(id) {
                    apply_bounds(element, spec.bounds, dpr, canvas_origin);
                }
            }
        }
    }

    pub(crate) fn destroy(&mut self) {
        *self.repaint_wakeup.borrow_mut() = None;
        self.listeners.clear();
        self.elements.clear();
        self.specs.clear();
        self.container.remove();
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
        let tag_changed = old_spec.is_some_and(|old| old.tag != spec.tag);
        let needs_rebind = old_spec.is_none_or(|old| {
            old.tag != spec.tag
                || InteractionKind::from_spec(old) != InteractionKind::from_spec(&spec)
                || old.focusable != spec.focusable
                || old.disabled != spec.disabled
        });

        let element = if tag_changed || !self.elements.contains_key(&id) {
            let Ok(new_el) = self.document.create_element(spec.tag.as_str()) else {
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

        apply_dom_attributes(&element, &spec, dpr, canvas_origin);

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
                    push_event(&queue, &wakeup, click_action(id));
                }) {
                    out.push(l);
                }
            }
            InteractionKind::Text => {
                if let Some(input_el) = element.dyn_ref::<HtmlInputElement>().cloned() {
                    let queue = Arc::clone(&self.queue);
                    let wakeup = Rc::clone(&self.repaint_wakeup);
                    if let Some(l) = add_listener(&target, "input", move |event| {
                        event.stop_propagation();
                        push_event(&queue, &wakeup, set_value_action(id, input_el.value()));
                    }) {
                        out.push(l);
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
            }
            InteractionKind::Numeric { unstepped } => {
                if let Some(input_el) = element.dyn_ref::<HtmlInputElement>().cloned() {
                    let queue = Arc::clone(&self.queue);
                    let wakeup = Rc::clone(&self.repaint_wakeup);
                    if let Some(l) = add_listener(&target, "input", move |event| {
                        event.stop_propagation();
                        if let Ok(val) = input_el.value().parse::<f64>() {
                            push_event(&queue, &wakeup, set_numeric_value_action(id, val));
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
                                    push_event(&queue, &wakeup, increment_action(id));
                                    return;
                                }
                                "ArrowLeft" | "ArrowDown" => {
                                    event.prevent_default();
                                    event.stop_propagation();
                                    push_event(&queue, &wakeup, decrement_action(id));
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
                    push_event(&queue, &wakeup, focus_action(id));
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
        if self.specs.get(&parent).is_some_and(|s| s.tag == DomTag::Input) {
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
            if !expected.iter().any(|wanted| *wanted == &node) {
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

fn apply_dom_attributes(
    element: &Element,
    spec: &DomNodeSpec,
    dpr: f64,
    canvas_origin: (f64, f64),
) {
    set_optional_attr(element, "role", spec.aria_role);
    set_optional_attr(element, "aria-label", spec.label.as_deref());

    if let Some(input_type) = spec.input_type {
        let _ = element.set_attribute("type", input_type);
    } else {
        let _ = element.remove_attribute("type");
    }

    if spec.tag == DomTag::Anchor {
        if spec.disabled {
            let _ = element.remove_attribute("href");
        } else {
            let _ = element.set_attribute("href", "#");
        }
    }

    if spec.disabled {
        let _ = element.set_attribute("disabled", "");
        let _ = element.set_attribute("aria-disabled", "true");
    } else {
        let _ = element.remove_attribute("disabled");
        let _ = element.remove_attribute("aria-disabled");
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
        spec.numeric_min.map(|v| v.to_string()).as_deref(),
    );
    set_optional_attr(
        element,
        "aria-valuemax",
        spec.numeric_max.map(|v| v.to_string()).as_deref(),
    );
    set_optional_attr(
        element,
        "aria-valuenow",
        spec.numeric_value.map(|v| v.to_string()).as_deref(),
    );

    if let Some(input_el) = element.dyn_ref::<HtmlInputElement>() {
        input_el.set_disabled(spec.disabled);
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
        if spec.input_type == Some("range") {
            let step_str = spec
                .numeric_step
                .map(|s| s.to_string())
                .unwrap_or_else(|| String::from("any"));
            input_el.set_step(&step_str);
            if let Some(min) = spec.numeric_min {
                input_el.set_min(&min.to_string());
            }
            if let Some(max) = spec.numeric_max {
                input_el.set_max(&max.to_string());
            }
            if let Some(val) = spec.numeric_value {
                let s = val.to_string();
                if input_el.value() != s {
                    input_el.set_value(&s);
                }
            }
        } else if !matches!(spec.input_type, Some("checkbox" | "radio")) {
            let val = spec.value.as_deref().unwrap_or("");
            if input_el.value() != val {
                input_el.set_value(val);
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
    matches!(tag, DomTag::Button | DomTag::Input | DomTag::Anchor)
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
        closure,
    })
}

fn push_event(
    queue: &ActionQueue,
    wakeup: &Rc<RefCell<Option<Box<dyn Fn()>>>>,
    request: egui::accesskit::ActionRequest,
) {
    if let Ok(mut guard) = queue.lock() {
        guard.push(egui::Event::AccessKitActionRequest(request));
    }
    if let Some(cb) = wakeup.borrow().as_ref() {
        cb();
    }
}

use crate::accesskit_consumer::{FilterResult, Tree, TreeChangeHandler};
use crate::egui::accesskit::{
    Action, ActionData, ActionRequest, NodeId, Rect, Role, Toggled, TreeId, TreeUpdate,
};
use crate::ConsumerNode;

/// Semantic HTML element tag chosen for an AccessKit node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomTag {
    Div,
    Button,
    Input,
    Span,
    Anchor,
    Heading,
    List,
    ListItem,
    Region,
}

impl DomTag {
    /// HTML tag name passed to `Document::create_element`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Div => "div",
            Self::Button => "button",
            Self::Input => "input",
            Self::Span => "span",
            Self::Anchor => "a",
            Self::Heading => "h2",
            Self::List => "ul",
            Self::ListItem => "li",
            Self::Region => "section",
        }
    }
}

/// Platform-independent snapshot of the DOM properties for one AccessKit node.
#[derive(Clone, Debug, PartialEq)]
pub struct DomNodeSpec {
    pub id: NodeId,
    pub tag: DomTag,
    pub aria_role: Option<&'static str>,
    pub input_type: Option<&'static str>,
    pub label: Option<String>,
    pub value: Option<String>,
    pub toggled: Option<Toggled>,
    pub expanded: Option<bool>,
    pub clickable: bool,
    pub disabled: bool,
    pub focusable: bool,
    pub numeric_value: Option<f64>,
    pub numeric_min: Option<f64>,
    pub numeric_max: Option<f64>,
    pub numeric_step: Option<f64>,
    pub bounds: Option<Rect>,
}

/// Incremental DOM mutation produced by [`TreeBridge::apply_update`].
#[derive(Clone, Debug, PartialEq)]
pub enum DomOp {
    Upsert(DomNodeSpec),
    SetChildren { parent: NodeId, children: Vec<NodeId> },
    Remove(NodeId),
    SetFocus(Option<NodeId>),
}

/// Maps an AccessKit consumer node into a [`DomNodeSpec`].
pub fn map_node(node: &ConsumerNode<'_>) -> DomNodeSpec {
    let (tag, aria_role, input_type) = map_role(node.role());

    let label = node.label().filter(|s| !s.is_empty());
    // `value()` skips text-range extraction on multiline text inputs, so
    // fall back to the raw value string when present.
    let value = node
        .value()
        .or_else(|| node.raw_value().map(str::to_owned));

    DomNodeSpec {
        id: node.locate().0,
        tag,
        aria_role,
        input_type,
        label,
        value,
        toggled: node.toggled(),
        expanded: node.data().is_expanded(),
        clickable: node.is_clickable(&|_| FilterResult::Include),
        disabled: node.is_disabled(),
        focusable: node.is_focusable(&|_| FilterResult::Include),
        numeric_value: node.numeric_value(),
        numeric_min: node.min_numeric_value(),
        numeric_max: node.max_numeric_value(),
        numeric_step: node.numeric_value_step(),
        bounds: (!node.is_hidden())
            .then(|| node.bounding_box())
            .flatten(),
    }
}

fn map_role(role: Role) -> (DomTag, Option<&'static str>, Option<&'static str>) {
    match role {
        Role::Button | Role::DefaultButton | Role::DisclosureTriangle => {
            (DomTag::Button, None, None)
        }
        Role::Tab => (DomTag::Button, Some("tab"), None),
        Role::MenuItem => (DomTag::Button, Some("menuitem"), None),
        Role::CheckBox => (DomTag::Input, Some("checkbox"), Some("checkbox")),
        Role::Switch => (DomTag::Input, Some("switch"), Some("checkbox")),
        Role::RadioButton => (DomTag::Input, Some("radio"), Some("radio")),
        Role::Slider => (DomTag::Input, Some("slider"), Some("range")),
        Role::TextInput => (DomTag::Input, None, Some("text")),
        Role::SearchInput => (DomTag::Input, None, Some("search")),
        Role::EmailInput => (DomTag::Input, None, Some("email")),
        Role::UrlInput => (DomTag::Input, None, Some("url")),
        Role::NumberInput => (DomTag::Input, None, Some("number")),
        Role::PasswordInput => (DomTag::Input, None, Some("password")),
        // shortcut: MultilineTextInput uses <input type="text"> instead of <textarea>; upgrade when multiline caret/newline syncing is added.
        Role::MultilineTextInput => (DomTag::Input, Some("textbox"), Some("text")),
        Role::Label | Role::TextRun => (DomTag::Span, None, None),
        Role::Link => (DomTag::Anchor, None, None),
        Role::Heading => (DomTag::Heading, Some("heading"), None),
        Role::List => (DomTag::List, None, None),
        Role::ListBox => (DomTag::List, Some("listbox"), None),
        Role::ListItem => (DomTag::ListItem, None, None),
        Role::ListBoxOption => (DomTag::ListItem, Some("option"), None),
        Role::Window | Role::Region | Role::ScrollView | Role::Pane => {
            (DomTag::Region, Some("region"), None)
        }
        Role::Dialog => (DomTag::Div, Some("dialog"), None),
        Role::AlertDialog => (DomTag::Div, Some("alertdialog"), None),
        Role::Alert => (DomTag::Div, Some("alert"), None),
        Role::ComboBox | Role::EditableComboBox => (DomTag::Div, Some("combobox"), None),
        Role::ProgressIndicator => (DomTag::Div, Some("progressbar"), None),
        Role::RadioGroup => (DomTag::Div, Some("radiogroup"), None),
        Role::Group => (DomTag::Div, Some("group"), None),
        _ => (DomTag::Div, None, None),
    }
}

fn should_skip_node(node: &ConsumerNode<'_>) -> bool {
    // egui emits inline TextRun children inside Labels and TextEdits whose text is
    // already carried on the parent node; mirroring them breaks <input> void tags
    // and duplicates label text for screen readers.
    node.role() == Role::TextRun
}

fn collect_dom_children(node: &ConsumerNode<'_>) -> Vec<NodeId> {
    node.children()
        .filter(|child| !should_skip_node(child))
        .map(|child| child.locate().0)
        .collect()
}

/// Retains an `accesskit_consumer::Tree` across frames and translates updates into [`DomOp`]s.
#[derive(Default)]
pub struct TreeBridge {
    tree: Option<Tree>,
}

impl TreeBridge {
    pub fn new() -> Self {
        Self { tree: None }
    }

    /// Root node ID of the retained tree, if initialized.
    pub fn root_id(&self) -> Option<NodeId> {
        self.tree.as_ref().map(|tree| tree.state().root().locate().0)
    }

    /// Applies an AccessKit [`TreeUpdate`] and returns ordered DOM mutations.
    pub fn apply_update(&mut self, update: TreeUpdate) -> Vec<DomOp> {
        if let Some(tree) = self.tree.as_mut() {
            let mut collector = DeltaCollector::default();
            tree.update_and_process_changes(update, &mut collector);
            collector.into_ops()
        } else {
            let tree = Tree::new(update, true);
            let ops = collect_initial_ops(&tree);
            self.tree = Some(tree);
            ops
        }
    }
}

fn collect_initial_ops(tree: &Tree) -> Vec<DomOp> {
    let state = tree.state();
    let mut upserts = Vec::new();
    let mut children_ops = Vec::new();
    let mut stack = vec![state.root()];

    while let Some(node) = stack.pop() {
        if should_skip_node(&node) {
            continue;
        }
        upserts.push(DomOp::Upsert(map_node(&node)));
        let child_ids = collect_dom_children(&node);
        if !child_ids.is_empty() {
            children_ops.push(DomOp::SetChildren {
                parent: node.locate().0,
                children: child_ids,
            });
        }
        for child in node.children().rev() {
            stack.push(child);
        }
    }

    let mut ops = Vec::with_capacity(upserts.len() + children_ops.len() + 1);
    ops.extend(upserts);
    ops.extend(children_ops);
    if let Some(focus) = state.focus().filter(|n| !should_skip_node(n)) {
        ops.push(DomOp::SetFocus(Some(focus.locate().0)));
    }
    ops
}

#[derive(Default)]
struct DeltaCollector {
    upserts: Vec<DomNodeSpec>,
    children_ops: Vec<DomOp>,
    removals: Vec<NodeId>,
    focus_change: Option<Option<NodeId>>,
}

impl DeltaCollector {
    fn into_ops(self) -> Vec<DomOp> {
        let focus_len = usize::from(self.focus_change.is_some());
        let mut ops = Vec::with_capacity(
            self.upserts.len() + self.children_ops.len() + self.removals.len() + focus_len,
        );
        // Upsert new/changed nodes before wiring parent-child links, detach before
        // removing nodes, and apply focus only after the target is attached.
        ops.extend(self.upserts.into_iter().map(DomOp::Upsert));
        ops.extend(self.children_ops);
        ops.extend(self.removals.into_iter().map(DomOp::Remove));
        if let Some(focus) = self.focus_change {
            ops.push(DomOp::SetFocus(focus));
        }
        ops
    }
}

impl TreeChangeHandler for DeltaCollector {
    fn node_added(&mut self, node: &ConsumerNode<'_>) {
        if should_skip_node(node) {
            return;
        }
        self.upserts.push(map_node(node));
        let children = collect_dom_children(node);
        if !children.is_empty() {
            self.children_ops.push(DomOp::SetChildren {
                parent: node.locate().0,
                children,
            });
        }
    }

    fn node_updated(&mut self, old_node: &ConsumerNode<'_>, new_node: &ConsumerNode<'_>) {
        if should_skip_node(new_node) {
            return;
        }
        let old_spec = map_node(old_node);
        let new_spec = map_node(new_node);
        if old_spec != new_spec {
            self.upserts.push(new_spec);
        }
        let old_children = collect_dom_children(old_node);
        let new_children = collect_dom_children(new_node);
        if old_children != new_children {
            self.children_ops.push(DomOp::SetChildren {
                parent: new_node.locate().0,
                children: new_children,
            });
        }
    }

    fn focus_moved(
        &mut self,
        _old_node: Option<&ConsumerNode<'_>>,
        new_node: Option<&ConsumerNode<'_>>,
    ) {
        self.focus_change = Some(
            new_node
                .filter(|node| !should_skip_node(node))
                .map(|node| node.locate().0),
        );
    }

    fn node_removed(&mut self, node: &ConsumerNode<'_>) {
        if should_skip_node(node) {
            return;
        }
        self.removals.push(node.locate().0);
    }
}

/// Builds an AccessKit [`Action::Click`] request for the root tree.
pub fn click_action(target_node: NodeId) -> ActionRequest {
    ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node,
        data: None,
    }
}

/// Builds an AccessKit [`Action::Focus`] request for the root tree.
pub fn focus_action(target_node: NodeId) -> ActionRequest {
    ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node,
        data: None,
    }
}

/// Builds an AccessKit [`Action::SetValue`] string request for the root tree.
pub fn set_value_action(target_node: NodeId, value: String) -> ActionRequest {
    ActionRequest {
        action: Action::SetValue,
        target_tree: TreeId::ROOT,
        target_node,
        data: Some(ActionData::Value(value.into_boxed_str())),
    }
}

/// Builds an AccessKit [`Action::SetValue`] numeric request for the root tree.
pub fn set_numeric_value_action(target_node: NodeId, value: f64) -> ActionRequest {
    ActionRequest {
        action: Action::SetValue,
        target_tree: TreeId::ROOT,
        target_node,
        data: Some(ActionData::NumericValue(value)),
    }
}

/// Builds an AccessKit [`Action::Increment`] request for the root tree.
pub fn increment_action(target_node: NodeId) -> ActionRequest {
    ActionRequest {
        action: Action::Increment,
        target_tree: TreeId::ROOT,
        target_node,
        data: None,
    }
}

/// Builds an AccessKit [`Action::Decrement`] request for the root tree.
pub fn decrement_action(target_node: NodeId) -> ActionRequest {
    ActionRequest {
        action: Action::Decrement,
        target_tree: TreeId::ROOT,
        target_node,
        data: None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::egui;

    fn find_upsert<'a>(ops: &'a [DomOp], pred: impl Fn(&DomNodeSpec) -> bool) -> &'a DomNodeSpec {
        ops.iter()
            .find_map(|op| match op {
                DomOp::Upsert(spec) if pred(spec) => Some(spec),
                _ => None,
            })
            .expect("expected matching DomOp::Upsert")
    }

    #[test]
    fn maps_egui_widgets_to_dom_specs_and_excludes_text_runs() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        let mut checked = true;
        let mut tri_checked = false;
        let mut opacity = 0.35_f64;
        let mut volume = 45.0_f64;
        let mut text = String::from("hello");

        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = ui.button("Save");
            ui.checkbox(&mut checked, "Agree");
            ui.add(egui::Checkbox::new(&mut tri_checked, "Mixed").indeterminate(true));
            ui.add(egui::Slider::new(&mut opacity, 0.0..=1.0).text("Opacity"));
            ui.add(
                egui::Slider::new(&mut volume, 0.0..=100.0)
                    .step_by(5.0)
                    .text("Volume"),
            );
            ui.label("Status text");
            ui.text_edit_singleline(&mut text);
            egui::CollapsingHeader::new("Advanced details").show(ui, |ui| {
                ui.label("Hidden");
            });
        });

        let update = output
            .platform_output
            .accesskit_update
            .take()
            .expect("accesskit_update must be emitted when enabled");
        output.drop_without_applying_deltas();

        let mut bridge = TreeBridge::new();
        let ops = bridge.apply_update(update);

        let upserted_ids: HashSet<NodeId> = ops
            .iter()
            .filter_map(|op| match op {
                DomOp::Upsert(spec) => Some(spec.id),
                _ => None,
            })
            .collect();
        for op in &ops {
            if let DomOp::SetChildren { children, .. } = op {
                for child in children {
                    assert!(
                        upserted_ids.contains(child),
                        "SetChildren contains skipped node {child:?}"
                    );
                }
            }
        }

        let button = find_upsert(&ops, |s| {
            s.tag == DomTag::Button && s.label.as_deref() == Some("Save")
        });
        assert!(button.clickable);
        assert!(button.focusable);
        assert!(!button.disabled);
        assert!(button.bounds.is_some());

        let checkbox = find_upsert(&ops, |s| {
            s.aria_role == Some("checkbox") && s.label.as_deref() == Some("Agree")
        });
        assert_eq!(checkbox.tag, DomTag::Input);
        assert_eq!(checkbox.input_type, Some("checkbox"));
        assert_eq!(checkbox.toggled, Some(Toggled::True));

        let mixed = find_upsert(&ops, |s| {
            s.aria_role == Some("checkbox") && s.label.as_deref() == Some("Mixed")
        });
        assert_eq!(mixed.toggled, Some(Toggled::Mixed));

        let opacity_slider = find_upsert(&ops, |s| {
            s.aria_role == Some("slider") && s.label.as_deref() == Some("Opacity")
        });
        assert_eq!(opacity_slider.numeric_value, Some(0.35));
        assert_eq!(opacity_slider.numeric_min, Some(0.0));
        assert_eq!(opacity_slider.numeric_max, Some(1.0));
        assert_eq!(opacity_slider.numeric_step, None);

        let volume_slider = find_upsert(&ops, |s| {
            s.aria_role == Some("slider") && s.label.as_deref() == Some("Volume")
        });
        assert_eq!(volume_slider.numeric_value, Some(45.0));
        assert_eq!(volume_slider.numeric_step, Some(5.0));

        let label = find_upsert(&ops, |s| {
            s.tag == DomTag::Span && s.value.as_deref() == Some("Status text")
        });
        assert_eq!(label.aria_role, None);

        let text_input = find_upsert(&ops, |s| {
            s.tag == DomTag::Input && s.input_type == Some("text")
        });
        assert_eq!(text_input.value.as_deref(), Some("hello"));

        let disclosure = find_upsert(&ops, |s| {
            s.tag == DomTag::Button && s.label.as_deref() == Some("Advanced details")
        });
        assert!(disclosure.clickable);
        #[cfg(feature = "egui-037")]
        assert_eq!(disclosure.expanded, Some(false));
        #[cfg(feature = "egui-036")]
        assert_eq!(disclosure.expanded, None);
    }

    #[test]
    fn emits_incremental_upsert_and_remove() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        let mut bridge = TreeBridge::new();
        let mut checked = false;
        let mut show_extra = true;

        let mut out1 = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.checkbox(&mut checked, "Subscribe");
            if show_extra {
                let _ = ui.button("Temporary");
            }
        });
        let ops1 = bridge.apply_update(out1.platform_output.accesskit_update.take().unwrap());
        out1.drop_without_applying_deltas();
        let temp_id = find_upsert(&ops1, |s| s.label.as_deref() == Some("Temporary")).id;

        checked = true;
        show_extra = false;

        let mut out2 = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.checkbox(&mut checked, "Subscribe");
            if show_extra {
                let _ = ui.button("Temporary");
            }
        });
        let ops2 = bridge.apply_update(out2.platform_output.accesskit_update.take().unwrap());
        out2.drop_without_applying_deltas();

        let updated_checkbox = find_upsert(&ops2, |s| {
            s.aria_role == Some("checkbox") && s.label.as_deref() == Some("Subscribe")
        });
        assert_eq!(updated_checkbox.toggled, Some(Toggled::True));

        assert!(
            ops2.iter().any(|op| *op == DomOp::Remove(temp_id)),
            "expected Remove for hidden button in {ops2:?}"
        );
    }

    #[test]
    fn action_requests_round_trip_into_egui() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        let mut bridge = TreeBridge::new();
        let mut volume = 50.0_f64;
        let mut note = String::from("abc");
        let mut out1 = ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = ui.button("Trigger");
            ui.add(
                egui::Slider::new(&mut volume, 0.0..=100.0)
                    .step_by(5.0)
                    .text("Volume"),
            );
            ui.text_edit_singleline(&mut note);
            egui::CollapsingHeader::new("Details").show(ui, |ui| {
                ui.label("Inside");
            });
        });
        let ops1 = bridge.apply_update(out1.platform_output.accesskit_update.take().unwrap());
        out1.drop_without_applying_deltas();

        let button_id = find_upsert(&ops1, |s| s.label.as_deref() == Some("Trigger")).id;
        let slider_id = find_upsert(&ops1, |s| s.label.as_deref() == Some("Volume")).id;
        let text_id = find_upsert(&ops1, |s| {
            s.tag == DomTag::Input && s.input_type == Some("text")
        })
        .id;
        let disclosure_id = find_upsert(&ops1, |s| s.label.as_deref() == Some("Details")).id;

        let mut clicked = false;
        let mut text_focused = false;
        let input2 = egui::RawInput {
            events: vec![
                egui::Event::AccessKitActionRequest(click_action(button_id)),
                egui::Event::AccessKitActionRequest(click_action(disclosure_id)),
                egui::Event::AccessKitActionRequest(set_numeric_value_action(slider_id, 60.0)),
                egui::Event::AccessKitActionRequest(focus_action(text_id)),
                egui::Event::AccessKitActionRequest(set_value_action(
                    text_id,
                    String::from("updated"),
                )),
            ],
            ..Default::default()
        };
        let mut out2 = ctx.run_ui(input2, |ui| {
            clicked = ui.button("Trigger").clicked();
            ui.add(
                egui::Slider::new(&mut volume, 0.0..=100.0)
                    .step_by(5.0)
                    .text("Volume"),
            );
            text_focused = ui.text_edit_singleline(&mut note).has_focus();
            egui::CollapsingHeader::new("Details").show(ui, |ui| {
                ui.label("Inside");
            });
        });
        let ops2 = bridge.apply_update(out2.platform_output.accesskit_update.take().unwrap());
        out2.drop_without_applying_deltas();

        assert!(clicked, "AccessKit click action must trigger egui button");
        assert!(text_focused, "AccessKit focus action must focus TextEdit");
        assert_eq!(volume, 60.0);

        #[cfg(feature = "egui-037")]
        {
            assert_eq!(note, "updated");
            let disclosure_after =
                find_upsert(&ops2, |s| s.label.as_deref() == Some("Details"));
            assert_eq!(disclosure_after.expanded, Some(true));
        }
        #[cfg(feature = "egui-036")]
        let _ = (&note, &ops2);

        let input3 = egui::RawInput {
            events: vec![egui::Event::AccessKitActionRequest(increment_action(
                slider_id,
            ))],
            ..Default::default()
        };
        let out3 = ctx.run_ui(input3, |ui| {
            let _ = ui.button("Trigger");
            ui.add(
                egui::Slider::new(&mut volume, 0.0..=100.0)
                    .step_by(5.0)
                    .text("Volume"),
            );
        });
        out3.drop_without_applying_deltas();
        assert_eq!(volume, 65.0);
    }
}

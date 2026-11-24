//! Platform-independent translation from `accesskit_consumer` trees into minimal DOM mutations.
//!
//! Most applications only need [`crate::WebAccessKitPlugin`]; this module is exposed for
//! headless testing and custom DOM hosts.

use accesskit_consumer::{FilterResult, Node, Tree, TreeChangeHandler};
use egui::accesskit::{
    Action, ActionData, ActionRequest, Live, NodeId, Rect, Role, TextSelection, Toggled, TreeId,
    TreeUpdate,
};

/// Semantic HTML element tag chosen for an AccessKit node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomTag {
    /// Generic container (`<div>`).
    Div,
    /// Action or disclosure button (`<button>`).
    Button,
    /// Single-line form input, checkbox, radio button, or range slider (`<input>`).
    Input,
    /// Multiline text input (`<textarea>`).
    TextArea,
    /// Inline static text (`<span>`).
    Span,
    /// Hyperlink (`<a>`).
    Anchor,
    /// Section heading (`<h1>`–`<h6>` or `<div role="heading">` above level 6).
    Heading,
    /// List container (`<ul>`).
    List,
    /// List entry or option (`<li>`).
    ListItem,
    /// Landmark or grouping section (`<section>`).
    Region,
}

impl DomTag {
    /// HTML tag name passed to `Document::create_element`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Div => "div",
            Self::Button => "button",
            Self::Input => "input",
            Self::TextArea => "textarea",
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
    /// AccessKit node identifier local to `TreeId::ROOT`.
    pub id: NodeId,
    /// HTML tag chosen for this node.
    pub tag: DomTag,
    /// Explicit WAI-ARIA `role` attribute override, if needed.
    pub aria_role: Option<&'static str>,
    /// HTML `<input type="...">` attribute when [`Self::tag`] is [`DomTag::Input`].
    pub input_type: Option<&'static str>,
    /// Accessible name (`aria-label` or fallback text content).
    pub label: Option<String>,
    /// Accessible description mapped to `aria-description`.
    pub description: Option<String>,
    /// Textual value (`value`, `textContent`, or `aria-valuetext` on sliders).
    pub value: Option<String>,
    /// Input placeholder text mapped to `placeholder`.
    pub placeholder: Option<String>,
    /// Hyperlink target URL mapped to safe `<a href="...">` values.
    pub url: Option<String>,
    /// Toggle or checkbox state (`aria-pressed` or `aria-checked`).
    pub toggled: Option<Toggled>,
    /// Disclosure expansion state (`aria-expanded`).
    pub expanded: Option<bool>,
    /// Whether the node accepts click actions.
    pub clickable: bool,
    /// Whether the control is disabled (`disabled` / `aria-disabled`).
    pub disabled: bool,
    /// Whether the control is explicitly read-only (`readonly` / `aria-readonly`).
    pub read_only: bool,
    /// Whether the node can receive keyboard focus.
    pub focusable: bool,
    /// Heading level (`1..`) mirrored to `aria-level` and native `<h1>`–`<h6>` tags.
    pub heading_level: Option<usize>,
    /// Whether dialogs block underlying content (`aria-modal="true"`).
    pub modal: bool,
    /// Live region politeness level (`aria-live`).
    pub live: Option<Live>,
    /// Current numeric value for range controls (`aria-valuenow`).
    pub numeric_value: Option<f64>,
    /// Minimum numeric value for range controls (`aria-valuemin` / `min`).
    pub numeric_min: Option<f64>,
    /// Maximum numeric value for range controls (`aria-valuemax` / `max`).
    pub numeric_max: Option<f64>,
    /// Step increment for range controls (`step`, or `"any"` when `None`).
    pub numeric_step: Option<f64>,
    /// Text selection `(anchor, focus)` measured in UTF-16 code units.
    pub text_selection: Option<(usize, usize)>,
    /// Node bounding box in physical `egui` pixels (`bounds / devicePixelRatio` in CSS px).
    pub bounds: Option<Rect>,
}

/// Incremental DOM mutation produced by [`TreeBridge::apply_update`].
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum DomOp {
    /// Insert or update an element matching `DomNodeSpec`.
    Upsert(DomNodeSpec),
    /// Reorder or attach the child list under `parent`.
    SetChildren {
        /// Parent node whose child elements should match `children`.
        parent: NodeId,
        /// Ordered child node IDs (excluding skipped inline `TextRun`s).
        children: Vec<NodeId>,
    },
    /// Remove a previously mirrored element.
    Remove(NodeId),
    /// Move DOM focus to the target node, if present.
    SetFocus(Option<NodeId>),
}

/// Maps an AccessKit consumer node into a [`DomNodeSpec`].
pub fn map_node(node: &Node<'_>) -> DomNodeSpec {
    let (mut tag, mut aria_role, mut input_type) = map_role(node.role());
    match node.role() {
        Role::Label if node.url().is_some_and(|u| !u.is_empty()) => {
            tag = DomTag::Anchor;
            aria_role = None;
        }
        Role::Button if node.toggled().is_some() && has_ancestor_role(node, Role::ListBox) => {
            tag = DomTag::Div;
            aria_role = Some("option");
            input_type = None;
        }
        Role::ListBox
            if !has_descendant(node, &|c| {
                c.role() == Role::ListBoxOption
                    || (c.role() == Role::Button && c.toggled().is_some())
            }) =>
        {
            aria_role = Some("group");
        }
        Role::ListBoxOption if !has_ancestor_role(node, Role::ListBox) => {
            tag = DomTag::Button;
            aria_role = None;
        }
        Role::Tab if !has_ancestor_role(node, Role::TabList) => {
            aria_role = None;
        }
        Role::TabList if !has_descendant(node, &|c| c.role() == Role::Tab) => {
            aria_role = Some("group");
        }
        Role::MenuItem | Role::MenuItemCheckBox | Role::MenuItemRadio
            if !(has_ancestor_role(node, Role::Menu) || has_ancestor_role(node, Role::MenuBar)) =>
        {
            match node.role() {
                Role::MenuItem => {
                    tag = DomTag::Button;
                    aria_role = None;
                }
                Role::MenuItemCheckBox => {
                    tag = DomTag::Input;
                    aria_role = Some("checkbox");
                    input_type = Some("checkbox");
                }
                Role::MenuItemRadio => {
                    tag = DomTag::Input;
                    aria_role = Some("radio");
                    input_type = Some("radio");
                }
                _ => {}
            }
        }
        _ => {}
    }

    let label = node.label().filter(|s| !s.is_empty());
    let description = node.description().filter(|s| !s.is_empty());
    // `value()` skips text-range extraction on multiline text inputs, so
    // fall back to the raw value string when present.
    let value = node
        .value()
        .or_else(|| node.raw_value().map(str::to_owned));
    let placeholder = node
        .data()
        .placeholder()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let url = node.url().filter(|s| !s.is_empty()).map(str::to_owned);

    let text_selection = if node.is_text_input() && node.supports_text_ranges() {
        node.text_selection_anchor()
            .zip(node.text_selection_focus())
            .map(|(anchor, focus)| {
                (
                    anchor.to_global_utf16_index(),
                    focus.to_global_utf16_index(),
                )
            })
    } else {
        None
    };

    DomNodeSpec {
        id: node.locate().0,
        tag,
        aria_role,
        input_type,
        label,
        description,
        value,
        placeholder,
        url,
        toggled: node.toggled(),
        expanded: node.data().is_expanded(),
        clickable: node.is_clickable(&|_| FilterResult::Include),
        disabled: node.is_disabled(),
        read_only: node.data().is_read_only(),
        focusable: node.is_focusable(&|_| FilterResult::Include),
        heading_level: node.level(),
        modal: node.is_modal(),
        live: node.data().live().filter(|l| *l != Live::Off),
        numeric_value: node.numeric_value(),
        numeric_min: node.min_numeric_value(),
        numeric_max: node.max_numeric_value(),
        numeric_step: node.numeric_value_step(),
        text_selection,
        bounds: (!node.is_hidden())
            .then(|| node.bounding_box())
            .flatten(),
    }
}

fn has_ancestor_role(node: &Node<'_>, target: Role) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.role() == target {
            return true;
        }
        current = parent.parent();
    }
    false
}

fn has_descendant(node: &Node<'_>, pred: &impl Fn(&Node<'_>) -> bool) -> bool {
    let mut stack: Vec<_> = node.children().collect();
    while let Some(child) = stack.pop() {
        if should_skip_node(&child) {
            continue;
        }
        if pred(&child) {
            return true;
        }
        stack.extend(child.children());
    }
    false
}

fn map_role(role: Role) -> (DomTag, Option<&'static str>, Option<&'static str>) {
    match role {
        Role::Button | Role::DefaultButton | Role::DisclosureTriangle => {
            (DomTag::Button, None, None)
        }
        Role::Tab => (DomTag::Button, Some("tab"), None),
        Role::TabList => (DomTag::Div, Some("tablist"), None),
        Role::MenuItem => (DomTag::Button, Some("menuitem"), None),
        Role::MenuItemCheckBox => (DomTag::Button, Some("menuitemcheckbox"), None),
        Role::MenuItemRadio => (DomTag::Button, Some("menuitemradio"), None),
        Role::CheckBox => (DomTag::Input, Some("checkbox"), Some("checkbox")),
        Role::Switch => (DomTag::Input, Some("switch"), Some("checkbox")),
        Role::RadioButton => (DomTag::Input, Some("radio"), Some("radio")),
        Role::Slider => (DomTag::Input, Some("slider"), Some("range")),
        Role::SpinButton => (DomTag::Input, Some("spinbutton"), Some("number")),
        Role::TextInput => (DomTag::Input, None, Some("text")),
        Role::SearchInput => (DomTag::Input, None, Some("search")),
        Role::EmailInput => (DomTag::Input, None, Some("email")),
        Role::UrlInput => (DomTag::Input, None, Some("url")),
        Role::NumberInput => (DomTag::Input, None, Some("number")),
        Role::PasswordInput => (DomTag::Input, None, Some("password")),
        Role::MultilineTextInput => (DomTag::TextArea, None, None),
        Role::Label | Role::TextRun => (DomTag::Span, None, None),
        Role::Link => (DomTag::Anchor, None, None),
        Role::Heading => (DomTag::Heading, Some("heading"), None),
        Role::List => (DomTag::List, None, None),
        Role::ListBox => (DomTag::Div, Some("listbox"), None),
        Role::ListItem => (DomTag::ListItem, None, None),
        Role::ListBoxOption => (DomTag::Div, Some("option"), None),
        Role::Window | Role::Region | Role::ScrollView | Role::Pane => (DomTag::Region, None, None),
        Role::Dialog => (DomTag::Div, Some("dialog"), None),
        Role::AlertDialog => (DomTag::Div, Some("alertdialog"), None),
        Role::Alert => (DomTag::Div, Some("alert"), None),
        Role::Status => (DomTag::Div, Some("status"), None),
        Role::ComboBox => (DomTag::Button, Some("combobox"), None),
        Role::EditableComboBox => (DomTag::Input, Some("combobox"), Some("text")),
        Role::ProgressIndicator => (DomTag::Div, Some("progressbar"), None),
        Role::Meter => (DomTag::Div, Some("meter"), None),
        Role::RadioGroup => (DomTag::Div, Some("radiogroup"), None),
        Role::Group => (DomTag::Div, Some("group"), None),
        _ => (DomTag::Div, None, None),
    }
}

fn should_skip_node(node: &Node<'_>) -> bool {
    // egui emits inline TextRun children inside Labels and TextEdits whose text is
    // already carried on the parent node; mirroring them breaks void/form controls
    // and duplicates label text for screen readers.
    node.role() == Role::TextRun
}

fn collect_dom_children(node: &Node<'_>) -> Vec<NodeId> {
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
    /// Creates an empty [`TreeBridge`] awaiting the first AccessKit [`TreeUpdate`].
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

    /// Whether `target_node` advertises support for programmatic text editing actions.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub(crate) fn supports_text_edit_actions(&self, target_node: NodeId) -> bool {
        self.tree
            .as_ref()
            .and_then(|t| t.state().node_by_tree_local_id(target_node, TreeId::ROOT))
            .is_some_and(|n| {
                n.data().supports_action(Action::SetValue)
                    || n.data().supports_action(Action::ReplaceSelectedText)
            })
    }

    /// Resolves UTF-16 selection offsets for `target_node` into an AccessKit `SetTextSelection` request.
    pub fn set_text_selection_action(
        &self,
        target_node: NodeId,
        anchor_utf16: usize,
        focus_utf16: usize,
    ) -> Option<ActionRequest> {
        let node = self
            .tree
            .as_ref()?
            .state()
            .node_by_tree_local_id(target_node, TreeId::ROOT)?;
        if !node.is_text_input() || !node.supports_text_ranges() {
            return None;
        }
        let anchor = node
            .text_position_from_global_utf16_index(anchor_utf16)?
            .to_raw();
        let focus = node
            .text_position_from_global_utf16_index(focus_utf16)?
            .to_raw();
        Some(ActionRequest {
            action: Action::SetTextSelection,
            target_tree: TreeId::ROOT,
            target_node,
            data: Some(ActionData::SetTextSelection(TextSelection { anchor, focus })),
        })
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
    fn node_added(&mut self, node: &Node<'_>) {
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

    fn node_updated(&mut self, old_node: &Node<'_>, new_node: &Node<'_>) {
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

    fn focus_moved(&mut self, _old_node: Option<&Node<'_>>, new_node: Option<&Node<'_>>) {
        self.focus_change = Some(
            new_node
                .filter(|node| !should_skip_node(node))
                .map(|node| node.locate().0),
        );
    }

    fn node_removed(&mut self, node: &Node<'_>) {
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

/// Builds an AccessKit [`Action::ReplaceSelectedText`] string request for the root tree.
pub fn replace_selected_text_action(target_node: NodeId, value: String) -> ActionRequest {
    ActionRequest {
        action: Action::ReplaceSelectedText,
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

/// Builds an AccessKit scroll request (such as `ScrollUp`, `ScrollDown`, or `ScrollIntoView`).
pub fn scroll_action(target_node: NodeId, action: Action) -> ActionRequest {
    ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node,
        data: None,
    }
}

/// Builds an AccessKit [`Action::ScrollIntoView`] request for the root tree.
pub fn scroll_into_view_action(target_node: NodeId) -> ActionRequest {
    scroll_action(target_node, Action::ScrollIntoView)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use egui::accesskit::{Node as AccessKitNode, Tree as AccessKitTreeInfo};

    fn find_upsert(ops: &[DomOp], pred: impl Fn(&DomNodeSpec) -> bool) -> &DomNodeSpec {
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
        let mut bio = String::from("line1\nline2");
        let mut ro_token: &str = "RO-123";
        let mut secret = String::from("s3cr3t");

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
            ui.add(egui::TextEdit::multiline(&mut bio).hint_text("Write bio"));
            ui.text_edit_singleline(&mut ro_token);
            ui.add(egui::TextEdit::singleline(&mut secret).password(true));
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
        assert!(!button.read_only);
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

        let volume_spin = find_upsert(&ops, |s| {
            s.aria_role == Some("spinbutton") && s.label.as_deref() == Some("Volume")
        });
        assert_eq!(volume_spin.tag, DomTag::Input);
        assert_eq!(volume_spin.input_type, Some("number"));
        assert_eq!(volume_spin.numeric_value, Some(45.0));

        let label = find_upsert(&ops, |s| {
            s.tag == DomTag::Span && s.value.as_deref() == Some("Status text")
        });
        assert_eq!(label.aria_role, None);
        assert!(!label.read_only);

        let text_input = find_upsert(&ops, |s| {
            s.tag == DomTag::Input
                && s.input_type == Some("text")
                && s.value.as_deref() == Some("hello")
        });
        assert!(!text_input.read_only);

        let bio_area = find_upsert(&ops, |s| s.tag == DomTag::TextArea);
        assert_eq!(bio_area.value.as_deref(), Some("line1\nline2"));
        assert_eq!(bio_area.placeholder.as_deref(), Some("Write bio"));
        assert!(!bio_area.read_only);

        let _ro_input = find_upsert(&ops, |s| {
            s.tag == DomTag::Input && s.value.as_deref() == Some("RO-123")
        });

        let pw_input = find_upsert(&ops, |s| {
            s.tag == DomTag::Input && s.input_type == Some("password")
        });
        let masked = pw_input
            .value
            .as_deref()
            .expect("password input must expose masked value");
        assert_eq!(masked.chars().count(), 6);
        assert!(!masked.chars().any(|c| c == 's' || c == '3'));

        let disclosure = find_upsert(&ops, |s| {
            s.tag == DomTag::Button && s.label.as_deref() == Some("Advanced details")
        });
        assert!(disclosure.clickable);
        assert_eq!(disclosure.expanded, None);
    }

    #[test]
    fn maps_showcase_widgets_and_listbox_context_roles() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        let mut density = 0_u8;
        let mut muted = true;
        let mut retries = 3_i32;
        let mut choice = 0_usize;

        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let _ = ui.add(egui::Button::new("Mute audio").selected(muted));
            let _ = ui.radio_value(&mut density, 0, "Compact");
            let _ = ui.radio_value(&mut density, 1, "Spacious");
            let _ = ui.add_enabled(false, egui::Button::new("Locked action"));
            let _ = ui.add(egui::DragValue::new(&mut retries).range(0..=10).speed(1.0));
            let _ = ui.add(egui::ProgressBar::new(0.75).text("Sync progress"));
            egui::ComboBox::from_label("Theme").show_index(ui, &mut choice, 2, |i| {
                ["Dark", "Light"][i]
            });
            let _ = &mut muted;
        });

        let update = output
            .platform_output
            .accesskit_update
            .take()
            .expect("accesskit_update must be emitted");
        output.drop_without_applying_deltas();

        let mut bridge = TreeBridge::new();
        let ops = bridge.apply_update(update);

        let toggle_btn = find_upsert(&ops, |s| {
            s.tag == DomTag::Button && s.label.as_deref() == Some("Mute audio")
        });
        assert_eq!(toggle_btn.toggled, Some(Toggled::True));

        let radio = find_upsert(&ops, |s| {
            s.aria_role == Some("radio") && s.label.as_deref() == Some("Compact")
        });
        assert_eq!(radio.tag, DomTag::Input);
        assert_eq!(radio.input_type, Some("radio"));
        assert_eq!(radio.toggled, Some(Toggled::True));

        let disabled_btn = find_upsert(&ops, |s| {
            s.tag == DomTag::Button && s.label.as_deref() == Some("Locked action")
        });
        assert!(disabled_btn.disabled);

        let spin = find_upsert(&ops, |s| s.aria_role == Some("spinbutton"));
        assert_eq!(spin.tag, DomTag::Input);
        assert_eq!(spin.input_type, Some("number"));
        assert_eq!(spin.numeric_value, Some(3.0));

        let progress = find_upsert(&ops, |s| {
            s.aria_role == Some("progressbar") && s.label.as_deref() == Some("Sync progress")
        });
        assert_eq!(progress.tag, DomTag::Div);
        assert_eq!(progress.numeric_value, Some(75.0));

        let combo = find_upsert(&ops, |s| {
            s.aria_role == Some("combobox") && s.label.as_deref() == Some("Theme")
        });
        assert_eq!(combo.tag, DomTag::Button);
        assert_eq!(combo.value.as_deref(), Some("Dark"));

        // Verify ListBox ancestor turns selectable Button into option, and empty ListBox into group.
        let root_id = NodeId(10);
        let filled_lb_id = NodeId(11);
        let opt_btn_id = NodeId(12);
        let empty_lb_id = NodeId(13);

        let mut root = AccessKitNode::new(Role::Window);
        root.set_children(vec![filled_lb_id, empty_lb_id]);

        let mut filled_lb = AccessKitNode::new(Role::ListBox);
        filled_lb.set_children(vec![opt_btn_id]);

        let mut opt_btn = AccessKitNode::new(Role::Button);
        opt_btn.set_label("Light");
        opt_btn.set_toggled(Toggled::False);

        let empty_lb = AccessKitNode::new(Role::ListBox);

        let synth_update = TreeUpdate {
            nodes: vec![
                (root_id, root),
                (filled_lb_id, filled_lb),
                (opt_btn_id, opt_btn),
                (empty_lb_id, empty_lb),
            ],
            tree: Some(AccessKitTreeInfo::new(root_id)),
            tree_id: TreeId::ROOT,
            focus: root_id,
        };
        let mut synth_bridge = TreeBridge::new();
        let synth_ops = synth_bridge.apply_update(synth_update);

        let lb = find_upsert(&synth_ops, |s| s.id == filled_lb_id);
        assert_eq!(lb.aria_role, Some("listbox"));

        let opt = find_upsert(&synth_ops, |s| s.id == opt_btn_id);
        assert_eq!(opt.tag, DomTag::Div);
        assert_eq!(opt.aria_role, Some("option"));
        assert_eq!(opt.toggled, Some(Toggled::False));

        let empty_lb_spec = find_upsert(&synth_ops, |s| s.id == empty_lb_id);
        assert_eq!(empty_lb_spec.aria_role, Some("group"));
    }

    #[test]
    fn maps_heading_level_modal_and_live_attributes() {
        let root_id = NodeId(1);
        let heading_id = NodeId(2);
        let dialog_id = NodeId(3);
        let alert_id = NodeId(4);
        let text_id = NodeId(5);
        let run_id = NodeId(6);
        let link_id = NodeId(7);

        let mut root = AccessKitNode::new(Role::Window);
        root.set_children(vec![heading_id, dialog_id, alert_id, text_id, link_id]);

        let mut heading = AccessKitNode::new(Role::Heading);
        heading.set_level(3);
        heading.set_label("Section");

        let mut dialog = AccessKitNode::new(Role::Dialog);
        dialog.set_modal();

        let mut alert = AccessKitNode::new(Role::Alert);
        alert.set_live(Live::Polite);

        let mut text = AccessKitNode::new(Role::TextInput);
        text.set_value("abc def");
        text.set_read_only();
        text.set_children(vec![run_id]);

        let mut run = AccessKitNode::new(Role::TextRun);
        run.set_value("abc def");
        run.set_character_lengths([1, 1, 1, 1, 1, 1, 1]);

        let mut link = AccessKitNode::new(Role::Link);
        link.set_label("egui docs");
        link.set_url("https://docs.rs/egui");
        link.set_description("External docs");

        let update = TreeUpdate {
            nodes: vec![
                (root_id, root),
                (heading_id, heading),
                (dialog_id, dialog),
                (alert_id, alert),
                (text_id, text),
                (run_id, run),
                (link_id, link),
            ],
            tree: Some(AccessKitTreeInfo::new(root_id)),
            tree_id: TreeId::ROOT,
            focus: root_id,
        };

        let mut bridge = TreeBridge::new();
        let ops = bridge.apply_update(update);

        let h = find_upsert(&ops, |s| s.id == heading_id);
        assert_eq!(h.tag, DomTag::Heading);
        assert_eq!(h.heading_level, Some(3));

        let d = find_upsert(&ops, |s| s.id == dialog_id);
        assert_eq!(d.aria_role, Some("dialog"));
        assert!(d.modal);

        let a = find_upsert(&ops, |s| s.id == alert_id);
        assert_eq!(a.aria_role, Some("alert"));
        assert_eq!(a.live, Some(Live::Polite));

        let t = find_upsert(&ops, |s| s.id == text_id);
        assert!(t.read_only);
        let sel = bridge
            .set_text_selection_action(text_id, 1, 5)
            .expect("synthetic TextInput with TextRun must resolve UTF-16 selection");
        assert_eq!(sel.action, Action::SetTextSelection);

        let l = find_upsert(&ops, |s| s.id == link_id);
        assert_eq!(l.tag, DomTag::Anchor);
        assert_eq!(l.url.as_deref(), Some("https://docs.rs/egui"));
        assert_eq!(l.description.as_deref(), Some("External docs"));
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
            ops2.contains(&DomOp::Remove(temp_id)),
            "expected Remove for hidden button in {ops2:?}"
        );
    }

    #[test]
    fn action_requests_round_trip_into_egui() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        let mut bridge = TreeBridge::new();
        let mut volume = 50.0_f64;
        let mut note = String::from("abc def");
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
            ],
            ..Default::default()
        };
        let out2 = ctx.run_ui(input2, |ui| {
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
        out2.drop_without_applying_deltas();

        assert!(clicked, "AccessKit click action must trigger egui button");
        assert!(text_focused, "AccessKit focus action must focus TextEdit");
        assert_eq!(volume, 60.0);

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

    #[test]
    fn scroll_actions_scroll_egui_scroll_area() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();

        let show_scroll_ui = |ui: &mut egui::Ui| {
            egui::ScrollArea::vertical()
                .max_height(60.0)
                .animated(false)
                .show(ui, |ui| {
                    for i in 0..10 {
                        let _ = ui.button(format!("Row {i}"));
                    }
                    let _ = ui.button("Bottom target");
                });
        };

        let mut bridge = TreeBridge::new();
        let mut out1 = ctx.run_ui(egui::RawInput::default(), show_scroll_ui);
        let ops1 = bridge.apply_update(out1.platform_output.accesskit_update.take().unwrap());
        out1.drop_without_applying_deltas();

        let row0_id = find_upsert(&ops1, |s| s.label.as_deref() == Some("Row 0")).id;
        let y_row0_before = find_upsert(&ops1, |s| s.id == row0_id)
            .bounds
            .expect("Row 0 must have bounds")
            .y0;

        let input2 = egui::RawInput {
            events: vec![egui::Event::AccessKitActionRequest(scroll_action(
                row0_id,
                Action::ScrollDown,
            ))],
            ..Default::default()
        };
        let mut out2 = ctx.run_ui(input2, show_scroll_ui);
        let _ = bridge.apply_update(out2.platform_output.accesskit_update.take().unwrap());
        out2.drop_without_applying_deltas();

        let mut out3 = ctx.run_ui(egui::RawInput::default(), show_scroll_ui);
        let ops3 = bridge.apply_update(out3.platform_output.accesskit_update.take().unwrap());
        out3.drop_without_applying_deltas();

        let y_row0_after = find_upsert(&ops3, |s| s.id == row0_id)
            .bounds
            .expect("Row 0 must have bounds")
            .y0;
        assert!(
            y_row0_after < y_row0_before,
            "expected ScrollIntoView to move Row 0 upward: before={y_row0_before}, after={y_row0_after}"
        );
    }
}

#![cfg_attr(not(feature = "execute"), allow(dead_code))]

use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One native accessible element. `role`, `states` and `actions` are the platform's own
/// vocabulary; the `normalized_*` fields use one vocabulary across macOS, Windows and Linux.
/// `bounds` are desktop input coordinates.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct AccessibilityNode {
    #[serde(default)]
    pub native_id: Option<String>,
    pub role: String,
    pub name: Option<String>,
    pub value: Option<String>,
    pub description: Option<String>,
    pub bounds: Option<AccessibilityBounds>,
    pub states: Vec<String>,
    pub actions: Vec<String>,
    pub children: Vec<AccessibilityNode>,
    /// button, link, text_field, checkbox, radio_button, combo_box, menu_item, tab, list_item, …
    #[serde(default)]
    pub normalized_role: String,
    /// enabled, disabled, focused, focusable, selected, checked, mixed, expanded, collapsed, editable, …
    #[serde(default)]
    pub normalized_states: Vec<String>,
    /// Actions accepted by Act on Accessibility Element: invoke, focus, select, expand, collapse, set_value.
    #[serde(default)]
    pub normalized_actions: Vec<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct AccessibilityBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerGetAccessibilityTreeNode {}

impl ComputerGetAccessibilityTreeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerGetAccessibilityTreeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_get_accessibility_tree",
            "Get Accessibility Tree",
            "Retrieves the accessibility tree for a window (requires platform-specific accessibility APIs)",
            "Automation/Computer/Accessibility",
        );
        node.set_version(2);
        node.set_flowscript_name("computer", "getAccessibilityTree");
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(6)
                .set_governance(6)
                .set_reliability(6)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "window_title",
            "Window Title",
            "Window title; empty targets the focused window outside Flow-Like (on Linux the desktop accessibility root when no window resolves)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "max_depth",
            "Max Depth",
            "Maximum tree depth (1–32); results are limited to 2000 elements",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "exec_error",
            "⚠",
            "Triggered if accessibility APIs are unavailable",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "tree",
            "Tree",
            "Accessibility tree root node",
            VariableType::Struct,
        )
        .set_schema::<AccessibilityNode>();

        node.add_output_pin(
            "tree_json",
            "Tree JSON",
            "Accessibility tree as JSON string for LLM processing",
            VariableType::String,
        );

        node.add_output_pin(
            "elements",
            "Elements",
            "Every element of the tree as a flat list in document order (children omitted); each can be passed to Act on Accessibility Element",
            VariableType::Struct,
        )
        .set_schema::<AccessibilityNode>()
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin(
            "error",
            "Error",
            "Error message if accessibility APIs are unavailable",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let window_title: String = context
            .evaluate_pin("window_title")
            .await
            .unwrap_or_default();
        let max_depth: i64 = context.evaluate_pin("max_depth").await.unwrap_or(10);

        context.set_pin_value("session_out", json!(session)).await?;
        match load_tree(&window_title, max_depth.clamp(1, 32) as usize).await {
            Ok(tree) => {
                context
                    .set_pin_value("tree_json", json!(flow_like_types::json::to_string(&tree)?))
                    .await?;
                context
                    .set_pin_value("elements", json!(flatten(&tree)))
                    .await?;
                context.set_pin_value("tree", json!(tree)).await?;
                context.set_pin_value("error", json!("")).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(error) => {
                context.set_pin_value("tree", json!(null)).await?;
                context.set_pin_value("tree_json", json!("")).await?;
                context.set_pin_value("elements", json!([])).await?;
                context
                    .set_pin_value("error", json!(error.to_string()))
                    .await?;
                context.activate_exec_pin("exec_error").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerFindAccessibilityElementNode {}

impl ComputerFindAccessibilityElementNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerFindAccessibilityElementNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_find_accessibility_element",
            "Find Accessibility Element",
            "Finds an element in the accessibility tree by role, name, or other attributes",
            "Automation/Computer/Accessibility",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "findAccessibilityElement");
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(6)
                .set_governance(6)
                .set_reliability(6)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "window_title",
            "Window",
            "Window title, or empty for the active window",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "role",
            "Role",
            "Accessibility role to match",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "name",
            "Name",
            "Element name to match (partial match)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "exec_not_found",
            "Not Found",
            "Triggered if element not found",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "element",
            "Element",
            "Found accessibility element",
            VariableType::Struct,
        )
        .set_schema::<AccessibilityNode>();

        node.add_output_pin(
            "x",
            "X",
            "Element center X coordinate",
            VariableType::Integer,
        );
        node.add_output_pin(
            "y",
            "Y",
            "Element center Y coordinate",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_not_found").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let role: String = context.evaluate_pin("role").await.unwrap_or_default();
        let name: String = context.evaluate_pin("name").await.unwrap_or_default();

        let window: String = context
            .evaluate_pin("window_title")
            .await
            .unwrap_or_default();
        if role.is_empty() && name.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Provide a role or name to find an element"
            ));
        }
        let tree = load_tree(&window, 32).await?;
        let mut matches = Vec::new();
        find_elements(&tree, &role, &name, &mut matches);
        if matches.len() > 1 {
            return Err(flow_like_types::anyhow!(
                "More than one accessibility element matches; refine role, name, or window"
            ));
        }
        context.set_pin_value("session_out", json!(session)).await?;
        if let Some(element) = matches.first() {
            let (x, y) = element
                .bounds
                .as_ref()
                .and_then(|b| {
                    Some((
                        b.x.checked_add(b.width / 2)?,
                        b.y.checked_add(b.height / 2)?,
                    ))
                })
                .unwrap_or((0, 0));
            context.set_pin_value("element", json!(element)).await?;
            context.set_pin_value("x", json!(x)).await?;
            context.set_pin_value("y", json!(y)).await?;
            context.activate_exec_pin("exec_out").await?;
        } else {
            context.set_pin_value("element", json!(null)).await?;
            context.set_pin_value("x", json!(0)).await?;
            context.set_pin_value("y", json!(0)).await?;
            context.activate_exec_pin("exec_not_found").await?;
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

/// The window a window title names, or — for an empty title — the focused window that does
/// not belong to Flow-Like itself.
#[cfg(feature = "execute")]
pub(crate) async fn resolve_window(
    title: &str,
) -> flow_like_types::Result<Option<super::window::WindowInfo>> {
    let title = title.to_owned();
    tokio::task::spawn_blocking(move || {
        Ok(super::native::select_target_window(&title)?
            .as_ref()
            .map(super::native::window_info))
    })
    .await?
}

#[cfg(feature = "execute")]
pub(crate) async fn load_window_tree(
    window_id: &str,
    depth: usize,
) -> flow_like_types::Result<AccessibilityNode> {
    let mut tree = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        super::native::accessibility_tree(window_id, depth),
    )
    .await
    .map_err(|_| {
        flow_like_types::anyhow!("Accessibility query for window {} timed out", window_id)
    })??;
    normalize_tree(&mut tree);
    Ok(tree)
}

#[cfg(feature = "execute")]
pub(crate) async fn load_tree(
    title: &str,
    depth: usize,
) -> flow_like_types::Result<AccessibilityNode> {
    let window = resolve_window(title).await;
    #[cfg(target_os = "linux")]
    if title.is_empty() {
        if let Ok(Some(window)) = &window {
            if let Ok(tree) = load_window_tree(&window.id, depth).await {
                return Ok(tree);
            }
        }
        return load_window_tree("", depth).await;
    }
    let window = window?.ok_or_else(|| {
        if title.is_empty() {
            flow_like_types::anyhow!("No focused application window outside Flow-Like was found")
        } else {
            flow_like_types::anyhow!("Window '{}' not found", title)
        }
    })?;
    load_window_tree(&window.id, depth).await
}

fn canonical(value: &str) -> String {
    value
        .strip_prefix("AX")
        .unwrap_or(value)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(crate) fn normalize_role(raw: &str) -> &'static str {
    match canonical(raw).as_str() {
        "button" | "pushbutton" | "splitbutton" | "menubutton" | "disclosuretriangle" => "button",
        "togglebutton" => "toggle_button",
        "link" | "hyperlink" => "link",
        "textfield" | "textarea" | "edit" | "entry" | "passwordtext" | "searchfield"
        | "securetextfield" => "text_field",
        "checkbox" => "checkbox",
        "radiobutton" => "radio_button",
        "combobox" | "popupbutton" => "combo_box",
        "menu" | "popupmenu" => "menu",
        "menubar" => "menu_bar",
        "menuitem" | "menubaritem" | "checkmenuitem" | "radiomenuitem" | "tearoffmenuitem" => {
            "menu_item"
        }
        "tabitem" | "pagetab" => "tab",
        "tab" | "tabgroup" | "pagetablist" => "tab_list",
        "list" | "listbox" => "list",
        "listitem" => "list_item",
        "tree" | "outline" => "tree",
        "treeitem" => "tree_item",
        "table" | "grid" | "datagrid" | "treetable" => "table",
        "row" | "tablerow" | "dataitem" => "row",
        "cell" | "tablecell" | "gridcell" => "cell",
        "columnheader" | "rowheader" | "headeritem" | "tablecolumnheader" | "tablerowheader" => {
            "column_header"
        }
        "slider" => "slider",
        "spinbutton" | "spinner" | "incrementor" | "stepper" => "spin_button",
        "scrollbar" => "scroll_bar",
        "progressbar" | "progressindicator" | "busyindicator" | "levelindicator" => "progress_bar",
        "image" | "icon" | "animation" => "image",
        "statictext" | "text" | "label" | "static" | "caption" | "paragraph" => "text",
        "heading" => "heading",
        "window" | "frame" => "window",
        "dialog" | "alert" | "sheet" | "filechooser" | "colorchooser" => "dialog",
        "toolbar" => "toolbar",
        "group" | "panel" | "filler" | "section" | "splitgroup" | "layoutarea" | "layoutitem"
        | "form" | "grouping" | "radiogroup" | "landmark" | "article" | "header" | "footer" => {
            "group"
        }
        "pane" | "scrollpane" | "scrollarea" | "viewport" | "splitpane" | "internalframe"
        | "rootpane" | "layeredpane" => "pane",
        "document"
        | "documentweb"
        | "documentframe"
        | "webarea"
        | "documenttext"
        | "documentemail"
        | "documentspreadsheet"
        | "documentpresentation" => "document",
        "statusbar" => "status_bar",
        "tooltip" | "helptag" => "tooltip",
        "separator" | "splitter" => "separator",
        "application" => "application",
        "titlebar" => "title_bar",
        _ => "other",
    }
}

pub(crate) fn normalize_state(raw: &str) -> Option<&'static str> {
    Some(match canonical(raw).as_str() {
        "enabled" | "sensitive" => "enabled",
        "disabled" => "disabled",
        "focused" => "focused",
        "focusable" => "focusable",
        "selected" => "selected",
        "checked" => "checked",
        "mixed" | "indeterminate" => "mixed",
        "expanded" => "expanded",
        "collapsed" => "collapsed",
        "editable" => "editable",
        "readonly" => "read_only",
        "pressed" => "pressed",
        "required" => "required",
        "busy" => "busy",
        "modal" => "modal",
        "offscreen" => "offscreen",
        "invalidentry" => "invalid",
        "visited" => "visited",
        "haspopup" => "has_popup",
        "multiline" => "multi_line",
        _ => return None,
    })
}

pub(crate) fn normalize_action(raw: &str) -> Option<&'static str> {
    Some(match canonical(raw).as_str() {
        "invoke" | "press" | "click" | "activate" | "jump" | "toggle" | "open" | "confirm" => {
            "invoke"
        }
        "select" | "pick" => "select",
        "focus" => "focus",
        "expand" => "expand",
        "collapse" => "collapse",
        "setvalue" => "set_value",
        _ => return None,
    })
}

/// AT-SPI state names for the two 32-bit words `GetState` returns.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn atspi_state_names(words: &[u32]) -> Vec<String> {
    const NAMES: [&str; 44] = [
        "invalid",
        "active",
        "armed",
        "busy",
        "checked",
        "collapsed",
        "defunct",
        "editable",
        "enabled",
        "expandable",
        "expanded",
        "focusable",
        "focused",
        "has-tooltip",
        "horizontal",
        "iconified",
        "modal",
        "multi-line",
        "multiselectable",
        "opaque",
        "pressed",
        "resizable",
        "selectable",
        "selected",
        "sensitive",
        "showing",
        "single-line",
        "stale",
        "transient",
        "vertical",
        "visible",
        "manages-descendants",
        "indeterminate",
        "required",
        "truncated",
        "animated",
        "invalid-entry",
        "supports-autocompletion",
        "selectable-text",
        "is-default",
        "visited",
        "checkable",
        "has-popup",
        "read-only",
    ];
    let bits = words.first().copied().unwrap_or(0) as u64
        | (words.get(1).copied().unwrap_or(0) as u64) << 32;
    NAMES
        .iter()
        .enumerate()
        .filter(|(index, _)| bits & (1u64 << index) != 0)
        .map(|(_, name)| name.to_string())
        .collect()
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

/// Fills the `normalized_*` fields of every node from the platform vocabulary.
pub(crate) fn normalize_tree(node: &mut AccessibilityNode) {
    let mut states = Vec::new();
    for state in node.states.iter().filter_map(|s| normalize_state(s)) {
        push_unique(&mut states, state);
    }
    let mut role = normalize_role(&node.role);
    if role == "text" && states.iter().any(|s| s == "editable") {
        role = "text_field";
    }
    let mut actions = Vec::new();
    for action in node.actions.iter().filter_map(|a| normalize_action(a)) {
        push_unique(&mut actions, action);
    }
    if role == "text_field" {
        push_unique(&mut actions, "focus");
        push_unique(&mut actions, "set_value");
    }
    if states.iter().any(|s| s == "focusable") {
        push_unique(&mut actions, "focus");
    }
    node.normalized_role = role.to_string();
    node.normalized_states = states;
    node.normalized_actions = actions;
    for child in &mut node.children {
        normalize_tree(child);
    }
}

/// Every node of the tree in document order, without children.
pub(crate) fn flatten(tree: &AccessibilityNode) -> Vec<AccessibilityNode> {
    let mut elements = Vec::new();
    let mut stack = vec![tree];
    while let Some(node) = stack.pop() {
        elements.push(AccessibilityNode {
            native_id: node.native_id.clone(),
            role: node.role.clone(),
            name: node.name.clone(),
            value: node.value.clone(),
            description: node.description.clone(),
            bounds: node.bounds.clone(),
            states: node.states.clone(),
            actions: node.actions.clone(),
            children: Vec::new(),
            normalized_role: node.normalized_role.clone(),
            normalized_states: node.normalized_states.clone(),
            normalized_actions: node.normalized_actions.clone(),
        });
        stack.extend(node.children.iter().rev());
    }
    elements
}

pub(crate) fn is_interactive(node: &AccessibilityNode) -> bool {
    const ROLES: [&str; 13] = [
        "button",
        "toggle_button",
        "link",
        "text_field",
        "checkbox",
        "radio_button",
        "combo_box",
        "menu_item",
        "tab",
        "list_item",
        "tree_item",
        "slider",
        "spin_button",
    ];
    ROLES.contains(&node.normalized_role.as_str())
        || node.normalized_actions.iter().any(|a| a == "invoke")
}

pub(crate) fn find_elements<'a>(
    node: &'a AccessibilityNode,
    role: &str,
    name: &str,
    matches: &mut Vec<&'a AccessibilityNode>,
) {
    let wanted = normalize_role(role);
    if (role.is_empty()
        || canonical(&node.role) == canonical(role)
        || (wanted != "other" && node.normalized_role == wanted))
        && (name.is_empty()
            || node
                .name
                .as_ref()
                .is_some_and(|n| n.to_lowercase().contains(&name.to_lowercase())))
    {
        matches.push(node);
    }
    for child in &node.children {
        find_elements(child, role, name, matches);
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerAccessibilityActionNode;
impl ComputerAccessibilityActionNode {
    pub fn new() -> Self {
        Self
    }
}
#[async_trait]
impl NodeLogic for ComputerAccessibilityActionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_accessibility_action",
            "Act on Accessibility Element",
            "Invokes, focuses, selects, expands, collapses, or edits a native accessible element",
            "Automation/Computer/Accessibility",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "accessibilityAction");
        node.add_icon("/flow/icons/computer.svg");
        node.set_only_offline(true);
        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "session",
            "Session",
            "Active automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();
        node.add_input_pin(
            "element",
            "Element",
            "Native element returned by Find Accessibility Element or the elements list of Get Accessibility Tree",
            VariableType::Struct,
        )
        .set_schema::<AccessibilityNode>();
        node.add_input_pin("action", "Action", "Native action", VariableType::String)
            .set_default_value(Some(json!("invoke")))
            .set_options(
                flow_like::flow::pin::PinOptions::new()
                    .set_valid_values(
                        [
                            "invoke",
                            "focus",
                            "select",
                            "expand",
                            "collapse",
                            "set_value",
                        ]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                    )
                    .build(),
            );
        node.add_input_pin(
            "value",
            "Value",
            "Value to write for set_value",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_output_pin("exec_out", "▶", "Action completed", VariableType::Execution);
        node.add_output_pin("session_out", "Session", "Session", VariableType::Struct)
            .set_schema::<AutomationSession>();
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let element: AccessibilityNode = context.evaluate_pin("element").await?;
        let action: String = context.evaluate_pin("action").await?;
        let value: String = context.evaluate_pin("value").await?;
        super::native::accessibility_action(
            &element,
            &action,
            &value,
            context.get_cancellation_token(),
        )
        .await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Native accessibility requires execute"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn element(role: &str, states: &[&str], actions: &[&str]) -> AccessibilityNode {
        AccessibilityNode {
            role: role.into(),
            states: states.iter().map(|s| s.to_string()).collect(),
            actions: actions.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn roles_share_one_vocabulary_across_platforms() {
        for (raw, normalized) in [
            ("AXButton", "button"),
            ("push button", "button"),
            ("Button", "button"),
            ("AXTextField", "text_field"),
            ("Edit", "text_field"),
            ("entry", "text_field"),
            ("AXLink", "link"),
            ("Hyperlink", "link"),
            ("AXCheckBox", "checkbox"),
            ("check box", "checkbox"),
            ("AXPopUpButton", "combo_box"),
            ("combo box", "combo_box"),
            ("AXMenuItem", "menu_item"),
            ("check menu item", "menu_item"),
            ("TabItem", "tab"),
            ("page tab", "tab"),
            ("Tab", "tab_list"),
            ("ListItem", "list_item"),
            ("AXStaticText", "text"),
            ("AXWebArea", "document"),
            ("Unknown(50099)", "other"),
        ] {
            assert_eq!(normalize_role(raw), normalized, "{raw}");
        }
    }

    #[test]
    fn states_and_actions_normalize_and_editable_text_becomes_a_field() {
        let mut node = element(
            "text",
            &[
                "sensitive",
                "focused",
                "editable",
                "read-only",
                "bits:00000001",
            ],
            &["AXPress", "click", "AXShowMenu"],
        );
        node.children
            .push(element("AXCheckBox", &["disabled", "checked"], &[]));
        normalize_tree(&mut node);
        assert_eq!(node.normalized_role, "text_field");
        assert_eq!(
            node.normalized_states,
            ["enabled", "focused", "editable", "read_only"]
        );
        assert_eq!(node.normalized_actions, ["invoke", "focus", "set_value"]);
        assert_eq!(node.children[0].normalized_states, ["disabled", "checked"]);
        assert!(is_interactive(&node.children[0]));
    }

    #[test]
    fn atspi_state_bits_decode_across_both_words() {
        let names = atspi_state_names(&[(1 << 8) | (1 << 12) | (1 << 24), 1 << 11]);
        assert_eq!(names, ["enabled", "focused", "sensitive", "read-only"]);
        assert!(atspi_state_names(&[]).is_empty());
    }

    #[test]
    fn flatten_keeps_document_order_without_children() {
        let mut root = element("AXWindow", &[], &[]);
        let mut group = element("AXGroup", &[], &[]);
        group.children.push(element("AXButton", &[], &["AXPress"]));
        root.children.push(group);
        root.children.push(element("AXTextField", &[], &[]));
        normalize_tree(&mut root);
        let flat = flatten(&root);
        let roles: Vec<_> = flat.iter().map(|n| n.normalized_role.as_str()).collect();
        assert_eq!(roles, ["window", "group", "button", "text_field"]);
        assert!(flat.iter().all(|n| n.children.is_empty()));
        let mut matches = Vec::new();
        find_elements(&root, "button", "", &mut matches);
        assert_eq!(matches.len(), 1);
    }
}

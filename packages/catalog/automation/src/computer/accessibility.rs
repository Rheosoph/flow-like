use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
        node.set_version(1);
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
            "Window title; empty uses the focused window, or the desktop accessibility root on Linux",
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
                context.set_pin_value("tree", json!(tree)).await?;
                context.set_pin_value("error", json!("")).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(error) => {
                context.set_pin_value("tree", json!(null)).await?;
                context.set_pin_value("tree_json", json!("")).await?;
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

#[cfg(feature = "execute")]
pub(crate) async fn load_tree(
    title: &str,
    depth: usize,
) -> flow_like_types::Result<AccessibilityNode> {
    #[cfg(target_os = "linux")]
    let id = if title.is_empty() {
        String::new()
    } else {
        super::native::select_window_async("", title, "", false)
            .await?
            .ok_or_else(|| flow_like_types::anyhow!("Window not found"))?
            .id()?
            .to_string()
    };
    #[cfg(not(target_os = "linux"))]
    let id = super::native::select_window_async("", title, "", false)
        .await?
        .ok_or_else(|| flow_like_types::anyhow!("Window not found"))?
        .id()?
        .to_string();
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        super::native::accessibility_tree(&id, depth),
    )
    .await
    .map_err(|_| flow_like_types::anyhow!("Accessibility query timed out"))?
}

pub(crate) fn find_elements<'a>(
    node: &'a AccessibilityNode,
    role: &str,
    name: &str,
    matches: &mut Vec<&'a AccessibilityNode>,
) {
    fn canonical(role: &str) -> String {
        role.strip_prefix("AX")
            .unwrap_or(role)
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }
    if (role.is_empty() || canonical(&node.role) == canonical(role))
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
            "Native element returned by Find Accessibility Element",
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

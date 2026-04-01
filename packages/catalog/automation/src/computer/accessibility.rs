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
            "Title of the window to inspect (leave empty for focused window)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "max_depth",
            "Max Depth",
            "Maximum tree depth to traverse (-1 for unlimited)",
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
        let _window_title: String = context
            .evaluate_pin("window_title")
            .await
            .unwrap_or_default();
        let _max_depth: i64 = context.evaluate_pin("max_depth").await.unwrap_or(10);

        // Note: Platform-specific accessibility API integration would be needed here
        // macOS: AXUIElement APIs
        // Windows: UI Automation (UIA) or MSAA
        // Linux: AT-SPI2

        // For now, return a placeholder indicating the feature requires platform-specific setup
        let error_msg = "Accessibility tree retrieval requires platform-specific APIs. \
                        Consider using browser automation accessibility APIs instead, \
                        or implement platform-specific bindings."
            .to_string();

        let placeholder_tree = AccessibilityNode {
            role: "application".to_string(),
            name: Some("Accessibility API not implemented".to_string()),
            value: None,
            description: Some(error_msg.clone()),
            bounds: None,
            states: vec![],
            actions: vec![],
            children: vec![],
        };

        let tree_json = flow_like_types::json::to_string_pretty(&placeholder_tree)
            .unwrap_or_else(|_| "{}".to_string());

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("tree", json!(placeholder_tree))
            .await?;
        context.set_pin_value("tree_json", json!(tree_json)).await?;
        context.set_pin_value("error", json!(error_msg)).await?;
        context.activate_exec_pin("exec_error").await?;

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
        let _role: String = context.evaluate_pin("role").await.unwrap_or_default();
        let _name: String = context.evaluate_pin("name").await.unwrap_or_default();

        // Platform-specific implementation would go here
        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("element", json!(AccessibilityNode::default()))
            .await?;
        context.set_pin_value("x", json!(0)).await?;
        context.set_pin_value("y", json!(0)).await?;
        context.activate_exec_pin("exec_not_found").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

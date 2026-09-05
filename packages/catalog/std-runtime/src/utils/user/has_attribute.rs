use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// Check if the executing user's role has a specific attribute (tag).
/// Attributes are simple string tags that can be assigned to roles for custom authorization logic.
#[crate::register_node]
#[derive(Default)]
pub struct HasAttributeNode {}

impl HasAttributeNode {
    pub fn new() -> Self {
        HasAttributeNode {}
    }
}

#[async_trait]
impl NodeLogic for HasAttributeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_user_has_attribute",
            "Has Attribute",
            "Checks if the executing user's role has a specific attribute (tag). Attributes are custom string tags assigned to roles for flexible authorization. Returns false if no user context is available or the user has no role.",
            "Utils/User",
        );
        node.set_flowscript_name("user", "hasAttribute");
        node.add_icon("/flow/icons/tag.svg");

        node.add_input_pin(
            "attribute",
            "Attribute",
            "The attribute (tag) to check for",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "has_attribute",
            "Has Attribute",
            "True if the user's role has the specified attribute",
            VariableType::Boolean,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(8) // Only checks attributes, doesn't expose user data
                .set_security(9) // Important for access control
                .set_performance(10) // Very fast, just array search
                .set_governance(9) // Useful for custom authorization
                .set_reliability(10) // Always succeeds
                .set_cost(10) // No external calls
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let attribute: String = context.evaluate_pin("attribute").await?;

        let user_context = context.user_context().cloned();
        let has_attribute = user_context
            .map(|uc| uc.has_attribute(&attribute))
            .unwrap_or(false);

        context
            .set_pin_value("has_attribute", json!(has_attribute))
            .await?;

        Ok(())
    }
}

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// Get the currently executing user context.
/// Returns the complete user context as a typed struct that can be broken down.
#[crate::register_node]
#[derive(Default)]
pub struct GetExecutingUserNode {}

impl GetExecutingUserNode {
    pub fn new() -> Self {
        GetExecutingUserNode {}
    }
}

#[async_trait]
impl NodeLogic for GetExecutingUserNode {
    fn get_node(&self) -> Node {
        use flow_like::flow::execution::UserExecutionContext;

        let mut node = Node::new(
            "utils_user_get_executing_user",
            "Get Executing User",
            "Gets the user context of the current execution. Returns a typed struct containing sub (user ID), role, permissions, attributes, and details of the calling principal. Use 'Break Struct' to access individual fields.",
            "Utils/User",
        );
        node.add_icon("/flow/icons/user.svg");

        node.add_output_pin(
            "user_context",
            "User Context",
            "The complete user execution context. Use 'Break Struct' to access: sub, role (with id, name, permissions, attributes), isTechnicalUser, keyId, principal, originAppId, onBehalfOf",
            VariableType::Struct,
        )
        .set_schema::<UserExecutionContext>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "has_user",
            "Has User",
            "True if user context is available",
            VariableType::Boolean,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(6) // Returns user data
                .set_security(8) // Read-only, no modification
                .set_performance(10) // Very fast, just reads context
                .set_governance(8) // Good for audit trails
                .set_reliability(10) // Always succeeds
                .set_cost(10) // No external calls
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let user_context = context.user_context().cloned();

        match user_context {
            Some(uc) => {
                context.set_pin_value("user_context", json!(uc)).await?;
                context.set_pin_value("has_user", json!(true)).await?;
            }
            None => {
                context.set_pin_value("user_context", json!(null)).await?;
                context.set_pin_value("has_user", json!(false)).await?;
            }
        }

        Ok(())
    }
}

use flow_like::flow::{
    execution::{ExecutionPrincipal, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// Check if the current execution is by a technical user (API key).
#[crate::register_node]
#[derive(Default)]
pub struct IsTechnicalUserNode {}

impl IsTechnicalUserNode {
    pub fn new() -> Self {
        IsTechnicalUserNode {}
    }
}

#[async_trait]
impl NodeLogic for IsTechnicalUserNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_user_is_technical_user",
            "Is Technical User",
            "Checks whether a machine rather than a person triggered this run. Machine callers have no human identity (sub): an API key reports its Key ID, an app calling through an app connection reports the calling app instead.",
            "Utils/User",
        );
        node.add_icon("/flow/icons/key.svg");
        node.set_version(2);

        node.add_output_pin(
            "is_technical",
            "Is Technical User",
            "True if a machine triggered the run (API key or app connection), false for a person",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "key_id",
            "Key ID",
            "The API key identifier, empty for every other caller",
            VariableType::String,
        );

        node.add_output_pin(
            "principal",
            "Principal",
            "How the caller authenticated: 'user', 'apiKey' or 'connectedApp'",
            VariableType::String,
        );

        node.add_output_pin(
            "origin_app_id",
            "Origin App",
            "The app that made the call when the principal is 'connectedApp', empty otherwise",
            VariableType::String,
        );

        node.add_output_pin(
            "on_behalf_of",
            "On Behalf Of",
            "The user the caller reported as the initiator: an API key's creator, or the user an app connection passed through. Attribution only — never authorize against it",
            VariableType::String,
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(5) // Returns minimal user info
                .set_security(9) // Read-only, useful for security checks
                .set_performance(10) // Very fast
                .set_governance(9) // Good for audit trails
                .set_reliability(10) // Always succeeds
                .set_cost(10) // No external calls
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let user_context = context.user_context().cloned();

        let (is_technical, key_id, principal, origin_app_id, on_behalf_of) = match user_context {
            Some(uc) => (
                uc.is_technical(),
                uc.get_key_id().unwrap_or("").to_string(),
                match uc.principal {
                    ExecutionPrincipal::User => "user",
                    ExecutionPrincipal::ApiKey => "apiKey",
                    ExecutionPrincipal::ConnectedApp => "connectedApp",
                }
                .to_string(),
                uc.origin_app_id().unwrap_or("").to_string(),
                uc.on_behalf_of().unwrap_or("").to_string(),
            ),
            None => (
                false,
                String::new(),
                String::new(),
                String::new(),
                String::new(),
            ),
        };

        context
            .set_pin_value("is_technical", json!(is_technical))
            .await?;
        context.set_pin_value("key_id", json!(key_id)).await?;
        context.set_pin_value("principal", json!(principal)).await?;
        context
            .set_pin_value("origin_app_id", json!(origin_app_id))
            .await?;
        context
            .set_pin_value("on_behalf_of", json!(on_behalf_of))
            .await?;

        Ok(())
    }
}

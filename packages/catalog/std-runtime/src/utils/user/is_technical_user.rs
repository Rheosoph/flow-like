use flow_like::flow::{
    execution::{ExecutionPrincipal, UserExecutionContext, context::ExecutionContext},
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

fn caller_id(context: &UserExecutionContext) -> String {
    if context.is_bot() {
        return context.sub.clone();
    }
    context.get_key_id().unwrap_or("").to_string()
}

#[async_trait]
impl NodeLogic for IsTechnicalUserNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_user_is_technical_user",
            "Is Technical User",
            "Checks whether a machine rather than a person triggered this run. Machine callers have no human identity (sub): an API key reports its Key ID, an app calling through an app connection reports the calling app instead, and a bot reports its bot id.",
            "Utils/User",
        );
        node.set_flowscript_name("user", "isTechnical");
        node.add_icon("/flow/icons/key.svg");
        node.set_version(3);

        node.add_output_pin(
            "is_technical",
            "Is Technical User",
            "True if a machine triggered the run (API key, app connection or bot), false for a person",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "key_id",
            "Key ID",
            "The API key identifier, or the bot id when a bot triggered the run, empty for every other caller",
            VariableType::String,
        );

        node.add_output_pin(
            "principal",
            "Principal",
            "How the caller authenticated: 'user', 'apiKey', 'connectedApp' or 'bot'",
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
            "The user the caller reported as the initiator: an API key's creator, the user an app connection passed through, or the bot's owner. Attribution only — never authorize against it",
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
                caller_id(&uc),
                match uc.principal {
                    ExecutionPrincipal::User => "user",
                    ExecutionPrincipal::ApiKey => "apiKey",
                    ExecutionPrincipal::ConnectedApp => "connectedApp",
                    ExecutionPrincipal::Bot => "bot",
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

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::execution::RoleContext;

    fn role() -> RoleContext {
        RoleContext {
            id: "role".into(),
            name: "Role".into(),
            permissions: 0,
            attributes: Vec::new(),
            custom_attributes: Default::default(),
        }
    }

    #[test]
    fn a_bot_run_is_technical_and_reports_the_bot() {
        let context =
            UserExecutionContext::bot("bot_1", role()).with_on_behalf_of(Some("owner".to_string()));
        assert!(context.is_technical());
        assert_eq!(context.principal, ExecutionPrincipal::Bot);
        assert_eq!(caller_id(&context), "bot_1");
        assert_eq!(context.on_behalf_of(), Some("owner"));
    }

    #[test]
    fn a_person_reports_no_caller_id() {
        let context = UserExecutionContext::new("person");
        assert!(!context.is_technical());
        assert_eq!(caller_id(&context), "");
    }

    #[test]
    fn the_pin_descriptions_name_the_bot() {
        let node = IsTechnicalUserNode::new().get_node();
        assert_eq!(node.version, Some(3));
        assert!(node.description.contains("bot"));
        let principal = node
            .pins
            .values()
            .find(|pin| pin.name == "principal")
            .unwrap();
        assert!(principal.description.contains("'bot'"));
    }
}

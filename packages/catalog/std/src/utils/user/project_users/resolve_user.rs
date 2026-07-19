use super::*;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct ResolveUserNode {}

impl ResolveUserNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ResolveUserNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "utils_user_resolve_user",
            "Resolve User",
            "Resolves a project user by user ID/sub or by email when email is exposed by platform lookup settings. Email matching is constrained to project members.",
        );
        add_app_pin(&mut node);
        node.add_input_pin(
            "identifier",
            "Identifier",
            "Email, sub, or user ID to resolve within the project.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "identifier_type",
            "Identifier Type",
            "How to interpret the identifier.",
            VariableType::String,
        )
        .set_default_value(Some(json!("auto")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "auto".to_string(),
                    "email".to_string(),
                    "sub".to_string(),
                    "user_id".to_string(),
                ])
                .build(),
        );
        add_project_user_output(&mut node);
        add_common_outputs(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let app_id_input = eval_string_pin(context, "app_id", "").await;
        let identifier = eval_string_pin(context, "identifier", "").await;
        let identifier_type = eval_string_pin(context, "identifier_type", "auto").await;

        let client = match HubClient::from_context(context) {
            Ok(client) => client,
            Err(err) => {
                context.set_pin_value("project_user", Value::Null).await?;
                context.set_pin_value("found", json!(false)).await?;
                set_common_outputs(context, false, err.status_code, &err.message).await?;
                return Ok(());
            }
        };
        let app_id = match app_id_from_context(context, app_id_input) {
            Ok(app_id) => app_id,
            Err(err) => {
                context.set_pin_value("project_user", Value::Null).await?;
                context.set_pin_value("found", json!(false)).await?;
                set_common_outputs(context, false, err.status_code, &err.message).await?;
                return Ok(());
            }
        };

        let result = async {
            let (roles, status) = load_roles(&client, &app_id).await?;
            let kind = identifier_type.trim().to_ascii_lowercase();
            let by_email = kind == "email" || (kind == "auto" && identifier.contains('@'));
            let found = if by_email {
                find_project_user_by_email(&client, &app_id, &identifier, &roles).await?
            } else {
                get_project_user_by_id(&client, &app_id, identifier.trim(), &roles).await?
            };

            Ok::<_, HubError>((found, status))
        }
        .await;

        match result {
            Ok((Some((user, status)), role_status)) => {
                context.set_pin_value("project_user", json!(user)).await?;
                context.set_pin_value("found", json!(true)).await?;
                set_common_outputs(context, true, status.max(role_status), "").await?;
            }
            Ok((None, status)) => {
                context.set_pin_value("project_user", Value::Null).await?;
                context.set_pin_value("found", json!(false)).await?;
                set_common_outputs(context, true, status, "").await?;
            }
            Err(err) => {
                context.log_message(&err.message, LogLevel::Warn);
                context.set_pin_value("project_user", Value::Null).await?;
                context.set_pin_value("found", json!(false)).await?;
                set_common_outputs(context, false, err.status_code, &err.message).await?;
            }
        }

        Ok(())
    }
}

use super::*;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct GetCurrentUserNode {}

impl GetCurrentUserNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetCurrentUserNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "utils_user_get_current_user",
            "Get Current User",
            "Gets the current runtime user and, when available, their project membership, role, effective permissions, and attributes.",
        );
        add_app_pin(&mut node);
        node.add_output_pin(
            "current_user",
            "Current User",
            "Current runtime user with project membership details when available.",
            VariableType::Struct,
        )
        .set_schema::<CurrentUser>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        add_common_outputs(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let app_id_input = eval_string_pin(context, "app_id", "").await;
        let client = match HubClient::from_context(context) {
            Ok(client) => client,
            Err(err) => {
                context.set_pin_value("current_user", Value::Null).await?;
                set_common_outputs(context, false, err.status_code, &err.message).await?;
                return Ok(());
            }
        };
        let app_id = match app_id_from_context(context, app_id_input) {
            Ok(app_id) => app_id,
            Err(err) => {
                context.set_pin_value("current_user", Value::Null).await?;
                set_common_outputs(context, false, err.status_code, &err.message).await?;
                return Ok(());
            }
        };

        let result = async {
            let (current_user, mut status) = client.current_user().await?;
            let (roles, role_status) = load_roles(&client, &app_id).await?;
            status = status.max(role_status);
            let project_user =
                get_project_user_by_id(&client, &app_id, &current_user.user_id, &roles).await?;
            let project_user = match project_user {
                Some((project_user, user_status)) => {
                    status = status.max(user_status);
                    Some(project_user)
                }
                None => None,
            };

            Ok::<_, HubError>((current_user, project_user, status))
        }
        .await;

        match result {
            Ok((user, project_user, status)) => {
                let current_user = CurrentUser {
                    user,
                    has_project_user: project_user.is_some(),
                    project_user,
                };
                context
                    .set_pin_value("current_user", json!(current_user))
                    .await?;
                set_common_outputs(context, true, status, "").await?;
            }
            Err(err) => {
                context.log_message(&err.message, LogLevel::Warn);
                context.set_pin_value("current_user", Value::Null).await?;
                set_common_outputs(context, false, err.status_code, &err.message).await?;
            }
        }

        Ok(())
    }
}

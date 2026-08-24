use super::*;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct CheckUserPermissionNode {}

impl CheckUserPermissionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CheckUserPermissionNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "utils_user_check_user_permission",
            "Check User Permission",
            "Checks whether a project user effectively has a permission. Owner and Admin imply all permissions.",
        );
        node.set_flowscript_name("user", "checkPermission");
        add_app_pin(&mut node);
        add_user_id_pin(&mut node);
        node.add_input_pin(
            "permission",
            "Permission",
            "Permission name or bit value to check.",
            VariableType::String,
        )
        .set_default_value(Some(json!("Read Boards")))
        .set_options(
            PinOptions::new()
                .set_valid_values(permission_names())
                .build(),
        );
        node.add_output_pin(
            "has_permission",
            "Has Permission",
            "True when the user effectively has the requested permission.",
            VariableType::Boolean,
        );
        add_project_user_output(&mut node);
        add_common_outputs(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let permission = eval_string_pin(context, "permission", "Read Boards").await;
        let requested_permission = permission_from_name(&permission);
        let result = load_single_project_user(context).await;

        match (requested_permission, result) {
            (Some(requested_permission), Ok(Some((user, status)))) => {
                let has_permission =
                    has_effective_permission(user.permissions, requested_permission);
                context
                    .set_pin_value("has_permission", json!(has_permission))
                    .await?;
                context.set_pin_value("project_user", json!(user)).await?;
                context.set_pin_value("found", json!(true)).await?;
                set_common_outputs(context, true, status, "").await?;
            }
            (None, _) => {
                let message = format!("Unknown permission: {permission}");
                context.log_message(&message, LogLevel::Warn);
                context
                    .set_pin_value("has_permission", json!(false))
                    .await?;
                context.set_pin_value("project_user", Value::Null).await?;
                context.set_pin_value("found", json!(false)).await?;
                set_common_outputs(context, false, 0, &message).await?;
            }
            (_, Ok(None)) => {
                context
                    .set_pin_value("has_permission", json!(false))
                    .await?;
                context.set_pin_value("project_user", Value::Null).await?;
                context.set_pin_value("found", json!(false)).await?;
                set_common_outputs(context, true, 200, "").await?;
            }
            (_, Err(err)) => {
                context.log_message(&err.message, LogLevel::Warn);
                context
                    .set_pin_value("has_permission", json!(false))
                    .await?;
                context.set_pin_value("project_user", Value::Null).await?;
                context.set_pin_value("found", json!(false)).await?;
                set_common_outputs(context, false, err.status_code, &err.message).await?;
            }
        }
        Ok(())
    }
}

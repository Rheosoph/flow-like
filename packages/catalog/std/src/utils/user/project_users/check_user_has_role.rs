use super::*;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct CheckUserHasRoleNode {}

impl CheckUserHasRoleNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CheckUserHasRoleNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "utils_user_check_user_has_role",
            "Check User Has Role",
            "Checks whether a project user has the specified role ID or exact role name.",
        );
        add_app_pin(&mut node);
        add_user_id_pin(&mut node);
        node.add_input_pin(
            "role",
            "Role",
            "Role ID or exact role name.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_output_pin(
            "has_role",
            "Has Role",
            "True when the user has the requested role.",
            VariableType::Boolean,
        );
        add_project_user_output(&mut node);
        add_common_outputs(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let role_query = eval_string_pin(context, "role", "").await;
        let result = load_single_project_user(context).await;
        match result {
            Ok(Some((user, status))) => {
                let has_role = role_matches(user.role.as_ref(), &role_query);
                context.set_pin_value("has_role", json!(has_role)).await?;
                context.set_pin_value("project_user", json!(user)).await?;
                context.set_pin_value("found", json!(true)).await?;
                set_common_outputs(context, true, status, "").await?;
            }
            Ok(None) => {
                context.set_pin_value("has_role", json!(false)).await?;
                context.set_pin_value("project_user", Value::Null).await?;
                context.set_pin_value("found", json!(false)).await?;
                set_common_outputs(context, true, 200, "").await?;
            }
            Err(err) => {
                context.log_message(&err.message, LogLevel::Warn);
                context.set_pin_value("has_role", json!(false)).await?;
                context.set_pin_value("project_user", Value::Null).await?;
                context.set_pin_value("found", json!(false)).await?;
                set_common_outputs(context, false, err.status_code, &err.message).await?;
            }
        }
        Ok(())
    }
}

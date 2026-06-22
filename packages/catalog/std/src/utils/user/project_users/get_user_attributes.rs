use super::*;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct GetUserAttributesNode {}

impl GetUserAttributesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetUserAttributesNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "utils_user_get_user_attributes",
            "Get User Attributes",
            "Gets custom role attributes assigned to a project user.",
        );
        add_app_pin(&mut node);
        add_user_id_pin(&mut node);
        node.add_output_pin(
            "user_attributes",
            "User Attributes",
            "Role attributes for the project user.",
            VariableType::Struct,
        )
        .set_schema::<UserAttributes>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "found",
            "Found",
            "True when the user was found.",
            VariableType::Boolean,
        );
        add_common_outputs(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let result = load_single_project_user(context).await;
        match result {
            Ok(Some((user, status))) => {
                let attributes = UserAttributes {
                    user: user.user,
                    attributes: user.attributes,
                };
                context
                    .set_pin_value("user_attributes", json!(attributes))
                    .await?;
                context.set_pin_value("found", json!(true)).await?;
                set_common_outputs(context, true, status, "").await?;
            }
            Ok(None) => {
                context
                    .set_pin_value("user_attributes", Value::Null)
                    .await?;
                context.set_pin_value("found", json!(false)).await?;
                set_common_outputs(context, true, 200, "").await?;
            }
            Err(err) => {
                context.log_message(&err.message, LogLevel::Warn);
                context
                    .set_pin_value("user_attributes", Value::Null)
                    .await?;
                context.set_pin_value("found", json!(false)).await?;
                set_common_outputs(context, false, err.status_code, &err.message).await?;
            }
        }
        Ok(())
    }
}

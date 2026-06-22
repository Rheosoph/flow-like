use super::*;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct GetUserAttributeNode {}

impl GetUserAttributeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetUserAttributeNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "utils_user_get_user_attribute",
            "Get User Attribute",
            "Checks for one custom role attribute on a project user.",
        );
        add_app_pin(&mut node);
        add_user_id_pin(&mut node);
        node.add_input_pin(
            "attribute",
            "Attribute",
            "Role attribute to read.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_output_pin(
            "has_attribute",
            "Has Attribute",
            "True when the user has the requested attribute.",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "attribute_value",
            "Attribute",
            "The matching attribute when present.",
            VariableType::String,
        );
        add_project_user_output(&mut node);
        add_common_outputs(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let attribute = eval_string_pin(context, "attribute", "").await;
        let result = load_single_project_user(context).await;

        match result {
            Ok(Some((user, status))) => {
                let found_attribute = user
                    .attributes
                    .iter()
                    .find(|candidate| candidate.eq_ignore_ascii_case(attribute.trim()))
                    .cloned();
                context
                    .set_pin_value("has_attribute", json!(found_attribute.is_some()))
                    .await?;
                context
                    .set_pin_value(
                        "attribute_value",
                        json!(found_attribute.unwrap_or_default()),
                    )
                    .await?;
                context.set_pin_value("project_user", json!(user)).await?;
                context.set_pin_value("found", json!(true)).await?;
                set_common_outputs(context, true, status, "").await?;
            }
            Ok(None) => {
                context.set_pin_value("has_attribute", json!(false)).await?;
                context.set_pin_value("attribute_value", json!("")).await?;
                context.set_pin_value("project_user", Value::Null).await?;
                context.set_pin_value("found", json!(false)).await?;
                set_common_outputs(context, true, 200, "").await?;
            }
            Err(err) => {
                context.log_message(&err.message, LogLevel::Warn);
                context.set_pin_value("has_attribute", json!(false)).await?;
                context.set_pin_value("attribute_value", json!("")).await?;
                context.set_pin_value("project_user", Value::Null).await?;
                context.set_pin_value("found", json!(false)).await?;
                set_common_outputs(context, false, err.status_code, &err.message).await?;
            }
        }
        Ok(())
    }
}

use super::*;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct ListUsersWithAttributeNode {}

impl ListUsersWithAttributeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListUsersWithAttributeNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "utils_user_list_users_with_attribute",
            "List Users with Attribute",
            "Lists project users whose assigned role contains a custom attribute.",
        );
        node.set_flowscript_name("user", "listWithAttribute");
        add_app_pin(&mut node);
        node.add_input_pin(
            "attribute",
            "Attribute",
            "Role attribute to match.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        add_pagination_pins(&mut node);
        add_users_output(&mut node);
        add_common_outputs(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let attribute = eval_string_pin(context, "attribute", "").await;
        run_user_list_node(context, |client, app_id, roles, offset, limit| {
            let attribute = attribute.clone();
            Box::pin(async move {
                filter_project_users(client, app_id, roles, offset, limit, |user| {
                    has_attribute(user, &attribute)
                })
                .await
            })
        })
        .await
    }
}

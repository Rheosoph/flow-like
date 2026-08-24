use super::*;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct ListUsersWithRoleNode {}

impl ListUsersWithRoleNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListUsersWithRoleNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "utils_user_list_users_with_role",
            "List Users with Role",
            "Lists project users assigned to a role ID or exact role name.",
        );
        node.set_flowscript_name("user", "listWithRole");
        add_app_pin(&mut node);
        node.add_input_pin(
            "role",
            "Role",
            "Role ID or exact role name. Leave empty to return all project users.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        add_pagination_pins(&mut node);
        add_users_output(&mut node);
        add_common_outputs(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let role_query = eval_string_pin(context, "role", "").await;
        run_user_list_node(context, |client, app_id, roles, offset, limit| {
            let role_query = role_query.clone();
            Box::pin(async move {
                filter_project_users(client, app_id, roles, offset, limit, |user| {
                    role_matches(user.role.as_ref(), &role_query)
                })
                .await
            })
        })
        .await
    }
}

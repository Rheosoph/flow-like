use super::*;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct SearchUsersNode {}

impl SearchUsersNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SearchUsersNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "utils_user_search_users",
            "Search Users",
            "Searches project users by exposed profile fields. Email is only searchable when the platform returns email in user lookup results.",
        );
        add_app_pin(&mut node);
        node.add_input_pin(
            "query",
            "Query",
            "Search text matched against project user ID, username, preferred username, name, visible email, or role name.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        add_pagination_pins(&mut node);
        add_users_output(&mut node);
        add_common_outputs(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let query = eval_string_pin(context, "query", "").await;
        run_user_list_node(context, |client, app_id, roles, offset, limit| {
            let query = query.clone();
            Box::pin(async move {
                search_project_users(client, app_id, roles, &query, offset, limit).await
            })
        })
        .await
    }
}

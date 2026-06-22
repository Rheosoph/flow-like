use super::*;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct ListProjectUsersNode {}

impl ListProjectUsersNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListProjectUsersNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "utils_user_list_project_users",
            "List Project Users",
            "Lists project users with pagination.",
        );
        add_app_pin(&mut node);
        add_pagination_pins(&mut node);
        add_users_output(&mut node);
        add_common_outputs(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_user_list_node(context, |client, app_id, roles, offset, limit| {
            Box::pin(async move {
                let (memberships, status) = client.memberships(app_id, offset, limit).await?;
                let has_more = memberships.len() == limit as usize;
                let (users, lookup_status) =
                    hydrate_project_users(client, memberships, roles).await?;
                Ok((users, has_more, status.max(lookup_status)))
            })
        })
        .await
    }
}

use super::*;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct GetProjectUserNode {}

impl GetProjectUserNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetProjectUserNode {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "utils_user_get_project_user",
            "Get Project User",
            "Gets a project user membership by user ID/sub.",
        );
        add_app_pin(&mut node);
        add_user_id_pin(&mut node);
        add_project_user_output(&mut node);
        add_common_outputs(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        run_single_project_user_node(context).await
    }
}

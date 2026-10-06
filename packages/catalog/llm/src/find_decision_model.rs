use flow_like::{
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
        variable::VariableType,
    },
};
#[cfg(feature = "execute")]
use flow_like::{models::device::Interaction, state::FlowLikeState};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct FindDecisionModelNode {}

impl FindDecisionModelNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for FindDecisionModelNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_systemone_find_model",
            "Find Decision Model",
            "Selects the first available SystemOne model in the active profile's model order.",
            "AI/Decisions",
        );
        node.set_flowscript_name("ai.systemone", "findModel");
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
        node.set_long_running(true);
        node.add_input_pin(
            "exec_in",
            "Input",
            "Find a decision model from the active profile",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Done",
            "Decision model selected",
            VariableType::Execution,
        );
        node.add_output_pin(
            "model",
            "Model",
            "A SystemOne Bit for the decision nodes",
            VariableType::Struct,
        )
        .set_schema::<Bit>();
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        flow_like_catalog_core::run_with_execute_gate!(context, {
            context.deactivate_exec_pin("exec_out").await?;
            let capabilities =
                FlowLikeState::completion_model_capabilities(&context.app_state).await;
            let devices = context.app_state.device_model_probe(Interaction::Allowed {
                run_label: Some(context.run_id().to_string()),
            });
            let bit = context
                .profile
                .find_decision_model(
                    capabilities,
                    devices.as_ref(),
                    context.app_state.http_client.clone(),
                )
                .await?;
            context
                .set_pin_value("model", flow_like_types::json::json!(bit))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        })
    }
}

use flow_like::{
    bit::{Bit, BitModelPreference},
    flow::{
        execution::{LogLevel, context::ExecutionContext},
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct FindLLMNode {}

impl FindLLMNode {
    pub fn new() -> Self {
        FindLLMNode {}
    }
}

#[async_trait]
impl NodeLogic for FindLLMNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_find_model",
            "Find Model",
            "Finds the best model based on certain selection criteria",
            "AI/Generative",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(9)
                .set_performance(8)
                .set_reliability(8)
                .set_governance(8)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger pin", VariableType::Execution);
        node.add_input_pin(
            "preferences",
            "Preferences",
            "Weights and requirements that guide model selection",
            VariableType::Struct,
        )
        .set_default_value(Some(json!(BitModelPreference::default())))
        .set_schema::<BitModelPreference>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Signals completion",
            VariableType::Execution,
        );
        node.add_output_pin(
            "model",
            "Model",
            "Bit describing the best-match model",
            VariableType::Struct,
        )
        .set_schema::<Bit>();

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut preference: BitModelPreference = context.evaluate_pin("preferences").await?;
        preference.enforce_bounds();

        let http_client = context.app_state.http_client.clone();
        let only_hosted =
            cfg!(target_os = "ios") || cfg!(target_os = "android") || !cfg!(feature = "tauri");
        let bit = context
            .profile
            .get_best_model_filtered(&preference, false, false, only_hosted, http_client)
            .await?;

        for meta in bit.meta.values() {
            context.log_message(
                &format!("Connected to model {}", meta.name),
                LogLevel::Debug,
            );
        }

        context
            .set_pin_value("model", flow_like_types::json::json!(bit))
            .await?;

        context.activate_exec_pin("exec_out").await?;

        return Ok(());
    }
}

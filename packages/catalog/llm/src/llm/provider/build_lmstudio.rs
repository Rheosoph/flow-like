use std::collections::HashMap;

use flow_like::{
    bit::{Bit, BitModelClassification, VLMParameters},
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        variable::VariableType,
    },
};
use flow_like_storage::blake3;
use flow_like_types::{
    async_trait,
    json::{json, to_value},
};

#[crate::register_node]
#[derive(Default)]
pub struct BuildLMStudioNode {}

impl BuildLMStudioNode {
    pub fn new() -> Self {
        BuildLMStudioNode {}
    }
}

#[async_trait]
impl NodeLogic for BuildLMStudioNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_build_lmstudio",
            "LM Studio Model",
            "Connects to a locally running LM Studio server via its OpenAI-compatible API",
            "AI/Generative/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(9)
                .set_performance(6)
                .set_governance(9)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to build or refresh the LM Studio Bit",
            VariableType::Execution,
        );

        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "LM Studio server URL (default: http://localhost:1234)",
            VariableType::String,
        )
        .set_default_value(Some(json!("http://localhost:1234")));

        node.add_input_pin(
            "model_id",
            "Model ID",
            "Model identifier as shown in LM Studio (e.g. lmstudio-community/gemma-3-12b)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Activated once the Bit is ready",
            VariableType::Execution,
        );
        node.add_output_pin(
            "model",
            "Model",
            "Structured Bit describing the LM Studio provider",
            VariableType::Struct,
        )
        .set_schema::<Bit>();

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut hasher = blake3::Hasher::new();
        hasher.update(b"lmstudio");

        let endpoint = context.evaluate_pin::<String>("endpoint").await?;
        let model_id = context.evaluate_pin::<String>("model_id").await?;

        let mut params = HashMap::new();
        params.insert("endpoint".to_string(), json!(endpoint));
        hasher.update(endpoint.as_bytes());

        if !model_id.is_empty() {
            params.insert("model_id".to_string(), json!(model_id.clone()));
            hasher.update(model_id.as_bytes());
        }

        let bit_hash = hasher.finalize().to_hex().to_string();

        let params_obj = VLMParameters {
            context_length: 20000,
            model_classification: BitModelClassification::default(),
            provider: flow_like_model_provider::provider::ModelProvider {
                provider_name: "custom:lmstudio".into(),
                model_id: if model_id.is_empty() {
                    None
                } else {
                    Some(model_id)
                },
                version: None,
                params: Some(params),
            },
        };
        let params = to_value(&params_obj).unwrap_or_default();

        let mut bit = Bit::default();
        bit.id = bit_hash;
        bit.bit_type = flow_like::bit::BitTypes::Vlm;
        bit.parameters = params;

        context
            .set_pin_value("model", flow_like_types::json::json!(bit))
            .await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

use std::collections::HashMap;

use flow_like::{
    bit::{Bit, BitModelClassification, VLMParameters},
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
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
pub struct BuildMiniMaxNode {}

impl BuildMiniMaxNode {
    pub fn new() -> Self {
        BuildMiniMaxNode {}
    }
}

#[async_trait]
impl NodeLogic for BuildMiniMaxNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_build_minimax",
            "MiniMax Model",
            "Prepares a Bit for MiniMax's OpenAI-compatible API using the provided credentials",
            "AI/Generative/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);

        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(7)
                .set_governance(4)
                .set_reliability(6)
                .set_cost(6)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to build/update the provider Bit",
            VariableType::Execution,
        );

        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "MiniMax OpenAI-compatible base URL (override only for a proxy)",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://api.minimax.io/v1")));

        node.add_input_pin(
            "api_key",
            "API Key",
            "MiniMax API key used for authentication",
            VariableType::String,
        )
        .set_default_value(Some(json!("")))
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "model_id",
            "Model ID",
            "MiniMax model identifier to request",
            VariableType::String,
        )
        .set_default_value(Some(json!("MiniMax-M3")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Fires when the Bit is ready",
            VariableType::Execution,
        );
        node.add_output_pin(
            "model",
            "Model",
            "Bit containing the provider configuration",
            VariableType::Struct,
        )
        .set_schema::<Bit>();

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut hasher = blake3::Hasher::new();
        hasher.update(b"minimax");

        let api_key = context.evaluate_pin::<String>("api_key").await?;
        let endpoint = context.evaluate_pin::<String>("endpoint").await?;
        let model_id = context.evaluate_pin::<String>("model_id").await?;

        let mut params = HashMap::new();
        params.insert("api_key".to_string(), json!(api_key));
        hasher.update(api_key.as_bytes());
        params.insert("endpoint".to_string(), json!(endpoint));
        hasher.update(endpoint.as_bytes());

        if !model_id.is_empty() {
            params.insert("model_id".to_string(), json!(model_id.clone()));
            hasher.update(model_id.as_bytes());
        }

        let bit_hash = hasher.finalize().to_hex().to_string();

        // MiniMax is OpenAI-compatible, so it reuses the existing
        // `custom:openai` model path (honors the `endpoint` / `api_key` /
        // `model_id` params) — no new model client is required.
        let params_obj = VLMParameters {
            context_length: 20000,
            model_classification: BitModelClassification::default(),
            provider: flow_like_model_provider::provider::ModelProvider {
                provider_name: "custom:openai".into(),
                model_id: Some(model_id),
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

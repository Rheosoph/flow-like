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
pub struct BuildMozillaNode {}

impl BuildMozillaNode {
    pub fn new() -> Self {
        BuildMozillaNode {}
    }
}

#[async_trait]
impl NodeLogic for BuildMozillaNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_build_mozilla",
            "Mozilla any-llm Model",
            "Builds a model via the Mozilla any-llm gateway (OpenAI-compatible). Supports both self-hosted gateways and the managed platform at any-llm.ai",
            "AI/Generative/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(3);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(6)
                .set_cost(6)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that builds or refreshes the Mozilla any-llm Bit",
            VariableType::Execution,
        );

        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Mozilla any-llm gateway base URL (e.g. http://localhost:8000/v1 for self-hosted or https://api.any-llm.ai/v1 for managed platform)",
            VariableType::String,
        )
        .set_default_value(Some(json!("http://localhost:8000/v1")));

        node.add_input_pin(
            "api_key",
            "API Key",
            "API key for authenticating against the any-llm gateway or platform",
            VariableType::String,
        )
        .set_default_value(Some(json!("")))
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "model_id",
            "Model ID",
            "Model identifier in provider:model format (e.g. openai:gpt-4o, anthropic:claude-sonnet-4-20250514)",
            VariableType::String,
        )
        .set_default_value(Some(json!("openai:gpt-4o")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Activated once the Bit is ready",
            VariableType::Execution,
        );
        node.add_output_pin(
            "model",
            "Model",
            "Structured Bit describing the Mozilla any-llm provider",
            VariableType::Struct,
        )
        .set_schema::<Bit>();

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut hasher = blake3::Hasher::new();
        hasher.update(b"mozilla");

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

        let params_obj = VLMParameters {
            context_length: 20000,
            model_classification: BitModelClassification::default(),
            provider: flow_like_model_provider::provider::ModelProvider {
                provider_name: "custom:mozilla".into(),
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

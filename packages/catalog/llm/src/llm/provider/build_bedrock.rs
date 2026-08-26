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
pub struct BuildBedrockNode {}

impl BuildBedrockNode {
    pub fn new() -> Self {
        BuildBedrockNode {}
    }
}

#[async_trait]
impl NodeLogic for BuildBedrockNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_build_bedrock",
            "AWS Bedrock Model",
            "Prepares a Bit for AWS Bedrock model endpoints",
            "AI/Generative/Provider",
        );
        node.set_flowscript_name("ai.provider", "bedrock");
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(3);

        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(6)
                .set_governance(6)
                .set_reliability(6)
                .set_cost(4)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that builds or refreshes the AWS Bedrock Bit",
            VariableType::Execution,
        );

        node.add_input_pin(
            "region",
            "Region",
            "AWS Bedrock runtime region",
            VariableType::String,
        )
        .set_default_value(Some(json!("us-east-1")));

        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Optional Bedrock Runtime endpoint override. Leave empty to derive from region.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "api_key",
            "API Key",
            "Credential used for Bedrock runtime requests",
            VariableType::String,
        )
        .set_default_value(Some(json!("")))
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "model_id",
            "Model ID",
            "AWS Bedrock model identifier",
            VariableType::String,
        )
        .set_default_value(Some(json!("amazon.titan-image-generator-v2:0")));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Activated once the Bit is ready",
            VariableType::Execution,
        );
        node.add_output_pin(
            "model",
            "Model",
            "Structured Bit describing the AWS Bedrock provider",
            VariableType::Struct,
        )
        .set_schema::<Bit>();

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut hasher = blake3::Hasher::new();
        hasher.update(b"bedrock");

        let api_key = context.evaluate_pin::<String>("api_key").await?;
        let region = context.evaluate_pin::<String>("region").await?;
        let endpoint = context.evaluate_pin::<String>("endpoint").await?;
        let model_id = context.evaluate_pin::<String>("model_id").await?;

        let mut params = HashMap::new();
        params.insert("api_key".to_string(), json!(api_key));
        hasher.update(api_key.as_bytes());

        if !region.is_empty() {
            params.insert("region".to_string(), json!(region));
            hasher.update(region.as_bytes());
        }

        if !endpoint.is_empty() {
            params.insert("endpoint".to_string(), json!(endpoint));
            hasher.update(endpoint.as_bytes());
        }

        if !model_id.is_empty() {
            params.insert("model_id".to_string(), json!(model_id.clone()));
            hasher.update(model_id.as_bytes());
        }

        let bit_hash = hasher.finalize().to_hex().to_string();

        let params_obj = VLMParameters {
            context_length: 20000,
            model_classification: BitModelClassification::default(),
            provider: flow_like_model_provider::provider::ModelProvider {
                provider_name: "custom:bedrock".into(),
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

        context.set_pin_value("model", json!(bit)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

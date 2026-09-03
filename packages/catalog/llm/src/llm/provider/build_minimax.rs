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

const GLOBAL_REGION: &str = "Global";
const CHINA_REGION: &str = "China";
const GLOBAL_ENDPOINT: &str = "https://api.minimax.io/v1";
const CHINA_ENDPOINT: &str = "https://api.minimaxi.com/v1";
const DEFAULT_MODEL_ID: &str = "MiniMax-M3";
const SUPPORTED_MODEL_IDS: [&str; 2] = [DEFAULT_MODEL_ID, "MiniMax-M2.7"];

fn endpoint_for_region(region: &str) -> &'static str {
    match region {
        CHINA_REGION => CHINA_ENDPOINT,
        _ => GLOBAL_ENDPOINT,
    }
}

fn context_length_for_model(model_id: &str) -> u32 {
    match model_id {
        "MiniMax-M2.7" => 204_800,
        _ => 1_000_000,
    }
}

fn is_multimodal_model(model_id: &str) -> bool {
    model_id == DEFAULT_MODEL_ID
}

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
            "Prepares a Bit for the MiniMax API using the provided credentials",
            "AI/Generative/Provider",
        );
        node.set_flowscript_name("ai.provider", "minimax");
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(2);

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
            "region",
            "Region",
            "MiniMax API region used when no custom endpoint is provided",
            VariableType::String,
        )
        .set_default_value(Some(json!(GLOBAL_REGION)))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![GLOBAL_REGION.into(), CHINA_REGION.into()])
                .build(),
        );

        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Optional MiniMax API base URL override for a proxy",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

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
        .set_default_value(Some(json!(DEFAULT_MODEL_ID)))
        .set_options(
            PinOptions::new()
                .set_valid_values(
                    SUPPORTED_MODEL_IDS
                        .iter()
                        .map(|value| (*value).into())
                        .collect(),
                )
                .build(),
        );

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
        let region = context.evaluate_pin::<String>("region").await?;
        let endpoint_override = context.evaluate_pin::<String>("endpoint").await?;
        let model_id = context.evaluate_pin::<String>("model_id").await?;
        let endpoint = if endpoint_override.trim().is_empty() {
            endpoint_for_region(&region).to_string()
        } else {
            endpoint_override
        };

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
            context_length: context_length_for_model(&model_id),
            model_classification: BitModelClassification::default(),
            provider: flow_like_model_provider::provider::ModelProvider {
                api_surface: None,
                provider_name: "custom:openai".into(),
                model_id: Some(model_id.clone()),
                version: None,
                params: Some(params),
            },
        };
        let params = to_value(&params_obj).unwrap_or_default();

        let mut bit = Bit::default();
        bit.id = bit_hash;
        bit.bit_type = if is_multimodal_model(&model_id) {
            flow_like::bit::BitTypes::Vlm
        } else {
            flow_like::bit::BitTypes::Llm
        };
        bit.parameters = params;

        context
            .set_pin_value("model", flow_like_types::json::json!(bit))
            .await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_current_models_and_regions() {
        let node = BuildMiniMaxNode::new().get_node();
        let region = node.get_pin_by_name("region").expect("region pin");
        let endpoint = node.get_pin_by_name("endpoint").expect("endpoint pin");
        let model = node.get_pin_by_name("model_id").expect("model pin");

        assert_eq!(node.version, Some(2));
        assert_eq!(
            region
                .options
                .as_ref()
                .and_then(|options| options.valid_values.as_ref()),
            Some(&vec![GLOBAL_REGION.into(), CHINA_REGION.into()])
        );
        assert_eq!(
            endpoint.default_value,
            Some(flow_like_types::json::to_vec(&json!("")).expect("endpoint default"))
        );
        assert_eq!(
            model
                .options
                .as_ref()
                .and_then(|options| options.valid_values.as_ref()),
            Some(&SUPPORTED_MODEL_IDS.map(String::from).to_vec())
        );
    }

    #[test]
    fn maps_model_and_region_metadata() {
        assert_eq!(endpoint_for_region(GLOBAL_REGION), GLOBAL_ENDPOINT);
        assert_eq!(endpoint_for_region(CHINA_REGION), CHINA_ENDPOINT);
        assert_eq!(context_length_for_model(DEFAULT_MODEL_ID), 1_000_000);
        assert_eq!(context_length_for_model("MiniMax-M2.7"), 204_800);
        assert!(is_multimodal_model(DEFAULT_MODEL_ID));
        assert!(!is_multimodal_model("MiniMax-M2.7"));
    }
}

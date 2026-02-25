use std::{collections::HashMap, sync::Arc};

use flow_like::{
    bit::{Bit, BitModelClassification, VLMParameters},
    flow::{
        board::Board,
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_storage::blake3;
use flow_like_types::{
    Value, async_trait,
    json::{json, to_value},
};

#[crate::register_node]
#[derive(Default)]
pub struct BuildOpenAiNode {}

impl BuildOpenAiNode {
    pub fn new() -> Self {
        BuildOpenAiNode {}
    }
}

#[async_trait]
impl NodeLogic for BuildOpenAiNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_build_openai",
            "OpenAI Model",
            "Prepares a Bit for OpenAI or Azure OpenAI endpoints with the provided credentials",
            "AI/Generative/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);

        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(4)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to build/update the provider Bit",
            VariableType::Execution,
        );
        node.add_input_pin(
            "provider",
            "Provider",
            "Choose OpenAI cloud or Azure OpenAI",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["OpenAI".into(), "Azure".into()])
                .build(),
        )
        .set_default_value(Some(json!("OpenAI")));

        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Base API endpoint (override for Azure or proxies)",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://api.openai.com/v1/")));

        node.add_input_pin(
            "api_key",
            "API Key",
            "API key or Azure key used for authentication",
            VariableType::String,
        )
        .set_default_value(Some(json!("")))
        .set_options(PinOptions::new().set_sensitive(true).build());

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

        let provider: String = context.evaluate_pin("provider").await?;

        hasher.update(provider.as_bytes());

        let api_key = context.evaluate_pin::<String>("api_key").await?;
        let endpoint = context.evaluate_pin::<String>("endpoint").await?;

        let mut params = HashMap::new();
        params.insert("api_key".to_string(), json!(api_key));
        hasher.update(api_key.as_bytes());
        params.insert("endpoint".to_string(), json!(endpoint));
        hasher.update(endpoint.as_bytes());

        if provider.to_lowercase() == "azure" {
            params.insert("is_azure".to_string(), json!(true));
            hasher.update(b"azure");
        }

        if let Ok(model_id) = context.evaluate_pin::<String>("model_id").await
            && !model_id.is_empty()
        {
            params.insert("model_id".to_string(), json!(model_id));
            hasher.update(model_id.as_bytes());
        }

        if let Ok(version) = context.evaluate_pin::<String>("version").await
            && !version.is_empty()
        {
            params.insert("version".to_string(), json!(version));
            hasher.update(version.as_bytes());
        }

        let bit_hash = hasher.finalize().to_hex().to_string();

        let model_id_value = context
            .evaluate_pin::<String>("model_id")
            .await
            .unwrap_or_default();
        let model_id = if model_id_value.is_empty() {
            None
        } else {
            Some(model_id_value)
        };

        let params = VLMParameters {
            context_length: 20000,
            model_classification: BitModelClassification::default(),
            provider: flow_like_model_provider::provider::ModelProvider {
                provider_name: "custom:openai".into(),
                model_id,
                version: {
                    let v = context
                        .evaluate_pin::<String>("version")
                        .await
                        .unwrap_or_default();
                    if v.is_empty() { None } else { Some(v) }
                },
                params: Some(params),
            },
        };
        let params = to_value(&params).unwrap_or_default();

        let mut bit = Bit::default();
        bit.id = bit_hash;
        bit.bit_type = flow_like::bit::BitTypes::Vlm;
        bit.parameters = params;

        context
            .set_pin_value("model", flow_like_types::json::json!(bit))
            .await?;

        context.activate_exec_pin("exec_out").await?;

        return Ok(());
    }

    async fn on_update(&self, node: &mut Node, _board: Arc<Board>) {
        let provider_pin: String = node
            .get_pin_by_name("provider")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        let model_id_pin_id: Option<String> = node
            .get_pin_by_name("model_id")
            .map(|p| p.id.clone());
        let version_pin_id: Option<String> = node
            .get_pin_by_name("version")
            .map(|p| p.id.clone());

        // Drop borrows before mutating
        let has_model_id = model_id_pin_id.is_some();
        let has_version = version_pin_id.is_some();
        let version_id_to_remove = version_pin_id;

        match provider_pin.as_str() {
            "OpenAI" => {
                if let Some(id) = version_id_to_remove {
                    node.pins.remove(&id);
                }
                if !has_model_id {
                    node.add_input_pin(
                        "model_id",
                        "Model ID",
                        "OpenAI Model ID (optional, leave empty to use provider default)",
                        VariableType::String,
                    )
                    .set_default_value(Some(json!("")));
                }
            }
            "Azure" => {
                if !has_model_id {
                    node.add_input_pin(
                        "model_id",
                        "Model ID",
                        "Azure Model ID",
                        VariableType::String,
                    )
                    .set_default_value(Some(json!("")));
                }
                if !has_version {
                    node.add_input_pin(
                        "version",
                        "Version",
                        "Azure API Version",
                        VariableType::String,
                    )
                    .set_default_value(Some(json!("2024-12-01-preview")));
                }
            }
            _ => {}
        }
    }
}

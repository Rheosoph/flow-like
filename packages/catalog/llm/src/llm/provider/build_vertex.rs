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
pub struct BuildVertexNode {}

impl BuildVertexNode {
    pub fn new() -> Self {
        BuildVertexNode {}
    }
}

#[async_trait]
impl NodeLogic for BuildVertexNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_build_vertex",
            "Vertex AI Model",
            "Prepares a Bit for Google Vertex AI Gemini endpoints using ADC or service account credentials",
            "AI/Generative/Provider",
        );
        node.set_flowscript_name("ai.provider", "vertex");
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(3);

        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(7)
                .set_governance(6)
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
            "project_id",
            "Project ID",
            "Google Cloud project ID. Leave empty to use GOOGLE_CLOUD_PROJECT or the service account project_id.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "location",
            "Location",
            "Vertex AI location",
            VariableType::String,
        )
        .set_default_value(Some(json!("global")));

        node.add_input_pin(
            "service_account_json",
            "Service Account JSON",
            "Optional Google Cloud service account key JSON. Leave empty to use Application Default Credentials.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")))
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "access_token",
            "Access Token",
            "Optional OAuth access token. Prefer ADC or a service account for long-running flows.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")))
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "model_id",
            "Model ID",
            "Vertex AI Gemini model identifier",
            VariableType::String,
        )
        .set_default_value(Some(json!("gemini-2.5-flash")));

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
        hasher.update(b"vertex");

        let project_id = context.evaluate_pin::<String>("project_id").await?;
        let location = context.evaluate_pin::<String>("location").await?;
        let service_account_json = context
            .evaluate_pin::<String>("service_account_json")
            .await?;
        let access_token = context.evaluate_pin::<String>("access_token").await?;
        let model_id = context.evaluate_pin::<String>("model_id").await?;

        if service_account_json.trim().is_empty() && access_token.trim().is_empty() {
            context
                .execution_environment()
                .ensure_no_ambient_credentials("custom:vertex", "application_default")?;
        }

        let mut params = HashMap::new();

        if !project_id.is_empty() {
            params.insert("project_id".to_string(), json!(project_id));
            hasher.update(project_id.as_bytes());
        }

        if !location.is_empty() {
            params.insert("location".to_string(), json!(location));
            hasher.update(location.as_bytes());
        }

        if !service_account_json.is_empty() {
            params.insert(
                "service_account_json".to_string(),
                json!(service_account_json),
            );
            hasher.update(service_account_json.as_bytes());
        }

        if !access_token.is_empty() {
            params.insert("access_token".to_string(), json!(access_token));
            hasher.update(access_token.as_bytes());
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
                api_surface: None,
                provider_name: "custom:vertex".into(),
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

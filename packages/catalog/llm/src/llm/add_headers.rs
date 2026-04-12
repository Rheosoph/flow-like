use std::collections::HashMap;

use flow_like::{
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_storage::blake3;
use flow_like_types::{Value, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

#[crate::register_node]
#[derive(Default)]
pub struct AddModelHeadersNode {}

impl AddModelHeadersNode {
    pub fn new() -> Self {
        AddModelHeadersNode {}
    }
}

#[async_trait]
impl NodeLogic for AddModelHeadersNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_add_headers",
            "Add Model Headers",
            "Adds custom HTTP headers to a model for use with custom API endpoints",
            "AI/Generative",
        );
        node.add_icon("/flow/icons/settings.svg");
        node.set_version(1);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(9)
                .set_governance(5)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Model to add headers to",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "header",
            "Header",
            "HTTP header to add (name-value pair)",
            VariableType::Struct,
        )
        .set_schema::<HttpHeader>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!(HttpHeader::default())));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Fires when the model is configured",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model_out",
            "Model",
            "Model with custom headers applied",
            VariableType::Struct,
        )
        .set_schema::<Bit>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let model: Bit = context.evaluate_pin("model").await?;

        let header_pins = context.get_pins_by_name("header").await?;
        let mut headers_map: HashMap<String, Value> = HashMap::new();

        for pin in header_pins {
            let header: HttpHeader = context.evaluate_pin_ref(pin).await?;
            if !header.name.is_empty() && !header.value.is_empty() {
                headers_map.insert(header.name, json!(header.value));
            }
        }

        if headers_map.is_empty() {
            context.set_pin_value("model_out", json!(model)).await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        let mut new_bit = model.clone();
        let mut hasher = blake3::Hasher::new();
        hasher.update(model.id.as_bytes());

        if let Some(params) = new_bit.parameters.as_object_mut()
            && let Some(provider) = params.get_mut("provider").and_then(|p| p.as_object_mut())
        {
            if let Some(inner_params) = provider.get_mut("params").and_then(|p| p.as_object_mut()) {
                let existing_headers = inner_params
                    .get("headers")
                    .and_then(|h| h.as_object())
                    .cloned()
                    .unwrap_or_default();

                let mut merged_headers = existing_headers;
                for (k, v) in &headers_map {
                    hasher.update(k.as_bytes());
                    if let Some(s) = v.as_str() {
                        hasher.update(s.as_bytes());
                    }
                    merged_headers.insert(k.clone(), v.clone());
                }

                inner_params.insert("headers".to_string(), json!(merged_headers));
            } else {
                let mut new_params: HashMap<String, Value> = HashMap::new();
                for (k, v) in &headers_map {
                    hasher.update(k.as_bytes());
                    if let Some(s) = v.as_str() {
                        hasher.update(s.as_bytes());
                    }
                }
                new_params.insert("headers".to_string(), json!(headers_map));
                provider.insert("params".to_string(), json!(new_params));
            }
        }

        new_bit.id = hasher.finalize().to_hex().to_string();

        context.set_pin_value("model_out", json!(new_bit)).await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

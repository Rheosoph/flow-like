use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

use crate::web::api::request::encode_form_fields;
use crate::web::api::{HttpBody, HttpRequest};

#[crate::register_node]
#[derive(Default)]
pub struct SetFormBodyNode {}

impl SetFormBodyNode {
    pub fn new() -> Self {
        SetFormBodyNode {}
    }
}

#[async_trait]
impl NodeLogic for SetFormBodyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "http_set_form_body",
            "Set Form Body",
            "Sets the body of a http request to form-encoded data",
            "Web/API/Request",
        );
        node.add_icon("/flow/icons/web.svg");

        node.add_input_pin(
            "request",
            "Request",
            "The http request",
            VariableType::Struct,
        )
        .set_schema::<HttpRequest>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "fields",
            "Fields",
            "Form fields to encode",
            VariableType::Struct,
        )
        .set_default_value(Some(json!({})))
        .set_open_schema();

        node.add_input_pin(
            "set_content_type",
            "Set Content-Type",
            "Adds application/x-www-form-urlencoded when missing",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "request_out",
            "Request",
            "The http request",
            VariableType::Struct,
        )
        .set_schema::<HttpRequest>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut request: HttpRequest = context.evaluate_pin("request").await?;
        let fields: Value = context.evaluate_pin("fields").await?;
        let set_content_type: bool = context.evaluate_pin("set_content_type").await?;

        let fields_map: std::collections::HashMap<String, String> = fields
            .as_object()
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| {
                        let value = match v {
                            Value::String(s) => s.clone(),
                            _ => v.to_string(),
                        };
                        (k.clone(), value)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let encoded = encode_form_fields(&fields_map);
        request.body = Some(HttpBody::String(encoded));

        if set_content_type {
            let has_content_type = request
                .headers
                .as_ref()
                .and_then(|headers| headers.get("Content-Type"))
                .is_some();
            if !has_content_type {
                request.set_header(
                    "Content-Type".to_string(),
                    "application/x-www-form-urlencoded".to_string(),
                );
            }
        }

        context.set_pin_value("request_out", json!(request)).await?;

        Ok(())
    }
}

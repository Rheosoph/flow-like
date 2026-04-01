use crate::types::fingerprints::ElementFingerprint;
use crate::types::selectors::SelectorSet;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::BoundingBox;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct CreateFingerprintNode {}

impl CreateFingerprintNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateFingerprintNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fingerprint_create",
            "Create Fingerprint",
            "Creates a new element fingerprint for identification",
            "Automation/Fingerprint",
        );
        node.add_icon("/flow/icons/fingerprint.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "id",
            "ID",
            "Unique identifier for the fingerprint",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "selectors",
            "Selectors",
            "Selector set for element location",
            VariableType::Struct,
        )
        .set_schema::<SelectorSet>()
        .set_default_value(Some(json!(SelectorSet::default())));

        node.add_input_pin(
            "role",
            "Role",
            "ARIA role of the element",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "name",
            "Name",
            "Accessible name of the element",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin("text", "Text", "Visible text content", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_input_pin(
            "bounding_box",
            "BBox",
            "Bounding box of the element (x1, y1, x2, y2)",
            VariableType::Struct,
        )
        .set_schema::<BoundingBox>()
        .set_default_value(Some(json!(BoundingBox::default())));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "fingerprint",
            "Fingerprint",
            "Created element fingerprint",
            VariableType::Struct,
        )
        .set_schema::<ElementFingerprint>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let id: String = context.evaluate_pin("id").await?;
        let selectors: SelectorSet = context.evaluate_pin("selectors").await?;
        let role: String = context.evaluate_pin("role").await?;
        let name: String = context.evaluate_pin("name").await?;
        let text: String = context.evaluate_pin("text").await?;

        let fingerprint_id = if id.is_empty() {
            flow_like_types::create_id()
        } else {
            id
        };

        let bbox: BoundingBox = context
            .evaluate_pin("bounding_box")
            .await
            .unwrap_or_default();

        let mut fingerprint = ElementFingerprint::new(fingerprint_id).with_selectors(selectors);

        if !role.is_empty() {
            fingerprint = fingerprint.with_role(role);
        }
        if !name.is_empty() {
            fingerprint = fingerprint.with_name(name);
        }
        if !text.is_empty() {
            fingerprint = fingerprint.with_text(text);
        }

        if bbox.area() > 0.0 {
            fingerprint = fingerprint.with_bounding_box(bbox);
        }

        context
            .set_pin_value("fingerprint", json!(fingerprint))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateFingerprintFromJsonNode {}

impl CreateFingerprintFromJsonNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateFingerprintFromJsonNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fingerprint_from_json",
            "Fingerprint From JSON",
            "Parses an element fingerprint from JSON",
            "Automation/Fingerprint",
        );
        node.add_icon("/flow/icons/fingerprint.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "json",
            "JSON",
            "JSON string containing fingerprint data",
            VariableType::String,
        )
        .set_default_value(Some(json!("{}")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "exec_error",
            "Error",
            "Parse error occurred",
            VariableType::Execution,
        );

        node.add_output_pin(
            "fingerprint",
            "Fingerprint",
            "Parsed element fingerprint",
            VariableType::Struct,
        )
        .set_schema::<ElementFingerprint>();

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error message if parsing failed",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let json_str: String = context.evaluate_pin("json").await?;

        match flow_like_types::json::from_str::<ElementFingerprint>(&json_str) {
            Ok(fingerprint) => {
                context
                    .set_pin_value("fingerprint", json!(fingerprint))
                    .await?;
                context.set_pin_value("error_message", json!("")).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("exec_error").await?;
            }
        }

        Ok(())
    }
}

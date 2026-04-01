use crate::types::fingerprints::ElementFingerprint;
use crate::types::selectors::SelectorSet;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct UpdateFingerprintNode {}

impl UpdateFingerprintNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateFingerprintNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fingerprint_update",
            "Update Fingerprint",
            "Updates an existing fingerprint with new data",
            "Automation/Fingerprint",
        );
        node.add_icon("/flow/icons/fingerprint.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "fingerprint",
            "Fingerprint",
            "Fingerprint to update",
            VariableType::Struct,
        )
        .set_schema::<ElementFingerprint>();

        node.add_input_pin(
            "selectors",
            "Selectors",
            "New selector set (optional)",
            VariableType::Struct,
        )
        .set_schema::<SelectorSet>();

        node.add_input_pin(
            "role",
            "Role",
            "New role (empty to keep existing)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "name",
            "Name",
            "New name (empty to keep existing)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "text",
            "Text",
            "New text (empty to keep existing)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "updated_fingerprint",
            "Updated Fingerprint",
            "Updated element fingerprint",
            VariableType::Struct,
        )
        .set_schema::<ElementFingerprint>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut fingerprint: ElementFingerprint = context.evaluate_pin("fingerprint").await?;
        let role: String = context.evaluate_pin("role").await?;
        let name: String = context.evaluate_pin("name").await?;
        let text: String = context.evaluate_pin("text").await?;

        if let Ok(selectors) = context.evaluate_pin::<SelectorSet>("selectors").await
            && !selectors.selectors.is_empty()
        {
            fingerprint.selectors = selectors;
        }

        if !role.is_empty() {
            fingerprint.role = Some(role);
        }
        if !name.is_empty() {
            fingerprint.name = Some(name);
        }
        if !text.is_empty() {
            fingerprint.text = Some(text);
        }

        context
            .set_pin_value("updated_fingerprint", json!(fingerprint))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RecordFingerprintMatchNode {}

impl RecordFingerprintMatchNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RecordFingerprintMatchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fingerprint_record_match",
            "Record Fingerprint Match",
            "Records that a fingerprint was successfully matched",
            "Automation/Fingerprint",
        );
        node.add_icon("/flow/icons/fingerprint.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "fingerprint",
            "Fingerprint",
            "Fingerprint that was matched",
            VariableType::Struct,
        )
        .set_schema::<ElementFingerprint>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "updated_fingerprint",
            "Updated Fingerprint",
            "Fingerprint with updated match stats",
            VariableType::Struct,
        )
        .set_schema::<ElementFingerprint>();

        node.add_output_pin(
            "match_count",
            "Match Count",
            "Total times this fingerprint has matched",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut fingerprint: ElementFingerprint = context.evaluate_pin("fingerprint").await?;

        fingerprint.record_match();

        context
            .set_pin_value("match_count", json!(fingerprint.match_count as i64))
            .await?;
        context
            .set_pin_value("updated_fingerprint", json!(fingerprint))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FingerprintToJsonNode {}

impl FingerprintToJsonNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for FingerprintToJsonNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fingerprint_to_json",
            "Fingerprint To JSON",
            "Serializes an element fingerprint to JSON",
            "Automation/Fingerprint",
        );
        node.add_icon("/flow/icons/fingerprint.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "fingerprint",
            "Fingerprint",
            "Fingerprint to serialize",
            VariableType::Struct,
        )
        .set_schema::<ElementFingerprint>();

        node.add_input_pin(
            "pretty",
            "Pretty",
            "Use pretty formatting",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin("json", "JSON", "JSON string", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let fingerprint: ElementFingerprint = context.evaluate_pin("fingerprint").await?;
        let pretty: bool = context.evaluate_pin("pretty").await?;

        let json_str = if pretty {
            flow_like_types::json::to_string_pretty(&fingerprint)?
        } else {
            flow_like_types::json::to_string(&fingerprint)?
        };

        context.set_pin_value("json", json!(json_str)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

use crate::types::fingerprints::ElementFingerprint;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ComputeFingerprintHashNode {}

impl ComputeFingerprintHashNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputeFingerprintHashNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fingerprint_compute_hash",
            "Compute Fingerprint Hash",
            "Computes a hash for fingerprint comparison",
            "Automation/Fingerprint",
        );
        node.add_icon("/flow/icons/fingerprint.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "fingerprint",
            "Fingerprint",
            "Fingerprint to hash",
            VariableType::Struct,
        )
        .set_schema::<ElementFingerprint>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin("hash", "Hash", "Computed hash string", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        context.deactivate_exec_pin("exec_out").await?;

        let fingerprint: ElementFingerprint = context.evaluate_pin("fingerprint").await?;

        let mut hasher = DefaultHasher::new();

        if let Some(ref role) = fingerprint.role {
            role.hash(&mut hasher);
        }
        if let Some(ref name) = fingerprint.name {
            name.hash(&mut hasher);
        }
        if let Some(ref text) = fingerprint.text {
            text.hash(&mut hasher);
        }
        if let Some(ref tag_name) = fingerprint.tag_name {
            tag_name.hash(&mut hasher);
        }

        for selector in &fingerprint.selectors.selectors {
            selector.value.hash(&mut hasher);
        }

        let hash = format!("{:016x}", hasher.finish());

        context.set_pin_value("hash", json!(hash)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ExtractFingerprintDataNode {}

impl ExtractFingerprintDataNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ExtractFingerprintDataNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fingerprint_extract_data",
            "Extract Fingerprint Data",
            "Extracts individual fields from a fingerprint",
            "Automation/Fingerprint",
        );
        node.add_icon("/flow/icons/fingerprint.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "fingerprint",
            "Fingerprint",
            "Fingerprint to extract from",
            VariableType::Struct,
        )
        .set_schema::<ElementFingerprint>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin("id", "ID", "Fingerprint ID", VariableType::String);
        node.add_output_pin("role", "Role", "Element role", VariableType::String);
        node.add_output_pin("name", "Name", "Element name", VariableType::String);
        node.add_output_pin("text", "Text", "Element text", VariableType::String);
        node.add_output_pin(
            "tag_name",
            "Tag Name",
            "HTML tag name",
            VariableType::String,
        );
        node.add_output_pin(
            "selector_count",
            "Selector Count",
            "Number of selectors",
            VariableType::Integer,
        );
        node.add_output_pin(
            "match_count",
            "Match Count",
            "Times fingerprint was matched",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let fingerprint: ElementFingerprint = context.evaluate_pin("fingerprint").await?;

        context.set_pin_value("id", json!(fingerprint.id)).await?;
        context
            .set_pin_value("role", json!(fingerprint.role.unwrap_or_default()))
            .await?;
        context
            .set_pin_value("name", json!(fingerprint.name.unwrap_or_default()))
            .await?;
        context
            .set_pin_value("text", json!(fingerprint.text.unwrap_or_default()))
            .await?;
        context
            .set_pin_value("tag_name", json!(fingerprint.tag_name.unwrap_or_default()))
            .await?;
        context
            .set_pin_value(
                "selector_count",
                json!(fingerprint.selectors.selectors.len() as i64),
            )
            .await?;
        context
            .set_pin_value("match_count", json!(fingerprint.match_count as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

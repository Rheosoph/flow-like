use crate::types::fingerprints::{ElementFingerprint, FingerprintMatchOptions, MatchStrategy};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct CreateMatchOptionsNode {}

impl CreateMatchOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateMatchOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fingerprint_match_options",
            "Match Options",
            "Creates fingerprint matching options",
            "Automation/Fingerprint",
        );
        node.add_icon("/flow/icons/fingerprint.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "strategy",
            "Strategy",
            "Matching strategy to use",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Dom".to_string(),
                    "Accessibility".to_string(),
                    "Vision".to_string(),
                    "Hybrid".to_string(),
                    "LlmAssisted".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Hybrid")));

        node.add_input_pin(
            "min_confidence",
            "Min Confidence",
            "Minimum confidence threshold (0.0-1.0)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.8)));

        node.add_input_pin(
            "max_fallback_attempts",
            "Max Fallback Attempts",
            "Maximum number of fallback attempts",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(3)));

        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "Maximum time to search",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10000)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "options",
            "Options",
            "Fingerprint match options",
            VariableType::Struct,
        )
        .set_schema::<FingerprintMatchOptions>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let strategy_str: String = context.evaluate_pin("strategy").await?;
        let min_confidence: f64 = context.evaluate_pin("min_confidence").await?;
        let max_fallback_attempts: i64 = context.evaluate_pin("max_fallback_attempts").await?;
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;

        let strategy = match strategy_str.as_str() {
            "Dom" => MatchStrategy::Dom,
            "Accessibility" => MatchStrategy::Accessibility,
            "Vision" => MatchStrategy::Vision,
            "LlmAssisted" => MatchStrategy::LlmAssisted,
            _ => MatchStrategy::Hybrid,
        };

        let options = FingerprintMatchOptions {
            strategy,
            min_confidence,
            max_fallback_attempts: max_fallback_attempts as u32,
            timeout_ms: timeout_ms as u64,
            search_region: None,
        };

        context.set_pin_value("options", json!(options)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CompareFingerprintsNode {}

impl CompareFingerprintsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CompareFingerprintsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fingerprint_compare",
            "Compare Fingerprints",
            "Compares two fingerprints and calculates similarity",
            "Automation/Fingerprint",
        );
        node.add_icon("/flow/icons/fingerprint.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "fingerprint_a",
            "Fingerprint A",
            "First fingerprint",
            VariableType::Struct,
        )
        .set_schema::<ElementFingerprint>();

        node.add_input_pin(
            "fingerprint_b",
            "Fingerprint B",
            "Second fingerprint",
            VariableType::Struct,
        )
        .set_schema::<ElementFingerprint>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "similarity",
            "Similarity",
            "Similarity score (0.0-1.0)",
            VariableType::Float,
        );

        node.add_output_pin(
            "is_match",
            "Is Match",
            "Whether fingerprints likely match the same element",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let fp_a: ElementFingerprint = context.evaluate_pin("fingerprint_a").await?;
        let fp_b: ElementFingerprint = context.evaluate_pin("fingerprint_b").await?;

        let similarity = calculate_fingerprint_similarity(&fp_a, &fp_b);
        let is_match = similarity >= 0.8;

        context
            .set_pin_value("similarity", json!(similarity))
            .await?;
        context.set_pin_value("is_match", json!(is_match)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

fn calculate_fingerprint_similarity(a: &ElementFingerprint, b: &ElementFingerprint) -> f64 {
    let mut score = 0.0;
    let mut weight_sum = 0.0;

    if a.role.is_some() && b.role.is_some() {
        weight_sum += 2.0;
        if a.role == b.role {
            score += 2.0;
        }
    }

    if a.name.is_some() && b.name.is_some() {
        weight_sum += 3.0;
        if a.name == b.name {
            score += 3.0;
        }
    }

    if a.text.is_some() && b.text.is_some() {
        weight_sum += 2.0;
        if a.text == b.text {
            score += 2.0;
        }
    }

    if a.tag_name.is_some() && b.tag_name.is_some() {
        weight_sum += 1.0;
        if a.tag_name == b.tag_name {
            score += 1.0;
        }
    }

    if !a.selectors.selectors.is_empty() && !b.selectors.selectors.is_empty() {
        weight_sum += 2.0;
        let a_values: std::collections::HashSet<_> =
            a.selectors.selectors.iter().map(|s| &s.value).collect();
        let b_values: std::collections::HashSet<_> =
            b.selectors.selectors.iter().map(|s| &s.value).collect();
        let intersection = a_values.intersection(&b_values).count();
        if intersection > 0 {
            score += 2.0 * (intersection as f64 / a_values.len().max(b_values.len()) as f64);
        }
    }

    if weight_sum > 0.0 {
        score / weight_sum
    } else {
        0.0
    }
}

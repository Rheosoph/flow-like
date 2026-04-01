use crate::types::fingerprints::{ElementFingerprint, MatchStrategy};
use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct MatchFingerprintNode {}

impl MatchFingerprintNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MatchFingerprintNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fingerprint_match",
            "Match Fingerprint",
            "Attempts to find an element matching the fingerprint",
            "Automation/Fingerprint",
        );
        node.add_icon("/flow/icons/fingerprint.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(5)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(8)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "fingerprint",
            "Fingerprint",
            "Fingerprint to match",
            VariableType::Struct,
        )
        .set_schema::<ElementFingerprint>();

        node.add_input_pin(
            "strategy",
            "Strategy",
            "Matching strategy",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Dom".to_string(),
                    "Accessibility".to_string(),
                    "Hybrid".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Hybrid")));

        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "Maximum time to search",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(5000)));

        node.add_output_pin(
            "exec_found",
            "Found",
            "Element was found",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_not_found",
            "Not Found",
            "Element was not found",
            VariableType::Execution,
        );

        node.add_output_pin(
            "found",
            "Found",
            "Whether element was found",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "selector_used",
            "Selector Used",
            "The selector that matched",
            VariableType::String,
        );
        node.add_output_pin(
            "confidence",
            "Confidence",
            "Match confidence",
            VariableType::Float,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::time::{Duration, Instant};
        use thirtyfour::By;

        context.deactivate_exec_pin("exec_found").await?;
        context.deactivate_exec_pin("exec_not_found").await?;

        let page: AutomationSession = context.evaluate_pin("session").await?;
        let fingerprint: ElementFingerprint = context.evaluate_pin("fingerprint").await?;
        let strategy_str: String = context.evaluate_pin("strategy").await?;
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;

        let driver = page.get_browser_driver_and_switch(context).await?;

        let strategy = match strategy_str.as_str() {
            "Dom" => MatchStrategy::Dom,
            "Accessibility" => MatchStrategy::Accessibility,
            _ => MatchStrategy::Hybrid,
        };

        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms as u64);

        let selectors_to_try: Vec<_> = fingerprint
            .selectors
            .iter_by_priority()
            .filter(|s| match strategy {
                MatchStrategy::Dom => matches!(
                    s.kind,
                    crate::types::selectors::SelectorKind::Css
                        | crate::types::selectors::SelectorKind::Xpath
                        | crate::types::selectors::SelectorKind::TestId
                ),
                MatchStrategy::Accessibility => matches!(
                    s.kind,
                    crate::types::selectors::SelectorKind::Role
                        | crate::types::selectors::SelectorKind::AriaLabel
                ),
                _ => true,
            })
            .collect();

        while start.elapsed() < timeout {
            for selector in &selectors_to_try {
                let by = match selector.kind {
                    crate::types::selectors::SelectorKind::Css => By::Css(&selector.value),
                    crate::types::selectors::SelectorKind::Xpath => By::XPath(&selector.value),
                    crate::types::selectors::SelectorKind::TestId => {
                        By::Css(format!("[data-testid='{}']", selector.value))
                    }
                    _ => continue,
                };

                if driver.find(by).await.is_ok() {
                    context.set_pin_value("found", json!(true)).await?;
                    context
                        .set_pin_value("selector_used", json!(selector.value.clone()))
                        .await?;
                    context
                        .set_pin_value("confidence", json!(selector.confidence.unwrap_or(0.8)))
                        .await?;
                    context.activate_exec_pin("exec_found").await?;
                    return Ok(());
                }
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        context.set_pin_value("found", json!(false)).await?;
        context.set_pin_value("selector_used", json!("")).await?;
        context.set_pin_value("confidence", json!(0.0)).await?;
        context.activate_exec_pin("exec_not_found").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Fingerprint matching requires the 'execute' feature"
        ))
    }
}

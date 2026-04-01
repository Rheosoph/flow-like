use crate::types::selectors::{RankedSelector, RankedSelectorSet, Selector, SelectorSet};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct RankSelectorsNode {}

impl RankSelectorsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RankSelectorsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "selector_rank",
            "Rank Selectors",
            "Ranks selectors in a set by their confidence and specificity",
            "Automation/Selector",
        );
        node.add_icon("/flow/icons/selector.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "selector_set",
            "Selector Set",
            "Selector set to rank",
            VariableType::Struct,
        )
        .set_schema::<SelectorSet>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "ranked_set",
            "Ranked Set",
            "Ranked selector set",
            VariableType::Struct,
        )
        .set_schema::<RankedSelectorSet>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let selector_set: SelectorSet = context.evaluate_pin("selector_set").await?;

        let mut ranked: Vec<RankedSelector> = selector_set
            .selectors
            .iter()
            .enumerate()
            .map(|(idx, selector)| {
                let base_confidence = selector.confidence.unwrap_or(0.5);

                let specificity_bonus = match &selector.kind {
                    crate::types::selectors::SelectorKind::TestId => 0.2,
                    crate::types::selectors::SelectorKind::Css => 0.15,
                    crate::types::selectors::SelectorKind::Xpath => 0.1,
                    crate::types::selectors::SelectorKind::AriaLabel => 0.1,
                    crate::types::selectors::SelectorKind::Role => 0.05,
                    crate::types::selectors::SelectorKind::Text => 0.0,
                    _ => 0.0,
                };

                let score = (base_confidence + specificity_bonus).min(1.0);

                RankedSelector {
                    selector: selector.clone(),
                    rank: idx,
                    score,
                    reason: Some(format!(
                        "Base: {:.2}, Specificity: +{:.2}",
                        base_confidence, specificity_bonus
                    )),
                }
            })
            .collect();

        ranked.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for (new_rank, item) in ranked.iter_mut().enumerate() {
            item.rank = new_rank;
        }

        let ranked_set = RankedSelectorSet::new(ranked);

        context
            .set_pin_value("ranked_set", json!(ranked_set))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetBestSelectorNode {}

impl GetBestSelectorNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetBestSelectorNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "selector_get_best",
            "Get Best Selector",
            "Gets the highest-ranked selector from a ranked set",
            "Automation/Selector",
        );
        node.add_icon("/flow/icons/selector.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "ranked_set",
            "Ranked Set",
            "Ranked selector set",
            VariableType::Struct,
        )
        .set_schema::<RankedSelectorSet>();

        node.add_output_pin(
            "exec_found",
            "Found",
            "Best selector found",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_empty",
            "Empty",
            "No selectors in set",
            VariableType::Execution,
        );

        node.add_output_pin(
            "selector",
            "Selector",
            "Best selector",
            VariableType::Struct,
        )
        .set_schema::<Selector>();

        node.add_output_pin("score", "Score", "Selector score", VariableType::Float);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_found").await?;
        context.deactivate_exec_pin("exec_empty").await?;

        let ranked_set: RankedSelectorSet = context.evaluate_pin("ranked_set").await?;

        if let Some(best) = ranked_set.best() {
            context
                .set_pin_value("selector", json!(best.selector))
                .await?;
            context.set_pin_value("score", json!(best.score)).await?;
            context.activate_exec_pin("exec_found").await?;
        } else {
            context.activate_exec_pin("exec_empty").await?;
        }

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ValidateSelectorNode {}

impl ValidateSelectorNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ValidateSelectorNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "selector_validate",
            "Validate Selector",
            "Validates a selector's format and structure",
            "Automation/Selector",
        );
        node.add_icon("/flow/icons/selector.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "selector",
            "Selector",
            "Selector to validate",
            VariableType::Struct,
        )
        .set_schema::<Selector>();

        node.add_output_pin(
            "exec_valid",
            "Valid",
            "Selector is valid",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_invalid",
            "Invalid",
            "Selector is invalid",
            VariableType::Execution,
        );

        node.add_output_pin(
            "is_valid",
            "Is Valid",
            "Whether selector is valid",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Validation error message",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_valid").await?;
        context.deactivate_exec_pin("exec_invalid").await?;

        let selector: Selector = context.evaluate_pin("selector").await?;

        let (is_valid, error) = validate_selector(&selector);

        context.set_pin_value("is_valid", json!(is_valid)).await?;
        context
            .set_pin_value("error", json!(error.unwrap_or_default()))
            .await?;

        if is_valid {
            context.activate_exec_pin("exec_valid").await?;
        } else {
            context.activate_exec_pin("exec_invalid").await?;
        }

        Ok(())
    }
}

fn validate_selector(selector: &Selector) -> (bool, Option<String>) {
    if selector.value.is_empty() {
        return (false, Some("Selector value is empty".to_string()));
    }

    match selector.kind {
        crate::types::selectors::SelectorKind::Css => {
            if selector.value.contains("{{") || selector.value.contains("}}") {
                return (
                    false,
                    Some("CSS selector contains template syntax".to_string()),
                );
            }
            (true, None)
        }
        crate::types::selectors::SelectorKind::Xpath => {
            if !selector.value.starts_with('/') && !selector.value.starts_with('.') {
                return (false, Some("XPath should start with / or .".to_string()));
            }
            (true, None)
        }
        _ => (true, None),
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RankedSetToSelectorSetNode {}

impl RankedSetToSelectorSetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RankedSetToSelectorSetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "selector_ranked_to_set",
            "Ranked To Selector Set",
            "Converts a ranked selector set back to a regular selector set",
            "Automation/Selector",
        );
        node.add_icon("/flow/icons/selector.svg");

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "ranked_set",
            "Ranked Set",
            "Ranked selector set to convert",
            VariableType::Struct,
        )
        .set_schema::<RankedSelectorSet>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "selector_set",
            "Selector Set",
            "Regular selector set with ranked order",
            VariableType::Struct,
        )
        .set_schema::<SelectorSet>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let ranked_set: RankedSelectorSet = context.evaluate_pin("ranked_set").await?;
        let selector_set = ranked_set.to_selector_set();

        context
            .set_pin_value("selector_set", json!(selector_set))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

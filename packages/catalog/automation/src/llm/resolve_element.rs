use super::{COORDINATE_SPACE, add_screenshot_pins};
#[cfg(feature = "execute")]
use super::{
    ModelView, SubmitTool, call_tool, parse_tool_args, require_screenshot, vision_history,
};
use flow_like::{
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::{anyhow, json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ElementCandidate {
    pub index: usize,
    pub x: i32,
    pub y: i32,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResolvedElement {
    pub resolved: bool,
    pub selected_index: Option<usize>,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub reasoning: String,
}

#[cfg(feature = "execute")]
const TOOL: &str = "submit_resolution";

#[cfg(feature = "execute")]
#[derive(Debug, Deserialize)]
struct ResolveArgs {
    resolved: bool,
    selected_index: Option<f64>,
    reasoning: String,
}

#[cfg(feature = "execute")]
fn unresolved(reasoning: String) -> ResolvedElement {
    ResolvedElement {
        resolved: false,
        selected_index: None,
        x: None,
        y: None,
        reasoning,
    }
}

#[cfg(feature = "execute")]
fn check_unique_indices(candidates: &[ElementCandidate]) -> flow_like_types::Result<()> {
    let mut seen = std::collections::HashSet::new();
    for candidate in candidates {
        if !seen.insert(candidate.index) {
            return Err(anyhow!(
                "Candidate index {} appears more than once; every candidate needs a unique index",
                candidate.index
            ));
        }
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn describe_candidates(candidates: &[ElementCandidate], view: &ModelView) -> String {
    candidates
        .iter()
        .map(|candidate| {
            let position = view.point_to_model(candidate.x, candidate.y).map_or_else(
                || "outside the screenshot".to_string(),
                |(x, y)| format!("at ({x}, {y})"),
            );
            format!(
                "[{}] {position}: {}",
                candidate.index, candidate.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The answer only counts when it names a listed candidate; its coordinates come from that
/// candidate, never from the model.
#[cfg(feature = "execute")]
fn resolve(args: ResolveArgs, candidates: &[ElementCandidate]) -> ResolvedElement {
    if !args.resolved {
        return unresolved(args.reasoning);
    }
    let chosen = args
        .selected_index
        .filter(|index| index.fract() == 0.0 && *index >= 0.0)
        .and_then(|index| {
            candidates
                .iter()
                .find(|candidate| candidate.index as f64 == index)
        });
    match chosen {
        Some(candidate) => ResolvedElement {
            resolved: true,
            selected_index: Some(candidate.index),
            x: Some(candidate.x),
            y: Some(candidate.y),
            reasoning: args.reasoning,
        },
        None => unresolved(format!(
            "Model selected index {:?}, which is not one of the candidates: {}",
            args.selected_index, args.reasoning
        )),
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct LLMResolveElementNode {}

impl LLMResolveElementNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMResolveElementNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_resolve_element",
            "LLM Resolve Element",
            "Uses LLM to disambiguate between multiple element candidates",
            "Automation/LLM/Vision",
        );
        node.set_flowscript_name("automation.llm", "resolveElement");
        node.add_icon("/flow/icons/bot-search.svg");
        node.set_version(6);

        node.set_scores(
            NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(4)
                .set_governance(5)
                .set_reliability(6)
                .set_cost(5)
                .build(),
        );

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "model",
            "Model",
            "Vision-capable LLM model",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        add_screenshot_pins(&mut node, "Screenshot", true);

        node.add_input_pin(
            "candidates",
            "Candidates",
            &format!(
                "Element candidates to choose from, each with a unique index. x/y are in {COORDINATE_SPACE}"
            ),
            VariableType::Struct,
        )
        .set_schema::<ElementCandidate>()
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_input_pin(
            "intent",
            "Intent",
            "What the user is trying to accomplish",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "exec_ambiguous",
            "Ambiguous",
            "No candidate could be chosen (none given, none matched, or the model picked an unknown index)",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Result",
            "Resolution result; x/y are copied from the chosen candidate",
            VariableType::Struct,
        )
        .set_schema::<ResolvedElement>();

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_ambiguous").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let candidates: Vec<ElementCandidate> = context.evaluate_pin("candidates").await?;
        let intent: String = context.evaluate_pin("intent").await?;
        check_unique_indices(&candidates)?;

        let resolved = if candidates.is_empty() {
            unresolved("No candidates to choose from".to_string())
        } else {
            let screenshot = require_screenshot(context).await?;
            let parameters = json::json!({
                "type": "object",
                "properties": {
                    "resolved": { "type": "boolean", "description": "Whether a single best match was identified" },
                    "selected_index": { "type": "integer", "description": "The bracketed index of the chosen candidate, exactly as listed" },
                    "reasoning": { "type": "string", "description": "Explanation of why this element was selected" }
                },
                "required": ["resolved", "reasoning"]
            });

            let instructions = format!(
                "User intent: {intent}\n\nCandidate elements (positions are pixels of the screenshot):\n{}\n\nSelect the candidate that best matches the intent and answer with its bracketed index.",
                describe_candidates(&candidates, &screenshot.view)
            );

            let preamble = "You are a UI element resolver. Given multiple candidate elements and the user's intent, select the most appropriate one. Consider visual context, element type, and user goal. Only choose among the listed candidates.";

            let arguments = call_tool(
                context,
                &model_bit,
                vision_history(&[&screenshot.image], &instructions),
                preamble,
                SubmitTool {
                    name: TOOL,
                    description: "Submit the resolved element selection",
                    parameters,
                },
            )
            .await?;

            match arguments {
                Some(arguments) => resolve(parse_tool_args(TOOL, &arguments)?, &candidates),
                None => unresolved(format!("The model answered without calling `{TOOL}`")),
            }
        };

        context
            .set_pin_value("result", json::json!(resolved))
            .await?;

        if resolved.resolved {
            context.activate_exec_pin("exec_out").await?;
        } else {
            context.activate_exec_pin("exec_ambiguous").await?;
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;

    fn candidates() -> Vec<ElementCandidate> {
        vec![
            ElementCandidate {
                index: 3,
                x: 10,
                y: 20,
                description: "Cancel".into(),
            },
            ElementCandidate {
                index: 7,
                x: 300,
                y: 400,
                description: "Submit".into(),
            },
        ]
    }

    fn args(resolved: bool, selected_index: Option<f64>) -> ResolveArgs {
        ResolveArgs {
            resolved,
            selected_index,
            reasoning: "why".into(),
        }
    }

    #[test]
    fn coordinates_come_from_the_selected_candidate() {
        let result = resolve(args(true, Some(7.0)), &candidates());
        assert!(result.resolved);
        assert_eq!(
            (result.selected_index, result.x, result.y),
            (Some(7), Some(300), Some(400))
        );
    }

    #[test]
    fn unknown_or_missing_index_is_unresolved() {
        assert!(!resolve(args(true, Some(1.0)), &candidates()).resolved);
        assert!(!resolve(args(true, Some(7.5)), &candidates()).resolved);
        assert!(!resolve(args(true, None), &candidates()).resolved);
        assert!(!resolve(args(false, Some(7.0)), &candidates()).resolved);
    }

    #[test]
    fn duplicate_candidate_indices_are_rejected() {
        let mut list = candidates();
        list[1].index = 3;
        assert!(check_unique_indices(&list).is_err());
    }
}

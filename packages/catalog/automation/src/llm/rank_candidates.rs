use super::{COORDINATE_SPACE, add_screenshot_pins};
#[cfg(feature = "execute")]
use super::{
    ModelView, SubmitTool, call_tool, missing_tool_call, parse_tool_args, require_screenshot,
    vision_history,
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
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{async_trait, json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CandidateInput {
    pub id: String,
    pub description: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub selector: Option<String>,
    pub additional_info: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RankedCandidate {
    pub id: String,
    pub rank: usize,
    pub score: f64,
    pub reasoning: String,
    pub is_recommended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RankingResult {
    pub ranked_candidates: Vec<RankedCandidate>,
    pub best_match_id: String,
    pub confidence: f64,
    pub ambiguity_warning: Option<String>,
}

#[cfg(feature = "execute")]
const TOOL: &str = "submit_ranking";

#[cfg(feature = "execute")]
fn check_candidates(candidates: &[CandidateInput]) -> flow_like_types::Result<()> {
    if candidates.is_empty() {
        return Err(anyhow!("No candidates to rank"));
    }
    let mut seen = std::collections::HashSet::new();
    for candidate in candidates {
        if !seen.insert(candidate.id.as_str()) {
            return Err(anyhow!(
                "Candidate id '{}' appears more than once; every candidate needs a unique id",
                candidate.id
            ));
        }
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn describe_candidates(candidates: &[CandidateInput], view: &ModelView) -> String {
    candidates
        .iter()
        .map(|candidate| {
            let mut line = format!("- [{}]: {}", candidate.id, candidate.description);
            if let (Some(x), Some(y)) = (candidate.x, candidate.y) {
                match view.point_to_model(x, y) {
                    Some((x, y)) => line.push_str(&format!(" at ({x}, {y})")),
                    None => line.push_str(" (outside the screenshot)"),
                }
            }
            if let Some(selector) = &candidate.selector {
                line.push_str(&format!("; selector: {selector}"));
            }
            if let Some(info) = &candidate.additional_info {
                line.push_str(&format!("; info: {info}"));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Keeps only rankings of given candidates (first entry per id), orders them by rank and
/// makes sure the best match is one of them. Returns the ranking and the ids the model invented.
#[cfg(feature = "execute")]
fn validate_ranking(
    mut ranking: RankingResult,
    candidates: &[CandidateInput],
) -> flow_like_types::Result<(RankingResult, Vec<String>)> {
    let known: std::collections::HashSet<&str> = candidates.iter().map(|c| c.id.as_str()).collect();
    let mut seen = std::collections::HashSet::new();
    let mut unknown = Vec::new();
    ranking.ranked_candidates.retain(|ranked| {
        if !known.contains(ranked.id.as_str()) {
            unknown.push(ranked.id.clone());
            return false;
        }
        seen.insert(ranked.id.clone())
    });
    ranking.ranked_candidates.sort_by_key(|ranked| ranked.rank);
    for (position, ranked) in ranking.ranked_candidates.iter_mut().enumerate() {
        ranked.rank = position + 1;
    }
    if !known.contains(ranking.best_match_id.as_str()) {
        let Some(first) = ranking.ranked_candidates.first() else {
            return Err(anyhow!(
                "The model ranked none of the {} given candidates (best match '{}')",
                candidates.len(),
                ranking.best_match_id
            ));
        };
        let invented = std::mem::replace(&mut ranking.best_match_id, first.id.clone());
        if !unknown.contains(&invented) {
            unknown.push(invented);
        }
    }
    Ok((ranking, unknown))
}

#[crate::register_node]
#[derive(Default)]
pub struct LLMRankCandidatesNode {}

impl LLMRankCandidatesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LLMRankCandidatesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_rank_candidates",
            "LLM Rank Candidates",
            "Uses LLM to rank multiple element candidates based on match quality",
            "Automation/LLM/Vision",
        );
        node.set_flowscript_name("automation.llm", "rankCandidates");
        node.add_icon("/flow/icons/bot-search.svg");
        node.set_version(6);

        node.set_scores(
            NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(4)
                .set_governance(5)
                .set_reliability(7)
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
                "Candidate elements to rank, each with a unique id. Optional x/y are in {COORDINATE_SPACE}"
            ),
            VariableType::Struct,
        )
        .set_schema::<CandidateInput>()
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_input_pin(
            "criteria",
            "Criteria",
            "What the target element should match (description/intent)",
            VariableType::String,
        );

        node.add_input_pin(
            "context",
            "Context",
            "Additional context for ranking",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "result",
            "Result",
            "Full ranking result; only given candidate ids appear",
            VariableType::Struct,
        )
        .set_schema::<RankingResult>();

        node.add_output_pin(
            "best_match",
            "Best Match",
            "ID of the best matching candidate",
            VariableType::String,
        );

        node.add_output_pin(
            "ranked",
            "Ranked",
            "Candidates sorted by rank",
            VariableType::Struct,
        )
        .set_schema::<RankedCandidate>()
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like::flow::execution::LogLevel;

        context.deactivate_exec_pin("exec_out").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let candidates: Vec<CandidateInput> = context.evaluate_pin("candidates").await?;
        let criteria: String = context.evaluate_pin("criteria").await?;
        let ctx: String = context.evaluate_pin("context").await.unwrap_or_default();
        check_candidates(&candidates)?;
        let screenshot = require_screenshot(context).await?;

        let parameters = json::json!({
            "type": "object",
            "properties": {
                "ranked_candidates": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "Candidate ID, exactly as listed" },
                            "rank": { "type": "integer", "description": "Rank (1 = best)" },
                            "score": { "type": "number", "description": "Match score 0-1" },
                            "reasoning": { "type": "string", "description": "Why this ranking" },
                            "is_recommended": { "type": "boolean", "description": "Is this a good match?" }
                        },
                        "required": ["id", "rank", "score", "reasoning", "is_recommended"]
                    }
                },
                "best_match_id": { "type": "string", "description": "ID of the best matching candidate" },
                "confidence": { "type": "number", "description": "Overall confidence in ranking 0-1" },
                "ambiguity_warning": { "type": "string", "description": "Warning if multiple candidates are equally good" }
            },
            "required": ["ranked_candidates", "best_match_id", "confidence"]
        });

        let context_text = if ctx.is_empty() {
            String::new()
        } else {
            format!("\nAdditional context: {ctx}")
        };

        let instructions = format!(
            "Criteria: {criteria}\n\nCandidates (positions are pixels of the screenshot):\n{}{context_text}\n\nRank these candidates from best to worst match, using their ids exactly as listed.",
            describe_candidates(&candidates, &screenshot.view)
        );

        let preamble = "You are an element matching expert. Rank multiple candidate elements based on how well they match the given criteria. Consider visual appearance, position, context, and semantics.";

        let arguments = call_tool(
            context,
            &model_bit,
            vision_history(&[&screenshot.image], &instructions),
            preamble,
            SubmitTool {
                name: TOOL,
                description: "Submit the candidate ranking",
                parameters,
            },
        )
        .await?
        .ok_or_else(|| missing_tool_call(TOOL))?;

        let (ranking, unknown) = validate_ranking(parse_tool_args(TOOL, &arguments)?, &candidates)?;
        if !unknown.is_empty() {
            context.log_message(
                &format!(
                    "Ignored candidate ids the model invented: {}",
                    unknown.join(", ")
                ),
                LogLevel::Warn,
            );
        }

        context
            .set_pin_value("result", json::json!(ranking))
            .await?;
        context
            .set_pin_value("best_match", json::json!(ranking.best_match_id))
            .await?;
        context
            .set_pin_value("ranked", json::json!(ranking.ranked_candidates))
            .await?;

        context.activate_exec_pin("exec_out").await?;

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

    fn candidate(id: &str) -> CandidateInput {
        CandidateInput {
            id: id.into(),
            description: id.into(),
            x: None,
            y: None,
            selector: None,
            additional_info: None,
        }
    }

    fn ranked(id: &str, rank: usize) -> RankedCandidate {
        RankedCandidate {
            id: id.into(),
            rank,
            score: 0.5,
            reasoning: String::new(),
            is_recommended: true,
        }
    }

    #[test]
    fn ranking_keeps_only_given_candidates_in_rank_order() {
        let candidates = [candidate("a"), candidate("b")];
        let ranking = RankingResult {
            ranked_candidates: vec![
                ranked("b", 2),
                ranked("ghost", 1),
                ranked("a", 3),
                ranked("b", 4),
            ],
            best_match_id: "ghost".into(),
            confidence: 0.7,
            ambiguity_warning: None,
        };
        let (ranking, unknown) = validate_ranking(ranking, &candidates).unwrap();
        let order: Vec<_> = ranking
            .ranked_candidates
            .iter()
            .map(|r| (r.id.as_str(), r.rank))
            .collect();
        assert_eq!(order, vec![("b", 1), ("a", 2)]);
        assert_eq!(ranking.best_match_id, "b");
        assert_eq!(unknown, vec!["ghost".to_string()]);
    }

    #[test]
    fn ranking_without_any_given_candidate_is_an_error() {
        let ranking = RankingResult {
            ranked_candidates: vec![ranked("ghost", 1)],
            best_match_id: "ghost".into(),
            confidence: 0.7,
            ambiguity_warning: None,
        };
        assert!(validate_ranking(ranking, &[candidate("a")]).is_err());
        assert!(check_candidates(&[]).is_err());
        assert!(check_candidates(&[candidate("a"), candidate("a")]).is_err());
    }
}

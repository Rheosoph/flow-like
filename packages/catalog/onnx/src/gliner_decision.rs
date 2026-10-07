//! Typed decisions answer choice, rubric score, and true-probability questions about text.
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, dynamic_pin_source_literal, remove_unwired_pins},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
#[cfg(any(feature = "execute", test))]
use flow_like_types::anyhow;
use flow_like_types::{Result, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DecisionQuestionType {
    Choice = 0,
    Score = 1,
    Noul = 2,
}

#[derive(Clone, Debug)]
pub struct DecisionOptions {
    pub question_type: DecisionQuestionType,
    pub instructions: String,
    /// Choice labels, ordered score levels, or optional false/true descriptions for noul.
    pub criteria: Vec<String>,
}

impl DecisionOptions {
    #[cfg(any(feature = "execute", test))]
    pub(crate) fn validate(&self) -> Result<()> {
        if self.instructions.trim().is_empty() {
            return Err(anyhow!("Decision Instructions must contain a question"));
        }
        match self.question_type {
            DecisionQuestionType::Choice | DecisionQuestionType::Score => {
                if self.criteria.is_empty()
                    || self.criteria.iter().any(|value| value.trim().is_empty())
                {
                    return Err(anyhow!(
                        "Decision choice and score questions need non-empty Criteria"
                    ));
                }
                if self.question_type == DecisionQuestionType::Choice {
                    let unique: std::collections::HashSet<_> = self.criteria.iter().collect();
                    if unique.len() != self.criteria.len() {
                        return Err(anyhow!("Decision choice labels must be unique"));
                    }
                }
            }
            DecisionQuestionType::Noul => {
                if !self.criteria.is_empty() && self.criteria.len() != 2 {
                    return Err(anyhow!(
                        "Decision noul Criteria must be empty or contain false and true descriptions, in that order"
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DecisionProbability {
    pub label: String,
    pub probability: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DecisionResult {
    pub question_type: DecisionQuestionType,
    /// Selected label for a choice question.
    pub choice: Option<String>,
    /// Expected zero-based rubric level for a score question.
    pub score: Option<f64>,
    /// Probability that the statement holds for a noul question.
    pub noul: Option<f64>,
    pub probabilities: Vec<DecisionProbability>,
    /// One minus normalized entropy; for noul, the larger of P(false) and P(true).
    pub confidence: f64,
    pub input_tokens: usize,
}

#[crate::register_node]
#[derive(Default)]
pub struct GlinerDecisionNode {}

impl GlinerDecisionNode {
    pub fn new() -> Self {
        Self {}
    }
}

pub(crate) const DEFAULT_DECISION_MODEL: &str = "fastino/GLiNER2.5-Decide";
pub(crate) const DECISION_MODELS: &[&str] = &[
    DEFAULT_DECISION_MODEL,
    "fastino/GLiNER2.5-multi-Decide",
    "fastino/GLiNER2.5-Decide-1B",
    "fastino/gliner2.5-multi-v1",
    "fastino/gliner2.5-base-v1",
    "fastino/gliner2.5-small-v1",
    "custom",
];

const MODEL_DIR_DESCRIPTION: &str = "FlowPath directory for downloaded ONNX models and reusable cache. Place your exported weights here for custom. Custom requires a complete GLiNER2 classification bundle and never downloads missing files";

fn ensure_model_input(node: &mut Node) {
    if node.get_pin_by_name("model").is_none() {
        node.add_input_pin(
            "model",
            "Model",
            "Model weights to use. ONNX presets download once and reuse the Model Directory cache. Custom loads your own complete bundle",
            VariableType::String,
        )
        .set_default_value(Some(json!(DEFAULT_DECISION_MODEL)))
        .set_options(
            PinOptions::new()
                .set_valid_values(DECISION_MODELS.iter().map(|model| (*model).into()).collect())
                .build(),
        );
    }
}

fn ensure_mode_input(
    node: &mut Node,
    name: &str,
    label: &str,
    description: &str,
    array: bool,
    index: u16,
) {
    if node.get_pin_by_name(name).is_none() {
        let pin = node.add_input_pin(name, label, description, VariableType::String);
        if array {
            pin.set_value_type(ValueType::Array)
                .set_default_value(Some(json!([])));
        } else {
            pin.set_default_value(Some(json!("")));
        }
    }
    let pin = node.get_pin_mut_by_name(name).unwrap();
    pin.friendly_name = label.into();
    pin.description = description.into();
    pin.index = index;
}

fn ensure_mode_output(
    node: &mut Node,
    name: &str,
    label: &str,
    description: &str,
    data_type: VariableType,
    index: u16,
) {
    if node.get_pin_by_name(name).is_none() {
        node.add_output_pin(name, label, description, data_type);
    }
    let pin = node.get_pin_mut_by_name(name).unwrap();
    pin.friendly_name = label.into();
    pin.description = description.into();
    pin.index = index;
}

/// A wired selector exposes every mode's pins because its value is only known at execution.
fn reconcile_mode_pins(node: &mut Node, mode: Option<DecisionQuestionType>) {
    let choices = mode.is_none() || mode == Some(DecisionQuestionType::Choice);
    let scores = mode.is_none() || mode == Some(DecisionQuestionType::Score);
    let noul = mode.is_none() || mode == Some(DecisionQuestionType::Noul);
    let wanted = [
        ("criteria", choices || scores),
        ("false_description", noul),
        ("true_description", noul),
        ("choice", choices),
        ("score", scores),
        ("noul", noul),
    ];
    let stale: Vec<_> = node
        .pins
        .values()
        .filter(|pin| wanted.iter().any(|(name, keep)| pin.name == *name && !keep))
        .map(|pin| pin.id.clone())
        .collect();
    remove_unwired_pins(node, &stale);
    if choices || scores {
        let (label, description) = match mode {
            Some(DecisionQuestionType::Choice) => ("Choices", "Unique labels to choose from"),
            Some(DecisionQuestionType::Score) => (
                "Levels",
                "Ordered rubric descriptions, starting at level zero",
            ),
            _ => (
                "Criteria",
                "Choice labels or ordered score levels. Unused for noul questions",
            ),
        };
        ensure_mode_input(node, "criteria", label, description, true, 7);
    }
    if noul {
        ensure_mode_input(
            node,
            "false_description",
            "False Description",
            "Optional description of when the statement is false. Leave empty to use the model default",
            false,
            8,
        );
        ensure_mode_input(
            node,
            "true_description",
            "True Description",
            "Optional description of when the statement is true. Leave empty to use the model default",
            false,
            9,
        );
    }
    if choices {
        ensure_mode_output(
            node,
            "choice",
            "Choice",
            if mode.is_none() {
                "Selected choice label; empty when the runtime mode is score or noul"
            } else {
                "Selected choice label"
            },
            VariableType::String,
            3,
        );
    }
    if scores {
        ensure_mode_output(
            node,
            "score",
            "Score",
            if mode.is_none() {
                "Expected zero-based rubric level; zero when the runtime mode is choice or noul"
            } else {
                "Expected zero-based rubric level"
            },
            VariableType::Float,
            4,
        );
    }
    if noul {
        ensure_mode_output(
            node,
            "noul",
            "P(True)",
            if mode.is_none() {
                "Probability that the statement holds; zero when the runtime mode is choice or score"
            } else {
                "Probability that the statement holds"
            },
            VariableType::Float,
            5,
        );
    }
}

fn normalize_pin_order(node: &mut Node) {
    let inputs = [
        "exec_in",
        "model_dir",
        "model",
        "text",
        "instructions",
        "question_type",
        "criteria",
        "false_description",
        "true_description",
    ];
    let outputs = [
        "exec_out",
        "result",
        "choice",
        "score",
        "noul",
        "confidence",
    ];
    for names in [inputs.as_slice(), outputs.as_slice()] {
        for (index, name) in names.iter().enumerate() {
            if let Some(pin) = node.get_pin_mut_by_name(name) {
                pin.index = (index + 1) as u16;
            }
        }
    }
}

#[async_trait]
impl NodeLogic for GlinerDecisionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_gliner_decision",
            "GLiNER Decision",
            "Answer a choice, rubric score, or true-probability question about text with GLiNER2.5. Select ONNX model weights and connect a Model Directory to cache downloads and reuse them across runs. Select custom to load your own complete bundle without downloading missing files.",
            "AI/ML/ONNX/NLP",
        );
        node.set_flowscript_name("onnx", "gliner_decision");
        node.set_version(1);
        node.add_icon("/flow/icons/type.svg");
        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "model_dir",
            "Model Directory",
            MODEL_DIR_DESCRIPTION,
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        ensure_model_input(&mut node);
        node.add_input_pin(
            "text",
            "Text",
            "Text or serialized JSON state to evaluate",
            VariableType::String,
        );
        node.add_input_pin(
            "instructions",
            "Instructions",
            "Question to answer about Text",
            VariableType::String,
        );
        node.add_input_pin(
            "question_type",
            "Question Type",
            "choice selects a label; score returns an expected rubric level; noul returns P(true)",
            VariableType::String,
        )
        .set_default_value(Some(json!("choice")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["choice".into(), "score".into(), "noul".into()])
                .build(),
        );
        node.add_output_pin(
            "exec_out",
            "Output",
            "Decision complete",
            VariableType::Execution,
        );
        node.add_output_pin(
            "result",
            "Result",
            "Typed answer, option probabilities, confidence and token count",
            VariableType::Struct,
        )
        .set_schema::<DecisionResult>();
        node.add_output_pin(
            "confidence",
            "Confidence",
            "Normalized entropy confidence, or max(P(false), P(true)) for noul",
            VariableType::Float,
        )
        .index = 6;
        reconcile_mode_pins(&mut node, Some(DecisionQuestionType::Choice));
        normalize_pin_order(&mut node);
        node
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;
        ensure_model_input(node);
        normalize_pin_order(node);
        let model = node.get_pin_by_name("model").unwrap();
        if model.depends_on.is_empty()
            && dynamic_pin_source_literal(node, "model")
                .is_none_or(|value| !DECISION_MODELS.contains(&value.as_str()))
        {
            node.error = Some("Select a supported decision model or custom".into());
        }
        let Some(selector) = node.get_pin_by_name("question_type") else {
            return;
        };
        if !selector.depends_on.is_empty() {
            reconcile_mode_pins(node, None);
            return;
        }
        let mode = dynamic_pin_source_literal(node, "question_type").and_then(|value| {
            flow_like_types::json::from_value::<DecisionQuestionType>(json!(value)).ok()
        });
        let Some(mode) = mode else {
            node.error = Some("Select choice, score, or noul as the decision question type".into());
            return;
        };
        reconcile_mode_pins(node, Some(mode));
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let question_type: DecisionQuestionType = context.evaluate_pin("question_type").await?;
            let criteria = match question_type {
                DecisionQuestionType::Choice | DecisionQuestionType::Score => {
                    context.evaluate_pin("criteria").await?
                }
                DecisionQuestionType::Noul => vec![
                    context.evaluate_pin("false_description").await?,
                    context.evaluate_pin("true_description").await?,
                ],
            };
            let options = DecisionOptions {
                question_type,
                instructions: context.evaluate_pin("instructions").await?,
                criteria,
            };
            options.validate()?;
            let text: String = context.evaluate_pin("text").await?;
            let model_dir: FlowPath = context.evaluate_pin("model_dir").await?;
            let model: String = context.evaluate_pin("model").await?;
            let result =
                crate::onnx::decision::infer_decision(context, &model, &model_dir, &text, &options)
                    .await?;
            // Wired selectors expose all outputs. Reset inactive answers so a previous
            // invocation's value cannot survive a mode change.
            for (name, value) in [
                (
                    "choice",
                    json!(result.choice.as_deref().unwrap_or_default()),
                ),
                ("score", json!(result.score.unwrap_or_default())),
                ("noul", json!(result.noul.unwrap_or_default())),
            ] {
                if context.get_pin_by_name(name).await.is_ok() {
                    context.set_pin_value(name, value).await?;
                }
            }
            context
                .set_pin_value("confidence", json!(result.confidence))
                .await?;
            context.set_pin_value("result", json!(result)).await?;
            context.activate_exec_pin("exec_out").await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(kind: DecisionQuestionType, labels: &[&str]) -> DecisionOptions {
        DecisionOptions {
            question_type: kind,
            instructions: "Which applies?".into(),
            criteria: labels.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn invalid_questions_fail() {
        assert!(
            options(DecisionQuestionType::Choice, &[])
                .validate()
                .is_err()
        );
        assert!(
            options(DecisionQuestionType::Choice, &["a", "a"])
                .validate()
                .is_err()
        );
        assert!(
            options(DecisionQuestionType::Noul, &["true"])
                .validate()
                .is_err()
        );
        let mut empty = options(DecisionQuestionType::Choice, &["a"]);
        empty.instructions.clear();
        assert!(empty.validate().is_err());
    }
}

//! Laya answers a typed question about text with calibrated option probabilities.
//! Prompt layout and calibration follow https://github.com/mizchi/laya-mlx.
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, dynamic_pin_source_literal, pin_is_wired, remove_unwired_pins},
    pin::{PinOptions, PinType, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{Result, anyhow, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[cfg(feature = "execute")]
use flow_like_model_provider::ml::{
    ndarray::{Array1, Array2},
    ort::{
        inputs,
        session::Session,
        value::{TensorElementType, Value},
    },
};
#[cfg(feature = "execute")]
use tokenizers::Tokenizer;

#[cfg(feature = "execute")]
#[path = "laya/loading.rs"]
mod loading;

/// Calibration and prompt limits from `rl_agent_config.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct LayaConfig {
    pub max_len: usize,
    pub head_max_len: usize,
    pub temperature: [f32; 3],
    pub temperature_by_options: BTreeMap<String, f32>,
}

impl Default for LayaConfig {
    fn default() -> Self {
        Self {
            max_len: 1024,
            head_max_len: 256,
            temperature: [1.0; 3],
            temperature_by_options: BTreeMap::new(),
        }
    }
}

impl LayaConfig {
    pub fn validate(&self) -> Result<()> {
        if self.head_max_len <= 4 || self.head_max_len >= self.max_len || self.max_len > 8192 {
            return Err(anyhow!("Laya requires 4 < head_max_len < max_len <= 8192"));
        }
        if self
            .temperature
            .iter()
            .chain(self.temperature_by_options.values())
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err(anyhow!(
                "Laya calibration temperatures must be finite and positive"
            ));
        }
        Ok(())
    }

    #[cfg(any(feature = "execute", test))]
    fn scale(&self, kind: LayaQuestionType, count: usize) -> f64 {
        let bucket = match count {
            0..=2 => "2",
            3..=5 => "3-5",
            6..=10 => "6-10",
            _ => "11+",
        };
        f64::from(
            *self
                .temperature_by_options
                .get(&format!("{}:{bucket}", kind.name()))
                .unwrap_or(&self.temperature[kind as usize]),
        )
        .max(1e-3)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LayaQuestionType {
    Choice = 0,
    Score = 1,
    Noul = 2,
}

impl LayaQuestionType {
    #[cfg(any(feature = "execute", test))]
    fn name(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::Score => "score",
            Self::Noul => "noul",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LayaOptions {
    pub question_type: LayaQuestionType,
    pub instructions: String,
    /// Choice labels, ordered score levels, or optional false/true descriptions for noul.
    pub criteria: Vec<String>,
}

impl LayaOptions {
    #[cfg(any(feature = "execute", test))]
    fn rendered_options(&self) -> Result<Vec<String>> {
        if self.instructions.trim().is_empty() {
            return Err(anyhow!("Laya Instructions must contain a question"));
        }
        match self.question_type {
            LayaQuestionType::Choice | LayaQuestionType::Score => {
                if self.criteria.is_empty()
                    || self.criteria.iter().any(|value| value.trim().is_empty())
                {
                    return Err(anyhow!(
                        "Laya choice and score questions need non-empty Criteria"
                    ));
                }
                if self.question_type == LayaQuestionType::Choice {
                    let unique: std::collections::HashSet<_> = self.criteria.iter().collect();
                    if unique.len() != self.criteria.len() {
                        return Err(anyhow!("Laya choice labels must be unique"));
                    }
                    Ok(self.criteria.clone())
                } else {
                    Ok(self
                        .criteria
                        .iter()
                        .enumerate()
                        .map(|(i, text)| format!("level {i}: {text}"))
                        .collect())
                }
            }
            LayaQuestionType::Noul => {
                if !self.criteria.is_empty() && self.criteria.len() != 2 {
                    return Err(anyhow!(
                        "Laya noul Criteria must be empty or contain false and true descriptions, in that order"
                    ));
                }
                let description = |index: usize, fallback: &str| {
                    self.criteria
                        .get(index)
                        .filter(|value| !value.is_empty())
                        .map(String::as_str)
                        .unwrap_or(fallback)
                        .to_string()
                };
                Ok(vec![
                    format!(
                        "false: {}",
                        description(0, "no, the statement does not hold")
                    ),
                    format!("true: {}", description(1, "yes, the statement holds")),
                ])
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LayaProbability {
    pub label: String,
    pub probability: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LayaResult {
    pub question_type: LayaQuestionType,
    /// Selected label for a choice question.
    pub choice: Option<String>,
    /// Expected zero-based rubric level for a score question.
    pub score: Option<f64>,
    /// Probability that the statement holds for a noul question.
    pub noul: Option<f64>,
    pub probabilities: Vec<LayaProbability>,
    /// One minus normalized entropy; for noul, the larger of P(false) and P(true).
    pub confidence: f64,
    /// Probability of action index zero from the model's separate action head.
    pub act_probability: f64,
    pub input_tokens: usize,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
pub struct LayaInput {
    pub input_ids: Vec<i64>,
    pub marker_pos: Vec<i64>,
}

/// Build one question using the reference head budget, option markers and right truncation.
#[cfg(feature = "execute")]
pub fn prepare_laya(
    tokenizer: &Tokenizer,
    text: &str,
    options: &LayaOptions,
    config: &LayaConfig,
) -> Result<LayaInput> {
    config.validate()?;
    let rendered = options.rendered_options()?;
    let tokens = [["<bos>", "<eos>", "<mask>"], ["[CLS]", "[SEP]", "[MASK]"]]
        .into_iter()
        .find(|names| {
            names
                .iter()
                .all(|name| tokenizer.token_to_id(name).is_some())
        })
        .ok_or_else(|| {
            anyhow!("Laya tokenizer must define <bos>/<eos>/<mask> or [CLS]/[SEP]/[MASK] tokens")
        })?;
    let cls = i64::from(tokenizer.token_to_id(tokens[0]).unwrap());
    let sep = i64::from(tokenizer.token_to_id(tokens[1]).unwrap());
    let mask = i64::from(tokenizer.token_to_id(tokens[2]).unwrap());
    if tokenizer.get_truncation().is_some() || tokenizer.get_padding().is_some() {
        return Err(anyhow!(
            "Disable tokenizer padding and truncation before preparing Laya inputs"
        ));
    }
    let encode = |text: &str| -> Result<Vec<i64>> {
        let encoding = tokenizer
            .encode(text.replace(tokens[2], " "), false)
            .map_err(|error| anyhow!("Laya tokenization failed: {error}"))?;
        Ok(encoding.get_ids().iter().map(|id| i64::from(*id)).collect())
    };
    // Each option needs at least its marker; reject impossible sets before tokenizing them.
    if rendered.len() > config.max_len.saturating_sub(4) {
        return Err(anyhow!("Laya has too many criteria for its token budget"));
    }
    let mut head = encode(&format!(
        "{} question: {}",
        options.question_type.name(),
        options.instructions
    ))?;
    let mut option_ids = Vec::with_capacity(rendered.len());
    for option in rendered {
        let mut ids = vec![mask];
        ids.extend(encode(&format!(" {option}"))?.into_iter().take(48));
        option_ids.push(ids);
    }
    let mut option_len: usize = option_ids.iter().map(Vec::len).sum();
    if config.head_max_len.saturating_sub(option_len) < 16 {
        let per = (config.head_max_len.saturating_sub(16) / option_ids.len()).max(4);
        for ids in &mut option_ids {
            ids.truncate(per);
        }
        option_len = option_ids.iter().map(Vec::len).sum();
    }
    head.truncate(config.head_max_len.saturating_sub(option_len).max(8));
    let mut input_ids = vec![cls];
    input_ids.extend(head);
    input_ids.push(sep);
    let mut marker_pos = Vec::with_capacity(option_ids.len());
    for ids in option_ids {
        marker_pos.push(input_ids.len() as i64);
        input_ids.extend(ids);
    }
    input_ids.push(sep);
    if marker_pos.iter().any(|pos| *pos as usize >= config.max_len) {
        return Err(anyhow!(
            "Laya has too many criteria for its token budget; reduce Criteria or their descriptions"
        ));
    }
    let room = config.max_len.saturating_sub(input_ids.len() + 1);
    input_ids.extend(encode(text)?.into_iter().take(room));
    input_ids.push(sep);
    input_ids.truncate(config.max_len);
    Ok(LayaInput {
        input_ids,
        marker_pos,
    })
}

#[cfg(feature = "execute")]
pub fn validate_session(session: &Session) -> Result<()> {
    let expected = [
        ("input_ids", TensorElementType::Int64, 2),
        ("attention_mask", TensorElementType::Int64, 2),
        ("marker_pos", TensorElementType::Int64, 2),
        ("marker_mask", TensorElementType::Bool, 2),
        ("qtype", TensorElementType::Int64, 1),
    ];
    if session.inputs().len() != expected.len() {
        return Err(anyhow!(
            "Expected a Laya ONNX graph with input_ids, attention_mask, marker_pos, marker_mask and qtype"
        ));
    }
    for (name, element, rank) in expected {
        let input = session
            .inputs()
            .iter()
            .find(|input| input.name() == name)
            .ok_or_else(|| anyhow!("Laya model is missing input {name}"))?;
        let dtype = input.dtype();
        if dtype.tensor_type() != Some(element)
            || dtype.tensor_shape().is_none_or(|shape| shape.len() != rank)
        {
            return Err(anyhow!(
                "Laya input {name} must be a rank-{rank} {element:?} tensor; got {dtype}"
            ));
        }
    }
    for name in ["logits", "act_logits"] {
        let output = session
            .outputs()
            .iter()
            .find(|output| output.name() == name)
            .ok_or_else(|| anyhow!("Laya model is missing output {name}"))?;
        let dtype = output.dtype();
        if dtype.tensor_type() != Some(TensorElementType::Float32)
            || dtype.tensor_shape().is_none_or(|shape| shape.len() != 2)
        {
            return Err(anyhow!(
                "Laya output {name} must be a rank-2 float32 tensor; got {dtype}"
            ));
        }
    }
    Ok(())
}

#[cfg(any(feature = "execute", test))]
fn softmax(logits: &[f32], scale: f64) -> Result<Vec<f64>> {
    if logits.is_empty() || logits.iter().any(|value| !value.is_finite()) {
        return Err(anyhow!("Laya returned empty or non-finite logits"));
    }
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    let mut probabilities: Vec<f64> = logits
        .iter()
        .map(|value| ((f64::from(*value) - max) / scale).exp())
        .collect();
    let sum: f64 = probabilities.iter().sum();
    for value in &mut probabilities {
        *value /= sum;
    }
    Ok(probabilities)
}

#[cfg(any(feature = "execute", test))]
fn decode_laya(
    logits: &[f32],
    actions: &[f32],
    input_tokens: usize,
    options: &LayaOptions,
    config: &LayaConfig,
) -> Result<LayaResult> {
    config.validate()?;
    let count = options.rendered_options()?.len();
    if logits.len() < count || actions.len() != 2 {
        return Err(anyhow!(
            "Laya output dimensions do not match the criteria or its two-action head"
        ));
    }
    let probabilities = softmax(&logits[..count], config.scale(options.question_type, count))?;
    let act_probability = softmax(actions, 1.0)?[0];
    // Strict comparison preserves the first option on ties, matching the reference argmax.
    let best = (1..count).fold(0, |best, i| {
        if probabilities[i] > probabilities[best] {
            i
        } else {
            best
        }
    });
    let confidence = if options.question_type == LayaQuestionType::Noul {
        probabilities[0].max(probabilities[1])
    } else if count == 1 {
        1.0
    } else {
        let entropy: f64 = probabilities.iter().map(|p| -p * p.max(1e-12).ln()).sum();
        (1.0 - entropy / (count as f64).ln()).clamp(0.0, 1.0)
    };
    let round = |value: f64| (value * 10000.0).round_ties_even() / 10000.0;
    let labels = match options.question_type {
        LayaQuestionType::Choice => options.criteria.clone(),
        LayaQuestionType::Score => (0..count).map(|i| i.to_string()).collect(),
        LayaQuestionType::Noul => vec!["false".into(), "true".into()],
    };
    Ok(LayaResult {
        question_type: options.question_type,
        choice: (options.question_type == LayaQuestionType::Choice)
            .then(|| options.criteria[best].clone()),
        score: (options.question_type == LayaQuestionType::Score).then(|| {
            round(
                probabilities
                    .iter()
                    .enumerate()
                    .map(|(i, p)| i as f64 * p)
                    .sum(),
            )
        }),
        noul: (options.question_type == LayaQuestionType::Noul).then(|| round(probabilities[1])),
        probabilities: labels
            .into_iter()
            .zip(probabilities)
            .map(|(label, probability)| LayaProbability {
                label,
                probability: round(probability),
            })
            .collect(),
        confidence: round(confidence),
        act_probability: round(act_probability),
        input_tokens,
    })
}

#[cfg(feature = "execute")]
pub fn infer_laya(
    session: &mut Session,
    tokenizer: &Tokenizer,
    text: &str,
    options: &LayaOptions,
    config: &LayaConfig,
) -> Result<LayaResult> {
    validate_session(session)?;
    let prepared = prepare_laya(tokenizer, text, options, config)?;
    let length = prepared.input_ids.len();
    let count = prepared.marker_pos.len();
    let markers = count.max(2);
    let mut positions = prepared.marker_pos;
    positions.resize(markers, 0);
    let outputs = session.run(inputs![
        "input_ids" => Value::from_array(Array2::from_shape_vec((1, length), prepared.input_ids)?)?,
        "attention_mask" => Value::from_array(Array2::from_elem((1, length), 1i64))?,
        "marker_pos" => Value::from_array(Array2::from_shape_vec((1, markers), positions)?)?,
        "marker_mask" => Value::from_array(Array2::from_shape_vec((1, markers), (0..markers).map(|i| i < count).collect())?)?,
        "qtype" => Value::from_array(Array1::from_vec(vec![options.question_type as i64]))?,
    ])?;
    let logits = outputs["logits"].try_extract_array::<f32>()?;
    let actions = outputs["act_logits"].try_extract_array::<f32>()?;
    if logits.shape() != [1, markers] || actions.shape() != [1, 2] {
        return Err(anyhow!(
            "Laya returned unexpected logits {:?} or act_logits {:?} shapes",
            logits.shape(),
            actions.shape()
        ));
    }
    decode_laya(
        &logits.iter().copied().collect::<Vec<_>>(),
        &actions.iter().copied().collect::<Vec<_>>(),
        length,
        options,
        config,
    )
}

#[crate::register_node]
#[derive(Default)]
pub struct LayaNode {}

impl LayaNode {
    pub fn new() -> Self {
        Self {}
    }
}

const MODEL_DIR_DESCRIPTION: &str = "Directory containing model.onnx, tokenizer/tokenizer.json (or tokenizer.json), and rl_agent_config.json. Missing files download automatically into this directory's managed cache (about 681 MB for the complete bundle)";

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
fn reconcile_mode_pins(node: &mut Node, mode: Option<LayaQuestionType>) {
    let choices = mode.is_none() || mode == Some(LayaQuestionType::Choice);
    let scores = mode.is_none() || mode == Some(LayaQuestionType::Score);
    let noul = mode.is_none() || mode == Some(LayaQuestionType::Noul);
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
            Some(LayaQuestionType::Choice) => ("Choices", "Unique labels to choose from"),
            Some(LayaQuestionType::Score) => (
                "Levels",
                "Ordered rubric descriptions, starting at level zero",
            ),
            _ => (
                "Criteria",
                "Choice labels or ordered score levels. Unused for noul questions",
            ),
        };
        ensure_mode_input(node, "criteria", label, description, true, 6);
    }
    if noul {
        ensure_mode_input(
            node,
            "false_description",
            "False Description",
            "Optional description of when the statement is false. Leave empty for Laya's default",
            false,
            7,
        );
        ensure_mode_input(
            node,
            "true_description",
            "True Description",
            "Optional description of when the statement is true. Leave empty for Laya's default",
            false,
            8,
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

/// Adopt the old directory pin so a catalog update keeps the user's existing connection.
fn migrate_model_directory(node: &mut Node) {
    let available = node.get_pin_by_name("model_dir").is_none_or(|pin| {
        !pin_is_wired(pin)
            && pin.default_value.as_deref().is_none_or(|bytes| {
                flow_like_types::json::from_slice::<flow_like_types::Value>(bytes)
                    .is_ok_and(|value| value.is_null())
            })
    });
    if available && let Some(legacy) = node.get_pin_by_name("cache_dir").cloned() {
        if let Some(fresh) = node.get_pin_by_name("model_dir") {
            node.pins.remove(&fresh.id.clone());
        }
        let pin = node.pins.get_mut(&legacy.id).unwrap();
        pin.name = "model_dir".into();
        pin.friendly_name = "Model Directory".into();
        pin.description = MODEL_DIR_DESCRIPTION.into();
        pin.index = 2;
    }
    let stale: Vec<_> = node
        .pins
        .values()
        .filter(|pin| ["weights", "tokenizer", "config", "cache_dir"].contains(&pin.name.as_str()))
        .map(|pin| pin.id.clone())
        .collect();
    remove_unwired_pins(node, &stale);
}

fn normalize_pin_order(node: &mut Node) {
    let inputs = [
        "exec_in",
        "model_dir",
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
    for (direction, names) in [
        (PinType::Input, inputs.as_slice()),
        (PinType::Output, outputs.as_slice()),
    ] {
        for (index, name) in names.iter().enumerate() {
            if let Some(pin) = node.get_pin_mut_by_name(name) {
                pin.index = (index + 1) as u16;
            }
        }
        // Old pins with wires stay visible until the user resolves their migration error.
        let mut remaining: Vec<_> = node
            .pins
            .values()
            .filter(|pin| pin.pin_type == direction && !names.contains(&pin.name.as_str()))
            .map(|pin| (pin.name.clone(), pin.id.clone()))
            .collect();
        remaining.sort();
        for (index, (_, id)) in remaining.into_iter().enumerate() {
            node.pins.get_mut(&id).unwrap().index = (names.len() + index + 1) as u16;
        }
    }
}

#[async_trait]
impl NodeLogic for LayaNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_laya",
            "Typed Decision (Laya)",
            "Answer a choice, rubric score, or true-probability question about text using Laya multilingual. Connect one Model Directory; existing model files are loaded and missing files download automatically from mizchi/laya-multilingual-onnx. Loaded models are reused within the execution cache.",
            "AI/ML/ONNX/NLP",
        );
        node.set_flowscript_name("onnx", "laya");
        node.set_version(2);
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
        node.add_output_pin("result", "Result", "Typed answer, calibrated probabilities, confidence, action probability and token count", VariableType::Struct).set_schema::<LayaResult>();
        node.add_output_pin(
            "confidence",
            "Confidence",
            "Normalized entropy confidence, or max(P(false), P(true)) for noul",
            VariableType::Float,
        )
        .index = 6;
        reconcile_mode_pins(&mut node, Some(LayaQuestionType::Choice));
        node
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;
        migrate_model_directory(node);
        normalize_pin_order(node);
        let Some(selector) = node.get_pin_by_name("question_type") else {
            return;
        };
        if !selector.depends_on.is_empty() {
            reconcile_mode_pins(node, None);
            return;
        }
        let mode = dynamic_pin_source_literal(node, "question_type").and_then(|value| {
            flow_like_types::json::from_value::<LayaQuestionType>(json!(value)).ok()
        });
        let Some(mode) = mode else {
            node.error = Some("Select choice, score, or noul as the Laya question type".into());
            return;
        };
        reconcile_mode_pins(node, Some(mode));
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let question_type: LayaQuestionType = context.evaluate_pin("question_type").await?;
            let criteria = match question_type {
                LayaQuestionType::Choice | LayaQuestionType::Score => {
                    context.evaluate_pin("criteria").await?
                }
                LayaQuestionType::Noul => vec![
                    context.evaluate_pin("false_description").await?,
                    context.evaluate_pin("true_description").await?,
                ],
            };
            let options = LayaOptions {
                question_type,
                instructions: context.evaluate_pin("instructions").await?,
                criteria,
            };
            options.rendered_options()?;
            let text: String = context.evaluate_pin("text").await?;
            let model_dir: FlowPath = context.evaluate_pin("model_dir").await?;
            let loaded = loading::load_laya(context, &model_dir).await?;
            let result = flow_like_types::tokio::task::spawn_blocking(move || {
                let mut session = loaded.session.blocking_lock();
                infer_laya(
                    &mut session,
                    &loaded.tokenizer,
                    &text,
                    &options,
                    &loaded.config,
                )
            })
            .await
            .map_err(|error| anyhow!("Laya inference task failed: {error}"))??;
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

    fn options(kind: LayaQuestionType, labels: &[&str]) -> LayaOptions {
        LayaOptions {
            question_type: kind,
            instructions: "Which applies?".into(),
            criteria: labels.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn choice_uses_option_temperature_and_first_label_on_ties() {
        let options = options(LayaQuestionType::Choice, &["billing", "sales"]);
        let mut config = LayaConfig::default();
        config.temperature_by_options.insert("choice:2".into(), 2.0);
        let result = decode_laya(&[0.0, 2.0], &[0.0, 0.0], 10, &options, &config).unwrap();
        assert_eq!(result.choice.as_deref(), Some("sales"));
        assert_eq!(result.probabilities[1].probability, 0.7311);
        assert_eq!(result.act_probability, 0.5);
        let tied = decode_laya(&[1000.0, 1000.0], &[0.0, 0.0], 10, &options, &config).unwrap();
        assert_eq!(tied.choice.as_deref(), Some("billing"));
        assert_eq!(tied.confidence, 0.0);
    }

    #[test]
    fn score_is_expected_zero_based_level() {
        let options = options(LayaQuestionType::Score, &["low", "medium", "high"]);
        assert_eq!(
            options.rendered_options().unwrap(),
            ["level 0: low", "level 1: medium", "level 2: high"]
        );
        let result =
            decode_laya(&[0.0; 3], &[0.0; 2], 10, &options, &LayaConfig::default()).unwrap();
        assert_eq!(result.score, Some(1.0));
        assert!(result.choice.is_none());
        assert!(result.noul.is_none());
    }

    #[test]
    fn noul_uses_true_probability_and_binary_confidence() {
        let options = options(LayaQuestionType::Noul, &[]);
        assert_eq!(
            options.rendered_options().unwrap(),
            [
                "false: no, the statement does not hold",
                "true: yes, the statement holds"
            ]
        );
        let result =
            decode_laya(&[2.0, 0.0], &[0.0; 2], 10, &options, &LayaConfig::default()).unwrap();
        assert_eq!(result.noul, Some(0.1192));
        assert_eq!(result.confidence, 0.8808);
    }

    #[test]
    fn invalid_questions_config_and_model_outputs_fail() {
        assert!(
            options(LayaQuestionType::Choice, &[])
                .rendered_options()
                .is_err()
        );
        assert!(
            options(LayaQuestionType::Choice, &["a", "a"])
                .rendered_options()
                .is_err()
        );
        assert!(
            options(LayaQuestionType::Noul, &["true"])
                .rendered_options()
                .is_err()
        );
        let mut config = LayaConfig::default();
        config.temperature[0] = f32::NAN;
        assert!(config.validate().is_err());
        config = LayaConfig::default();
        config.max_len = config.head_max_len;
        assert!(config.validate().is_err());
        assert!(softmax(&[f32::INFINITY], 1.0).is_err());
        let options = options(LayaQuestionType::Choice, &["a", "b"]);
        assert!(decode_laya(&[0.0], &[0.0; 2], 10, &options, &LayaConfig::default()).is_err());
        assert!(
            decode_laya(
                &[0.0; 2],
                &[f32::NAN; 2],
                10,
                &options,
                &LayaConfig::default()
            )
            .is_err()
        );
    }

    #[test]
    fn one_choice_has_full_confidence() {
        let options = options(LayaQuestionType::Choice, &["only"]);
        let result = decode_laya(
            &[0.0, -10000.0],
            &[0.0; 2],
            10,
            &options,
            &LayaConfig::default(),
        )
        .unwrap();
        assert_eq!(result.confidence, 1.0);
        assert_eq!(result.probabilities[0].probability, 1.0);
    }
}

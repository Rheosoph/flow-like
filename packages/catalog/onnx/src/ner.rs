/// # ONNX Named Entity Recognition (NER) Nodes
/// Token classification for extracting entities from text (persons, organizations, locations, etc.)
/// Supports various tagging schemes (BIO, BIOES, IOB) and custom label sets.
use crate::onnx::NodeOnnxSession;
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
#[cfg(feature = "execute")]
use flow_like_model_provider::ml::{
    ndarray::Array2,
    ort::{
        inputs,
        session::Session,
        value::{Value, ValueType as OrtValueType},
    },
};
use flow_like_types::{Result, anyhow, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[cfg(feature = "execute")]
use std::str::FromStr;
#[cfg(feature = "execute")]
use tokenizers::Tokenizer;

/// Tagging scheme used by the NER model
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq, Eq)]
pub enum TaggingScheme {
    /// BIO: Begin, Inside, Outside (most common)
    #[default]
    BIO,
    /// BIOES: Begin, Inside, Outside, End, Single
    BIOES,
    /// IOB: Inside, Outside, Begin (legacy format)
    IOB,
    /// BILOU: Begin, Inside, Last, Outside, Unit
    BILOU,
}

/// NER entity label with flexible parsing
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub enum EntityLabel {
    /// Outside any entity
    O,
    /// Beginning of an entity (B-TYPE)
    Begin(String),
    /// Inside an entity (I-TYPE)
    Inside(String),
    /// End of an entity (E-TYPE, for BIOES)
    End(String),
    /// Single-token entity (S-TYPE, for BIOES)
    Single(String),
    /// Last token of entity (L-TYPE, for BILOU)
    Last(String),
    /// Unit/single token (U-TYPE, for BILOU)
    Unit(String),
}

impl EntityLabel {
    /// Parse from label string, auto-detecting prefix format
    pub fn from_str(s: &str) -> Self {
        let s = s.trim();
        if s == "O" || s.is_empty() {
            return Self::O;
        }

        // Handle various formats: B-PER, B_PER, PER-B, etc.
        let (prefix, entity_type) = if s.contains('-') {
            let parts: Vec<&str> = s.splitn(2, '-').collect();
            if parts.len() == 2 {
                // Check if prefix is first or last
                match parts[0].to_uppercase().as_str() {
                    "B" | "I" | "E" | "S" | "L" | "U" => (parts[0], parts[1]),
                    _ => match parts[1].to_uppercase().as_str() {
                        "B" | "I" | "E" | "S" | "L" | "U" => (parts[1], parts[0]),
                        _ => ("B", s), // Default to B if unclear
                    },
                }
            } else {
                ("B", s)
            }
        } else if s.contains('_') {
            let parts: Vec<&str> = s.splitn(2, '_').collect();
            if parts.len() == 2 {
                match parts[0].to_uppercase().as_str() {
                    "B" | "I" | "E" | "S" | "L" | "U" => (parts[0], parts[1]),
                    _ => ("B", s),
                }
            } else {
                ("B", s)
            }
        } else {
            // No prefix, treat as entity type with implicit B
            ("B", s)
        };

        let entity_type = entity_type.to_string();
        match prefix.to_uppercase().as_str() {
            "B" => Self::Begin(entity_type),
            "I" => Self::Inside(entity_type),
            "E" => Self::End(entity_type),
            "S" => Self::Single(entity_type),
            "L" => Self::Last(entity_type),
            "U" => Self::Unit(entity_type),
            _ => Self::Begin(entity_type),
        }
    }

    /// Get the entity type (PER, ORG, LOC, etc.)
    pub fn entity_type(&self) -> Option<&str> {
        match self {
            Self::O => None,
            Self::Begin(t)
            | Self::Inside(t)
            | Self::End(t)
            | Self::Single(t)
            | Self::Last(t)
            | Self::Unit(t) => Some(t),
        }
    }

    /// Check if this starts a new entity
    pub fn is_beginning(&self) -> bool {
        matches!(self, Self::Begin(_) | Self::Single(_) | Self::Unit(_))
    }

    /// Check if this is a single-token entity
    pub fn is_single(&self) -> bool {
        matches!(self, Self::Single(_) | Self::Unit(_))
    }

    /// Check if this ends an entity
    pub fn is_ending(&self) -> bool {
        matches!(
            self,
            Self::End(_) | Self::Last(_) | Self::Single(_) | Self::Unit(_)
        )
    }
}

/// A recognized named entity
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct NamedEntity {
    /// The entity text
    pub text: String,
    /// Entity type (PER, ORG, LOC, etc.)
    pub entity_type: String,
    /// Character start position in original text
    pub start_char: usize,
    /// Character end position in original text (exclusive)
    pub end_char: usize,
    /// Start token index
    pub start_token: usize,
    /// End token index (exclusive)
    pub end_token: usize,
    /// Average confidence score
    pub confidence: f32,
}

/// NER result containing all recognized entities
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct NerResult {
    /// Recognized entities
    pub entities: Vec<NamedEntity>,
    /// Token-level predictions
    pub tokens: Vec<TokenPrediction>,
    /// Original input text
    pub text: String,
    /// Tokens per window the model actually ran on, including special tokens. Below the model's
    /// declared limit means the graph refused that limit and the window was walked down.
    pub window: usize,
    /// Number of overlapping windows the text was split across. More than one means the input was
    /// longer than a single pass and was chunked, not truncated.
    pub windows: usize,
}

/// Token-level NER prediction
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct TokenPrediction {
    /// Token text (may include ## for wordpiece)
    pub token: String,
    /// Predicted label (raw from model)
    pub label: String,
    /// Confidence score
    pub confidence: f32,
    /// Character offset start
    pub start: usize,
    /// Character offset end
    pub end: usize,
}

/// Merge BIO-tagged tokens into entities
pub fn merge_entities(
    tokens: &[String],
    labels: &[EntityLabel],
    confidences: &[f32],
    offsets: Option<&[(usize, usize)]>,
    original_text: &str,
) -> Vec<NamedEntity> {
    let mut entities = Vec::new();
    let mut current_entity: Option<(Vec<String>, String, usize, usize, usize, f32, usize)> = None;
    // (tokens, entity_type, start_token, start_char, end_char, sum_conf, count)

    for (i, (token, label)) in tokens.iter().zip(labels.iter()).enumerate() {
        let conf = confidences.get(i).copied().unwrap_or(0.0);
        let (char_start, char_end) = offsets.and_then(|o| o.get(i)).copied().unwrap_or((0, 0));

        match label {
            EntityLabel::O => {
                // Finalize current entity if any
                if let Some((toks, etype, start_tok, start_c, end_c, sum_conf, count)) =
                    current_entity.take()
                {
                    let text = reconstruct_text(&toks, start_c, end_c, original_text);
                    entities.push(NamedEntity {
                        text,
                        entity_type: etype,
                        start_char: start_c,
                        end_char: end_c,
                        start_token: start_tok,
                        end_token: i,
                        confidence: sum_conf / count as f32,
                    });
                }
            }
            _ if label.is_single() => {
                // Finalize any previous entity
                if let Some((toks, etype, start_tok, start_c, end_c, sum_conf, count)) =
                    current_entity.take()
                {
                    let text = reconstruct_text(&toks, start_c, end_c, original_text);
                    entities.push(NamedEntity {
                        text,
                        entity_type: etype,
                        start_char: start_c,
                        end_char: end_c,
                        start_token: start_tok,
                        end_token: i,
                        confidence: sum_conf / count as f32,
                    });
                }
                // Add single-token entity
                if let Some(etype) = label.entity_type() {
                    let text = reconstruct_text(
                        std::slice::from_ref(token),
                        char_start,
                        char_end,
                        original_text,
                    );
                    entities.push(NamedEntity {
                        text,
                        entity_type: etype.to_string(),
                        start_char: char_start,
                        end_char: char_end,
                        start_token: i,
                        end_token: i + 1,
                        confidence: conf,
                    });
                }
            }
            _ if label.is_beginning() => {
                // Finalize previous entity
                if let Some((toks, etype, start_tok, start_c, end_c, sum_conf, count)) =
                    current_entity.take()
                {
                    let text = reconstruct_text(&toks, start_c, end_c, original_text);
                    entities.push(NamedEntity {
                        text,
                        entity_type: etype,
                        start_char: start_c,
                        end_char: end_c,
                        start_token: start_tok,
                        end_token: i,
                        confidence: sum_conf / count as f32,
                    });
                }
                // Start new entity
                if let Some(etype) = label.entity_type() {
                    current_entity = Some((
                        vec![token.clone()],
                        etype.to_string(),
                        i,
                        char_start,
                        char_end,
                        conf,
                        1,
                    ));
                }
            }
            _ => {
                // Inside/End/Last - extend or start entity
                if let Some((
                    ref mut toks,
                    ref etype,
                    _,
                    _,
                    ref mut end_c,
                    ref mut sum_conf,
                    ref mut count,
                )) = current_entity
                {
                    if label.entity_type() == Some(etype.as_str()) {
                        toks.push(token.clone());
                        *end_c = char_end;
                        *sum_conf += conf;
                        *count += 1;

                        // If this is an ending tag, finalize
                        if label.is_ending() {
                            let (toks, etype, start_tok, start_c, end_c, sum_conf, count) =
                                current_entity.take().unwrap();
                            let text = reconstruct_text(&toks, start_c, end_c, original_text);
                            entities.push(NamedEntity {
                                text,
                                entity_type: etype,
                                start_char: start_c,
                                end_char: end_c,
                                start_token: start_tok,
                                end_token: i + 1,
                                confidence: sum_conf / count as f32,
                            });
                        }
                    } else {
                        // Type mismatch, finalize current and start new
                        let (toks, etype, start_tok, start_c, end_c, sum_conf, count) =
                            current_entity.take().unwrap();
                        let text = reconstruct_text(&toks, start_c, end_c, original_text);
                        entities.push(NamedEntity {
                            text,
                            entity_type: etype,
                            start_char: start_c,
                            end_char: end_c,
                            start_token: start_tok,
                            end_token: i,
                            confidence: sum_conf / count as f32,
                        });
                        if let Some(etype) = label.entity_type() {
                            current_entity = Some((
                                vec![token.clone()],
                                etype.to_string(),
                                i,
                                char_start,
                                char_end,
                                conf,
                                1,
                            ));
                        }
                    }
                } else if let Some(etype) = label.entity_type() {
                    // No current entity but got I/E tag - start new (robustness)
                    current_entity = Some((
                        vec![token.clone()],
                        etype.to_string(),
                        i,
                        char_start,
                        char_end,
                        conf,
                        1,
                    ));
                }
            }
        }
    }

    // Finalize last entity
    if let Some((toks, etype, start_tok, start_c, end_c, sum_conf, count)) = current_entity {
        let text = reconstruct_text(&toks, start_c, end_c, original_text);
        entities.push(NamedEntity {
            text,
            entity_type: etype,
            start_char: start_c,
            end_char: end_c,
            start_token: start_tok,
            end_token: tokens.len(),
            confidence: sum_conf / count as f32,
        });
    }

    entities
}

/// Tighten entity spans onto the text they actually cover. BPE tokenizers fold the preceding
/// space into a token's offsets, which otherwise leaks into the entity text.
pub fn trim_entity_spans(entities: &mut Vec<NamedEntity>, text: &str) {
    for entity in entities.iter_mut() {
        if entity.end_char > text.len() || entity.start_char >= entity.end_char {
            continue;
        }
        let span = &text[entity.start_char..entity.end_char];
        let trimmed = span.trim();
        if trimmed.len() == span.len() {
            continue;
        }
        let leading = span.len() - span.trim_start().len();
        entity.start_char += leading;
        entity.end_char = entity.start_char + trimmed.len();
        entity.text = trimmed.to_string();
    }
    entities.retain(|entity| entity.start_char < entity.end_char);
}

/// Reconstruct entity text from tokens or original text
fn reconstruct_text(
    tokens: &[String],
    start_char: usize,
    end_char: usize,
    original_text: &str,
) -> String {
    if start_char < end_char && end_char <= original_text.len() {
        // Use original text span for accurate reconstruction
        original_text[start_char..end_char].to_string()
    } else {
        // Fallback: join tokens, handling wordpiece markers
        tokens
            .iter()
            .map(|t| t.strip_prefix("##").unwrap_or(t))
            .collect::<Vec<_>>()
            .join("")
            .replace(" ##", "")
    }
}

/// Label set assumed when neither the Labels pin nor a `config.json` supplies one. This is the
/// `dslim/bert-base-NER` ordering; many other CoNLL-2003 models (every
/// `xlm-roberta-large-finetuned-conll03-*` export, for one) sort their labels alphabetically
/// instead, which is the same length and therefore indistinguishable. Always prefer `id2label`.
pub const CONLL_2003_LABELS: [&str; 9] = [
    "O", "B-MISC", "I-MISC", "B-PER", "I-PER", "B-ORG", "I-ORG", "B-LOC", "I-LOC",
];

/// Read `id2label` out of a HuggingFace `config.json`, ordered by class index. Returns `None`
/// when the file is not a token-classification config or its indices are not contiguous from 0,
/// because a partial mapping decodes into confidently wrong entity types.
pub fn labels_from_config(config_json: &str) -> Option<Vec<String>> {
    let config: flow_like_types::Value = flow_like_types::json::from_str(config_json).ok()?;
    let id2label = config.get("id2label")?.as_object()?;

    let mut ordered: BTreeMap<usize, String> = BTreeMap::new();
    for (key, value) in id2label {
        ordered.insert(key.parse::<usize>().ok()?, value.as_str()?.to_string());
    }

    if ordered.is_empty() || ordered.keys().copied().ne(0..ordered.len()) {
        return None;
    }

    // `LABEL_0`, `LABEL_1`, … is what HuggingFace fills in when a checkpoint never declared its
    // labels. Decoding against it yields entity types named after class indices, so it carries no
    // more information than having no config at all.
    let labels: Vec<String> = ordered.into_values().collect();
    if labels.iter().all(|label| {
        label
            .strip_prefix("LABEL_")
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    }) {
        return None;
    }

    Some(labels)
}

/// Path of `file_name` in the same directory as `path`. HuggingFace repositories ship
/// `config.json` next to `tokenizer.json`, so the label mapping can be found without a second pin.
pub fn sibling_path(path: &str, file_name: &str) -> String {
    match path.rfind('/') {
        Some(index) => format!("{}/{}", &path[..index], file_name),
        None => file_name.to_string(),
    }
}

/// Graph inputs this node knows how to build. Anything else means the model is not a plain
/// token classifier and cannot be driven from here.
#[cfg(feature = "execute")]
const SUPPORTED_MODEL_INPUTS: [&str; 3] = ["input_ids", "attention_mask", "token_type_ids"];

/// Inputs that identify a GLiNER / span-based zero-shot graph, used to explain the rejection.
#[cfg(feature = "execute")]
const SPAN_MODEL_INPUTS: [&str; 5] = [
    "words_mask",
    "text_lengths",
    "span_idx",
    "span_mask",
    "class_ids",
];

/// Window size used when the model's `config.json` is unavailable. Every BERT-era encoder accepts
/// at least this much.
pub const DEFAULT_MAX_SEQUENCE: usize = 512;

/// Upper bound on the tokens two neighbouring windows share. Enough to carry a sentence of context
/// across the seam without paying for it on every window of a long document.
pub const MAX_WINDOW_OVERLAP: usize = 128;

/// Floor for the shrink-and-retry ladder. A model that still refuses a window this small is not
/// failing because of sequence length, so there is nothing left to readjust.
pub const MIN_WINDOW_RETRY: usize = 64;

/// Longest sequence the model accepts, read from a HuggingFace `config.json`.
pub fn max_sequence_from_config(config_json: &str) -> Option<usize> {
    let config: flow_like_types::Value = flow_like_types::json::from_str(config_json).ok()?;
    let max_positions = config.get("max_position_embeddings")?.as_u64()? as usize;

    // RoBERTa-family position ids start at `pad_token_id + 1`, so the embedding table is that much
    // larger than the sequence it can actually encode: XLM-R declares 514 and accepts 512.
    let model_type = config
        .get("model_type")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let reserved = if model_type.contains("roberta") || model_type == "camembert" {
        config
            .get("pad_token_id")
            .and_then(|value| value.as_u64())
            .unwrap_or(1) as usize
            + 1
    } else {
        0
    };

    max_positions.checked_sub(reserved).filter(|size| *size > 0)
}

/// Parameters for a single NER inference pass
#[derive(Clone, Debug)]
pub struct NerOptions {
    /// Entity label names in model output order. Empty falls back to [`CONLL_2003_LABELS`].
    pub labels: Vec<String>,
    /// Minimum per-token confidence for a label to count as an entity tag
    pub threshold: f32,
    /// Tokens per window, including special tokens. `None` falls back to [`DEFAULT_MAX_SEQUENCE`].
    /// Text longer than this is not truncated — it is split into overlapping windows.
    pub max_length: Option<usize>,
}

impl Default for NerOptions {
    fn default() -> Self {
        Self {
            labels: Vec::new(),
            threshold: 0.5,
            max_length: None,
        }
    }
}

/// Split `range` into windows of at most `window` tokens that overlap by up to
/// [`MAX_WINDOW_OVERLAP`], so a token near a seam is still seen with context by one of them.
///
/// Boundaries snap backwards onto a word start where one is close enough. A word split across two
/// windows would be decoded by neither with its full spelling in view, and the sub-token
/// continuation rule in [`infer_ner`] assumes every piece of a word carries the same prediction.
pub fn plan_windows(
    range: std::ops::Range<usize>,
    window: usize,
    word_ids: &[Option<u32>],
) -> Vec<std::ops::Range<usize>> {
    if window == 0 || range.is_empty() {
        return Vec::new();
    }
    if range.len() <= window {
        return vec![range];
    }

    let overlap = (window / 4).clamp(1, MAX_WINDOW_OVERLAP).min(window - 1);
    let mut windows = Vec::new();
    let mut start = range.start;

    loop {
        let mut end = (start + window).min(range.end);
        if end < range.end {
            end = snap_to_word_start(end, end.saturating_sub(overlap).max(start + 1), word_ids);
        }
        windows.push(start..end);

        if end >= range.end {
            break;
        }

        let next = end.saturating_sub(overlap).max(start + 1);
        start = snap_to_word_start(next, next.saturating_sub(overlap).max(start + 1), word_ids);
    }

    windows
}

/// Walk `index` back to the first token of its word, giving up at `floor` so a word longer than the
/// search budget cannot stall the split.
fn snap_to_word_start(index: usize, floor: usize, word_ids: &[Option<u32>]) -> usize {
    let mut index = index;
    while index > floor {
        let previous = word_ids.get(index - 1).copied().flatten();
        let current = word_ids.get(index).copied().flatten();
        if previous.is_none() || current.is_none() || previous != current {
            break;
        }
        index -= 1;
    }
    index
}

/// Reject graphs that are not plain token classifiers before spending an inference on them.
#[cfg(feature = "execute")]
fn ensure_token_classification_inputs(session: &Session) -> Result<()> {
    let names: Vec<&str> = session.inputs().iter().map(|input| input.name()).collect();

    if !names.contains(&"input_ids") {
        return Err(anyhow!(
            "ONNX model has no `input_ids` input (inputs: [{}]); the NER node only drives token-classification graphs",
            names.join(", ")
        ));
    }

    let unsupported: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| !SUPPORTED_MODEL_INPUTS.contains(name))
        .collect();

    if unsupported.is_empty() {
        return Ok(());
    }

    let hint = if unsupported
        .iter()
        .any(|name| SPAN_MODEL_INPUTS.contains(name))
    {
        ". This is a GLiNER zero-shot/span graph — use the Zero-Shot NER (GLiNER) node instead"
    } else {
        ""
    };

    Err(anyhow!(
        "ONNX model requires inputs the NER node cannot supply: [{}]{}",
        unsupported.join(", "),
        hint
    ))
}

/// A single token's decoded prediction, plus how much context the window that produced it had.
#[cfg(feature = "execute")]
#[derive(Clone, Copy)]
struct TokenVote {
    label_idx: usize,
    confidence: f32,
    /// Distance to the nearer edge of the window. Larger means the model saw more on both sides.
    context: usize,
}

/// Why a window failed, which decides whether retrying it smaller could help.
#[cfg(feature = "execute")]
enum WindowError {
    /// The graph refused the input. A sequence longer than the weights can encode lands here, so a
    /// smaller window is worth trying.
    Inference(flow_like_types::Error),
    /// The model is not shaped like a token classifier. No window size fixes that.
    Incompatible(flow_like_types::Error),
}

#[cfg(feature = "execute")]
impl WindowError {
    fn into_error(self) -> flow_like_types::Error {
        match self {
            Self::Inference(error) | Self::Incompatible(error) => error,
        }
    }
}

/// Sequence length baked into the graph, when the export fixed it instead of leaving the axis
/// dynamic. Such a model accepts exactly this many tokens and nothing else, so every window has to
/// be padded out to it.
#[cfg(feature = "execute")]
fn graph_sequence_length(session: &Session) -> Option<usize> {
    let input = session
        .inputs()
        .iter()
        .find(|input| input.name() == "input_ids")?;

    let OrtValueType::Tensor { shape, .. } = input.dtype() else {
        return None;
    };
    if shape.len() != 2 {
        return None;
    }

    usize::try_from(shape[1]).ok().filter(|length| *length > 0)
}

/// Token the model pads with. Only consulted for graphs with a fixed sequence length, where short
/// windows have to be filled out; attention masks these positions off either way.
#[cfg(feature = "execute")]
fn padding_token(tokenizer: &Tokenizer) -> u32 {
    if let Some(padding) = tokenizer.get_padding() {
        return padding.pad_id;
    }
    ["<pad>", "[PAD]", "<|endoftext|>"]
        .iter()
        .find_map(|token| tokenizer.token_to_id(token))
        .unwrap_or(0)
}

/// Feed one window through the graph and decode a prediction per content token.
///
/// `pad_to` fills the input out to a fixed length for graphs that demand one. Returns the votes
/// aligned to `content` and the model's label count, so the caller can validate the label names
/// once rather than per window.
#[cfg(feature = "execute")]
fn run_window(
    session: &mut Session,
    ids: &[u32],
    content: std::ops::Range<usize>,
    pad_to: Option<(usize, u32)>,
) -> std::result::Result<(Vec<TokenVote>, usize), WindowError> {
    let real_len = ids.len();
    let seq_len = match pad_to {
        Some((length, _)) if length >= real_len => length,
        _ => real_len,
    };
    let batch_size = 1usize;

    let mut input_ids: Vec<i64> = Vec::with_capacity(seq_len);
    input_ids.extend(ids.iter().map(|&id| id as i64));
    input_ids.resize(seq_len, pad_to.map(|(_, token)| token).unwrap_or(0) as i64);

    let mut attention_mask = vec![1i64; real_len];
    attention_mask.resize(seq_len, 0);

    let build = |values: Vec<i64>| -> std::result::Result<Value<_>, WindowError> {
        let array = Array2::from_shape_vec((batch_size, seq_len), values)
            .map_err(|e| WindowError::Incompatible(anyhow!("Failed to shape model input: {e}")))?;
        Value::from_array(array)
            .map_err(|e| WindowError::Incompatible(anyhow!("Failed to build model input: {e}")))
    };

    let input_ids_value = build(input_ids)?;
    let attention_mask_value = build(attention_mask)?;

    let has_token_type_ids = session
        .inputs()
        .iter()
        .any(|input| input.name() == "token_type_ids");

    let outputs = if has_token_type_ids {
        let token_type_ids_value = build(vec![0i64; seq_len])?;
        session.run(inputs![
            "input_ids" => input_ids_value,
            "attention_mask" => attention_mask_value,
            "token_type_ids" => token_type_ids_value
        ])
    } else {
        session.run(inputs![
            "input_ids" => input_ids_value,
            "attention_mask" => attention_mask_value
        ])
    }
    .map_err(|e| WindowError::Inference(anyhow!("{e}")))?;

    let logits_key = outputs
        .keys()
        .find(|key| key.contains("logits") || key.contains("output"))
        .or_else(|| outputs.keys().next())
        .ok_or_else(|| WindowError::Incompatible(anyhow!("NER model produced no outputs")))?
        .to_string();

    let logits = outputs[logits_key.as_str()]
        .try_extract_array::<f32>()
        .map_err(|e| {
            WindowError::Incompatible(anyhow!(
                "NER model output `{}` is not a float32 tensor ({:?}); this node cannot decode it. Error: {}",
                logits_key,
                outputs[logits_key.as_str()].dtype(),
                e
            ))
        })?;

    let shape = logits.shape();
    if shape.len() != 3 {
        return Err(WindowError::Incompatible(anyhow!(
            "NER model output `{}` has shape {:?}; expected a rank-3 [batch, sequence, labels] token-classification tensor",
            logits_key,
            shape
        )));
    }
    if shape[0] != batch_size {
        return Err(WindowError::Incompatible(anyhow!(
            "NER model output `{}` has batch dimension {}; expected {}",
            logits_key,
            shape[0],
            batch_size
        )));
    }
    if shape[1] != seq_len {
        return Err(WindowError::Incompatible(anyhow!(
            "NER model output `{}` has sequence dimension {} but {} tokens were fed in; the model does not emit one prediction per token",
            logits_key,
            shape[1],
            seq_len
        )));
    }

    let num_labels = shape[2];
    let last = content.end.saturating_sub(1);
    let mut votes = Vec::with_capacity(content.len());

    for position in content.clone() {
        let mut label_idx = 0;
        let mut max_val = f32::NEG_INFINITY;
        for candidate in 0..num_labels {
            let val = logits[[0, position, candidate]];
            if val > max_val {
                max_val = val;
                label_idx = candidate;
            }
        }

        if !max_val.is_finite() {
            return Err(WindowError::Incompatible(anyhow!(
                "NER model produced a non-finite logit at token {}; the graph or its quantization is broken",
                position
            )));
        }

        let exp_sum: f32 = (0..num_labels)
            .map(|candidate| (logits[[0, position, candidate]] - max_val).exp())
            .sum();

        votes.push(TokenVote {
            label_idx,
            confidence: if exp_sum > 0.0 { 1.0 / exp_sum } else { 0.0 },
            context: (position - content.start).min(last - position),
        });
    }

    Ok((votes, num_labels))
}

/// Run token classification and decode entities. Text longer than the model's window is split into
/// overlapping windows rather than truncated, and every shape and dtype assumption is checked so an
/// incompatible model fails loudly instead of yielding an empty result.
#[cfg(feature = "execute")]
pub fn infer_ner(
    session: &mut Session,
    tokenizer: &Tokenizer,
    text: &str,
    options: &NerOptions,
) -> Result<NerResult> {
    ensure_token_classification_inputs(session)?;

    let encoding = tokenizer
        .encode(text, true)
        .map_err(|e| anyhow!("Tokenization failed: {}", e))?;

    let tokens: Vec<String> = encoding.get_tokens().to_vec();
    let offsets: Vec<(usize, usize)> = encoding.get_offsets().to_vec();
    let special_tokens_mask: Vec<u32> = encoding.get_special_tokens_mask().to_vec();
    let word_ids: Vec<Option<u32>> = encoding.get_word_ids().to_vec();
    let ids: Vec<u32> = encoding.get_ids().to_vec();

    if ids.is_empty() {
        return Err(anyhow!("Tokenizer produced no tokens for the input text"));
    }

    // The post-processor wraps the content in the model's own markers (`[CLS]`/`[SEP]`,
    // `<s>`/`</s>`). Every window has to carry them, so lift them off once and re-attach per window
    // instead of guessing which ids they are.
    let prefix = special_tokens_mask
        .iter()
        .take_while(|&&mask| mask == 1)
        .count();
    let suffix = special_tokens_mask
        .iter()
        .rev()
        .take_while(|&&mask| mask == 1)
        .count();
    if prefix + suffix >= ids.len() {
        return Err(anyhow!(
            "Every token was masked as special; the tokenizer does not match this model"
        ));
    }

    // A graph that fixed its sequence axis accepts exactly that many tokens and nothing else, so it
    // overrides whatever the config declared and every window gets padded out to it.
    let fixed_length = graph_sequence_length(session);
    let declared = match fixed_length {
        Some(fixed) => fixed,
        None => options.max_length.unwrap_or(DEFAULT_MAX_SEQUENCE).max(1),
    };
    let pad_to = fixed_length.map(|length| (length, padding_token(tokenizer)));

    let content = prefix..ids.len() - suffix;
    let mut window = declared;
    let mut votes: Vec<Option<TokenVote>> = vec![None; ids.len()];
    let mut num_labels = 0usize;
    let mut window_count = 0usize;

    // `max_position_embeddings` is a claim, not a guarantee: an export can bake in a shorter limit,
    // and a family whose position offset we do not model overshoots by a token or two. Rather than
    // hand a confusing runtime error to the user, walk the window down and try again.
    loop {
        let content_window = window.saturating_sub(prefix + suffix);
        if content_window == 0 {
            return Err(anyhow!(
                "The model accepts only {} tokens, which its {} special tokens consume entirely",
                window,
                prefix + suffix
            ));
        }

        votes.iter_mut().for_each(|vote| *vote = None);
        let mut failure = None;
        let planned = plan_windows(content.clone(), content_window, &word_ids);
        window_count = planned.len();

        for span in planned {
            let mut window_ids = Vec::with_capacity(prefix + span.len() + suffix);
            window_ids.extend_from_slice(&ids[..prefix]);
            window_ids.extend_from_slice(&ids[span.clone()]);
            window_ids.extend_from_slice(&ids[ids.len() - suffix..]);

            let window_content = prefix..prefix + span.len();
            match run_window(session, &window_ids, window_content, pad_to) {
                Ok((window_votes, labels)) => {
                    num_labels = labels;
                    for (vote, token_idx) in window_votes.into_iter().zip(span) {
                        // Overlapping windows both predict the seam. Keep whichever saw the token
                        // with more text on either side of it.
                        if votes[token_idx].is_none_or(|existing| vote.context > existing.context) {
                            votes[token_idx] = Some(vote);
                        }
                    }
                }
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }

        match failure {
            None => break,
            // Padding to a fixed length means the graph dictates the size; halving it cannot help.
            Some(WindowError::Inference(_)) if pad_to.is_none() && window > MIN_WINDOW_RETRY => {
                window = (window / 2).max(MIN_WINDOW_RETRY);
            }
            Some(WindowError::Inference(error)) => {
                return Err(anyhow!(
                    "NER model rejected every window from {} down to {} tokens; it cannot process this input. Last error: {}",
                    declared,
                    window,
                    error
                ));
            }
            Some(other) => return Err(other.into_error()),
        }
    }

    let label_names: Vec<String> = if options.labels.is_empty() {
        if num_labels != CONLL_2003_LABELS.len() {
            return Err(anyhow!(
                "NER model emits {} labels but no label names were supplied and the CoNLL-2003 fallback only covers {}. Connect the model's config.json to the Config pin, or pass its id2label values to the Labels pin",
                num_labels,
                CONLL_2003_LABELS.len()
            ));
        }
        CONLL_2003_LABELS.iter().map(|s| s.to_string()).collect()
    } else {
        if options.labels.len() != num_labels {
            return Err(anyhow!(
                "{} label names were supplied but the model emits {} labels; the Labels pin must list every class in model output order",
                options.labels.len(),
                num_labels
            ));
        }
        options.labels.clone()
    };

    let mut token_predictions = Vec::new();
    let mut parsed_labels = Vec::new();
    let mut confidences = Vec::new();
    let mut valid_offsets = Vec::new();
    let mut valid_tokens = Vec::new();
    let mut previous_word: Option<u32> = None;

    for token_idx in content {
        if special_tokens_mask.get(token_idx).copied().unwrap_or(0) == 1 {
            continue;
        }
        let Some(vote) = votes[token_idx] else {
            continue;
        };

        let (char_start, char_end) = offsets.get(token_idx).copied().unwrap_or((0, 0));
        let label_str = label_names[vote.label_idx].as_str();
        let confidence = vote.confidence;

        token_predictions.push(TokenPrediction {
            token: tokens[token_idx].clone(),
            label: label_str.to_string(),
            confidence,
            start: char_start,
            end: char_end,
        });

        let predicted = if confidence >= options.threshold {
            EntityLabel::from_str(label_str)
        } else {
            EntityLabel::O
        };

        // A word split into several sub-tokens is one entity, so a continuation piece may never
        // open a new one: `Red` + `##mond` stays a single `Redmond`. It can still be `O` — that
        // is how a tokenizer which folds trailing punctuation into the word (SentencePiece
        // reads `Redmond,` as one word) keeps the comma out of the span.
        let word_id = word_ids.get(token_idx).copied().flatten();
        let continues_word = word_id.is_some() && word_id == previous_word;
        previous_word = word_id;

        let label = if continues_word && predicted != EntityLabel::O {
            match parsed_labels.last().and_then(EntityLabel::entity_type) {
                Some(entity_type) => EntityLabel::Inside(entity_type.to_string()),
                None => EntityLabel::O,
            }
        } else {
            predicted
        };

        let is_outside = label == EntityLabel::O;
        parsed_labels.push(label);
        confidences.push(if is_outside { 0.0 } else { confidence });
        valid_offsets.push((char_start, char_end));
        valid_tokens.push(tokens[token_idx].clone());
    }

    if token_predictions.is_empty() {
        return Err(anyhow!(
            "Every token was masked as special; the tokenizer does not match this model"
        ));
    }

    let mut entities = merge_entities(
        &valid_tokens,
        &parsed_labels,
        &confidences,
        Some(&valid_offsets),
        text,
    );
    trim_entity_spans(&mut entities, text);

    Ok(NerResult {
        entities,
        tokens: token_predictions,
        text: text.to_string(),
        window,
        windows: window_count,
    })
}

/// The model's `config.json`, which carries both the label mapping and the sequence length. An
/// explicitly wired Config pin must work — a silent fallback there is how a model gets decoded
/// against the wrong ordering. The sibling probe is best effort, since not every tokenizer sits in
/// a full HuggingFace checkout.
#[cfg(feature = "execute")]
async fn load_model_config(
    context: &mut ExecutionContext,
    config_path: Option<FlowPath>,
    tokenizer_path: &FlowPath,
) -> Result<Option<String>> {
    if let Some(config_path) = config_path {
        let bytes = config_path.get(context, false).await.map_err(|error| {
            anyhow!("Failed to read config.json `{}`: {error}", config_path.path)
        })?;
        let raw = String::from_utf8(bytes)
            .map_err(|error| anyhow!("`{}` is not valid UTF-8: {error}", config_path.path))?;
        return Ok(Some(raw));
    }

    let sibling = FlowPath::new(
        sibling_path(&tokenizer_path.path, "config.json"),
        tokenizer_path.store_ref.clone(),
        tokenizer_path.cache_store_ref.clone(),
    );
    match sibling.get(context, false).await {
        Ok(bytes) => {
            let raw = String::from_utf8(bytes).ok();
            if raw.is_some() {
                context.log_message(
                    &format!("Read model config from `{}`", sibling.path),
                    LogLevel::Debug,
                );
            }
            Ok(raw)
        }
        Err(_) => {
            context.log_message(
                &format!(
                    "No config.json at `{}`; falling back to the assumed label order [{}] and a {}-token window. Connect the model's config.json to the Config pin if entity types or long-text results look wrong",
                    sibling.path,
                    CONLL_2003_LABELS.join(", "),
                    DEFAULT_MAX_SEQUENCE
                ),
                LogLevel::Warn,
            );
            Ok(None)
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct NerNode {}

impl NerNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for NerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_ner",
            "Named Entity Recognition",
            "Extract named entities (persons, organizations, locations, dates, etc.) from text using ONNX models. Supports BERT, RoBERTa, and other transformer-based NER models with automatic tokenization. Download models from: BERT-base-NER (https://huggingface.co/dslim/bert-base-NER), Multilingual NER (https://huggingface.co/Davlan/bert-base-multilingual-cased-ner-hrl), spaCy NER (https://huggingface.co/spacy). Text longer than the model's window is split into overlapping chunks rather than truncated, so entities are found throughout a long document. Download tokenizer.json and config.json from the same model repository — config.json carries the id2label mapping that names the entity types and the sequence length the model accepts.",
            "AI/ML/ONNX/NLP",
        );
        node.set_version(2);

        node.add_icon("/flow/icons/type.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "ONNX NER Model Session",
            VariableType::Struct,
        )
        .set_schema::<NodeOnnxSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("tokenizer", "Tokenizer", "HuggingFace tokenizer.json file for BERT/RoBERTa tokenization. Download from the same model repository.", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("config", "Config", "HuggingFace config.json of the model. Supplies the id2label mapping that decides which class index means which entity type, and max_position_embeddings, which sets how many tokens fit in one window. Left empty, the node looks for config.json next to the tokenizer. Strongly recommended: label orderings differ between models of the same size, and a wrong one mislabels every entity.", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "text",
            "Text",
            "Input text to analyze for named entities",
            VariableType::String,
        );

        node.add_input_pin("labels", "Labels", "Entity label names in model output order (e.g. ['O', 'B-PER', 'I-PER', 'B-ORG', ...]). Overrides the Config pin. If both are empty, the node falls back to the CoNLL-2003 ordering of dslim/bert-base-NER.", VariableType::String)
            .set_value_type(ValueType::Array);

        node.add_input_pin(
            "scheme",
            "Tagging Scheme",
            "Tagging scheme: BIO, BIOES, IOB, or BILOU",
            VariableType::Struct,
        )
        .set_schema::<TaggingScheme>()
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "BIO".to_string(),
                    "BIOES".to_string(),
                    "IOB".to_string(),
                    "BILOU".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!(TaggingScheme::BIO)));

        node.add_input_pin(
            "threshold",
            "Threshold",
            "Minimum confidence threshold for entity extraction (0.0-1.0)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.5)))
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build());

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "result",
            "Result",
            "Full NER result with entities and token predictions",
            VariableType::Struct,
        )
        .set_schema::<NerResult>();

        node.add_output_pin(
            "entities",
            "Entities",
            "Extracted named entities as array",
            VariableType::Struct,
        )
        .set_schema::<NamedEntity>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "entity_count",
            "Count",
            "Number of entities found",
            VariableType::Integer,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let model_ref: NodeOnnxSession = context.evaluate_pin("model").await?;
            let tokenizer_path: FlowPath = context.evaluate_pin("tokenizer").await?;
            let config_path: Option<FlowPath> = context.evaluate_pin("config").await.ok();
            let text: String = context.evaluate_pin("text").await?;
            let labels: Vec<String> = context.evaluate_pin("labels").await.unwrap_or_default();
            let _scheme: TaggingScheme = context.evaluate_pin("scheme").await.unwrap_or_default();
            let threshold: f64 = context.evaluate_pin("threshold").await.unwrap_or(0.5);

            let tokenizer_bytes = tokenizer_path.get(context, false).await?;
            let tokenizer_json = String::from_utf8(tokenizer_bytes)
                .map_err(|e| anyhow!("Invalid tokenizer.json encoding: {}", e))?;
            let tokenizer = Tokenizer::from_str(&tokenizer_json)
                .map_err(|e| anyhow!("Failed to load tokenizer: {}", e))?;

            let config_explicit = config_path
                .as_ref()
                .map(|path| path.path.clone())
                .unwrap_or_default();
            let config = load_model_config(context, config_path, &tokenizer_path).await?;

            let labels = if !labels.is_empty() {
                labels
            } else {
                match config.as_deref().and_then(labels_from_config) {
                    Some(labels) => labels,
                    None if !config_explicit.is_empty() => {
                        return Err(anyhow!(
                            "`{}` carries no usable id2label mapping (it must map every class index from 0 to a real label name)",
                            config_explicit
                        ));
                    }
                    None => Vec::new(),
                }
            };

            let max_length = config.as_deref().and_then(max_sequence_from_config);
            let options = NerOptions {
                labels,
                threshold: threshold as f32,
                max_length,
            };

            let result = {
                let session_wrapper = model_ref.get_session(context).await?;
                let mut session_guard = session_wrapper.lock().await;
                infer_ner(&mut session_guard.session, &tokenizer, &text, &options)?
            };

            let declared = max_length.unwrap_or(DEFAULT_MAX_SEQUENCE);
            if result.window < declared {
                context.log_message(
                    &format!(
                        "Ran on a {}-token window instead of the {declared} the model declares: the export fixes a shorter sequence length, or the graph refused the declared one",
                        result.window
                    ),
                    LogLevel::Warn,
                );
            }
            context.log_message(
                &format!(
                    "NER ran {} window(s) of {} tokens over {} tokens of text",
                    result.windows,
                    result.window,
                    result.tokens.len()
                ),
                LogLevel::Debug,
            );

            let entity_count = result.entities.len() as i64;

            context
                .set_pin_value("entities", json!(result.entities))
                .await?;
            context
                .set_pin_value("entity_count", json!(entity_count))
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

    #[test]
    fn test_entity_label_parsing_bio() {
        assert_eq!(EntityLabel::from_str("O"), EntityLabel::O);
        assert!(matches!(EntityLabel::from_str("B-PER"), EntityLabel::Begin(t) if t == "PER"));
        assert!(matches!(EntityLabel::from_str("I-ORG"), EntityLabel::Inside(t) if t == "ORG"));
        assert!(matches!(EntityLabel::from_str("B-LOC"), EntityLabel::Begin(t) if t == "LOC"));
    }

    #[test]
    fn test_entity_label_parsing_bioes() {
        assert!(matches!(EntityLabel::from_str("S-PER"), EntityLabel::Single(t) if t == "PER"));
        assert!(matches!(EntityLabel::from_str("E-ORG"), EntityLabel::End(t) if t == "ORG"));
    }

    #[test]
    fn test_entity_label_parsing_bilou() {
        assert!(matches!(EntityLabel::from_str("L-PER"), EntityLabel::Last(t) if t == "PER"));
        assert!(matches!(EntityLabel::from_str("U-ORG"), EntityLabel::Unit(t) if t == "ORG"));
    }

    #[test]
    fn test_entity_label_alternative_formats() {
        // Underscore separator
        assert!(matches!(EntityLabel::from_str("B_PER"), EntityLabel::Begin(t) if t == "PER"));
        // Various entity types
        assert!(matches!(EntityLabel::from_str("B-DATE"), EntityLabel::Begin(t) if t == "DATE"));
        assert!(matches!(EntityLabel::from_str("I-MONEY"), EntityLabel::Inside(t) if t == "MONEY"));
    }

    #[test]
    fn test_entity_merging_bio() {
        let tokens = vec![
            "John".to_string(),
            "Smith".to_string(),
            "works".to_string(),
            "at".to_string(),
            "Google".to_string(),
        ];
        let labels = vec![
            EntityLabel::Begin("PER".to_string()),
            EntityLabel::Inside("PER".to_string()),
            EntityLabel::O,
            EntityLabel::O,
            EntityLabel::Begin("ORG".to_string()),
        ];
        let confidences = vec![0.95, 0.92, 0.1, 0.1, 0.88];
        let offsets = vec![(0, 4), (5, 10), (11, 16), (17, 19), (20, 26)];

        let entities = merge_entities(
            &tokens,
            &labels,
            &confidences,
            Some(&offsets),
            "John Smith works at Google",
        );

        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].text, "John Smith");
        assert_eq!(entities[0].entity_type, "PER");
        assert_eq!(entities[0].start_char, 0);
        assert_eq!(entities[0].end_char, 10);
        assert_eq!(entities[1].text, "Google");
        assert_eq!(entities[1].entity_type, "ORG");
    }

    #[test]
    fn test_entity_merging_bioes() {
        let tokens = vec!["Paris".to_string()];
        let labels = vec![EntityLabel::Single("LOC".to_string())];
        let confidences = vec![0.99];
        let offsets = vec![(0, 5)];

        let entities = merge_entities(&tokens, &labels, &confidences, Some(&offsets), "Paris");

        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].text, "Paris");
        assert_eq!(entities[0].entity_type, "LOC");
    }

    #[test]
    fn test_entity_type_extraction() {
        assert_eq!(
            EntityLabel::Begin("PER".to_string()).entity_type(),
            Some("PER")
        );
        assert_eq!(
            EntityLabel::Inside("ORG".to_string()).entity_type(),
            Some("ORG")
        );
        assert_eq!(EntityLabel::O.entity_type(), None);
    }

    #[test]
    fn test_trim_entity_spans_strips_bpe_leading_space() {
        let text = "met Satya Nadella today";
        let mut entities = vec![NamedEntity {
            text: " Satya".to_string(),
            entity_type: "PER".to_string(),
            start_char: 3,
            end_char: 9,
            start_token: 1,
            end_token: 2,
            confidence: 0.9,
        }];

        trim_entity_spans(&mut entities, text);

        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].text, "Satya");
        assert_eq!(entities[0].start_char, 4);
        assert_eq!(entities[0].end_char, 9);
        assert_eq!(
            &text[entities[0].start_char..entities[0].end_char],
            entities[0].text
        );
    }

    #[test]
    fn test_trim_entity_spans_drops_whitespace_only() {
        let text = "a b";
        let mut entities = vec![NamedEntity {
            text: " ".to_string(),
            entity_type: "LOC".to_string(),
            start_char: 1,
            end_char: 2,
            start_token: 1,
            end_token: 2,
            confidence: 0.9,
        }];

        trim_entity_spans(&mut entities, text);

        assert!(entities.is_empty());
    }

    #[test]
    fn test_subword_continuation_merges_into_one_entity() {
        // What a WordPiece model emits for "Redmond": both pieces tagged B-LOC.
        let tokens = vec!["Red".to_string(), "##mond".to_string()];
        let labels = vec![
            EntityLabel::Begin("LOC".to_string()),
            EntityLabel::Inside("LOC".to_string()),
        ];
        let confidences = vec![0.99, 0.98];
        let offsets = vec![(0, 3), (3, 7)];

        let entities = merge_entities(&tokens, &labels, &confidences, Some(&offsets), "Redmond");

        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].text, "Redmond");
        assert_eq!(entities[0].entity_type, "LOC");
    }

    #[test]
    fn labels_from_config_orders_by_class_index() {
        // xlm-roberta-large-finetuned-conll03-german: 9 labels like the fallback, different order.
        let config = r#"{
            "id2label": {
                "0": "B-LOC", "1": "B-MISC", "2": "B-ORG", "3": "B-PER", "4": "I-LOC",
                "5": "I-MISC", "6": "I-ORG", "7": "I-PER", "8": "O"
            }
        }"#;

        let labels = labels_from_config(config).expect("id2label should parse");

        assert_eq!(labels.len(), CONLL_2003_LABELS.len());
        assert_eq!(labels[7], "I-PER");
        assert_eq!(labels[8], "O");
        // The fallback would have decoded these two indices as B-LOC and I-LOC.
        assert_ne!(labels.as_slice(), CONLL_2003_LABELS.as_slice());
    }

    #[test]
    fn labels_from_config_rejects_gaps_and_non_configs() {
        assert!(labels_from_config(r#"{"id2label": {"0": "O", "2": "B-PER"}}"#).is_none());
        assert!(labels_from_config(r#"{"id2label": {}}"#).is_none());
        assert!(labels_from_config(r#"{"model_type": "bert"}"#).is_none());
        assert!(labels_from_config("not json").is_none());
        // Placeholder labels name class indices, not entity types.
        assert!(labels_from_config(r#"{"id2label": {"0": "LABEL_0", "1": "LABEL_1"}}"#).is_none());
        assert!(labels_from_config(r#"{"id2label": {"0": "O", "1": "LABEL_1"}}"#).is_some());
    }

    #[test]
    fn max_sequence_reserves_the_roberta_position_offset() {
        // XLM-R declares 514 positions but only encodes 512 tokens.
        let xlm_r =
            r#"{"model_type": "xlm-roberta", "max_position_embeddings": 514, "pad_token_id": 1}"#;
        assert_eq!(max_sequence_from_config(xlm_r), Some(512));

        let bert = r#"{"model_type": "bert", "max_position_embeddings": 512}"#;
        assert_eq!(max_sequence_from_config(bert), Some(512));

        let modern = r#"{"model_type": "modernbert", "max_position_embeddings": 8192}"#;
        assert_eq!(max_sequence_from_config(modern), Some(8192));

        assert_eq!(max_sequence_from_config(r#"{"model_type": "bert"}"#), None);
        assert_eq!(max_sequence_from_config("not json"), None);
    }

    /// One word id per token, changing every `word_len` tokens.
    fn word_ids_every(count: usize, word_len: usize) -> Vec<Option<u32>> {
        (0..count)
            .map(|index| Some((index / word_len) as u32))
            .collect()
    }

    #[test]
    fn short_input_is_a_single_window() {
        let word_ids = word_ids_every(50, 1);
        assert_eq!(plan_windows(0..50, 512, &word_ids), vec![0..50]);
    }

    #[test]
    fn windows_overlap_and_cover_every_token() {
        let total = 2000;
        let word_ids = word_ids_every(total, 1);
        let windows = plan_windows(0..total, 500, &word_ids);

        assert!(windows.len() > 1);
        assert_eq!(windows[0].start, 0);
        assert_eq!(windows.last().unwrap().end, total);

        for pair in windows.windows(2) {
            let (current, next) = (&pair[0], &pair[1]);
            assert!(next.start > current.start, "windows must advance");
            assert!(next.start < current.end, "windows must overlap");
            assert!(current.len() <= 500);
        }

        // No token falls between two windows.
        let mut covered = vec![false; total];
        for span in &windows {
            for index in span.clone() {
                covered[index] = true;
            }
        }
        assert!(covered.into_iter().all(|seen| seen));
    }

    #[test]
    fn window_boundaries_land_on_word_starts() {
        let total = 1200;
        let word_len = 4;
        let word_ids = word_ids_every(total, word_len);
        let windows = plan_windows(0..total, 300, &word_ids);

        for span in &windows {
            if span.start != 0 {
                assert_eq!(span.start % word_len, 0, "window {span:?} splits a word");
            }
            if span.end != total {
                assert_eq!(span.end % word_len, 0, "window {span:?} splits a word");
            }
        }
    }

    #[test]
    fn a_word_longer_than_the_budget_still_terminates() {
        // Every token belongs to one enormous word, so no boundary is ever findable.
        let word_ids = vec![Some(0u32); 1000];
        let windows = plan_windows(0..1000, 128, &word_ids);

        assert!(windows.len() > 1);
        assert_eq!(windows.last().unwrap().end, 1000);
        for pair in windows.windows(2) {
            assert!(pair[1].start > pair[0].start);
        }
    }

    #[test]
    fn sibling_path_replaces_the_file_name() {
        assert_eq!(
            sibling_path("models/ner/tokenizer.json", "config.json"),
            "models/ner/config.json"
        );
        assert_eq!(sibling_path("tokenizer.json", "config.json"), "config.json");
    }

    #[test]
    fn test_is_beginning() {
        assert!(EntityLabel::Begin("PER".to_string()).is_beginning());
        assert!(EntityLabel::Single("LOC".to_string()).is_beginning());
        assert!(EntityLabel::Unit("ORG".to_string()).is_beginning());
        assert!(!EntityLabel::Inside("PER".to_string()).is_beginning());
        assert!(!EntityLabel::O.is_beginning());
    }
}

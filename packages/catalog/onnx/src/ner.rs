/// # ONNX Named Entity Recognition (NER) Nodes
/// Token classification for extracting entities from text (persons, organizations, locations, etc.)
/// Supports various tagging schemes (BIO, BIOES, IOB) and custom label sets.
use crate::onnx::NodeOnnxSession;
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
    ort::{inputs, session::Session, value::Value},
};
use flow_like_types::{Result, anyhow, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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
                    let text =
                        reconstruct_text(std::slice::from_ref(token), char_start, char_end, original_text);
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

/// Label set assumed when the caller supplies none. Matches CoNLL-2003 ordering.
pub const CONLL_2003_LABELS: [&str; 9] = [
    "O", "B-MISC", "I-MISC", "B-PER", "I-PER", "B-ORG", "I-ORG", "B-LOC", "I-LOC",
];

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

/// Parameters for a single NER inference pass
#[derive(Clone, Debug)]
pub struct NerOptions {
    /// Entity label names in model output order. Empty falls back to [`CONLL_2003_LABELS`].
    pub labels: Vec<String>,
    /// Minimum per-token confidence for a label to count as an entity tag
    pub threshold: f32,
    /// Maximum tokenized sequence length
    pub max_length: usize,
}

impl Default for NerOptions {
    fn default() -> Self {
        Self {
            labels: Vec::new(),
            threshold: 0.5,
            max_length: 512,
        }
    }
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

/// Run token classification and decode entities. Every shape and dtype assumption is checked so
/// an incompatible model fails loudly instead of yielding an empty result.
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

    let max_length = options.max_length.max(1);
    let seq_len = encoding.get_ids().len().min(max_length);
    if seq_len == 0 {
        return Err(anyhow!("Tokenizer produced no tokens for the input text"));
    }

    let input_ids: Vec<i64> = encoding
        .get_ids()
        .iter()
        .take(seq_len)
        .map(|&id| id as i64)
        .collect();
    let attention_mask: Vec<i64> = encoding
        .get_attention_mask()
        .iter()
        .take(seq_len)
        .map(|&mask| mask as i64)
        .collect();

    let batch_size = 1usize;
    let input_ids_value =
        Value::from_array(Array2::from_shape_vec((batch_size, seq_len), input_ids)?)?;
    let attention_mask_value = Value::from_array(Array2::from_shape_vec(
        (batch_size, seq_len),
        attention_mask,
    )?)?;

    let has_token_type_ids = session
        .inputs()
        .iter()
        .any(|input| input.name() == "token_type_ids");

    let outputs = if has_token_type_ids {
        let token_type_ids_value = Value::from_array(Array2::from_shape_vec(
            (batch_size, seq_len),
            vec![0i64; seq_len],
        )?)?;
        session.run(inputs![
            "input_ids" => input_ids_value,
            "attention_mask" => attention_mask_value,
            "token_type_ids" => token_type_ids_value
        ])?
    } else {
        session.run(inputs![
            "input_ids" => input_ids_value,
            "attention_mask" => attention_mask_value
        ])?
    };

    let logits_key = outputs
        .keys()
        .find(|key| key.contains("logits") || key.contains("output"))
        .or_else(|| outputs.keys().next())
        .ok_or_else(|| anyhow!("NER model produced no outputs"))?
        .to_string();

    let logits = outputs[logits_key.as_str()]
        .try_extract_array::<f32>()
        .map_err(|e| {
            anyhow!(
                "NER model output `{}` is not a float32 tensor ({:?}); this node cannot decode it. Error: {}",
                logits_key,
                outputs[logits_key.as_str()].dtype(),
                e
            )
        })?;

    let shape = logits.shape();
    if shape.len() != 3 {
        return Err(anyhow!(
            "NER model output `{}` has shape {:?}; expected a rank-3 [batch, sequence, labels] token-classification tensor",
            logits_key,
            shape
        ));
    }
    if shape[0] != batch_size {
        return Err(anyhow!(
            "NER model output `{}` has batch dimension {}; expected {}",
            logits_key,
            shape[0],
            batch_size
        ));
    }
    if shape[1] != seq_len {
        return Err(anyhow!(
            "NER model output `{}` has sequence dimension {} but {} tokens were fed in; the model does not emit one prediction per token",
            logits_key,
            shape[1],
            seq_len
        ));
    }

    let num_labels = shape[2];
    let label_names: Vec<String> = if options.labels.is_empty() {
        if num_labels != CONLL_2003_LABELS.len() {
            return Err(anyhow!(
                "NER model emits {} labels but no label names were supplied and the CoNLL-2003 fallback only covers {}. Pass the model's id2label values (from its config.json) to the Labels pin",
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

    for (token_idx, token) in tokens.iter().enumerate().take(seq_len) {
        if special_tokens_mask.get(token_idx).copied().unwrap_or(0) == 1 {
            continue;
        }

        let (char_start, char_end) = offsets.get(token_idx).copied().unwrap_or((0, 0));

        let mut max_idx = 0;
        let mut max_val = f32::NEG_INFINITY;
        for label_idx in 0..num_labels {
            let val = logits[[0, token_idx, label_idx]];
            if val > max_val {
                max_val = val;
                max_idx = label_idx;
            }
        }

        if !max_val.is_finite() {
            return Err(anyhow!(
                "NER model produced a non-finite logit at token {}; the graph or its quantization is broken",
                token_idx
            ));
        }

        let exp_sum: f32 = (0..num_labels)
            .map(|label_idx| (logits[[0, token_idx, label_idx]] - max_val).exp())
            .sum();
        let confidence = if exp_sum > 0.0 { 1.0 / exp_sum } else { 0.0 };

        let label_str = label_names[max_idx].as_str();

        token_predictions.push(TokenPrediction {
            token: token.to_string(),
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
        valid_tokens.push(token.clone());
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
    })
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
            "Extract named entities (persons, organizations, locations, dates, etc.) from text using ONNX models. Supports BERT, RoBERTa, and other transformer-based NER models with automatic tokenization. Download models from: BERT-base-NER (https://huggingface.co/dslim/bert-base-NER), Multilingual NER (https://huggingface.co/Davlan/bert-base-multilingual-cased-ner-hrl), spaCy NER (https://huggingface.co/spacy). Download tokenizer.json from the same model repository.",
            "AI/ML/ONNX/NLP",
        );
        node.set_version(1);

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

        node.add_input_pin(
            "text",
            "Text",
            "Input text to analyze for named entities",
            VariableType::String,
        );

        node.add_input_pin("labels", "Labels", "Entity label names in model output order (e.g. ['O', 'B-PER', 'I-PER', 'B-ORG', ...]). If empty, uses CoNLL-2003 default.", VariableType::String)
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

        node.add_input_pin(
            "max_length",
            "Max Length",
            "Maximum sequence length for tokenization (default: 512)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(512)));

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
            let text: String = context.evaluate_pin("text").await?;
            let labels: Vec<String> = context.evaluate_pin("labels").await.unwrap_or_default();
            let _scheme: TaggingScheme = context.evaluate_pin("scheme").await.unwrap_or_default();
            let threshold: f64 = context.evaluate_pin("threshold").await.unwrap_or(0.5);
            let max_length: i64 = context.evaluate_pin("max_length").await.unwrap_or(512);

            let tokenizer_bytes = tokenizer_path.get(context, false).await?;
            let tokenizer_json = String::from_utf8(tokenizer_bytes)
                .map_err(|e| anyhow!("Invalid tokenizer.json encoding: {}", e))?;
            let tokenizer = Tokenizer::from_str(&tokenizer_json)
                .map_err(|e| anyhow!("Failed to load tokenizer: {}", e))?;

            let options = NerOptions {
                labels,
                threshold: threshold as f32,
                max_length: max_length.max(1) as usize,
            };

            let result = {
                let session_wrapper = model_ref.get_session(context).await?;
                let mut session_guard = session_wrapper.lock().await;
                infer_ner(&mut session_guard.session, &tokenizer, &text, &options)?
            };

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
    fn test_is_beginning() {
        assert!(EntityLabel::Begin("PER".to_string()).is_beginning());
        assert!(EntityLabel::Single("LOC".to_string()).is_beginning());
        assert!(EntityLabel::Unit("ORG".to_string()).is_beginning());
        assert!(!EntityLabel::Inside("PER".to_string()).is_beginning());
        assert!(!EntityLabel::O.is_beginning());
    }
}

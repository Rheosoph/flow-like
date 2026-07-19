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
    ort::{inputs, value::Value},
};
use flow_like_types::{Result, anyhow, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::str::FromStr;

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
                        reconstruct_text(&[token.clone()], char_start, char_end, original_text);
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
            use tokenizers::Tokenizer;

            context.deactivate_exec_pin("exec_out").await?;

            let model_ref: NodeOnnxSession = context.evaluate_pin("model").await?;
            let tokenizer_path: FlowPath = context.evaluate_pin("tokenizer").await?;
            let text: String = context.evaluate_pin("text").await?;
            let labels_input: Vec<String> =
                context.evaluate_pin("labels").await.unwrap_or_default();
            let _scheme: TaggingScheme = context.evaluate_pin("scheme").await.unwrap_or_default();
            let threshold: f64 = context.evaluate_pin("threshold").await.unwrap_or(0.5);
            let max_length: i64 = context.evaluate_pin("max_length").await.unwrap_or(512);

            // Load tokenizer
            let tokenizer_bytes = tokenizer_path.get(context, false).await?;
            let tokenizer_json = String::from_utf8(tokenizer_bytes)
                .map_err(|e| anyhow!("Invalid tokenizer.json encoding: {}", e))?;
            let tokenizer = Tokenizer::from_str(&tokenizer_json)
                .map_err(|e| anyhow!("Failed to load tokenizer: {}", e))?;

            // Tokenize input
            let encoding = tokenizer
                .encode(text.as_str(), true)
                .map_err(|e| anyhow!("Tokenization failed: {}", e))?;

            let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
            let attention_mask: Vec<i64> = encoding
                .get_attention_mask()
                .iter()
                .map(|&m| m as i64)
                .collect();
            let tokens: Vec<String> = encoding.get_tokens().to_vec();
            let offsets: Vec<(usize, usize)> = encoding.get_offsets().to_vec();
            let special_tokens_mask: Vec<u32> = encoding.get_special_tokens_mask().to_vec();

            // Truncate if needed
            let seq_len = (input_ids.len()).min(max_length as usize);
            let input_ids: Vec<i64> = input_ids.into_iter().take(seq_len).collect();
            let attention_mask: Vec<i64> = attention_mask.into_iter().take(seq_len).collect();

            // Get session
            let session_wrapper = model_ref.get_session(context).await?;
            let mut session_guard = session_wrapper.lock().await;
            let session = &mut session_guard.session;

            // Create input tensors
            let batch_size = 1usize;
            let input_ids_arr = Array2::from_shape_vec((batch_size, seq_len), input_ids.clone())?;
            let attention_mask_arr = Array2::from_shape_vec((batch_size, seq_len), attention_mask)?;

            let input_ids_value = Value::from_array(input_ids_arr)?;
            let attention_mask_value = Value::from_array(attention_mask_arr)?;

            // Check for token_type_ids input (BERT has it, RoBERTa doesn't)
            let has_token_type_ids = session
                .inputs()
                .iter()
                .any(|i| i.name() == "token_type_ids");

            // Run inference
            let outputs = if has_token_type_ids {
                let token_type_ids: Vec<i64> = vec![0i64; seq_len];
                let token_type_ids_arr =
                    Array2::from_shape_vec((batch_size, seq_len), token_type_ids)?;
                let token_type_ids_value = Value::from_array(token_type_ids_arr)?;
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

            // Get logits - try common output names
            let logits_key = outputs
                .keys()
                .find(|k| k.contains("logits") || k.contains("output"))
                .or_else(|| outputs.keys().next())
                .ok_or_else(|| anyhow!("No output from NER model"))?;
            let logits = outputs[logits_key].try_extract_array::<f32>()?;

            // Get default labels or use provided ones
            let label_names: Vec<String> = if !labels_input.is_empty() {
                labels_input
            } else {
                // CoNLL-2003 default labels (common for BERT-base-NER)
                vec![
                    "O".to_string(),
                    "B-MISC".to_string(),
                    "I-MISC".to_string(),
                    "B-PER".to_string(),
                    "I-PER".to_string(),
                    "B-ORG".to_string(),
                    "I-ORG".to_string(),
                    "B-LOC".to_string(),
                    "I-LOC".to_string(),
                ]
            };

            // Process predictions
            let num_labels = logits.shape().last().copied().unwrap_or(label_names.len());
            let mut token_predictions = Vec::new();
            let mut parsed_labels = Vec::new();
            let mut confidences = Vec::new();
            let mut valid_offsets = Vec::new();
            let mut valid_tokens = Vec::new();

            for (token_idx, token) in tokens.iter().enumerate().take(seq_len) {
                // Skip special tokens ([CLS], [SEP], [PAD])
                let is_special = special_tokens_mask.get(token_idx).copied().unwrap_or(0) == 1;
                if is_special {
                    continue;
                }

                let (char_start, char_end) = offsets.get(token_idx).copied().unwrap_or((0, 0));

                // Find max logit
                let mut max_idx = 0;
                let mut max_val = f32::NEG_INFINITY;
                let mut logit_sum = 0.0f32;

                for label_idx in 0..num_labels {
                    let val = logits[[0, token_idx, label_idx]];
                    logit_sum += val.exp();
                    if val > max_val {
                        max_val = val;
                        max_idx = label_idx;
                    }
                }

                let confidence = max_val.exp() / logit_sum;
                let label_str = label_names.get(max_idx).map(|s| s.as_str()).unwrap_or("O");

                token_predictions.push(TokenPrediction {
                    token: token.to_string(),
                    label: label_str.to_string(),
                    confidence,
                    start: char_start,
                    end: char_end,
                });

                // Parse label and apply threshold
                if confidence >= threshold as f32 {
                    parsed_labels.push(EntityLabel::from_str(label_str));
                    confidences.push(confidence);
                } else {
                    parsed_labels.push(EntityLabel::O);
                    confidences.push(0.0);
                }
                valid_offsets.push((char_start, char_end));
                valid_tokens.push(token.clone());
            }

            // Merge tokens into entities
            let entities = merge_entities(
                &valid_tokens,
                &parsed_labels,
                &confidences,
                Some(&valid_offsets),
                &text,
            );

            let result = NerResult {
                entities: entities.clone(),
                tokens: token_predictions,
                text: text.clone(),
            };

            let entity_count = entities.len() as i64;

            context.set_pin_value("result", json!(result)).await?;
            context.set_pin_value("entities", json!(entities)).await?;
            context
                .set_pin_value("entity_count", json!(entity_count))
                .await?;
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
    fn test_is_beginning() {
        assert!(EntityLabel::Begin("PER".to_string()).is_beginning());
        assert!(EntityLabel::Single("LOC".to_string()).is_beginning());
        assert!(EntityLabel::Unit("ORG".to_string()).is_beginning());
        assert!(!EntityLabel::Inside("PER".to_string()).is_beginning());
        assert!(!EntityLabel::O.is_beginning());
    }
}

/// # ONNX GLiNER Zero-Shot NER Nodes
/// Extract entities for labels supplied at runtime, without retraining or a fixed label set.
/// GLiNER encodes the requested labels as a prompt in front of the text and scores every
/// candidate word span against them, so it needs a different graph contract than the
/// token-classification path in [`crate::ner`].
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
    ndarray::{Array2, Array3},
    ort::{inputs, session::Session, value::Value},
};
use flow_like_types::{Result, anyhow, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::str::FromStr;
#[cfg(feature = "execute")]
use tokenizers::Tokenizer;

/// Marker that introduces one entity label in the prompt.
pub const DEFAULT_ENT_TOKEN: &str = "<<ENT>>";
/// Marker that separates the label prompt from the text.
pub const DEFAULT_SEP_TOKEN: &str = "<<SEP>>";

/// Graph inputs a GLiNER export requires.
#[cfg(feature = "execute")]
const GLINER_INPUTS: [&str; 6] = [
    "input_ids",
    "attention_mask",
    "words_mask",
    "text_lengths",
    "span_idx",
    "span_mask",
];

/// A span GLiNER assigned one of the runtime labels
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct GlinerEntity {
    /// The entity text, sliced from the input
    pub text: String,
    /// The runtime label this span matched
    pub label: String,
    /// Character start position in the original text
    pub start_char: usize,
    /// Character end position in the original text (exclusive)
    pub end_char: usize,
    /// Index of the first word in the span
    pub start_word: usize,
    /// Index of the last word in the span (inclusive)
    pub end_word: usize,
    /// Sigmoid score for this span/label pair
    pub score: f32,
}

/// Result of a zero-shot extraction pass
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct GlinerResult {
    /// Accepted entities, ordered by position
    pub entities: Vec<GlinerEntity>,
    /// Original input text
    pub text: String,
    /// Labels the model was asked about
    pub labels: Vec<String>,
    /// Words the text was split into
    pub word_count: usize,
}

/// A whitespace-split word and where it sits in the source text
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    pub start_char: usize,
    pub end_char: usize,
}

/// Parameters for a single zero-shot extraction
#[derive(Clone, Debug)]
pub struct GlinerOptions {
    /// Entity types to look for, in plain language
    pub labels: Vec<String>,
    /// Minimum sigmoid score for a span to be accepted
    pub threshold: f32,
    /// Longest span, in words, the graph was exported for
    pub max_width: usize,
    /// Keep every label that clears the threshold for a span instead of only the best one
    pub multi_label: bool,
    /// Join neighbouring same-label entities separated only by whitespace. Token-level exports
    /// (`max_width` 1, e.g. NuNER Zero) score one word at a time and need this to form phrases.
    pub merge_adjacent: bool,
    /// Prompt marker introducing each label
    pub ent_token: String,
    /// Prompt marker separating labels from the text
    pub sep_token: String,
}

impl Default for GlinerOptions {
    fn default() -> Self {
        Self {
            labels: Vec::new(),
            threshold: 0.5,
            max_width: 12,
            multi_label: false,
            merge_adjacent: true,
            ent_token: DEFAULT_ENT_TOKEN.to_string(),
            sep_token: DEFAULT_SEP_TOKEN.to_string(),
        }
    }
}

/// Split text the way GLiNER does: runs of word characters (keeping internal `-`/`_`), and every
/// other non-space character on its own. Mirrors GLiNER's `\w+(?:[-_]\w+)*|\S` splitter.
pub fn split_words(text: &str) -> Vec<Word> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let is_word_char = |c: char| c.is_alphanumeric() || c == '_';

    let mut words = Vec::new();
    let mut index = 0usize;

    while index < chars.len() {
        let (start, character) = chars[index];

        if character.is_whitespace() {
            index += 1;
            continue;
        }

        let end_index = if is_word_char(character) {
            let mut cursor = index + 1;
            while cursor < chars.len() && is_word_char(chars[cursor].1) {
                cursor += 1;
            }
            while cursor + 1 < chars.len()
                && matches!(chars[cursor].1, '-' | '_')
                && is_word_char(chars[cursor + 1].1)
            {
                cursor += 2;
                while cursor < chars.len() && is_word_char(chars[cursor].1) {
                    cursor += 1;
                }
            }
            cursor
        } else {
            index + 1
        };

        let end = chars
            .get(end_index)
            .map(|(offset, _)| *offset)
            .unwrap_or(text.len());
        words.push(Word {
            text: text[start..end].to_string(),
            start_char: start,
            end_char: end,
        });
        index = end_index;
    }

    words
}

/// Build the pre-tokenized sequence GLiNER expects: `<<ENT>> label … <<SEP>> word …`.
/// Returns the sequence and how many leading elements belong to the prompt.
pub fn build_prompt(options: &GlinerOptions, words: &[Word]) -> (Vec<String>, usize) {
    let mut sequence = Vec::with_capacity(options.labels.len() * 2 + 1 + words.len());
    for label in &options.labels {
        sequence.push(options.ent_token.clone());
        sequence.push(label.to_lowercase());
    }
    sequence.push(options.sep_token.clone());

    let prompt_length = sequence.len();
    sequence.extend(words.iter().map(|word| word.text.clone()));
    (sequence, prompt_length)
}

/// `words_mask` marks the first sub-token of every *text* word with its 1-based word index.
/// Prompt tokens, continuation sub-tokens and specials are all zero.
pub fn build_words_mask(word_ids: &[Option<u32>], prompt_length: usize) -> Vec<i64> {
    let mut mask = Vec::with_capacity(word_ids.len());
    let mut previous: Option<u32> = None;
    let mut seen_words = 0usize;

    for word_id in word_ids {
        match word_id {
            None => mask.push(0),
            Some(id) => {
                if previous != Some(*id) {
                    seen_words += 1;
                    if seen_words > prompt_length {
                        mask.push((seen_words - prompt_length) as i64);
                    } else {
                        mask.push(0);
                    }
                } else {
                    mask.push(0);
                }
                previous = Some(*id);
            }
        }
    }

    mask
}

/// Candidate spans, laid out as `word_count * max_width` pairs so the graph can reshape the
/// logits to `[batch, words, width, labels]`. Invalid spans stay in place but are masked off.
pub fn build_spans(word_count: usize, max_width: usize) -> (Vec<[i64; 2]>, Vec<bool>) {
    let mut spans = Vec::with_capacity(word_count * max_width);
    let mut mask = Vec::with_capacity(word_count * max_width);

    for start in 0..word_count {
        for width in 0..max_width {
            let end = start + width;
            let valid = end < word_count;
            spans.push([start as i64, if valid { end as i64 } else { 0 }]);
            mask.push(valid);
        }
    }

    (spans, mask)
}

#[cfg(feature = "execute")]
fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value).exp())
}

/// Reject anything that is not a GLiNER export before spending an inference on it.
#[cfg(feature = "execute")]
fn ensure_gliner_inputs(session: &Session) -> Result<()> {
    let names: Vec<&str> = session.inputs().iter().map(|input| input.name()).collect();
    let missing: Vec<&str> = GLINER_INPUTS
        .iter()
        .copied()
        .filter(|expected| !names.contains(expected))
        .collect();

    if missing.is_empty() {
        return Ok(());
    }

    let hint = if names.contains(&"input_ids") && names.len() <= 3 {
        ". This looks like a plain token-classification model — use the Named Entity Recognition node instead"
    } else {
        ""
    };

    Err(anyhow!(
        "ONNX model is missing the GLiNER inputs [{}] (found: [{}]){}",
        missing.join(", "),
        names.join(", "),
        hint
    ))
}

/// Keep the highest-scoring spans, dropping any that overlap an already accepted one.
fn resolve_overlaps(mut candidates: Vec<GlinerEntity>, multi_label: bool) -> Vec<GlinerEntity> {
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut accepted: Vec<GlinerEntity> = Vec::new();
    for candidate in candidates {
        let clashes = accepted.iter().any(|kept| {
            if multi_label
                && kept.start_word == candidate.start_word
                && kept.end_word == candidate.end_word
            {
                return kept.label == candidate.label;
            }
            candidate.start_word <= kept.end_word && kept.start_word <= candidate.end_word
        });
        if !clashes {
            accepted.push(candidate);
        }
    }

    accepted.sort_by_key(|entity| (entity.start_char, entity.end_char));
    accepted
}

/// Join neighbouring entities that carry the same label and are touching or separated only by
/// whitespace. A token-level export scores `Satya` and `Nadella` — or `sarah`, `.`, `mueller` —
/// independently; this rebuilds the phrase. An unlabelled word in between (`Berlin, Hamburg`)
/// leaves a non-whitespace gap and keeps the two apart.
fn merge_adjacent_entities(entities: Vec<GlinerEntity>, text: &str) -> Vec<GlinerEntity> {
    let mut merged: Vec<GlinerEntity> = Vec::with_capacity(entities.len());

    for entity in entities {
        let joinable = merged.last().is_some_and(|previous| {
            previous.label == entity.label
                && previous.end_char <= entity.start_char
                && text
                    .get(previous.end_char..entity.start_char)
                    .is_some_and(|gap| gap.chars().all(char::is_whitespace))
        });

        if joinable {
            let previous = merged.last_mut().expect("checked above");
            previous.end_char = entity.end_char;
            previous.end_word = entity.end_word;
            previous.score = previous.score.min(entity.score);
            previous.text = text[previous.start_char..previous.end_char].to_string();
        } else {
            merged.push(entity);
        }
    }

    merged
}

/// Run zero-shot extraction for the labels in `options`.
#[cfg(feature = "execute")]
pub fn infer_gliner(
    session: &mut Session,
    tokenizer: &Tokenizer,
    text: &str,
    options: &GlinerOptions,
) -> Result<GlinerResult> {
    ensure_gliner_inputs(session)?;

    if options.labels.is_empty() {
        return Err(anyhow!(
            "GLiNER needs at least one label to look for; the Labels pin is empty"
        ));
    }
    let max_width = options.max_width.max(1);

    let words = split_words(text);
    if words.is_empty() {
        return Ok(GlinerResult {
            entities: Vec::new(),
            text: text.to_string(),
            labels: options.labels.clone(),
            word_count: 0,
        });
    }

    let (sequence, prompt_length) = build_prompt(options, &words);
    let encoding = tokenizer
        .encode(sequence, true)
        .map_err(|e| anyhow!("Tokenization failed: {}", e))?;

    let seq_len = encoding.get_ids().len();
    let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
    let attention_mask: Vec<i64> = encoding
        .get_attention_mask()
        .iter()
        .map(|&mask| mask as i64)
        .collect();
    let words_mask = build_words_mask(encoding.get_word_ids(), prompt_length);

    let highest_marked = words_mask.iter().copied().max().unwrap_or(0);
    if highest_marked as usize != words.len() {
        return Err(anyhow!(
            "Tokenizer mapped {} of {} words into the prompt; the text is longer than the model's window or the tokenizer does not match this model",
            highest_marked,
            words.len()
        ));
    }

    let (spans, span_mask) = build_spans(words.len(), max_width);
    let span_count = spans.len();
    let flat_spans: Vec<i64> = spans.iter().flat_map(|span| [span[0], span[1]]).collect();

    let input_ids_value = Value::from_array(Array2::from_shape_vec((1, seq_len), input_ids)?)?;
    let attention_value = Value::from_array(Array2::from_shape_vec((1, seq_len), attention_mask)?)?;
    let words_mask_value = Value::from_array(Array2::from_shape_vec((1, seq_len), words_mask)?)?;
    let text_lengths_value =
        Value::from_array(Array2::from_shape_vec((1, 1), vec![words.len() as i64])?)?;
    let span_idx_value =
        Value::from_array(Array3::from_shape_vec((1, span_count, 2), flat_spans)?)?;
    let span_mask_value =
        Value::from_array(Array2::from_shape_vec((1, span_count), span_mask.clone())?)?;

    let outputs = session.run(inputs![
        "input_ids" => input_ids_value,
        "attention_mask" => attention_value,
        "words_mask" => words_mask_value,
        "text_lengths" => text_lengths_value,
        "span_idx" => span_idx_value,
        "span_mask" => span_mask_value
    ])?;

    let logits = outputs["logits"].try_extract_array::<f32>().map_err(|e| {
        anyhow!(
            "GLiNER output `logits` is not a float32 tensor ({:?}): {e}",
            outputs["logits"].dtype()
        )
    })?;

    let shape = logits.shape();
    if shape.len() != 4 {
        return Err(anyhow!(
            "GLiNER output `logits` has shape {:?}; expected rank-4 [batch, words, width, labels]",
            shape
        ));
    }
    if shape[3] != options.labels.len() {
        return Err(anyhow!(
            "GLiNER scored {} labels but {} were requested; the prompt and the graph disagree",
            shape[3],
            options.labels.len()
        ));
    }

    let scored_words = shape[1].min(words.len());
    let scored_widths = shape[2].min(max_width);

    let mut candidates = Vec::new();
    for start in 0..scored_words {
        for width in 0..scored_widths {
            let end = start + width;
            if end >= words.len() {
                continue;
            }
            for (label_index, label) in options.labels.iter().enumerate() {
                let score = sigmoid(logits[[0, start, width, label_index]]);
                if score < options.threshold {
                    continue;
                }
                let start_char = words[start].start_char;
                let end_char = words[end].end_char;
                candidates.push(GlinerEntity {
                    text: text[start_char..end_char].to_string(),
                    label: label.clone(),
                    start_char,
                    end_char,
                    start_word: start,
                    end_word: end,
                    score,
                });
            }
        }
    }

    let mut entities = resolve_overlaps(candidates, options.multi_label);
    if options.merge_adjacent {
        entities = merge_adjacent_entities(entities, text);
    }

    Ok(GlinerResult {
        entities,
        text: text.to_string(),
        labels: options.labels.clone(),
        word_count: words.len(),
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct GlinerNode {}

impl GlinerNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GlinerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_gliner",
            "Zero-Shot NER (GLiNER)",
            "Extract entities for any labels you name at runtime, with no fixed label set and no retraining. Load a GLiNER ONNX export (e.g. https://huggingface.co/onnx-community/gliner_small-v2.1, gliner_multi-v2.1, gliner_medium_news-v2.1, gliner_multi_pii-v1, NuNER_Zero) plus the tokenizer.json from the same repository. For models with a fixed label set, use the Named Entity Recognition node instead.",
            "AI/ML/ONNX/NLP",
        );
        node.set_flowscript_name("onnx", "gliner");
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
            "ONNX GLiNER Model Session",
            VariableType::Struct,
        )
        .set_schema::<NodeOnnxSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "tokenizer",
            "Tokenizer",
            "HuggingFace tokenizer.json from the same model repository",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "text",
            "Text",
            "Input text to analyze for named entities",
            VariableType::String,
        );

        node.add_input_pin(
            "labels",
            "Labels",
            "Entity types to look for, in plain language (e.g. person, company, medication, invoice number)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "threshold",
            "Threshold",
            "Minimum confidence for a span to be reported (0.0-1.0)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.5)))
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build());

        node.add_input_pin(
            "max_width",
            "Max Span Width",
            "Longest entity in words. Must match the model's max_width from gliner_config.json (12 for most GLiNER models, 1 for NuNER Zero)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(12)));

        node.add_input_pin(
            "multi_label",
            "Multi Label",
            "Report every label that clears the threshold for a span instead of only the best one",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "merge_adjacent",
            "Merge Adjacent",
            "Join neighbouring same-label entities separated only by whitespace. Required for token-level models such as NuNER Zero, which score one word at a time",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);

        node.add_output_pin(
            "result",
            "Result",
            "Full zero-shot result with entities and the labels that were requested",
            VariableType::Struct,
        )
        .set_schema::<GlinerResult>();

        node.add_output_pin(
            "entities",
            "Entities",
            "Extracted entities as array",
            VariableType::Struct,
        )
        .set_schema::<GlinerEntity>()
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
            let threshold: f64 = context.evaluate_pin("threshold").await.unwrap_or(0.5);
            let max_width: i64 = context.evaluate_pin("max_width").await.unwrap_or(12);
            let multi_label: bool = context.evaluate_pin("multi_label").await.unwrap_or(false);
            let merge_adjacent: bool = context.evaluate_pin("merge_adjacent").await.unwrap_or(true);

            let tokenizer_bytes = tokenizer_path.get(context, false).await?;
            let tokenizer_json = String::from_utf8(tokenizer_bytes)
                .map_err(|e| anyhow!("Invalid tokenizer.json encoding: {}", e))?;
            let tokenizer = Tokenizer::from_str(&tokenizer_json)
                .map_err(|e| anyhow!("Failed to load tokenizer: {}", e))?;

            let options = GlinerOptions {
                labels,
                threshold: threshold as f32,
                max_width: max_width.max(1) as usize,
                multi_label,
                merge_adjacent,
                ..Default::default()
            };

            let result = {
                let session_wrapper = model_ref.get_session(context).await?;
                let mut session_guard = session_wrapper.lock().await;
                infer_gliner(&mut session_guard.session, &tokenizer, &text, &options)?
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
    fn split_words_keeps_offsets_and_punctuation() {
        let text = "Angela Merkel visited Redmond, WA.";
        let words = split_words(text);
        let rendered: Vec<&str> = words.iter().map(|word| word.text.as_str()).collect();

        assert_eq!(
            rendered,
            vec!["Angela", "Merkel", "visited", "Redmond", ",", "WA", "."]
        );
        for word in &words {
            assert_eq!(&text[word.start_char..word.end_char], word.text);
        }
    }

    #[test]
    fn split_words_keeps_hyphenated_words_together() {
        let words = split_words("state-of-the-art e_mail x- -y");
        let rendered: Vec<&str> = words.iter().map(|word| word.text.as_str()).collect();
        assert_eq!(
            rendered,
            vec!["state-of-the-art", "e_mail", "x", "-", "-", "y"]
        );
    }

    #[test]
    fn split_words_handles_multibyte_text() {
        let text = "Müller wohnt in Köln";
        let words = split_words(text);
        assert_eq!(words.len(), 4);
        for word in &words {
            assert_eq!(&text[word.start_char..word.end_char], word.text);
        }
        assert_eq!(words[0].text, "Müller");
    }

    #[test]
    fn prompt_puts_labels_before_the_text() {
        let options = GlinerOptions {
            labels: vec!["Person".to_string(), "city".to_string()],
            ..Default::default()
        };
        let words = split_words("Angela lives in Berlin");
        let (sequence, prompt_length) = build_prompt(&options, &words);

        assert_eq!(prompt_length, 5);
        assert_eq!(
            &sequence[..5],
            &[
                DEFAULT_ENT_TOKEN.to_string(),
                "person".to_string(),
                DEFAULT_ENT_TOKEN.to_string(),
                "city".to_string(),
                DEFAULT_SEP_TOKEN.to_string(),
            ]
        );
        assert_eq!(sequence.len(), 5 + words.len());
    }

    #[test]
    fn words_mask_numbers_only_text_words() {
        // Two prompt elements, then three text words; the second text word has two sub-tokens.
        let word_ids = vec![
            None,
            Some(0),
            Some(1),
            Some(2),
            Some(3),
            Some(3),
            Some(4),
            None,
        ];
        let mask = build_words_mask(&word_ids, 2);
        assert_eq!(mask, vec![0, 0, 0, 1, 2, 0, 3, 0]);
    }

    #[test]
    fn spans_cover_every_start_and_width() {
        let (spans, mask) = build_spans(3, 2);
        assert_eq!(spans.len(), 6);
        assert_eq!(spans, vec![[0, 0], [0, 1], [1, 1], [1, 2], [2, 2], [2, 0]]);
        assert_eq!(mask, vec![true, true, true, true, true, false]);
    }

    fn entity(start_word: usize, end_word: usize, label: &str, score: f32) -> GlinerEntity {
        GlinerEntity {
            text: String::new(),
            label: label.to_string(),
            start_char: start_word,
            end_char: end_word + 1,
            start_word,
            end_word,
            score,
        }
    }

    #[test]
    fn overlapping_spans_keep_the_best_score() {
        let resolved = resolve_overlaps(
            vec![
                entity(0, 1, "person", 0.9),
                entity(1, 2, "city", 0.7),
                entity(4, 4, "city", 0.8),
            ],
            false,
        );

        let kept: Vec<(usize, usize)> = resolved
            .iter()
            .map(|entity| (entity.start_word, entity.end_word))
            .collect();
        assert_eq!(kept, vec![(0, 1), (4, 4)]);
    }

    #[test]
    fn adjacent_same_label_entities_are_joined() {
        let text = "Satya Nadella visited Berlin, Hamburg";
        let entities = vec![
            GlinerEntity {
                text: "Satya".to_string(),
                label: "person".to_string(),
                start_char: 0,
                end_char: 5,
                start_word: 0,
                end_word: 0,
                score: 0.9,
            },
            GlinerEntity {
                text: "Nadella".to_string(),
                label: "person".to_string(),
                start_char: 6,
                end_char: 13,
                start_word: 1,
                end_word: 1,
                score: 0.8,
            },
        ];

        let merged = merge_adjacent_entities(entities, text);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "Satya Nadella");
        assert_eq!(merged[0].end_word, 1);
        assert!((merged[0].score - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn punctuation_between_spans_prevents_merging() {
        let text = "visited Berlin, Hamburg";
        let entities = vec![
            GlinerEntity {
                text: "Berlin".to_string(),
                label: "city".to_string(),
                start_char: 8,
                end_char: 14,
                start_word: 1,
                end_word: 1,
                score: 0.9,
            },
            GlinerEntity {
                text: "Hamburg".to_string(),
                label: "city".to_string(),
                start_char: 16,
                end_char: 23,
                start_word: 3,
                end_word: 3,
                score: 0.9,
            },
        ];

        let merged = merge_adjacent_entities(entities, text);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn touching_same_label_tokens_are_joined() {
        // What a token-level model emits for an email: every piece scored on its own.
        let text = "mail sarah.mueller@example.com now";
        let pieces = [
            (5, 10),
            (10, 11),
            (11, 18),
            (18, 19),
            (19, 26),
            (26, 27),
            (27, 30),
        ];
        let entities: Vec<GlinerEntity> = pieces
            .iter()
            .enumerate()
            .map(|(index, (start, end))| GlinerEntity {
                text: text[*start..*end].to_string(),
                label: "email address".to_string(),
                start_char: *start,
                end_char: *end,
                start_word: index + 1,
                end_word: index + 1,
                score: 0.99,
            })
            .collect();

        let merged = merge_adjacent_entities(entities, text);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "sarah.mueller@example.com");
    }

    #[test]
    fn different_labels_are_never_joined() {
        let text = "Angela Berlin";
        let entities = vec![
            GlinerEntity {
                text: "Angela".to_string(),
                label: "person".to_string(),
                start_char: 0,
                end_char: 6,
                start_word: 0,
                end_word: 0,
                score: 0.9,
            },
            GlinerEntity {
                text: "Berlin".to_string(),
                label: "city".to_string(),
                start_char: 7,
                end_char: 13,
                start_word: 1,
                end_word: 1,
                score: 0.9,
            },
        ];

        assert_eq!(merge_adjacent_entities(entities, text).len(), 2);
    }

    #[test]
    fn multi_label_keeps_both_labels_for_one_span() {
        let resolved = resolve_overlaps(
            vec![entity(0, 1, "person", 0.9), entity(0, 1, "politician", 0.8)],
            true,
        );

        assert_eq!(resolved.len(), 2);
    }
}

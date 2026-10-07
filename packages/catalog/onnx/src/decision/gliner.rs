//! GLiNER2 classification uses a schema prefix and scores its `[L]` label markers.
//! The prompt layout follows Fastino's GLiNER2 processor and whitespace word splitter.

use crate::gliner_decision::{
    DecisionOptions, DecisionProbability, DecisionQuestionType, DecisionResult,
};
use flow_like_model_provider::ml::{
    ndarray::Array2,
    ort::{
        inputs,
        session::Session,
        value::{TensorElementType, Value},
    },
};
use flow_like_types::{Result, anyhow, regex::Regex};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use tokenizers::Tokenizer;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct GlinerConfig {
    pub max_length: usize,
    pub temperature: f64,
}

impl Default for GlinerConfig {
    fn default() -> Self {
        Self {
            max_length: 512,
            temperature: 1.0,
        }
    }
}

impl GlinerConfig {
    pub fn validate(&self) -> Result<()> {
        if !(8..=32768).contains(&self.max_length) {
            return Err(anyhow!(
                "GLiNER max_length must be between 8 and 32768 tokens"
            ));
        }
        if !self.temperature.is_finite() || self.temperature <= 0.0 {
            return Err(anyhow!("GLiNER temperature must be finite and positive"));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct GlinerInput {
    pub input_ids: Vec<i64>,
    pub label_positions: Vec<i64>,
}

struct Question {
    prompt: String,
    labels: Vec<String>,
}

fn question(options: &DecisionOptions) -> Result<Question> {
    options.validate()?;
    let mut prompt = options.instructions.clone();
    let labels = match options.question_type {
        DecisionQuestionType::Choice | DecisionQuestionType::Score => {
            if options.question_type == DecisionQuestionType::Choice {
                options.criteria.clone()
            } else {
                for (index, description) in options.criteria.iter().enumerate() {
                    prompt.push_str(&format!(" [DESCRIPTION] {index}: {description}"));
                }
                (0..options.criteria.len())
                    .map(|index| index.to_string())
                    .collect()
            }
        }
        DecisionQuestionType::Noul => {
            for (label, description) in ["false", "true"].into_iter().zip(&options.criteria) {
                if !description.trim().is_empty() {
                    prompt.push_str(&format!(" [DESCRIPTION] {label}: {description}"));
                }
            }
            vec!["false".into(), "true".into()]
        }
    };
    Ok(Question { prompt, labels })
}

// Python's Unicode \w includes letters, numbers and underscore, but excludes combining
// marks and joiners. Its \s also includes U+001C through U+001F.
static WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)(?:https?://[^\s\x{001c}-\x{001f}]+|www\.[^\s\x{001c}-\x{001f}]+)
        |[a-zİıſK0-9._%+\-]+@[a-zİıſK0-9.\-]+\.[a-zİıſK]{2,}
        |@[a-zİıſK0-9_]+
        |[\p{L}\p{N}_]+(?:[-_][\p{L}\p{N}_]+)*
        |[^\s\x{001c}-\x{001f}]",
    )
    .expect("GLiNER word splitter is a valid regex")
});

/// Keep the complete schema and right-truncate the text to the exported graph's budget.
pub fn prepare_gliner(
    tokenizer: &Tokenizer,
    text: &str,
    options: &DecisionOptions,
    config: &GlinerConfig,
) -> Result<GlinerInput> {
    config.validate()?;
    let question = question(options)?;
    if tokenizer.get_truncation().is_some() || tokenizer.get_padding().is_some() {
        return Err(anyhow!(
            "Disable tokenizer padding and truncation before preparing GLiNER inputs"
        ));
    }
    for token in ["[P]", "[L]", "[SEP_TEXT]", "[DESCRIPTION]"] {
        if tokenizer.token_to_id(token).is_none() {
            return Err(anyhow!("GLiNER tokenizer must define {token}"));
        }
    }
    if question.labels.len() >= config.max_length {
        return Err(anyhow!("GLiNER has too many criteria for its token budget"));
    }
    let encode = |piece: &str| -> Result<Vec<i64>> {
        let encoding = tokenizer
            .encode(piece, false)
            .map_err(|error| anyhow!("GLiNER tokenization failed: {error}"))?;
        Ok(encoding.get_ids().iter().map(|&id| i64::from(id)).collect())
    };
    let mut input_ids = Vec::new();
    for piece in ["(", "[P]", &question.prompt, "("] {
        input_ids.extend(encode(piece)?);
    }
    let marker_id = i64::from(tokenizer.token_to_id("[L]").unwrap());
    if encode("[L]")? != [marker_id] {
        return Err(anyhow!(
            "GLiNER tokenizer must encode [L] as one marker token"
        ));
    }
    let mut label_positions = Vec::with_capacity(question.labels.len());
    for label in question.labels {
        label_positions.push(input_ids.len() as i64);
        input_ids.push(marker_id);
        input_ids.extend(encode(&label)?);
    }
    for piece in [")", ")", "[SEP_TEXT]"] {
        input_ids.extend(encode(piece)?);
    }
    if input_ids.len() >= config.max_length {
        return Err(anyhow!(
            "GLiNER question and criteria exhaust its {}-token budget; shorten Instructions or Criteria",
            config.max_length
        ));
    }
    let normalized = if text.ends_with(['.', '!', '?']) {
        text.to_owned()
    } else {
        format!("{text}.")
    };
    for word in WORDS.find_iter(&normalized) {
        let available = config.max_length - input_ids.len();
        if available == 0 {
            break;
        }
        input_ids.extend(
            encode(&word.as_str().to_lowercase())?
                .into_iter()
                .take(available),
        );
    }
    Ok(GlinerInput {
        input_ids,
        label_positions,
    })
}

fn check_tensor(
    name: &str,
    dtype: &flow_like_model_provider::ml::ort::value::ValueType,
    element: TensorElementType,
    rank: usize,
) -> Result<()> {
    if dtype.tensor_type() != Some(element)
        || dtype.tensor_shape().is_none_or(|shape| shape.len() != rank)
    {
        return Err(anyhow!(
            "GLiNER tensor {name} must be rank-{rank} {element:?}; got {dtype}"
        ));
    }
    Ok(())
}

fn check_input(
    session: &Session,
    name: &str,
    element: TensorElementType,
    rank: usize,
) -> Result<()> {
    let input = session
        .inputs()
        .iter()
        .find(|input| input.name() == name)
        .ok_or_else(|| anyhow!("GLiNER model is missing input {name}"))?;
    check_tensor(name, input.dtype(), element, rank)
}

fn check_output(session: &Session, name: &str, rank: usize) -> Result<()> {
    let output = session
        .outputs()
        .iter()
        .find(|output| output.name() == name)
        .ok_or_else(|| anyhow!("GLiNER model is missing output {name}"))?;
    check_tensor(name, output.dtype(), TensorElementType::Float32, rank)
}

fn positions_name(session: &Session) -> Result<&'static str> {
    ["label_positions", "marker_positions"]
        .into_iter()
        .find(|name| session.inputs().iter().any(|input| input.name() == *name))
        .ok_or_else(|| anyhow!("GLiNER classifier requires label_positions or marker_positions"))
}

pub fn validate_session(session: &Session) -> Result<()> {
    if session.inputs().len() != 3 {
        return Err(anyhow!(
            "Expected a GLiNER classifier graph with input_ids, attention_mask and label_positions or marker_positions"
        ));
    }
    for name in ["input_ids", "attention_mask", positions_name(session)?] {
        check_input(session, name, TensorElementType::Int64, 2)?;
    }
    check_output(session, "logits", 2)
}

pub fn validate_split_sessions(encoder: &Session, classifier: &Session) -> Result<()> {
    if encoder.inputs().len() != 2 || classifier.inputs().len() != 1 {
        return Err(anyhow!(
            "Expected GLiNER encoder inputs input_ids and attention_mask, and classifier input cls_embeds"
        ));
    }
    for name in ["input_ids", "attention_mask"] {
        check_input(encoder, name, TensorElementType::Int64, 2)?;
    }
    check_output(encoder, "last_hidden_state", 3)?;
    check_input(classifier, "cls_embeds", TensorElementType::Float32, 2)?;
    check_output(classifier, "logits", 1)
}

fn decode_gliner(
    logits: &[f32],
    input_tokens: usize,
    options: &DecisionOptions,
    config: &GlinerConfig,
) -> Result<DecisionResult> {
    config.validate()?;
    let labels = question(options)?.labels;
    if logits.len() != labels.len() || logits.iter().any(|value| !value.is_finite()) {
        return Err(anyhow!(
            "GLiNER must return one finite logit for each criterion"
        ));
    }
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    let mut probabilities: Vec<f64> = logits
        .iter()
        .map(|&value| ((f64::from(value) - max) / config.temperature).exp())
        .collect();
    let sum: f64 = probabilities.iter().sum();
    for probability in &mut probabilities {
        *probability /= sum;
    }
    let best = (1..labels.len()).fold(0, |best, i| {
        if probabilities[i] > probabilities[best] {
            i
        } else {
            best
        }
    });
    let confidence = if options.question_type == DecisionQuestionType::Noul {
        probabilities[0].max(probabilities[1])
    } else if labels.len() == 1 {
        1.0
    } else {
        let entropy: f64 = probabilities.iter().map(|p| -p * p.max(1e-12).ln()).sum();
        (1.0 - entropy / (labels.len() as f64).ln()).clamp(0.0, 1.0)
    };
    let round = |value: f64| (value * 10000.0).round_ties_even() / 10000.0;
    Ok(DecisionResult {
        question_type: options.question_type,
        choice: (options.question_type == DecisionQuestionType::Choice)
            .then(|| labels[best].clone()),
        score: (options.question_type == DecisionQuestionType::Score).then(|| {
            round(
                probabilities
                    .iter()
                    .enumerate()
                    .map(|(i, p)| i as f64 * p)
                    .sum(),
            )
        }),
        noul: (options.question_type == DecisionQuestionType::Noul)
            .then(|| round(probabilities[1])),
        probabilities: labels
            .into_iter()
            .zip(probabilities)
            .map(|(label, probability)| DecisionProbability {
                label,
                probability: round(probability),
            })
            .collect(),
        confidence: round(confidence),
        input_tokens,
    })
}

pub fn infer_gliner(
    session: &mut Session,
    tokenizer: &Tokenizer,
    text: &str,
    options: &DecisionOptions,
    config: &GlinerConfig,
) -> Result<DecisionResult> {
    validate_session(session)?;
    let prepared = prepare_gliner(tokenizer, text, options, config)?;
    let length = prepared.input_ids.len();
    let count = prepared.label_positions.len();
    let positions = positions_name(session)?;
    let outputs = session.run(inputs![
        "input_ids" => Value::from_array(Array2::from_shape_vec((1, length), prepared.input_ids)?)?,
        "attention_mask" => Value::from_array(Array2::from_elem((1, length), 1i64))?,
        positions => Value::from_array(Array2::from_shape_vec((1, count), prepared.label_positions)?)?,
    ])?;
    let logits = outputs["logits"].try_extract_array::<f32>()?;
    if logits.shape() != [1, count] {
        return Err(anyhow!(
            "GLiNER returned unexpected logits shape {:?}",
            logits.shape()
        ));
    }
    decode_gliner(
        &logits.iter().copied().collect::<Vec<_>>(),
        length,
        options,
        config,
    )
}

pub fn infer_gliner_split(
    encoder: &mut Session,
    classifier: &mut Session,
    tokenizer: &Tokenizer,
    text: &str,
    options: &DecisionOptions,
    config: &GlinerConfig,
) -> Result<DecisionResult> {
    validate_split_sessions(encoder, classifier)?;
    let prepared = prepare_gliner(tokenizer, text, options, config)?;
    let length = prepared.input_ids.len();
    let count = prepared.label_positions.len();
    let outputs = encoder.run(inputs![
        "input_ids" => Value::from_array(Array2::from_shape_vec((1, length), prepared.input_ids)?)?,
        "attention_mask" => Value::from_array(Array2::from_elem((1, length), 1i64))?,
    ])?;
    let hidden = outputs["last_hidden_state"].try_extract_array::<f32>()?;
    if hidden.ndim() != 3
        || hidden.shape()[0] != 1
        || hidden.shape()[1] != length
        || hidden.shape()[2] == 0
    {
        return Err(anyhow!(
            "GLiNER encoder returned unexpected hidden-state shape {:?}",
            hidden.shape()
        ));
    }
    let width = hidden.shape()[2];
    let mut embeddings = Vec::with_capacity(count * width);
    for position in prepared.label_positions {
        for column in 0..width {
            embeddings.push(hidden[[0, position as usize, column]]);
        }
    }
    let outputs = classifier.run(inputs![
        "cls_embeds" => Value::from_array(Array2::from_shape_vec((count, width), embeddings)?)?,
    ])?;
    let logits = outputs["logits"].try_extract_array::<f32>()?;
    if logits.shape() != [count] {
        return Err(anyhow!(
            "GLiNER classifier returned unexpected logits shape {:?}",
            logits.shape()
        ));
    }
    decode_gliner(
        &logits.iter().copied().collect::<Vec<_>>(),
        length,
        options,
        config,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(kind: DecisionQuestionType, criteria: &[&str]) -> DecisionOptions {
        DecisionOptions {
            question_type: kind,
            instructions: "Which applies?".into(),
            criteria: criteria.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn tokenizer() -> Tokenizer {
        let vocabulary = [
            "<unk>",
            "(",
            ")",
            "[P]",
            "[L]",
            "[SEP_TEXT]",
            "[DESCRIPTION]",
            "Which",
            "applies?",
            "positive",
            "negative",
            "good",
            "bad",
            ".",
        ]
        .into_iter()
        .enumerate()
        .map(|(id, token)| (token.to_string(), id as u32))
        .collect();
        let model = tokenizers::models::wordlevel::WordLevel::builder()
            .vocab(vocabulary)
            .unk_token("<unk>".into())
            .build()
            .unwrap();
        let mut tokenizer = Tokenizer::new(model);
        tokenizer.with_pre_tokenizer(Some(
            tokenizers::pre_tokenizers::whitespace::WhitespaceSplit,
        ));
        tokenizer
    }

    #[test]
    fn schema_is_case_preserved_and_text_is_lowercased_word_by_word() {
        let input = prepare_gliner(
            &tokenizer(),
            "GOOD bad",
            &options(DecisionQuestionType::Choice, &["positive", "negative"]),
            &GlinerConfig::default(),
        )
        .unwrap();
        assert_eq!(
            input.input_ids,
            [1, 3, 7, 8, 1, 4, 9, 4, 10, 2, 2, 5, 11, 12, 13]
        );
        assert_eq!(input.label_positions, [5, 7]);
    }

    #[test]
    fn splitter_preserves_urls_emails_mentions_and_python_word_boundaries() {
        let words: Vec<_> = WORDS.find_iter("https://EXAMPLE.COM/A?a=1 User@Example.COM @Name one-two café Cafe\u{301} 中國 ²\u{001c}Next")
            .map(|word| word.as_str().to_lowercase()).collect();
        assert_eq!(
            words,
            [
                "https://example.com/a?a=1",
                "user@example.com",
                "@name",
                "one-two",
                "café",
                "cafe",
                "\u{301}",
                "中國",
                "²",
                "next"
            ]
        );
    }

    #[test]
    fn text_truncation_preserves_every_label_and_schema_overflow_fails() {
        let options = options(DecisionQuestionType::Choice, &["positive", "negative"]);
        let config = GlinerConfig {
            max_length: 13,
            ..Default::default()
        };
        let input = prepare_gliner(&tokenizer(), "good bad", &options, &config).unwrap();
        assert_eq!(input.input_ids.len(), 13);
        assert_eq!(input.label_positions, [5, 7]);
        assert_eq!(input.input_ids[12], 11);
        assert!(
            prepare_gliner(
                &tokenizer(),
                "good",
                &options,
                &GlinerConfig {
                    max_length: 12,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn empty_text_gets_a_terminal_period_and_extra_label_tokens_do_not_add_positions() {
        let input = prepare_gliner(
            &tokenizer(),
            "",
            &options(DecisionQuestionType::Choice, &["positive [L]", "negative"]),
            &GlinerConfig::default(),
        )
        .unwrap();
        assert_eq!(input.label_positions.len(), 2);
        assert_eq!(input.input_ids.last(), Some(&13));
    }

    #[test]
    fn score_and_noul_use_label_descriptions() {
        let score = question(&options(DecisionQuestionType::Score, &["bad", "good"])).unwrap();
        assert_eq!(score.labels, ["0", "1"]);
        assert_eq!(
            score.prompt,
            "Which applies? [DESCRIPTION] 0: bad [DESCRIPTION] 1: good"
        );
        let noul = question(&options(DecisionQuestionType::Noul, &["absent", "present"])).unwrap();
        assert_eq!(noul.labels, ["false", "true"]);
        assert_eq!(
            noul.prompt,
            "Which applies? [DESCRIPTION] false: absent [DESCRIPTION] true: present"
        );
    }

    #[test]
    fn decoding_preserves_typed_results() {
        let config = GlinerConfig::default();
        let choice = decode_gliner(
            &[1000.0, 1000.0],
            10,
            &options(DecisionQuestionType::Choice, &["first", "second"]),
            &config,
        )
        .unwrap();
        assert_eq!(choice.choice.as_deref(), Some("first"));
        assert_eq!(choice.confidence, 0.0);
        let score = decode_gliner(
            &[0.0; 3],
            10,
            &options(DecisionQuestionType::Score, &["low", "medium", "high"]),
            &config,
        )
        .unwrap();
        assert_eq!(score.score, Some(1.0));
        let noul = decode_gliner(
            &[0.0, 2.0],
            10,
            &options(DecisionQuestionType::Noul, &[]),
            &config,
        )
        .unwrap();
        assert_eq!(noul.noul, Some(0.8808));
        assert_eq!(noul.confidence, 0.8808);
    }

    #[test]
    fn invalid_question_configuration_and_outputs_fail() {
        assert!(question(&options(DecisionQuestionType::Choice, &["same", "same"])).is_err());
        assert!(question(&options(DecisionQuestionType::Choice, &[])).is_err());
        assert!(question(&options(DecisionQuestionType::Noul, &["yes"])).is_err());
        assert!(
            GlinerConfig {
                temperature: f64::NAN,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            decode_gliner(
                &[f32::INFINITY],
                1,
                &options(DecisionQuestionType::Choice, &["one"]),
                &GlinerConfig::default()
            )
            .is_err()
        );
        assert!(
            decode_gliner(
                &[0.0],
                1,
                &options(DecisionQuestionType::Choice, &["one", "two"]),
                &GlinerConfig::default()
            )
            .is_err()
        );
    }
}

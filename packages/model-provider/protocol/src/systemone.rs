//! Native typed-decision requests and responses shared by model clients and device hosts.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_QUESTIONS: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SystemOneRequest {
    pub state: Value,
    pub questions: BTreeMap<String, SystemOneQuestion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum SystemOneQuestion {
    Choice {
        instructions: Value,
        criteria: BTreeMap<String, Value>,
    },
    Score {
        instructions: Value,
        criteria: Vec<Value>,
    },
    Noul {
        instructions: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoulCriteria {
    #[serde(rename = "true", default, skip_serializing_if = "Option::is_none")]
    pub when_true: Option<Value>,
    #[serde(rename = "false", default, skip_serializing_if = "Option::is_none")]
    pub when_false: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SystemOneAnswer {
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Score {
        score: f64,
        legend: BTreeMap<String, Value>,
        probabilities: BTreeMap<String, f64>,
        confidence: f64,
    },
    Noul {
        noul: f64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SystemOneUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SystemOneResponse {
    pub model: String,
    pub answers: BTreeMap<String, SystemOneAnswer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<SystemOneUsage>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

fn structured(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Object(_) | Value::Array(_))
}

impl SystemOneRequest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            serde_json::to_vec(self)?.len() <= MAX_REQUEST_BYTES,
            "System One request exceeds 16 MiB"
        );
        ensure!(
            structured(&self.state),
            "System One state must be text, an object, or an array"
        );
        ensure!(
            !self.questions.is_empty() && self.questions.len() <= MAX_QUESTIONS,
            "System One requires between 1 and {MAX_QUESTIONS} questions"
        );
        for (id, question) in &self.questions {
            ensure!(!id.is_empty(), "System One question IDs must not be empty");
            let instructions = match question {
                SystemOneQuestion::Choice {
                    instructions,
                    criteria,
                } => {
                    ensure!(
                        !criteria.is_empty() && criteria.len() <= 255,
                        "Question {id}: choice requires 1 to 255 options"
                    );
                    ensure!(
                        criteria.values().all(|v| v.is_null() || structured(v)),
                        "Question {id}: invalid choice description"
                    );
                    instructions
                }
                SystemOneQuestion::Score {
                    instructions,
                    criteria,
                } => {
                    ensure!(
                        (2..=10).contains(&criteria.len()),
                        "Question {id}: score requires 2 to 10 ordered levels"
                    );
                    ensure!(
                        criteria.iter().all(structured),
                        "Question {id}: invalid score description"
                    );
                    instructions
                }
                SystemOneQuestion::Noul {
                    instructions,
                    criteria,
                } => {
                    if let Some(criteria) = criteria {
                        ensure!(
                            [criteria.when_true.as_ref(), criteria.when_false.as_ref()]
                                .into_iter()
                                .flatten()
                                .all(structured),
                            "Question {id}: invalid yes/no description"
                        );
                    }
                    instructions
                }
            };
            ensure!(
                structured(instructions),
                "Question {id}: instructions must be text, an object, or an array"
            );
        }
        ensure!(
            self.images.len() <= 8,
            "System One supports at most 8 images"
        );
        for image in &self.images {
            validate_image(image)?;
        }
        let embedded_images = validate_state_images(&self.state)?;
        ensure!(
            self.images.len() + embedded_images <= 8,
            "System One supports at most 8 images"
        );
        Ok(())
    }
}

fn validate_image(image: &str) -> Result<()> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let (header, data) = image
        .split_once(',')
        .context("System One images must be base64 data URLs")?;
    ensure!(
        header.starts_with("data:image/") && header.ends_with(";base64"),
        "System One images must be base64 image data URLs"
    );
    ensure!(!data.is_empty(), "System One image must not be empty");
    STANDARD
        .decode(data)
        .context("System One image has invalid base64")?;
    Ok(())
}

fn validate_state_images(state: &Value) -> Result<usize> {
    let messages = state.get("messages").unwrap_or(state);
    let Some(messages) = messages.as_array() else {
        return Ok(0);
    };
    let mut count = 0;
    for part in messages
        .iter()
        .filter_map(|m| m.get("content").and_then(Value::as_array))
        .flatten()
    {
        if part.get("type").and_then(Value::as_str) == Some("image_url") {
            let image = part.get("image_url").context("Missing image_url")?;
            let image = image
                .get("url")
                .unwrap_or(image)
                .as_str()
                .context("Invalid image_url")?;
            validate_image(image)?;
            count += 1;
        }
    }
    Ok(count)
}

impl SystemOneResponse {
    pub fn validate_for(&self, request: &SystemOneRequest) -> Result<()> {
        ensure!(
            !self.model.trim().is_empty(),
            "System One response model must not be empty"
        );
        ensure!(
            self.answers.keys().eq(request.questions.keys()),
            "System One response question IDs do not match the request"
        );
        for (id, question) in &request.questions {
            let answer = &self.answers[id];
            match (question, answer) {
                (SystemOneQuestion::Noul { .. }, SystemOneAnswer::Noul { noul }) => {
                    probability(*noul)?
                }
                (
                    SystemOneQuestion::Choice { criteria, .. },
                    SystemOneAnswer::Choice {
                        choice,
                        probabilities,
                        confidence,
                    },
                ) => {
                    ensure!(
                        criteria.contains_key(choice),
                        "System One returned an unknown choice for {id}"
                    );
                    ensure!(
                        criteria.keys().eq(probabilities.keys()),
                        "System One returned different choice options for {id}"
                    );
                    distribution(probabilities, *confidence)?;
                }
                (
                    SystemOneQuestion::Score { criteria, .. },
                    SystemOneAnswer::Score {
                        score,
                        legend,
                        probabilities,
                        confidence,
                    },
                ) => {
                    ensure!(
                        score.is_finite()
                            && *score >= 0.0
                            && *score <= criteria.len().saturating_sub(1) as f64,
                        "System One returned an invalid score for {id}"
                    );
                    ensure!(
                        probabilities.len() == criteria.len()
                            && legend.len() == criteria.len()
                            && (0..criteria.len())
                                .all(|i| probabilities.contains_key(&i.to_string())
                                    && legend.contains_key(&i.to_string())),
                        "System One returned different score levels for {id}"
                    );
                    distribution(probabilities, *confidence)?;
                }
                _ => bail!("System One answer type does not match question {id}"),
            }
        }
        Ok(())
    }
}

fn probability(value: f64) -> Result<()> {
    ensure!(
        value.is_finite() && (0.0..=1.0).contains(&value),
        "System One returned a probability outside 0 to 1"
    );
    Ok(())
}

fn distribution(values: &BTreeMap<String, f64>, confidence: f64) -> Result<()> {
    probability(confidence)?;
    for value in values.values() {
        probability(*value)?;
    }
    ensure!(
        (values.values().sum::<f64>() - 1.0).abs() <= 0.01,
        "System One probabilities do not sum to 1"
    );
    Ok(())
}

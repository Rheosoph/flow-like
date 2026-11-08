//! The two things a host plugs in: a model for summaries, the Now block and fact work, and an
//! embedder. Tests stub both (see [`crate::testing`]).

use async_trait::async_trait;
use flow_like_model_provider::systemone::{SystemOneRequest, SystemOneResponse};
use flow_like_types::{Result, anyhow};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a model call is for, so a host can route idle work to a larger model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmTask {
    SegmentSummary,
    MergeSummaries,
    NowBlock,
    FactExtraction,
    FactUpdate,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmRequest {
    pub task: LlmTask,
    pub system: String,
    pub prompt: String,
    /// The JSON schema the answer follows. Backends with constrained decoding should enforce it;
    /// the parsers also accept a JSON object wrapped in prose or a code fence.
    pub json_schema: Option<Value>,
    pub max_output_tokens: u32,
}

#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait marks the boxed future it already returns as must_use"
)]
pub trait MemoryLlm: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<String>;

    /// Optional bounded fact curation. Summaries and fact extraction still use `complete`.
    async fn classify_fact(
        &self,
        _request: &SystemOneRequest,
    ) -> Result<Option<SystemOneResponse>> {
        Ok(None)
    }
}

#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait marks the boxed future it already returns as must_use"
)]
pub trait Embedder: Send + Sync {
    /// Stored next to every vector, so a model change is detected and re-embedded.
    fn model_id(&self) -> &str;
    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    async fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
}

/// Document vectors for `texts`, checked for count and size.
pub(crate) async fn embed_checked(
    embedder: &dyn Embedder,
    texts: &[String],
) -> Result<Vec<Vec<f32>>> {
    let vectors = embedder.embed_documents(texts).await?;
    let size = vectors.first().map_or(0, Vec::len);
    if vectors.len() != texts.len()
        || vectors
            .iter()
            .any(|vector| vector.is_empty() || vector.len() != size)
    {
        return Err(anyhow!(
            "Embedder '{}' returned {} vectors of unusable or mixed size for {} texts",
            embedder.model_id(),
            vectors.len(),
            texts.len()
        ));
    }
    Ok(vectors)
}

fn fenced(answer: &str) -> Option<&str> {
    let start = answer.find("```")?;
    let body = &answer[start + 3..];
    let body = body.strip_prefix("json").unwrap_or(body);
    let end = body.find("```")?;
    Some(body[..end].trim())
}

fn outer_object(answer: &str) -> Option<&str> {
    let start = answer.find('{')?;
    let end = answer.rfind('}')?;
    (start < end).then(|| &answer[start..=end])
}

/// The JSON object in a model answer: bare, in a code fence or surrounded by prose.
pub fn parse_json_answer<T: DeserializeOwned>(answer: &str) -> Result<T> {
    let trimmed = answer.trim();
    let mut last_error = None;
    for candidate in [Some(trimmed), fenced(trimmed), outer_object(trimmed)]
        .into_iter()
        .flatten()
    {
        match serde_json::from_str::<T>(candidate) {
            Ok(value) => return Ok(value),
            Err(error) => last_error = Some(error),
        }
    }
    let preview: String = trimmed.chars().take(160).collect();
    Err(anyhow!(
        "The model answer is not the expected JSON ({}): {preview}",
        last_error.map_or_else(|| "no JSON object".to_string(), |error| error.to_string())
    ))
}

/// Cosine similarity; 0 for empty or mismatched vectors.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut norm_a, mut norm_b) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Answer {
        action: String,
    }

    #[test]
    fn json_answers_are_found_bare_fenced_or_in_prose() {
        let expected = Answer {
            action: "add".into(),
        };
        for answer in [
            r#"{"action":"add"}"#,
            "```json\n{\"action\": \"add\"}\n```",
            "Sure! Here it is: {\"action\": \"add\"} Hope that helps.",
        ] {
            assert_eq!(
                parse_json_answer::<Answer>(answer).unwrap(),
                expected,
                "{answer}"
            );
        }
        let error = parse_json_answer::<Answer>("no json here").unwrap_err();
        assert!(error.to_string().contains("no json here"));
    }

    #[test]
    fn cosine_handles_degenerate_vectors() {
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine(&[], &[]), 0.0);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 2.0]), 0.0);
    }
}

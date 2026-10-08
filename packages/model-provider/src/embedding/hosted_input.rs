//! Wire inputs accepted by the hosted embedding API and its Internal gateway.

use crate::provider::{EmbeddingModelProvider, RemoteEmbeddingProvider, is_hosted_provider_name};
use serde::{Deserialize, Serialize};

/// The Internal gateway exposes multimodal inputs only for this deployed model.
pub fn supports_internal_multimodal(provider: &EmbeddingModelProvider) -> bool {
    let remote = provider.remote.as_ref();
    let implementation = remote
        .and_then(|remote| remote.implementation)
        .unwrap_or_default();
    let model = remote
        .and_then(|remote| remote.model_id.as_deref())
        .filter(|model| !model.trim().is_empty())
        .or(provider.provider.model_id.as_deref());
    (remote.is_some() || is_hosted_provider_name(&provider.provider.provider_name))
        && implementation == RemoteEmbeddingProvider::Internal
        && model.is_some_and(|model| model.trim() == "embeddinggemma-2")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HostedEmbeddingInput {
    Text(String),
    Structured(HostedEmbeddingStructuredInput),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostedEmbeddingStructuredInput {
    Text {
        text: String,
    },
    Image {
        image: String,
    },
    Audio {
        audio: String,
    },
    Video {
        video: String,
    },
    Multimodal {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        audio: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        video: Option<String>,
    },
}

impl From<String> for HostedEmbeddingInput {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for HostedEmbeddingInput {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_legacy_text_and_signed_media_urls() {
        let text = HostedEmbeddingInput::from("query");
        assert_eq!(
            serde_json::to_value(text).unwrap(),
            serde_json::json!("query")
        );
        let wire = serde_json::json!({
            "type": "multimodal",
            "text": "Compare <|image|> and <|video|>",
            "image": "https://media.example/image?token=a%2Fb&expires=123",
            "video": "data:video/mp4;base64,AAAA"
        });
        let parsed: HostedEmbeddingInput = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), wire);
        assert!(
            serde_json::from_value::<HostedEmbeddingInput>(serde_json::json!({
                "type": "image", "image": "AAAA", "audio": "BBBB"
            }))
            .is_err()
        );
    }
}

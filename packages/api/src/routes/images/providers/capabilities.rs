use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use flow_like::flow_like_model_provider::image_generation::openrouter::{
    ImageCapabilities, capabilities_url,
};
use flow_like_types::reqwest;

use crate::error::ApiError;

const TTL: Duration = Duration::from_secs(300);
const MAX_MODELS: usize = 256;
const MAX_BYTES: usize = 512 * 1024;
type Cache = HashMap<String, (Instant, ImageCapabilities)>;
static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Only a 404 means the model's endpoints are gone; 400 and 422 also answer a rejected prompt.
pub(super) fn invalidate_for_status(model: &str, status: u16) {
    if status == 404 {
        invalidate(model);
    }
}

fn invalidate(model: &str) {
    CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(model);
}

pub(super) fn unavailable() -> ApiError {
    ApiError::coded(
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "IMAGE_CAPABILITIES_UNAVAILABLE",
        "Image model capabilities could not be checked. No image generation request was sent.",
    )
}

pub(super) async fn get(model: &str) -> Result<ImageCapabilities, ApiError> {
    {
        let mut cache = CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.retain(|_, (loaded, _)| loaded.elapsed() < TTL);
        if let Some((_, capabilities)) = cache.get(model) {
            return Ok(capabilities.clone());
        }
    }
    let url = capabilities_url("https://openrouter.ai/api/v1", model)
        .map_err(|_| ApiError::bad_request("Invalid image model ID"))?;
    // Discovery is public and never carries generation credentials or prompts.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| unavailable())?;
    let response = client.get(url).send().await.map_err(|_| unavailable())?;
    let capabilities = read_response(response, model).await?;
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.retain(|_, (loaded, _)| loaded.elapsed() < TTL);
    if cache.len() >= MAX_MODELS {
        if let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, (loaded, _))| *loaded)
            .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest);
        }
    }
    cache.insert(model.to_owned(), (Instant::now(), capabilities.clone()));
    Ok(capabilities)
}

async fn read_response(
    mut response: reqwest::Response,
    model: &str,
) -> Result<ImageCapabilities, ApiError> {
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(ApiError::coded(
            axum::http::StatusCode::BAD_REQUEST,
            "IMAGE_MODEL_UNAVAILABLE",
            "The image provider no longer offers this model. Choose another image model.",
        ));
    }
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_BYTES as u64)
    {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
        if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
            return Err(unavailable());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    if value.get("id").and_then(serde_json::Value::as_str) != Some(model) {
        return Err(unavailable());
    }
    ImageCapabilities::from_endpoints(&value).map_err(|_| unavailable())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn discovery_rejects_wrong_models_errors_and_oversized_metadata() {
        for (status, body, expected_code) in [
            (
                200,
                json!({"id":"other/model", "endpoints":[]}).to_string(),
                "IMAGE_CAPABILITIES_UNAVAILABLE",
            ),
            (
                200,
                "x".repeat(MAX_BYTES + 1),
                "IMAGE_CAPABILITIES_UNAVAILABLE",
            ),
            (
                502,
                "private-provider-error".into(),
                "IMAGE_CAPABILITIES_UNAVAILABLE",
            ),
            (
                404,
                "private-provider-error".into(),
                "IMAGE_MODEL_UNAVAILABLE",
            ),
        ] {
            let response = reqwest::Response::from(
                axum::http::Response::builder()
                    .status(status)
                    .body(body)
                    .unwrap(),
            );
            let error = read_response(response, "qwen/qwen-image-3")
                .await
                .unwrap_err();
            assert_eq!(error.public_code(), expected_code);
            assert!(
                !error
                    .public_message()
                    .unwrap()
                    .contains("private-provider-error")
            );
        }
    }

    #[test]
    fn only_a_404_evicts_the_cached_capabilities() {
        let model = "evict/test-model";
        let capabilities = ImageCapabilities::from_endpoints(&json!({"id": model, "endpoints": [{
            "provider_tag": "test", "supported_parameters": {
                "aspect_ratio": {"type":"enum", "values":["1:1"]},
                "resolution": {"type":"enum", "values":["1K"]}
            }
        }]}))
        .unwrap();
        let cached = || {
            CACHE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains_key(model)
        };
        CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(model.to_owned(), (Instant::now(), capabilities));
        for status in [400, 422, 429, 500] {
            invalidate_for_status(model, status);
            assert!(cached(), "{status} must keep the entry");
        }
        invalidate_for_status(model, 404);
        assert!(!cached());
    }

    #[tokio::test]
    async fn discovery_accepts_current_endpoint_capabilities() {
        let body = json!({"id":"qwen/qwen-image-3", "endpoints":[{
            "provider_tag":"alibaba", "supported_parameters":{
                "resolution":{"type":"enum","values":["1K","2K"]},
                "aspect_ratio":{"type":"enum","values":["1:1","3:2"]}
            }
        }]})
        .to_string();
        let response = reqwest::Response::from(axum::http::Response::new(body));
        assert!(read_response(response, "qwen/qwen-image-3").await.is_ok());
    }
}

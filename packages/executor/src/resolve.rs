//! Resolves [`DispatchPayloadRef`] into a concrete [`DispatchPayload`].
//!
//! Queue-based runtimes (SQS → Lambda, EventBridge → ECS) may receive
//! either an inline payload or a presigned URL pointing to the full payload
//! in object storage. This module transparently handles both cases.

use flow_like_types::dispatch::{DispatchPayload, DispatchPayloadRef};
use std::sync::OnceLock;
use std::time::Duration;

const FETCH_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
/// The HTTP sink accepts bodies up to 10 MiB, and JSON string escaping can
/// inflate the staged form up to ~6x, so the cap must leave that headroom
/// while still preventing unbounded buffering.
pub const MAX_REMOTE_PAYLOAD_BYTES: u64 = 64 * 1024 * 1024;

/// Payload cap, overridable via `EXECUTOR_MAX_REMOTE_PAYLOAD_BYTES` (min 1 MiB).
pub fn max_remote_payload_bytes() -> u64 {
    static CAP: OnceLock<u64> = OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("EXECUTOR_MAX_REMOTE_PAYLOAD_BYTES")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .filter(|v| *v >= 1024 * 1024)
            .unwrap_or(MAX_REMOTE_PAYLOAD_BYTES)
    })
}

/// Resolve a [`DispatchPayloadRef`] into a [`DispatchPayload`].
///
/// - `Inline` → returned as-is.
/// - `Remote` → fetches the JSON from the presigned URL and deserialises it.
pub async fn resolve_payload(
    payload_ref: DispatchPayloadRef,
) -> Result<DispatchPayload, ResolveError> {
    match payload_ref {
        DispatchPayloadRef::Inline(payload) => Ok(*payload),
        DispatchPayloadRef::Remote { remote_url } => {
            // The presigned URL carries its bearer-equivalent signature in the
            // query string, so it must never appear in logs.
            tracing::info!("Fetching remote dispatch payload");

            let body = fetch_bounded(&remote_url, max_remote_payload_bytes()).await?;
            serde_json::from_slice(&body).map_err(|e| ResolveError::Deserialize(e.to_string()))
        }
    }
}

/// GET a presigned URL with connect/total timeouts, redirects refused, and the
/// response body capped at `max_bytes`.
pub async fn fetch_bounded(url: &str, max_bytes: u64) -> Result<Vec<u8>, ResolveError> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(FETCH_CONNECT_TIMEOUT)
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|e| ResolveError::Fetch(e.to_string()))?;

    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| ResolveError::Fetch(e.to_string()))?;

    if !response.status().is_success() {
        return Err(ResolveError::Fetch(format!(
            "HTTP {} from payload URL",
            response.status()
        )));
    }

    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(ResolveError::Fetch(format!(
            "remote payload exceeds the {max_bytes} byte limit"
        )));
    }

    let mut body =
        Vec::with_capacity(response.content_length().unwrap_or_default().min(max_bytes) as usize);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ResolveError::Fetch(e.to_string()))?
    {
        if (body.len() as u64).saturating_add(chunk.len() as u64) > max_bytes {
            return Err(ResolveError::Fetch(format!(
                "remote payload exceeds the {max_bytes} byte limit"
            )));
        }
        body.extend_from_slice(&chunk);
    }

    Ok(body)
}

/// Convenience: parse a JSON string as [`DispatchPayloadRef`] and resolve it.
pub async fn resolve_payload_from_str(json: &str) -> Result<DispatchPayload, ResolveError> {
    let payload_ref: DispatchPayloadRef =
        serde_json::from_str(json).map_err(|e| ResolveError::Deserialize(e.to_string()))?;
    resolve_payload(payload_ref).await
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("Failed to fetch remote payload: {0}")]
    Fetch(String),
    #[error("Failed to deserialize payload: {0}")]
    Deserialize(String),
}

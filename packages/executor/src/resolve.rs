//! Resolves [`DispatchPayloadRef`] into a concrete [`DispatchPayload`].
//!
//! Queue-based runtimes (SQS → Lambda, EventBridge → ECS) may receive
//! either an inline payload or a presigned URL pointing to the full payload
//! in object storage. This module transparently handles both cases.

use flow_like_types::dispatch::{DispatchPayload, DispatchPayloadRef};

/// Resolve a [`DispatchPayloadRef`] into a [`DispatchPayload`].
///
/// - `Inline` → returned as-is.
/// - `Remote` → fetches the JSON from the presigned URL and deserialises it.
pub async fn resolve_payload(
    payload_ref: DispatchPayloadRef,
) -> Result<DispatchPayload, ResolveError> {
    match payload_ref {
        DispatchPayloadRef::Inline(payload) => Ok(payload),
        DispatchPayloadRef::Remote { remote_url } => {
            tracing::info!(url = %remote_url, "Fetching remote dispatch payload");

            let response = reqwest::get(&remote_url)
                .await
                .map_err(|e| ResolveError::Fetch(e.to_string()))?;

            if !response.status().is_success() {
                return Err(ResolveError::Fetch(format!(
                    "HTTP {} from payload URL",
                    response.status()
                )));
            }

            let payload: DispatchPayload = response
                .json()
                .await
                .map_err(|e| ResolveError::Deserialize(e.to_string()))?;

            Ok(payload)
        }
    }
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

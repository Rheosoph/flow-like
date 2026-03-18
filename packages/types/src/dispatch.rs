//! Canonical wire-format payloads for execution and compilation dispatch.
//!
//! Defined once in `flow-like-types` so that both the API (producer) and every
//! executor/compiler runtime (consumer) share the same schema at compile time.

use crate::OAuthTokenInput;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Reference to a WASM package pre-resolved by the API.
/// Contains presigned download URLs for the pre-compiled `.cwasm` artifact
/// and its blake3 checksum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmPackageRef {
    pub version: String,
    pub wasm_hash: String,
    pub cwasm_url: String,
    pub cwasm_checksum_url: String,
}

// ============================================================================
// Compilation Dispatch
// ============================================================================

/// Job dispatched from the API to a compilation worker.
///
/// The worker downloads the raw `.wasm` via a presigned GET URL, compiles it
/// to `.cwasm` for each target platform, uploads artifacts via presigned PUT
/// URLs, and reports back via JWT-signed callback. Workers never receive raw
/// bucket credentials.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompilationJob {
    pub job_id: String,
    pub package_id: String,
    pub version: String,
    /// Presigned GET URL for the raw `.wasm` file.
    pub wasm_download_url: String,
    /// blake3 hash of the raw `.wasm` file for integrity verification.
    pub wasm_hash: String,
    /// Targets to compile for, each with presigned upload URLs.
    pub targets: Vec<CompilationTarget>,
    /// JWT signed by the API. Audience: `flow-like-compiler`.
    pub compiler_jwt: String,
}

/// A single compilation target with presigned upload URLs for the output
/// artifacts. The API generates per-file PUT URLs so the worker never needs
/// bucket credentials.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompilationTarget {
    pub platform_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross_triple: Option<String>,
    /// Presigned PUT URL for the compiled `.cwasm` artifact.
    pub cwasm_upload_url: String,
    /// Presigned PUT URL for the blake3 checksum file.
    pub checksum_upload_url: String,
}

/// Reference to a compilation job that may be either embedded inline or
/// stored remotely behind a (presigned) URL.
///
/// When the serialised `CompilationJob` exceeds ECS container-override env
/// var size limits (~8 KB) the API stages the full payload to object storage
/// and sends only the URL through SQS.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompilationJobRef {
    /// Full job embedded in the message body.
    Inline(CompilationJob),
    /// Job stored at a remote URL (presigned S3 GET URL).
    Remote { remote_url: String },
}

/// Result reported back to the API from a compilation worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompilationResult {
    pub job_id: String,
    pub package_id: String,
    pub version: String,
    pub status: CompilationStatus,
    #[serde(default)]
    pub compiled_platforms: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nodes: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompilationStatus {
    Compiled,
    Failed,
}

/// Payload produced by the API dispatcher and consumed by every executor runtime
/// (HTTP, Lambda, SQS, Redis, Kafka, Kubernetes).
///
/// Fields like `credentials`, `runtime_variables`, and `user_context` are kept
/// as [`serde_json::Value`] because their concrete types live in the heavier
/// `flow-like` core crate.  Typed conversion happens in the executor via
/// `TryFrom<DispatchPayload> for ExecutionRequest`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DispatchPayload {
    pub job_id: String,
    pub run_id: String,
    pub app_id: String,
    pub board_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_version: Option<(u32, u32, u32)>,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    pub user_id: String,
    pub credentials: serde_json::Value,
    pub executor_jwt: String,
    pub callback_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_tokens: Option<HashMap<String, OAuthTokenInput>>,
    #[serde(default)]
    pub stream_state: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_variables: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_context: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm_packages: Option<HashMap<String, WasmPackageRef>>,
}

/// Reference to a dispatch payload that may be either embedded inline or
/// stored remotely behind a (presigned) URL.
///
/// This is the wire format consumed by queue-based executor runtimes (SQS →
/// Lambda, EventBridge → ECS, etc.). When the payload exceeds queue size
/// limits (~256 KB for SQS, ~8 KB for ECS container overrides) the API
/// stages the full payload to object storage and sends only the URL.
///
/// Deserialisation is untagged so that a plain `DispatchPayload` JSON object
/// is accepted as `Inline` while `{ "remote_url": "https://..." }` is parsed
/// as `Remote`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DispatchPayloadRef {
    /// Full payload embedded in the message body.
    Inline(DispatchPayload),
    /// Payload stored at a remote URL (presigned S3/GCS/Azure GET URL).
    Remote {
        /// Presigned GET URL from which the full `DispatchPayload` JSON can
        /// be downloaded. The executor fetches this URL, deserialises the
        /// body, and then proceeds as if it received an inline payload.
        remote_url: String,
    },
}

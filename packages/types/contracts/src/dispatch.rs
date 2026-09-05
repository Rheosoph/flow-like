//! Canonical wire-format payloads for execution and compilation dispatch.
//!
//! Defined once in `flow-like-types` so that both the API (producer) and every
//! executor/compiler runtime (consumer) share the same schema at compile time.

use crate::OAuthTokenInput;
use serde::{Deserialize, Serialize};

/// Wire-only version selector for an ETag-bound Latest run. Executors that
/// predate ETag selection interpret this as an immutable version that cannot
/// exist and fail closed during rolling deployments. Current executors decode
/// it back to `None` only when a non-empty `board_etag` accompanies it.
pub const ETAG_BOUND_LATEST_VERSION_SENTINEL: (u32, u32, u32) = (u32::MAX, u32::MAX, u32::MAX);

/// Synthetic API Gateway identity used only by direct asynchronous Lambda
/// dispatch. The executor uses it to turn route failures into invocation
/// errors without changing Function URL or API Gateway HTTP behavior.
pub const DIRECT_LAMBDA_INVOKE_API_ID: &str = "lambda-invoke";
use std::collections::HashMap;

/// Store reference used for files uploaded through HTTP event sink requests.
///
/// The API/desktop sink maps multipart file fields to FlowPath JSON objects
/// with this store ref. The backing object bytes live in the configured
/// temporary store, so execution dispatch never carries raw file bytes.
pub const REQUEST_FILES_STORE_REF: &str = "__flow_like_http_request_files";

/// Reference to a WASM package pre-resolved by the API.
/// Contains presigned download URLs for the pre-compiled `.cwasm` artifact
/// and its blake3 checksum.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmPackageRef {
    pub version: String,
    pub wasm_hash: String,
    pub wasm_url: String,
    pub cwasm_url: String,
    pub cwasm_checksum: String,
}

/// Deterministic authority revision for the executable WASM package set.
/// Presigned URLs are transport details and do not participate.
pub fn wasm_package_set_revision(packages: Option<&HashMap<String, WasmPackageRef>>) -> String {
    let mut entries = packages
        .into_iter()
        .flat_map(HashMap::iter)
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.0.cmp(b.0));

    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like/wasm-package-set/v1");
    for (package_id, package) in entries {
        for value in [
            package_id.as_str(),
            package.version.as_str(),
            package.wasm_hash.as_str(),
            package.cwasm_checksum.as_str(),
        ] {
            hasher.update(&(value.len() as u64).to_le_bytes());
            hasher.update(value.as_bytes());
        }
    }
    hasher.finalize().to_hex().to_string()
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
    /// Storage implementation that authenticated the download URL.
    ///
    /// This is part of the signed job envelope. Consumers use it to select
    /// provider-specific request requirements and must never infer upload
    /// headers from an untrusted URL alone.
    pub wasm_download_provider: CompilationStorageProvider,
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
    /// Storage implementation that authenticated both PUT URLs.
    pub upload_provider: CompilationStorageProvider,
}

/// Cloud object-store protocol used by a compilation job.
///
/// External compilation intentionally supports only the three cloud stores
/// whose signed URL formats can be validated. Local, memory, and opaque object
/// stores must use in-process compilation instead of handing an arbitrary URL
/// to a worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompilationStorageProvider {
    AwsS3,
    AzureBlob,
    GoogleCloudStorage,
}

/// Return a deterministic blake3 binding for every executable field in a job.
///
/// The JWT itself is deliberately excluded because it carries this hash. The
/// concrete helper lives with the wire type so producers and consumers cannot
/// drift onto different canonicalization rules.
pub fn compilation_job_payload_hash(job: &CompilationJob) -> Result<String, serde_json::Error> {
    #[derive(Serialize)]
    struct SignedCompilationEnvelope<'a> {
        schema: &'static str,
        job_id: &'a str,
        package_id: &'a str,
        version: &'a str,
        wasm_download_url: &'a str,
        wasm_download_provider: CompilationStorageProvider,
        wasm_hash: &'a str,
        targets: &'a [CompilationTarget],
    }

    let canonical = serde_json::to_vec(&SignedCompilationEnvelope {
        schema: "flow-like-compilation-job/v1",
        job_id: &job.job_id,
        package_id: &job.package_id,
        version: &job.version,
        wasm_download_url: &job.wasm_download_url,
        wasm_download_provider: job.wasm_download_provider,
        wasm_hash: &job.wasm_hash,
        targets: &job.targets,
    })?;
    Ok(blake3::hash(&canonical).to_hex().to_string())
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

/// The compiled board an executor runs, delivered as a presigned GET so the
/// executor needs no storage credential to obtain it. Produced by the API's
/// pre-dispatch artifact assurance; consumed verbatim by the executor.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompiledArtifactRef {
    /// Presigned GET for the `.flcb` object.
    pub url: String,
    /// Object key on the API's meta store. Diagnostics only — the executor never addresses storage.
    pub path: String,
    /// ETag of the source `.board` the artifact was compiled from, for floating Latest runs
    /// (`None` for versioned runs, whose identity is the version). The executor keys its
    /// template cache on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_etag: Option<String>,
    /// Hex blake3 fingerprint of the registry the API compiled against. The executor validates the
    /// artifact header against its own registry; this value only makes a mismatch diagnosable.
    pub registry_fingerprint: String,
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
    /// Exact source object ETag for a floating Latest board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_etag: Option<String>,
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
    pub execution_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_variables: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_context: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm_packages: Option<HashMap<String, WasmPackageRef>>,
    /// Channel credentials for this run: how the executor waits for client replies and the
    /// client handle it forwards inside every request. Absent for runs with no attached client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<crate::channel::ChannelGrant>,
    /// Shadow/replay isolation: the run must not write app storage. This byte
    /// only mirrors the signed executor JWT claim — the executor rejects the
    /// request when the two disagree.
    #[serde(default)]
    pub shadow: bool,
    /// Compiled `.flcb` the executor runs, as a presigned GET. Optional on the
    /// wire so an executor that predates it ignores the field; current executors
    /// require it and fail closed when it is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<CompiledArtifactRef>,
}

/// Bind all dispatch inputs to the executor JWT. Object keys are sorted at
/// every depth so HashMap iteration order cannot change the signature.
pub fn dispatch_payload_hash(payload: &DispatchPayload) -> Result<String, serde_json::Error> {
    fn canonical(value: &serde_json::Value, out: &mut Vec<u8>) -> Result<(), serde_json::Error> {
        match value {
            serde_json::Value::Object(object) => {
                out.push(b'{');
                let sorted: std::collections::BTreeMap<_, _> = object.iter().collect();
                for (index, (key, value)) in sorted.into_iter().enumerate() {
                    if index > 0 {
                        out.push(b',');
                    }
                    serde_json::to_writer(&mut *out, key)?;
                    out.push(b':');
                    canonical(value, out)?;
                }
                out.push(b'}');
            }
            serde_json::Value::Array(values) => {
                out.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(b',');
                    }
                    canonical(value, out)?;
                }
                out.push(b']');
            }
            value => serde_json::to_writer(out, value)?,
        }
        Ok(())
    }
    let mut value = serde_json::to_value(payload)?;
    value
        .as_object_mut()
        .expect("dispatch is an object")
        .remove("executor_jwt");
    let mut bytes = b"flow-like-execution-dispatch/v1\0".to_vec();
    canonical(&value, &mut bytes)?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

#[cfg(test)]
mod execution_binding_tests {
    use super::*;
    use serde_json::json;

    fn payload() -> DispatchPayload {
        serde_json::from_value(json!({
            "job_id": "job", "run_id": "run", "app_id": "app",
            "board_id": "board", "node_id": "node", "user_id": "user",
            "credentials": {"secret": "scoped-key"}, "executor_jwt": "before-signing",
            "callback_url": "https://callback.example", "payload": {"z": 1, "a": 2},
            "artifact": {"url": "https://store.example/artifact", "path": "artifact", "registry_fingerprint": "hash"}
        })).unwrap()
    }

    #[test]
    fn signatures_cover_authority_credentials_artifacts_and_execution_inputs() {
        let original = payload();
        let digest = dispatch_payload_hash(&original).unwrap();
        for (field, replacement) in [
            ("job_id", json!("other")),
            ("run_id", json!("other")),
            ("app_id", json!("other")),
            ("board_id", json!("other")),
            ("board_version", json!([1, 2, 3])),
            ("node_id", json!("other")),
            ("user_id", json!("other")),
            ("credentials", json!({"secret": "substituted"})),
            ("callback_url", json!("https://attacker.example")),
            ("shadow", json!(true)),
            ("token", json!("new-authority")),
            ("runtime_variables", json!({"input": "changed"})),
            ("payload", json!({"a": 3})),
            ("profile", json!({"hubs": ["new-hub"]})),
            (
                "wasm_packages",
                json!({"package": {"version": "1", "wasm_hash": "h", "wasm_url": "u", "cwasm_url": "u", "cwasm_checksum": "attacker"}}),
            ),
            (
                "artifact",
                json!({"url": "https://attacker.example/native", "path": "artifact", "registry_fingerprint": "hash"}),
            ),
        ] {
            let mut changed = serde_json::to_value(&original).unwrap();
            changed[field] = replacement;
            let changed: DispatchPayload = serde_json::from_value(changed).unwrap();
            assert_ne!(
                digest,
                dispatch_payload_hash(&changed).unwrap(),
                "unsigned field: {field}"
            );
        }
    }

    #[test]
    fn resigning_and_object_key_order_do_not_change_the_digest() {
        let original = payload();
        let mut changed = original.clone();
        changed.executor_jwt = "after-signing".into();
        changed.payload = Some(serde_json::from_str(r#"{"a":2,"z":1}"#).unwrap());
        assert_eq!(
            dispatch_payload_hash(&original).unwrap(),
            dispatch_payload_hash(&changed).unwrap()
        );
    }
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
    Inline(Box<DispatchPayload>),
    /// Payload stored at a remote URL (presigned S3/GCS/Azure GET URL).
    Remote {
        /// Presigned GET URL from which the full `DispatchPayload` JSON can
        /// be downloaded. The executor fetches this URL, deserialises the
        /// body, and then proceeds as if it received an inline payload.
        remote_url: String,
    },
}

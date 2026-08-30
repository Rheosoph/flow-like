//! Passwordless Azure Queue Storage publishing for queue-backed workloads.
//!
//! This module intentionally supports only a user-assigned managed identity.
//! Connection strings, shared access keys, SAS tokens, custom endpoints, and
//! implicit local developer credentials are not accepted by the production API
//! path. The account is reached over its private `queue` endpoint, so the only
//! authentication the service will accept is a Microsoft Entra bearer token.
//!
//! Queue Storage has no Rust data-plane client on the `azure_core` line this
//! crate already links, so the single `Put Message` call is issued directly
//! against the documented REST contract, in the same style as the user
//! delegation key request in `credentials/azure_credentials.rs`.

use azure_core::credentials::TokenCredential;
use azure_identity::{ManagedIdentityCredential, ManagedIdentityCredentialOptions, UserAssignedId};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde::Serialize;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Hard ceiling on the pre-base64 message body. Queue Storage caps the
/// *encoded* message at 64 KiB and every message this module sends is
/// base64-encoded, which expands 3:4, so 49_152 bytes is the exact largest
/// body that can still be put on the queue. Exceeding it is a caller bug:
/// anything at or above [`CLAIM_CHECK_THRESHOLD_BYTES`] must already have been
/// staged to Blob Storage and replaced by a claim-check reference.
const MAX_MESSAGE_BODY_BYTES: usize = 49_152;
/// Envelopes at or above this size are written to private Blob Storage and
/// replaced by a claim-check reference instead of being inlined. 32 KiB is
/// deliberately two thirds of the 49_152-byte hard ceiling: measured execution
/// and compilation payloads run 8-40 KiB, so the upper half of that range takes
/// the already-proven claim-check path instead of sitting within 20% of a
/// service limit that rejects the send outright.
pub(crate) const CLAIM_CHECK_THRESHOLD_BYTES: usize = 32 * 1024;
/// Lifetime of the signed URL a claim-check message points at. The message TTL
/// set on every send is `MESSAGE_TTL_SECS`, so a signature of exactly that
/// length would expire at the same instant the message it belongs to does. The
/// margin covers the delivery delay of a message that sits until the end of its
/// TTL, redelivery after a visibility timeout, the HTTP fetch of the staged
/// body, and clock skew between the signing service and Blob Storage.
pub(crate) const CLAIM_CHECK_URL_TTL: Duration = Duration::from_secs(86_400 + 6 * 3_600);
/// Queue Storage has no broker-side default TTL, so it is set explicitly on
/// every send to reproduce the `default_message_ttl = P1D` of the queue this
/// path replaces.
const MESSAGE_TTL_SECS: i64 = 86_400;

/// Envelope version understood by the queue worker. Queue Storage mints its own
/// `MessageId` and carries no user properties, so the job identity the worker
/// checks the resolved payload against has to travel inside the body.
const ENVELOPE_VERSION: u8 = 1;

pub(crate) const WORKLOAD_EXECUTION: &str = "execution";
pub(crate) const WORKLOAD_COMPILATION: &str = "compilation";
/// Fed by Event Grid blob subscriptions in the deployment; the API never
/// dispatches into these queues today, but the worker accepts the names and a
/// future re-enqueue path must not be rejected here.
pub(crate) const WORKLOAD_FILE_TRACKING: &str = "file-tracking";
pub(crate) const WORKLOAD_MEDIA_TRANSFORMATION: &str = "media-transformation";
const SUPPORTED_WORKLOADS: [&str; 4] = [
    WORKLOAD_EXECUTION,
    WORKLOAD_COMPILATION,
    WORKLOAD_FILE_TRACKING,
    WORKLOAD_MEDIA_TRANSFORMATION,
];

const STORAGE_SCOPE: &str = "https://storage.azure.com/.default";
const QUEUE_API_VERSION: &str = "2023-11-03";
const QUEUE_ENDPOINT_SUFFIX: &str = ".queue.core.windows.net";
const MAX_ERROR_BODY_BYTES: usize = 2_048;
const FORBIDDEN_CREDENTIAL_SETTINGS: &[&str] = &[
    "AZURE_STORAGE_CONNECTION_STRING",
    "AZURE_STORAGE_ACCOUNT_KEY",
    "AZURE_STORAGE_KEY",
    "AZURE_STORAGE_SAS_TOKEN",
    "AZURE_STORAGE_ACCESS_KEY",
    "AZURE_CLIENT_SECRET",
];

static QUEUE_HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static QUEUE_CREDENTIAL: OnceLock<Arc<dyn TokenCredential>> = OnceLock::new();

/// Body of every message this API puts on the execution or compilation queue.
///
/// `payload` is byte-for-byte what the broker body used to be: the untagged
/// inline job object or the `{"remote_url": …}` claim-check reference.
#[derive(Serialize)]
struct MessageEnvelope<'a, T: Serialize> {
    v: u8,
    job_id: &'a str,
    correlation_id: &'a str,
    workload: &'a str,
    payload: &'a T,
}

/// Byte cost of wrapping a payload in [`MessageEnvelope`], excluding the
/// payload itself. `serde_json` emits the payload verbatim inside the envelope,
/// so adding this to the serialized payload length is exact.
fn envelope_overhead_bytes(job_id: &str, correlation_id: &str, workload: &str) -> usize {
    serde_json::to_vec(&MessageEnvelope {
        v: ENVELOPE_VERSION,
        job_id,
        correlation_id,
        workload,
        payload: &(),
    })
    .map(|bytes| bytes.len().saturating_sub("null".len()))
    .unwrap_or(0)
}

/// Whether a payload of `payload_bytes` must be staged to Blob Storage instead
/// of being inlined. The threshold applies to the enveloped message, not to the
/// bare payload, because the envelope is what has to fit on the queue.
pub(crate) fn requires_claim_check(
    payload_bytes: usize,
    job_id: &str,
    correlation_id: &str,
    workload: &str,
) -> bool {
    payload_bytes.saturating_add(envelope_overhead_bytes(job_id, correlation_id, workload))
        >= CLAIM_CHECK_THRESHOLD_BYTES
}

pub(crate) async fn send_json<T: Serialize>(
    account_name: &str,
    queue_name: &str,
    job_id: &str,
    correlation_id: &str,
    workload: &str,
    payload: &T,
) -> Result<(), String> {
    reject_secret_credentials()?;
    validate_account_name(account_name)?;
    validate_queue_name(queue_name)?;
    validate_workload(workload)?;
    validate_identifier("job ID", job_id)?;
    validate_identifier("correlation ID", correlation_id)?;

    let body = serde_json::to_vec(&MessageEnvelope {
        v: ENVELOPE_VERSION,
        job_id,
        correlation_id,
        workload,
        payload,
    })
    .map_err(|error| format!("failed to serialize queue message: {error}"))?;
    if body.len() > MAX_MESSAGE_BODY_BYTES {
        return Err(format!(
            "queue message is {} bytes, above the {MAX_MESSAGE_BODY_BYTES} byte ceiling; bodies of {CLAIM_CHECK_THRESHOLD_BYTES} bytes or more should have been staged to Blob Storage and sent as a claim-check reference",
            body.len()
        ));
    }

    // Queue Storage stores the message inside an XML document, so any body byte
    // that is illegal in XML makes the message permanently unreadable — it could
    // not even be read in order to be moved to the poison queue. Base64 removes
    // that failure mode and makes the size budget exactly computable. The
    // encoded text uses only `A-Za-z0-9+/=`, none of which needs XML escaping.
    let message_text = BASE64_STANDARD.encode(&body);
    let request_body =
        format!("<QueueMessage><MessageText>{message_text}</MessageText></QueueMessage>");

    let token = queue_credential()?
        .get_token(&[STORAGE_SCOPE], None)
        .await
        .map_err(|error| format!("failed to acquire Azure Storage identity token: {error}"))?;

    let endpoint = format!(
        "https://{account_name}{QUEUE_ENDPOINT_SUFFIX}/{queue_name}/messages?messagettl={MESSAGE_TTL_SECS}"
    );
    let response = queue_http_client()?
        .post(endpoint)
        .bearer_auth(token.token.secret())
        .header("x-ms-version", QUEUE_API_VERSION)
        .header(
            "x-ms-date",
            chrono::Utc::now()
                .format("%a, %d %b %Y %H:%M:%S GMT")
                .to_string(),
        )
        .header(reqwest::header::CONTENT_TYPE, "application/xml")
        .body(request_body)
        .send()
        .await
        .map_err(|error| format!("failed to send queue message: {error}"))?;

    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let request_id = response
        .headers()
        .get("x-ms-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unavailable")
        .to_string();
    let body = response.bytes().await.unwrap_or_default();
    let body = String::from_utf8_lossy(&body[..body.len().min(MAX_ERROR_BODY_BYTES)]);
    Err(format!(
        "Azure Queue Storage rejected the message with HTTP {status} (request {request_id}): {body}"
    ))
}

fn queue_http_client() -> Result<&'static reqwest::Client, String> {
    if let Some(client) = QUEUE_HTTP_CLIENT.get() {
        return Ok(client);
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .pool_idle_timeout(Duration::from_secs(90))
        .user_agent(concat!("flow-like/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("failed to construct Azure Queue Storage client: {error}"))?;
    let _ = QUEUE_HTTP_CLIENT.set(client);
    QUEUE_HTTP_CLIENT
        .get()
        .ok_or_else(|| "failed to initialize Azure Queue Storage client".to_string())
}

/// The credential is cached because it caches the identity token it fetches;
/// rebuilding it per dispatch would mean an IMDS round trip per message.
fn queue_credential() -> Result<&'static Arc<dyn TokenCredential>, String> {
    if let Some(credential) = QUEUE_CREDENTIAL.get() {
        return Ok(credential);
    }
    let client_id = required_uuid_env("AZURE_CLIENT_ID")?;
    let credential = ManagedIdentityCredential::new(Some(ManagedIdentityCredentialOptions {
        user_assigned_id: Some(UserAssignedId::ClientId(client_id)),
        ..Default::default()
    }))
    .map_err(|error| format!("failed to initialize managed identity: {error}"))?;
    let _ = QUEUE_CREDENTIAL.set(credential as Arc<dyn TokenCredential>);
    QUEUE_CREDENTIAL
        .get()
        .ok_or_else(|| "failed to initialize managed identity".to_string())
}

fn reject_secret_credentials() -> Result<(), String> {
    for variable in FORBIDDEN_CREDENTIAL_SETTINGS {
        if std::env::var(variable)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(format!(
                "{variable} is forbidden for Azure Queue Storage; use the assigned managed identity"
            ));
        }
    }
    Ok(())
}

fn required_uuid_env(name: &str) -> Result<String, String> {
    let value = std::env::var(name)
        .map_err(|_| format!("{name} is required for user-assigned managed identity"))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!(
            "{name} is required for user-assigned managed identity"
        ));
    }
    uuid::Uuid::parse_str(value).map_err(|_| format!("{name} must be a valid UUID client ID"))?;
    Ok(value.to_string())
}

fn validate_account_name(value: &str) -> Result<(), String> {
    let valid = (3..=24).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
    if !valid {
        return Err(
            "AZURE_QUEUE_STORAGE_ACCOUNT_NAME must be 3-24 lowercase alphanumeric characters"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_queue_name(value: &str) -> Result<(), String> {
    let valid = (3..=63).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--");
    if !valid {
        return Err("Azure Queue Storage queue name is invalid".to_string());
    }
    Ok(())
}

fn validate_workload(value: &str) -> Result<(), String> {
    if !SUPPORTED_WORKLOADS.contains(&value) {
        return Err(format!("unsupported queue workload '{value}'"));
    }
    Ok(())
}

fn validate_identifier(kind: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(format!("queue message {kind} is invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_only_expected_storage_account_names() {
        assert!(validate_account_name("flowlikedevdata").is_ok());
        assert!(validate_account_name("flowlike-dev").is_err());
        assert!(validate_account_name("FLOWLIKEDEV").is_err());
        assert!(validate_account_name("ab").is_err());
        assert!(validate_account_name("a".repeat(25).as_str()).is_err());
    }

    #[test]
    fn queue_names_are_restricted_to_deployment_generated_names() {
        assert!(validate_queue_name("execution").is_ok());
        assert!(validate_queue_name("media-transformation-poison").is_ok());
        assert!(validate_queue_name("execution/jobs").is_err());
        assert!(validate_queue_name("-execution").is_err());
        assert!(validate_queue_name("exec--ution").is_err());
    }

    #[test]
    fn workloads_are_restricted_to_the_deployed_worker_set() {
        assert!(validate_workload(WORKLOAD_EXECUTION).is_ok());
        assert!(validate_workload(WORKLOAD_COMPILATION).is_ok());
        assert!(validate_workload(WORKLOAD_FILE_TRACKING).is_ok());
        assert!(validate_workload(WORKLOAD_MEDIA_TRANSFORMATION).is_ok());
        assert!(validate_workload("file-tracker").is_err());
        assert!(validate_workload("").is_err());
    }

    #[test]
    fn envelope_overhead_matches_the_serialized_envelope() {
        let payload = serde_json::json!({ "remote_url": "https://example.invalid/blob" });
        let payload_bytes = serde_json::to_vec(&payload).expect("payload serializes");
        let envelope = serde_json::to_vec(&MessageEnvelope {
            v: ENVELOPE_VERSION,
            job_id: "job-1",
            correlation_id: "app-1",
            workload: WORKLOAD_EXECUTION,
            payload: &payload,
        })
        .expect("envelope serializes");

        assert_eq!(
            envelope.len(),
            payload_bytes.len() + envelope_overhead_bytes("job-1", "app-1", WORKLOAD_EXECUTION)
        );
    }

    #[test]
    fn claim_check_triggers_before_the_hard_wire_limit() {
        let overhead = envelope_overhead_bytes("job-1", "app-1", WORKLOAD_EXECUTION);
        assert!(!requires_claim_check(
            CLAIM_CHECK_THRESHOLD_BYTES - overhead - 1,
            "job-1",
            "app-1",
            WORKLOAD_EXECUTION
        ));
        assert!(requires_claim_check(
            CLAIM_CHECK_THRESHOLD_BYTES - overhead,
            "job-1",
            "app-1",
            WORKLOAD_EXECUTION
        ));
        // Every inlined message still leaves room under the 64 KiB encoded cap.
        const { assert!(CLAIM_CHECK_THRESHOLD_BYTES < MAX_MESSAGE_BODY_BYTES) };
        assert_eq!(MAX_MESSAGE_BODY_BYTES, 65_536 * 3 / 4);
    }

    #[test]
    fn encoded_message_text_never_needs_xml_escaping() {
        let encoded = BASE64_STANDARD.encode(b"{\"payload\":\"<&>\\u001f\"}");
        assert!(
            encoded
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
        );
    }
}

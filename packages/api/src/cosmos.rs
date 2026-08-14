//! Minimal Azure Cosmos DB for NoSQL data-plane client.
//!
//! The official Rust Cosmos data-plane SDK is still a preview.  This client deliberately
//! uses the stable REST contract instead, and only implements the small set of item
//! operations needed by the cache and execution-state abstractions.  Authentication is
//! always Microsoft Entra ID; account keys and resource tokens are not supported.

use azure_core::credentials::TokenCredential;
use azure_identity::{
    DeveloperToolsCredential, ManagedIdentityCredential, ManagedIdentityCredentialOptions,
    UserAssignedId, WorkloadIdentityCredential,
};
use reqwest::{
    Method, StatusCode, Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{fmt::Debug, sync::Arc, time::Duration};

const COSMOS_SCOPE: &str = "https://cosmos.azure.com/.default";
const REST_API_VERSION: &str = "2018-12-31";
const DEFAULT_DATABASE: &str = "flowlike";
const DEFAULT_MAX_RETRIES: u32 = 8;
const MAX_RETRY_DELAY_MS: u64 = 30_000;
const MAX_ERROR_BODY_BYTES: usize = 4_096;

const PARTITION_KEY_HEADER: &str = "x-ms-documentdb-partitionkey";
const CONTINUATION_HEADER: &str = "x-ms-continuation";

#[derive(Debug, thiserror::Error)]
pub(crate) enum CosmosError {
    #[error("Cosmos configuration error: {0}")]
    Configuration(String),
    #[error("Cosmos authentication error: {0}")]
    Authentication(String),
    #[error("Cosmos transport error: {0}")]
    Transport(String),
    #[error(
        "Cosmos request failed with HTTP {status}{activity}{substatus}: {message}",
        activity = activity_id.as_deref().map(|id| format!(" (activity {id})")).unwrap_or_default(),
        substatus = substatus.as_deref().map(|value| format!(" (substatus {value})")).unwrap_or_default()
    )]
    Service {
        status: StatusCode,
        activity_id: Option<String>,
        substatus: Option<String>,
        message: String,
    },
    #[error("Cosmos response serialization error: {0}")]
    Serialization(String),
}

/// Result of an operation whose target can be absent or can lose an ETag race.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MutationOutcome {
    Applied,
    NotFound,
    Conflict,
    PreconditionFailed,
}

#[derive(Debug)]
pub(crate) struct QueryPage<T> {
    pub(crate) documents: Vec<T>,
    pub(crate) continuation: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(bound(deserialize = "T: serde::Deserialize<'de>"))]
struct QueryEnvelope<T> {
    #[serde(rename = "Documents", default)]
    documents: Vec<T>,
}

#[derive(Serialize)]
struct QueryRequest<'a> {
    query: &'a str,
    parameters: &'a [QueryParameter],
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct QueryParameter {
    name: String,
    value: Value,
}

impl QueryParameter {
    pub(crate) fn new(name: impl Into<String>, value: impl Into<Value>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

struct RawResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: bytes::Bytes,
}

/// A cloneable REST client shared by the cache and execution-state stores.
#[derive(Clone)]
pub(crate) struct CosmosClient {
    http: reqwest::Client,
    endpoint: String,
    database: String,
    credential: Arc<dyn TokenCredential>,
    max_retries: u32,
    session_tokens: moka::sync::Cache<String, String>,
}

impl Debug for CosmosClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CosmosClient")
            .field("endpoint", &self.endpoint)
            .field("database", &self.database)
            .field("max_retries", &self.max_retries)
            .finish_non_exhaustive()
    }
}

impl CosmosClient {
    pub(crate) fn from_env() -> Result<Self, CosmosError> {
        let endpoint = required_env("COSMOS_ENDPOINT")?;
        let database = std::env::var("COSMOS_DATABASE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_DATABASE.to_string());
        let max_retries = match std::env::var("COSMOS_MAX_RETRIES") {
            Ok(value) => value.trim().parse::<u32>().map_err(|_| {
                CosmosError::Configuration(
                    "COSMOS_MAX_RETRIES must be an integer between 0 and 20".to_string(),
                )
            })?,
            Err(std::env::VarError::NotPresent) => DEFAULT_MAX_RETRIES,
            Err(error) => {
                return Err(CosmosError::Configuration(format!(
                    "COSMOS_MAX_RETRIES could not be read: {error}"
                )));
            }
        };
        if max_retries > 20 {
            return Err(CosmosError::Configuration(
                "COSMOS_MAX_RETRIES must be between 0 and 20".to_string(),
            ));
        }
        let credential = credential_from_env()?;

        Self::new(endpoint, database, credential, max_retries)
    }

    fn new(
        endpoint: String,
        database: String,
        credential: Arc<dyn TokenCredential>,
        max_retries: u32,
    ) -> Result<Self, CosmosError> {
        validate_resource_id("COSMOS_DATABASE", &database)?;

        let mut url = Url::parse(endpoint.trim()).map_err(|error| {
            CosmosError::Configuration(format!("COSMOS_ENDPOINT is not a valid URL: {error}"))
        })?;
        if url.scheme() != "https" {
            return Err(CosmosError::Configuration(
                "COSMOS_ENDPOINT must use HTTPS".to_string(),
            ));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(CosmosError::Configuration(
                "COSMOS_ENDPOINT must not contain credentials".to_string(),
            ));
        }
        if url.query().is_some() || url.fragment().is_some() {
            return Err(CosmosError::Configuration(
                "COSMOS_ENDPOINT must not contain a query string or fragment".to_string(),
            ));
        }
        url.set_path("");

        let http = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(concat!("flow-like/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| CosmosError::Configuration(error.to_string()))?;

        Ok(Self {
            http,
            endpoint: url.as_str().trim_end_matches('/').to_string(),
            database,
            credential,
            max_retries,
            session_tokens: moka::sync::Cache::builder()
                .max_capacity(10_000)
                .time_to_idle(Duration::from_secs(60 * 60))
                .build(),
        })
    }

    pub(crate) async fn read_document<T: DeserializeOwned>(
        &self,
        container: &str,
        id: &str,
        partition_key: &str,
    ) -> Result<Option<T>, CosmosError> {
        let path = self.document_path(container, Some(id))?;
        let response = self
            .send(
                Method::GET,
                path,
                Some(partition_key),
                None,
                HeaderMap::new(),
            )
            .await?;

        match response.status {
            StatusCode::OK => deserialize(&response.body).map(Some),
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(service_error(response)),
        }
    }

    /// Create a document without upsert semantics. A conflict is returned to the caller
    /// so `try_insert` can retain its atomic contract.
    pub(crate) async fn create_document<T: Serialize>(
        &self,
        container: &str,
        partition_key: &str,
        document: &T,
    ) -> Result<MutationOutcome, CosmosError> {
        self.write_document(
            Method::POST,
            container,
            None,
            partition_key,
            document,
            false,
            None,
        )
        .await
    }

    pub(crate) async fn upsert_document<T: Serialize>(
        &self,
        container: &str,
        partition_key: &str,
        document: &T,
    ) -> Result<(), CosmosError> {
        match self
            .write_document(
                Method::POST,
                container,
                None,
                partition_key,
                document,
                true,
                None,
            )
            .await?
        {
            MutationOutcome::Applied => Ok(()),
            outcome => Err(CosmosError::Transport(format!(
                "unexpected upsert result: {outcome:?}"
            ))),
        }
    }

    pub(crate) async fn replace_document<T: Serialize>(
        &self,
        container: &str,
        id: &str,
        partition_key: &str,
        document: &T,
        etag: Option<&str>,
    ) -> Result<MutationOutcome, CosmosError> {
        self.write_document(
            Method::PUT,
            container,
            Some(id),
            partition_key,
            document,
            false,
            etag,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn write_document<T: Serialize>(
        &self,
        method: Method,
        container: &str,
        id: Option<&str>,
        partition_key: &str,
        document: &T,
        upsert: bool,
        etag: Option<&str>,
    ) -> Result<MutationOutcome, CosmosError> {
        let path = self.document_path(container, id)?;
        let body = serde_json::to_vec(document)
            .map_err(|error| CosmosError::Serialization(error.to_string()))?;
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        if upsert {
            headers.insert(
                HeaderName::from_static("x-ms-documentdb-is-upsert"),
                HeaderValue::from_static("true"),
            );
        }
        if let Some(etag) = etag {
            headers.insert(
                reqwest::header::IF_MATCH,
                HeaderValue::from_str(etag).map_err(|error| {
                    CosmosError::Serialization(format!("invalid Cosmos ETag: {error}"))
                })?,
            );
        }

        let response = self
            .send(method, path, Some(partition_key), Some(body), headers)
            .await?;

        match response.status {
            StatusCode::OK | StatusCode::CREATED => Ok(MutationOutcome::Applied),
            StatusCode::NOT_FOUND => Ok(MutationOutcome::NotFound),
            StatusCode::CONFLICT => Ok(MutationOutcome::Conflict),
            StatusCode::PRECONDITION_FAILED => Ok(MutationOutcome::PreconditionFailed),
            _ => Err(service_error(response)),
        }
    }

    pub(crate) async fn delete_document(
        &self,
        container: &str,
        id: &str,
        partition_key: &str,
        etag: Option<&str>,
    ) -> Result<MutationOutcome, CosmosError> {
        let path = self.document_path(container, Some(id))?;
        let mut headers = HeaderMap::new();
        if let Some(etag) = etag {
            headers.insert(
                reqwest::header::IF_MATCH,
                HeaderValue::from_str(etag).map_err(|error| {
                    CosmosError::Serialization(format!("invalid Cosmos ETag: {error}"))
                })?,
            );
        }
        let response = self
            .send(Method::DELETE, path, Some(partition_key), None, headers)
            .await?;

        match response.status {
            StatusCode::NO_CONTENT => Ok(MutationOutcome::Applied),
            StatusCode::NOT_FOUND => Ok(MutationOutcome::NotFound),
            StatusCode::PRECONDITION_FAILED => Ok(MutationOutcome::PreconditionFailed),
            StatusCode::CONFLICT => Ok(MutationOutcome::Conflict),
            _ => Err(service_error(response)),
        }
    }

    pub(crate) async fn query_page<T: DeserializeOwned>(
        &self,
        container: &str,
        query: &str,
        parameters: &[QueryParameter],
        partition_key: Option<&str>,
        max_item_count: i32,
        continuation: Option<&str>,
    ) -> Result<QueryPage<T>, CosmosError> {
        let path = self.document_path(container, None)?;
        let body = serde_json::to_vec(&QueryRequest { query, parameters })
            .map_err(|error| CosmosError::Serialization(error.to_string()))?;
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            HeaderValue::from_static("application/query+json"),
        );
        headers.insert(
            HeaderName::from_static("x-ms-documentdb-isquery"),
            HeaderValue::from_static("true"),
        );
        headers.insert(
            HeaderName::from_static("x-ms-max-item-count"),
            HeaderValue::from_str(&max_item_count.clamp(1, 1_000).to_string()).map_err(
                |error| CosmosError::Serialization(format!("invalid max item count: {error}")),
            )?,
        );
        if partition_key.is_none() {
            headers.insert(
                HeaderName::from_static("x-ms-documentdb-query-enablecrosspartition"),
                HeaderValue::from_static("true"),
            );
        }
        if let Some(continuation) = continuation {
            headers.insert(
                HeaderName::from_static(CONTINUATION_HEADER),
                HeaderValue::from_str(continuation).map_err(|error| {
                    CosmosError::Serialization(format!("invalid continuation token: {error}"))
                })?,
            );
        }

        let response = self
            .send(Method::POST, path, partition_key, Some(body), headers)
            .await?;
        if response.status != StatusCode::OK {
            return Err(service_error(response));
        }

        let continuation = response
            .headers
            .get(CONTINUATION_HEADER)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let envelope: QueryEnvelope<T> = deserialize(&response.body)?;
        Ok(QueryPage {
            documents: envelope.documents,
            continuation,
        })
    }

    async fn send(
        &self,
        method: Method,
        path: String,
        partition_key: Option<&str>,
        body: Option<Vec<u8>>,
        extra_headers: HeaderMap,
    ) -> Result<RawResponse, CosmosError> {
        let url = format!("{}{}", self.endpoint, path);
        let request_session_key = session_key(&path, partition_key);

        for attempt in 0..=self.max_retries {
            let token = self
                .credential
                .get_token(&[COSMOS_SCOPE], None)
                .await
                .map_err(|error| CosmosError::Authentication(error.to_string()))?;
            // Cosmos uses its historical authorization envelope for both signed account
            // keys and Entra tokens. The complete value must be percent-encoded.
            let authorization =
                urlencoding::encode(&format!("type=aad&ver=1.0&sig={}", token.token.secret()))
                    .into_owned();

            let date = cosmos_date();
            let mut request = self
                .http
                .request(method.clone(), &url)
                .header("authorization", authorization)
                .header("x-ms-date", date)
                .header("x-ms-version", REST_API_VERSION)
                .header(reqwest::header::ACCEPT, "application/json")
                .headers(extra_headers.clone());

            if let Some(session_token) = self.session_tokens.get(&request_session_key) {
                request = request.header("x-ms-session-token", session_token);
            }

            if let Some(partition_key) = partition_key {
                let encoded = serde_json::to_string(&[partition_key])
                    .map_err(|error| CosmosError::Serialization(error.to_string()))?;
                request = request.header(PARTITION_KEY_HEADER, encoded);
            }
            if let Some(body) = body.as_ref() {
                request = request.body(body.clone());
            }

            let response = request
                .send()
                .await
                .map_err(|error| CosmosError::Transport(error.to_string()))?;
            let status = response.status();
            let headers = response.headers().clone();
            let body = response
                .bytes()
                .await
                .map_err(|error| CosmosError::Transport(error.to_string()))?;

            if status.is_success()
                && let Some(session_token) = headers
                    .get("x-ms-session-token")
                    .and_then(|value| value.to_str().ok())
            {
                self.session_tokens
                    .insert(request_session_key.clone(), session_token.to_string());
                if partition_key.is_some() {
                    // Cross-partition queries (notably get_run, whose trait only has a
                    // run ID) also need to observe a write performed with a concrete
                    // partition key. Retain the most recent collection-level token in
                    // addition to the exact-partition token.
                    self.session_tokens
                        .insert(session_key(&path, None), session_token.to_string());
                }
            }

            // A 429 means Cosmos did not accept the operation, so retrying writes is
            // safe. Respect the service-provided delay rather than guessing at RU load.
            if status == StatusCode::TOO_MANY_REQUESTS && attempt < self.max_retries {
                let delay_ms = headers
                    .get("x-ms-retry-after-ms")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or_else(|| 100_u64.saturating_mul(1_u64 << attempt.min(8)))
                    .min(MAX_RETRY_DELAY_MS);
                tracing::warn!(
                    attempt = attempt + 1,
                    delay_ms,
                    activity_id = headers
                        .get("x-ms-activity-id")
                        .and_then(|value| value.to_str().ok()),
                    "Cosmos DB throttled a request; retrying after the server delay"
                );
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                continue;
            }

            return Ok(RawResponse {
                status,
                headers,
                body,
            });
        }

        unreachable!("retry loop always returns on its final attempt")
    }

    fn document_path(&self, container: &str, id: Option<&str>) -> Result<String, CosmosError> {
        validate_resource_id("Cosmos container", container)?;
        if let Some(id) = id {
            validate_document_id(id)?;
        }

        let base = format!(
            "/dbs/{}/colls/{}/docs",
            urlencoding::encode(&self.database),
            urlencoding::encode(container)
        );
        Ok(match id {
            Some(id) => format!("{base}/{}", urlencoding::encode(id)),
            None => base,
        })
    }
}

fn credential_from_env() -> Result<Arc<dyn TokenCredential>, CosmosError> {
    let mode = std::env::var("COSMOS_AUTH_MODE")
        .unwrap_or_else(|_| "auto".to_string())
        .trim()
        .to_ascii_lowercase();
    let has_workload_identity = std::env::var_os("AZURE_FEDERATED_TOKEN_FILE").is_some();
    let has_managed_identity_endpoint = [
        "IDENTITY_ENDPOINT",
        "MSI_ENDPOINT",
        "IMDS_ENDPOINT",
        "IDENTITY_HEADER",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some());

    match mode.as_str() {
        "auto" if has_workload_identity => WorkloadIdentityCredential::new(None)
            .map(|credential| credential as Arc<dyn TokenCredential>)
            .map_err(|error| CosmosError::Authentication(error.to_string())),
        "auto" if has_managed_identity_endpoint => managed_identity_credential(),
        "auto" => DeveloperToolsCredential::new(None)
            .map(|credential| credential as Arc<dyn TokenCredential>)
            .map_err(|error| CosmosError::Authentication(error.to_string())),
        "managed_identity" | "managed-identity" | "mi" => managed_identity_credential(),
        "workload_identity" | "workload-identity" | "wi" => WorkloadIdentityCredential::new(None)
            .map(|credential| credential as Arc<dyn TokenCredential>)
            .map_err(|error| CosmosError::Authentication(error.to_string())),
        "developer_tools" | "developer-tools" | "dev" => DeveloperToolsCredential::new(None)
            .map(|credential| credential as Arc<dyn TokenCredential>)
            .map_err(|error| CosmosError::Authentication(error.to_string())),
        _ => Err(CosmosError::Configuration(format!(
            "unsupported COSMOS_AUTH_MODE '{mode}'; expected auto, managed_identity, \
             workload_identity, or developer_tools"
        ))),
    }
}

fn managed_identity_credential() -> Result<Arc<dyn TokenCredential>, CosmosError> {
    let user_assigned_id = std::env::var("AZURE_CLIENT_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(UserAssignedId::ClientId);
    ManagedIdentityCredential::new(Some(ManagedIdentityCredentialOptions {
        user_assigned_id,
        ..Default::default()
    }))
    .map(|credential| credential as Arc<dyn TokenCredential>)
    .map_err(|error| CosmosError::Authentication(error.to_string()))
}

fn required_env(name: &str) -> Result<String, CosmosError> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CosmosError::Configuration(format!("{name} is required")))
}

fn validate_resource_id(label: &str, value: &str) -> Result<(), CosmosError> {
    if value.trim().is_empty() {
        return Err(CosmosError::Configuration(format!(
            "{label} must not be empty"
        )));
    }
    if value.len() > 255 || value.ends_with(' ') || value.ends_with('.') {
        return Err(CosmosError::Configuration(format!(
            "{label} is not a valid Cosmos DB resource ID"
        )));
    }
    if value
        .chars()
        .any(|character| matches!(character, '/' | '\\' | '?' | '#'))
    {
        return Err(CosmosError::Configuration(format!(
            "{label} contains a character Cosmos DB resource IDs do not allow"
        )));
    }
    Ok(())
}

pub(crate) fn validate_container_id(value: &str) -> Result<(), CosmosError> {
    validate_resource_id("Cosmos container", value)
}

fn validate_document_id(id: &str) -> Result<(), CosmosError> {
    validate_resource_id("Cosmos document ID", id)
}

fn cosmos_date() -> String {
    chrono::Utc::now()
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

fn deserialize<T: DeserializeOwned>(body: &[u8]) -> Result<T, CosmosError> {
    serde_json::from_slice(body).map_err(|error| CosmosError::Serialization(error.to_string()))
}

fn service_error(response: RawResponse) -> CosmosError {
    let activity_id = header_string(&response.headers, "x-ms-activity-id");
    let substatus = header_string(&response.headers, "x-ms-substatus");
    let body = &response.body[..response.body.len().min(MAX_ERROR_BODY_BYTES)];
    let message = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .or_else(|| value.get("Message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| String::from_utf8_lossy(body).into_owned());
    CosmosError::Service {
        status: response.status,
        activity_id,
        substatus,
        message,
    }
}

fn header_string(headers: &HeaderMap, name: &'static str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn session_key(path: &str, partition_key: Option<&str>) -> String {
    let collection_path = path.split_once("/docs").map_or(path, |(prefix, _)| prefix);
    format!("{collection_path}|{}", partition_key.unwrap_or("*"))
}

/// Convert an absolute epoch-millisecond expiry into Cosmos' relative `ttl` seconds.
/// A minimum of one second avoids accidentally serializing `0`, which disables TTL for
/// that item rather than expiring it.
pub(crate) fn ttl_seconds(expires_at_ms: Option<i64>, now_ms: i64) -> Option<i64> {
    expires_at_ms.map(|expires_at| {
        let remaining_ms = expires_at.saturating_sub(now_ms);
        remaining_ms.saturating_add(999).div_euclid(1_000).max(1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_rounds_up_and_never_disables_expiry() {
        assert_eq!(ttl_seconds(None, 1_000), None);
        assert_eq!(ttl_seconds(Some(1_001), 1_000), Some(1));
        assert_eq!(ttl_seconds(Some(2_001), 1_000), Some(2));
        assert_eq!(ttl_seconds(Some(999), 1_000), Some(1));
    }

    #[test]
    fn cosmos_ids_reject_ambiguous_path_characters() {
        for id in ["a/b", "a\\b", "a?b", "a#b"] {
            assert!(validate_document_id(id).is_err());
        }
        assert!(validate_document_id("safe-0123_ABC").is_ok());
    }
}

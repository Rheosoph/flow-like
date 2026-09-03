//! Minimal Google Cloud Firestore (Native mode) data-plane client.
//!
//! The generated Firestore Rust client is a gRPC surface with a large transitive
//! dependency footprint. This client deliberately uses the stable REST v1
//! contract instead, and only implements the small set of document operations
//! needed by the cache and execution-state abstractions. Authentication is
//! always a metadata-server access token; service-account keys and the
//! emulator environment variables are not supported.
//!
//! The public surface mirrors `flow-like-azure-data::cosmos` so the two stores
//! port between clouds mechanically. Three differences are forced by Firestore
//! and are called out where they occur: there is no partition key, the
//! concurrency token is the server-owned `updateTime` rather than a field inside
//! the document, and time-to-live has to be written as a typed `timestampValue`
//! that no `serde` shape can express.

use crate::metadata::{DATASTORE_SCOPE, MetadataError, TokenSource};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{Method, StatusCode, header::HeaderValue};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use std::{fmt::Debug, time::Duration};

const FIRESTORE_ENDPOINT: &str = "https://firestore.googleapis.com";
const REST_API_VERSION: &str = "v1";
const DEFAULT_DATABASE: &str = "(default)";
const DEFAULT_MAX_RETRIES: u32 = 8;
const MAX_RETRY_DELAY_MS: u64 = 30_000;
const MAX_ERROR_BODY_BYTES: usize = 4_096;

/// Firestore rejects a commit carrying more than 500 writes, and rejects two
/// writes that touch the same document. Both are checked locally so the caller
/// gets a diagnosable error instead of an opaque `INVALID_ARGUMENT`.
const MAX_WRITES_PER_COMMIT: usize = 500;
const MAX_PAGE_SIZE: i32 = 1_000;
/// Firestore document and collection IDs are capped at 1500 bytes.
const MAX_ID_BYTES: usize = 1_500;

/// The one TTL field name every collection uses. Firestore's TTL is a per-field
/// policy declared in Terraform, and the policy names exactly one field per
/// collection — a second name would mean a second policy, so the client pins it.
///
/// Contract with `modules/gcp/state` (`local.ttl_field`, exported as
/// `ttl_fields`): the policy watches `expires_at_ts`, never the epoch-milliseconds
/// `expires_at` integer the records are typed with. Firestore's TTL only collects
/// a `Timestamp`, and a name that differs from the policy's produces documents
/// that are never collected — no error, just unbounded growth — so this string
/// and the Terraform local must change together.
pub const EXPIRES_AT_FIELD: &str = "expires_at_ts";
/// Firestore's implicit ordering key. Every query is ordered by it so a cursor
/// is always well defined.
const DOCUMENT_NAME_FIELD: &str = "__name__";

/// Ceiling on the scan behind [`FirestoreClient::count_documents`].
///
/// Firestore keeps no document counter: a count is an aggregation query, billed one
/// read per 1,000 documents it matches and taking longer the more it matches. The
/// cap trades an exact number for a bounded one — at most 1,000 reads and a
/// predictable latency — so a collection that grows without bound cannot turn a
/// dashboard refresh into the most expensive request the deployment serves. A count
/// that reaches the cap means "at least this many".
const MAX_COUNT_DOCUMENTS: i64 = 1_000_000;
/// Name the COUNT aggregate is returned under; the request chooses it.
const COUNT_ALIAS: &str = "count";

/// Emulator and endpoint overrides are refused rather than ignored: a workload
/// that silently talks to an emulator writes production data nowhere, and a
/// workload that honours an endpoint override sends its access token to whoever
/// set the variable.
const FORBIDDEN_SETTINGS: &[&str] = &[
    "FIRESTORE_EMULATOR_HOST",
    "DATASTORE_EMULATOR_HOST",
    "CLOUDSDK_API_ENDPOINT_OVERRIDES_FIRESTORE",
    "CLOUDSDK_API_ENDPOINT_OVERRIDES_DATASTORE",
    "GOOGLE_FIRESTORE_CUSTOM_ENDPOINT",
];

#[derive(Debug, thiserror::Error)]
pub enum FirestoreError {
    #[error("Firestore configuration error: {0}")]
    Configuration(String),
    #[error("Firestore authentication error: {0}")]
    Authentication(String),
    #[error("Firestore transport error: {0}")]
    Transport(String),
    #[error(
        "Firestore request failed with HTTP {status}{code}{reason}: {message}",
        code = code.as_deref().map(|value| format!(" ({value})")).unwrap_or_default(),
        reason = reason.as_deref().map(|value| format!(" (reason {value})")).unwrap_or_default()
    )]
    Service {
        status: StatusCode,
        code: Option<String>,
        reason: Option<String>,
        message: String,
    },
    #[error("Firestore response serialization error: {0}")]
    Serialization(String),
}

impl From<MetadataError> for FirestoreError {
    fn from(error: MetadataError) -> Self {
        match error {
            MetadataError::Configuration(message) => Self::Configuration(message),
            other => Self::Authentication(other.to_string()),
        }
    }
}

/// Result of an operation whose target can be absent or can lose an `updateTime`
/// race. `Conflict` and `PreconditionFailed` are load-bearing: the cache's
/// compare-and-swap and the execution-run lease claim both decide whether they
/// won by reading them, so they must never be collapsed into a generic error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MutationOutcome {
    Applied,
    NotFound,
    Conflict,
    PreconditionFailed,
}

/// A document plus the server metadata the caller needs for its next write.
///
/// Cosmos carries its ETag inside the document body as `_etag`, so its reads can
/// return `T` alone. Firestore keeps `updateTime` in the envelope, so a
/// compare-and-swap caller can only obtain it here.
#[derive(Clone, Debug)]
pub struct Document<T> {
    pub id: String,
    pub create_time: Option<String>,
    pub update_time: Option<String>,
    pub value: T,
}

#[derive(Debug)]
pub struct QueryPage<T> {
    pub documents: Vec<T>,
    pub continuation: Option<String>,
}

/// Outcome of a `commit`. Firestore applies the whole write set atomically, so
/// there is one outcome for the batch rather than one per write.
#[derive(Debug)]
pub struct CommitOutcome {
    pub outcome: MutationOutcome,
    pub update_times: Vec<Option<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterOperator {
    Equal,
    NotEqual,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    ArrayContains,
    ArrayContainsAny,
    In,
    NotIn,
}

impl FilterOperator {
    fn wire_name(self) -> &'static str {
        match self {
            Self::Equal => "EQUAL",
            Self::NotEqual => "NOT_EQUAL",
            Self::LessThan => "LESS_THAN",
            Self::LessThanOrEqual => "LESS_THAN_OR_EQUAL",
            Self::GreaterThan => "GREATER_THAN",
            Self::GreaterThanOrEqual => "GREATER_THAN_OR_EQUAL",
            Self::ArrayContains => "ARRAY_CONTAINS",
            Self::ArrayContainsAny => "ARRAY_CONTAINS_ANY",
            Self::In => "IN",
            Self::NotIn => "NOT_IN",
        }
    }

    fn requires_array_operand(self) -> bool {
        matches!(self, Self::In | Self::NotIn | Self::ArrayContainsAny)
    }
}

/// A comparison operand, either plain JSON to be encoded or a Firestore value
/// the caller already built. Timestamps have to arrive pre-encoded: nothing in a
/// JSON value says "this string is a timestamp".
#[derive(Clone, Debug)]
enum Operand {
    Json(Value),
    Encoded(Value),
}

/// One `AND`-ed condition. The Cosmos analogue is a `QueryParameter` bound into
/// a SQL string; Firestore has no query language, so the predicate itself is the
/// parameter.
#[derive(Clone, Debug)]
pub struct QueryFilter {
    field: String,
    operator: FilterOperator,
    operand: Operand,
}

impl QueryFilter {
    pub fn new(
        field: impl Into<String>,
        operator: FilterOperator,
        value: impl Into<Value>,
    ) -> Self {
        Self {
            field: field.into(),
            operator,
            operand: Operand::Json(value.into()),
        }
    }

    /// Compare against a real `timestampValue`. A timestamp field compared
    /// against a `stringValue` matches nothing at all — Firestore orders values
    /// by type before it orders them by content — and it fails silently, so the
    /// typed constructor exists to keep callers off the plain `new`.
    pub fn timestamp(
        field: impl Into<String>,
        operator: FilterOperator,
        epoch_millis: i64,
    ) -> Result<Self, FirestoreError> {
        Ok(Self {
            field: field.into(),
            operator,
            operand: Operand::Encoded(timestamp_value(epoch_millis)?),
        })
    }

    /// Compare against a Firestore value built by [`encode_value`], [`bytes_value`]
    /// or [`timestamp_value`].
    pub fn encoded(field: impl Into<String>, operator: FilterOperator, value: Value) -> Self {
        Self {
            field: field.into(),
            operator,
            operand: Operand::Encoded(value),
        }
    }

    fn to_wire(&self) -> Result<Value, FirestoreError> {
        validate_field_path(&self.field)?;

        // A `null` operand is not a field filter at all on Firestore: equality
        // against null is expressed as a unary `IS_NULL`. Sending a fieldFilter
        // with a nullValue is rejected with INVALID_ARGUMENT, which is the kind
        // of failure that only shows up once real data has nulls in it.
        if matches!(&self.operand, Operand::Json(Value::Null)) {
            let unary = match self.operator {
                FilterOperator::Equal => "IS_NULL",
                FilterOperator::NotEqual => "IS_NOT_NULL",
                other => {
                    return Err(FirestoreError::Configuration(format!(
                        "Firestore cannot apply {} to a null operand",
                        other.wire_name()
                    )));
                }
            };
            return Ok(json!({
                "unaryFilter": { "field": { "fieldPath": self.field }, "op": unary }
            }));
        }

        let value = match &self.operand {
            Operand::Json(value) => {
                if self.operator.requires_array_operand() && !value.is_array() {
                    return Err(FirestoreError::Configuration(format!(
                        "Firestore operator {} requires an array operand",
                        self.operator.wire_name()
                    )));
                }
                encode_value(value, false)?
            }
            Operand::Encoded(value) => value.clone(),
        };

        Ok(json!({
            "fieldFilter": {
                "field": { "fieldPath": self.field },
                "op": self.operator.wire_name(),
                "value": value,
            }
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

impl SortDirection {
    fn wire_name(self) -> &'static str {
        match self {
            Self::Ascending => "ASCENDING",
            Self::Descending => "DESCENDING",
        }
    }
}

#[derive(Clone, Debug)]
pub struct QueryOrder {
    field: String,
    direction: SortDirection,
}

impl QueryOrder {
    pub fn new(field: impl Into<String>, direction: SortDirection) -> Self {
        Self {
            field: field.into(),
            direction,
        }
    }
}

struct RawResponse {
    status: StatusCode,
    body: bytes::Bytes,
}

#[derive(serde::Deserialize)]
struct RawDocument {
    name: String,
    #[serde(default)]
    fields: Map<String, Value>,
    #[serde(rename = "createTime")]
    create_time: Option<String>,
    #[serde(rename = "updateTime")]
    update_time: Option<String>,
}

#[derive(serde::Deserialize)]
struct RunQueryEntry {
    #[serde(default)]
    document: Option<RawDocument>,
}

#[derive(serde::Deserialize)]
struct RunAggregationEntry {
    #[serde(default)]
    result: Option<AggregationResult>,
}

#[derive(serde::Deserialize)]
struct AggregationResult {
    #[serde(rename = "aggregateFields", default)]
    aggregate_fields: Map<String, Value>,
}

#[derive(serde::Deserialize)]
struct CommitResponse {
    #[serde(rename = "writeResults", default)]
    write_results: Vec<WriteResult>,
}

#[derive(serde::Deserialize)]
struct WriteResult {
    #[serde(rename = "updateTime")]
    update_time: Option<String>,
}

/// A cloneable REST client shared by the cache and execution-state stores.
#[derive(Clone)]
pub struct FirestoreClient {
    http: reqwest::Client,
    project_id: String,
    database: String,
    collection_prefix: String,
    tokens: TokenSource,
    max_retries: u32,
}

impl Debug for FirestoreClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FirestoreClient")
            .field("project_id", &self.project_id)
            .field("database", &self.database)
            .field("collection_prefix", &self.collection_prefix)
            .field("max_retries", &self.max_retries)
            .finish_non_exhaustive()
    }
}

impl FirestoreClient {
    pub fn from_env() -> Result<Self, FirestoreError> {
        for name in FORBIDDEN_SETTINGS {
            if std::env::var_os(name).is_some() {
                return Err(FirestoreError::Configuration(format!(
                    "{name} is set; the Firestore client only talks to {FIRESTORE_ENDPOINT}"
                )));
            }
        }

        let project_id = required_env("GCP_PROJECT_ID")?;
        let database = std::env::var("FIRESTORE_DATABASE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_DATABASE.to_string());
        let collection_prefix = std::env::var("FIRESTORE_COLLECTION_PREFIX").unwrap_or_default();
        let max_retries = match std::env::var("FIRESTORE_MAX_RETRIES") {
            Ok(value) => value.trim().parse::<u32>().map_err(|_| {
                FirestoreError::Configuration(
                    "FIRESTORE_MAX_RETRIES must be an integer between 0 and 20".to_string(),
                )
            })?,
            Err(std::env::VarError::NotPresent) => DEFAULT_MAX_RETRIES,
            Err(error) => {
                return Err(FirestoreError::Configuration(format!(
                    "FIRESTORE_MAX_RETRIES could not be read: {error}"
                )));
            }
        };

        Self::new(
            project_id,
            database,
            collection_prefix,
            TokenSource::metadata(&[DATASTORE_SCOPE])?,
            max_retries,
        )
    }

    pub fn new(
        project_id: String,
        database: String,
        collection_prefix: String,
        tokens: TokenSource,
        max_retries: u32,
    ) -> Result<Self, FirestoreError> {
        if max_retries > 20 {
            return Err(FirestoreError::Configuration(
                "Firestore max_retries must be between 0 and 20".to_string(),
            ));
        }
        validate_project_id(&project_id)?;
        validate_database_id(&database)?;
        validate_collection_prefix(&collection_prefix)?;

        let http = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(concat!("flow-like/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| FirestoreError::Configuration(error.to_string()))?;

        Ok(Self {
            http,
            project_id,
            database,
            collection_prefix,
            tokens,
            max_retries,
        })
    }

    pub async fn read_document<T: DeserializeOwned>(
        &self,
        collection: &str,
        id: &str,
    ) -> Result<Option<Document<T>>, FirestoreError> {
        let path = self.document_path(collection, Some(id))?;
        let response = self.send(Method::GET, &path, &[], None).await?;

        match response.status {
            StatusCode::OK => {
                let raw: RawDocument = deserialize(&response.body)?;
                document_from_raw(raw).map(Some)
            }
            StatusCode::NOT_FOUND => Ok(None),
            _ => Err(service_error(response)),
        }
    }

    /// Create a document without upsert semantics. A conflict is returned to the
    /// caller so `try_insert` can retain its atomic contract.
    pub async fn create_document<T: Serialize>(
        &self,
        collection: &str,
        id: &str,
        document: &T,
        expires_at_ms: Option<i64>,
    ) -> Result<MutationOutcome, FirestoreError> {
        self.write_document(
            collection,
            id,
            document,
            expires_at_ms,
            Precondition::MustNotExist,
        )
        .await
    }

    pub async fn upsert_document<T: Serialize>(
        &self,
        collection: &str,
        id: &str,
        document: &T,
        expires_at_ms: Option<i64>,
    ) -> Result<(), FirestoreError> {
        match self
            .write_document(collection, id, document, expires_at_ms, Precondition::None)
            .await?
        {
            MutationOutcome::Applied => Ok(()),
            outcome => Err(FirestoreError::Transport(format!(
                "unexpected upsert result: {outcome:?}"
            ))),
        }
    }

    /// Replace an existing document. `update_time` is Firestore's ETag analogue:
    /// pass the value read alongside the document to make this a
    /// compare-and-swap. With no `update_time` the write still requires the
    /// document to exist, matching the Cosmos `PUT` it replaces.
    pub async fn replace_document<T: Serialize>(
        &self,
        collection: &str,
        id: &str,
        document: &T,
        expires_at_ms: Option<i64>,
        update_time: Option<&str>,
    ) -> Result<MutationOutcome, FirestoreError> {
        let precondition = match update_time {
            Some(update_time) => Precondition::UpdateTime(update_time),
            None => Precondition::MustExist,
        };
        self.write_document(collection, id, document, expires_at_ms, precondition)
            .await
    }

    async fn write_document<T: Serialize>(
        &self,
        collection: &str,
        id: &str,
        document: &T,
        expires_at_ms: Option<i64>,
        precondition: Precondition<'_>,
    ) -> Result<MutationOutcome, FirestoreError> {
        let path = self.document_path(collection, Some(id))?;
        let mut fields = to_fields(document)?;
        set_expires_at(&mut fields, expires_at_ms)?;
        let body = serde_json::to_vec(&json!({ "fields": fields }))
            .map_err(|error| FirestoreError::Serialization(error.to_string()))?;

        // `patch` with no `updateMask` is a full replace, which is the semantic
        // both callers want. Adding a mask would turn every write into a merge
        // and leave deleted keys behind in the stored document.
        let response = self
            .send(Method::PATCH, &path, &precondition.query(), Some(body))
            .await?;
        classify(response).map(|outcome| precondition.normalize(outcome))
    }

    pub async fn delete_document(
        &self,
        collection: &str,
        id: &str,
        update_time: Option<&str>,
    ) -> Result<MutationOutcome, FirestoreError> {
        let path = self.document_path(collection, Some(id))?;
        // Firestore treats deleting an absent document as a success. An
        // existence precondition is added unconditionally so the caller can
        // still distinguish "I removed it" from "it was already gone" — the
        // cache's invalidation accounting depends on that distinction.
        let precondition = match update_time {
            Some(update_time) => Precondition::UpdateTime(update_time),
            None => Precondition::MustExist,
        };
        let response = self
            .send(Method::DELETE, &path, &precondition.query(), None)
            .await?;
        classify(response).map(|outcome| precondition.normalize(outcome))
    }

    /// Run one page of a structured query.
    ///
    /// `runQuery` has no page token, so the continuation is a cursor this client
    /// derives from the last document of the page. `__name__` is appended to the
    /// sort so the cursor is total and a document can never be returned twice
    /// or skipped between pages.
    pub async fn query_page<T: DeserializeOwned>(
        &self,
        collection: &str,
        filters: &[QueryFilter],
        order_by: &[QueryOrder],
        page_size: i32,
        continuation: Option<&str>,
    ) -> Result<QueryPage<Document<T>>, FirestoreError> {
        let collection = self.resolve_collection(collection)?;
        let page_size = page_size.clamp(1, MAX_PAGE_SIZE);
        let order_by = effective_order(order_by);

        let mut query = Map::new();
        query.insert(
            "from".to_string(),
            json!([{ "collectionId": collection, "allDescendants": false }]),
        );
        if let [only] = filters {
            query.insert("where".to_string(), only.to_wire()?);
        } else if !filters.is_empty() {
            let wire = filters
                .iter()
                .map(QueryFilter::to_wire)
                .collect::<Result<Vec<_>, _>>()?;
            query.insert(
                "where".to_string(),
                json!({ "compositeFilter": { "op": "AND", "filters": wire } }),
            );
        }
        query.insert(
            "orderBy".to_string(),
            Value::Array(
                order_by
                    .iter()
                    .map(|order| {
                        json!({
                            "field": { "fieldPath": order.field },
                            "direction": order.direction.wire_name(),
                        })
                    })
                    .collect(),
            ),
        );
        query.insert("limit".to_string(), json!(page_size));
        if let Some(continuation) = continuation {
            query.insert("startAt".to_string(), decode_cursor(continuation)?);
        }

        let body = serde_json::to_vec(&json!({ "structuredQuery": query }))
            .map_err(|error| FirestoreError::Serialization(error.to_string()))?;
        let path = format!("{}:runQuery", self.documents_root());
        let response = self.send(Method::POST, &path, &[], Some(body)).await?;
        if response.status != StatusCode::OK {
            return Err(service_error(response));
        }

        // The stream also carries bookkeeping entries that hold only a
        // `readTime` or a `skippedResults` count; they are not results and must
        // not be counted towards the page.
        let entries: Vec<RunQueryEntry> = deserialize(&response.body)?;
        let raw_documents: Vec<RawDocument> = entries
            .into_iter()
            .filter_map(|entry| entry.document)
            .collect();

        // A short page is the end of the result set. Emitting a cursor there
        // would make every caller pay one extra, always-empty round trip.
        let continuation = if raw_documents.len() as i32 == page_size {
            raw_documents
                .last()
                .map(|last| encode_cursor(last, &order_by))
                .transpose()?
        } else {
            None
        };

        let documents = raw_documents
            .into_iter()
            .map(document_from_raw)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(QueryPage {
            documents,
            continuation,
        })
    }

    /// Count the documents in a collection with a server-side `COUNT` aggregation,
    /// bounded by [`MAX_COUNT_DOCUMENTS`].
    ///
    /// Firestore exposes no collection metadata — no document count, and no storage
    /// size at all — so this is a query, not a free counter: it is billed one read per
    /// 1,000 documents matched. `Ok(None)` means the service answered without an
    /// aggregate, which is what an empty result stream looks like.
    pub async fn count_documents(&self, collection: &str) -> Result<Option<i64>, FirestoreError> {
        let collection = self.resolve_collection(collection)?;
        // `upTo` is an Int64Value, whose JSON form is a string.
        let body = serde_json::to_vec(&json!({
            "structuredAggregationQuery": {
                "structuredQuery": {
                    "from": [{ "collectionId": collection, "allDescendants": false }],
                },
                "aggregations": [{
                    "alias": COUNT_ALIAS,
                    "count": { "upTo": MAX_COUNT_DOCUMENTS.to_string() },
                }],
            }
        }))
        .map_err(|error| FirestoreError::Serialization(error.to_string()))?;

        let path = format!("{}:runAggregationQuery", self.documents_root());
        let response = self.send(Method::POST, &path, &[], Some(body)).await?;
        if response.status != StatusCode::OK {
            return Err(service_error(response));
        }

        // Like `runQuery`, the stream carries bookkeeping entries that hold only a
        // `readTime`; the aggregate arrives in the first entry that has a result.
        let entries: Vec<RunAggregationEntry> = deserialize(&response.body)?;
        let Some(value) = entries
            .into_iter()
            .filter_map(|entry| entry.result)
            .find_map(|result| result.aggregate_fields.get(COUNT_ALIAS).cloned())
        else {
            return Ok(None);
        };

        decode_value(&value)?.as_i64().map(Some).ok_or_else(|| {
            FirestoreError::Serialization(format!(
                "Firestore returned a non-integer '{COUNT_ALIAS}' aggregate: {value}"
            ))
        })
    }

    pub fn write_batch(&self) -> WriteBatch {
        WriteBatch {
            documents_root: self.documents_resource(),
            collection_prefix: self.collection_prefix.clone(),
            writes: Vec::new(),
            names: Vec::new(),
        }
    }

    /// Apply a batch atomically.
    ///
    /// This deliberately uses `:commit`, not `:batchWrite`. The two differ by
    /// one word in the docs and by everything in behaviour: `batchWrite` applies
    /// writes independently and reports per-write failures, so a half-applied
    /// state-machine transition would become possible.
    pub async fn commit(&self, batch: WriteBatch) -> Result<CommitOutcome, FirestoreError> {
        if batch.writes.is_empty() {
            return Ok(CommitOutcome {
                outcome: MutationOutcome::Applied,
                update_times: Vec::new(),
            });
        }
        if batch.writes.len() > MAX_WRITES_PER_COMMIT {
            return Err(FirestoreError::Configuration(format!(
                "a Firestore commit accepts at most {MAX_WRITES_PER_COMMIT} writes, got {}",
                batch.writes.len()
            )));
        }

        let body = serde_json::to_vec(&json!({ "writes": batch.writes }))
            .map_err(|error| FirestoreError::Serialization(error.to_string()))?;
        let path = format!("{}:commit", self.documents_root());
        let response = self.send(Method::POST, &path, &[], Some(body)).await?;

        if response.status == StatusCode::OK {
            let parsed: CommitResponse = deserialize(&response.body)?;
            return Ok(CommitOutcome {
                outcome: MutationOutcome::Applied,
                update_times: parsed
                    .write_results
                    .into_iter()
                    .map(|result| result.update_time)
                    .collect(),
            });
        }

        let outcome = mutation_outcome(&response);
        match outcome {
            Some(outcome) => Ok(CommitOutcome {
                outcome,
                update_times: Vec::new(),
            }),
            None => Err(service_error(response)),
        }
    }

    async fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        body: Option<Vec<u8>>,
    ) -> Result<RawResponse, FirestoreError> {
        let mut url = format!("{FIRESTORE_ENDPOINT}/{REST_API_VERSION}{path}");
        if !query.is_empty() {
            let encoded = query
                .iter()
                .map(|(name, value)| format!("{name}={}", urlencoding::encode(value)))
                .collect::<Vec<_>>()
                .join("&");
            url.push('?');
            url.push_str(&encoded);
        }

        for attempt in 0..=self.max_retries {
            let token = self.tokens.token().await?;
            let mut request = self
                .http
                .request(method.clone(), &url)
                .header(
                    reqwest::header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {}", token.secret())).map_err(
                        |error| {
                            FirestoreError::Authentication(format!(
                                "access token is not a valid header value: {error}"
                            ))
                        },
                    )?,
                )
                .header(reqwest::header::ACCEPT, "application/json");
            if let Some(body) = body.as_ref() {
                request = request
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body.clone());
            }

            let response = request
                .send()
                .await
                .map_err(|error| FirestoreError::Transport(error.to_string()))?;
            let status = response.status();
            let body_bytes = response
                .bytes()
                .await
                .map_err(|error| FirestoreError::Transport(error.to_string()))?;

            // 429/500/502/503/504 and a canonical ABORTED are the Firestore
            // answers that mean "not now". ABORTED needs the body, not the
            // status: it transcodes to HTTP 409 exactly like ALREADY_EXISTS,
            // but it is transient write contention on a hot document, not a
            // permanent state. Surfacing it as `Conflict` made `create_run`
            // report "already exists" and a heartbeat report a lost claim for
            // a document that was never touched, so it is retried here and
            // never mapped onto an outcome.
            //
            // Retrying a write on them is safe for the document itself because
            // every write in this client targets a caller-chosen ID and is
            // therefore idempotent. It is not free of consequence: if a `create`
            // was applied but its response was lost, the retry observes
            // ALREADY_EXISTS and reports `Conflict`. That is the conservative
            // direction for both callers — a cache insert declines, a lease
            // claim backs off — so the ambiguity fails towards doing less.
            if attempt < self.max_retries && is_retryable(status, &body_bytes) {
                let delay_ms = retry_delay_ms(&body_bytes)
                    .unwrap_or_else(|| jittered_delay_ms(attempt))
                    .min(MAX_RETRY_DELAY_MS);
                tracing::warn!(
                    attempt = attempt + 1,
                    delay_ms,
                    status = status.as_u16(),
                    "Firestore rejected a request as retryable; backing off"
                );
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                continue;
            }

            return Ok(RawResponse {
                status,
                body: body_bytes,
            });
        }

        unreachable!("retry loop always returns on its final attempt")
    }

    fn documents_resource(&self) -> String {
        format!(
            "projects/{}/databases/{}/documents",
            self.project_id, self.database
        )
    }

    /// The URL path form. The database ID is written verbatim rather than
    /// percent-encoded: the default database is literally `(default)`, and
    /// Firestore matches the parentheses, not their escapes. `validate_database_id`
    /// is what keeps that safe.
    fn documents_root(&self) -> String {
        format!(
            "/projects/{}/databases/{}/documents",
            urlencoding::encode(&self.project_id),
            self.database
        )
    }

    fn document_path(&self, collection: &str, id: Option<&str>) -> Result<String, FirestoreError> {
        let collection = self.resolve_collection(collection)?;
        let base = format!(
            "{}/{}",
            self.documents_root(),
            urlencoding::encode(&collection)
        );
        Ok(match id {
            Some(id) => {
                validate_document_id(id)?;
                format!("{base}/{}", urlencoding::encode(id))
            }
            None => base,
        })
    }

    fn resolve_collection(&self, collection: &str) -> Result<String, FirestoreError> {
        let resolved = format!("{}{collection}", self.collection_prefix);
        validate_collection_id(&resolved)?;
        Ok(resolved)
    }
}

/// Which precondition a write carries. Firestore expresses all three as query
/// parameters on the document URL.
#[derive(Clone, Copy)]
enum Precondition<'a> {
    None,
    MustExist,
    MustNotExist,
    UpdateTime(&'a str),
}

impl Precondition<'_> {
    /// Collapse the two canonical codes Firestore may use for the same refusal.
    ///
    /// An existence precondition can come back as either the specific code
    /// (`ALREADY_EXISTS`, `NOT_FOUND`) or the generic `FAILED_PRECONDITION`,
    /// depending on the method and the path through the service. Both mean the
    /// same thing when the precondition was existence, so they are folded here
    /// rather than left for `try_insert` to guess at. An `updateTime`
    /// precondition is left alone: there `FAILED_PRECONDITION` means "you lost
    /// the race" and `NOT_FOUND` means "it is gone", and the caller needs both.
    fn normalize(self, outcome: MutationOutcome) -> MutationOutcome {
        match (self, outcome) {
            (Self::MustNotExist, MutationOutcome::PreconditionFailed) => MutationOutcome::Conflict,
            (Self::MustExist, MutationOutcome::PreconditionFailed) => MutationOutcome::NotFound,
            (_, outcome) => outcome,
        }
    }

    fn query(self) -> Vec<(String, String)> {
        match self {
            Self::None => Vec::new(),
            Self::MustExist => vec![("currentDocument.exists".to_string(), "true".to_string())],
            Self::MustNotExist => vec![("currentDocument.exists".to_string(), "false".to_string())],
            Self::UpdateTime(update_time) => vec![(
                "currentDocument.updateTime".to_string(),
                update_time.to_string(),
            )],
        }
    }
}

/// A set of writes applied atomically by [`FirestoreClient::commit`].
pub struct WriteBatch {
    documents_root: String,
    collection_prefix: String,
    writes: Vec<Value>,
    names: Vec<String>,
}

impl WriteBatch {
    pub fn len(&self) -> usize {
        self.writes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }

    pub fn create<T: Serialize>(
        &mut self,
        collection: &str,
        id: &str,
        document: &T,
        expires_at_ms: Option<i64>,
    ) -> Result<&mut Self, FirestoreError> {
        self.update(
            collection,
            id,
            document,
            expires_at_ms,
            json!({ "exists": false }),
        )
    }

    pub fn upsert<T: Serialize>(
        &mut self,
        collection: &str,
        id: &str,
        document: &T,
        expires_at_ms: Option<i64>,
    ) -> Result<&mut Self, FirestoreError> {
        self.update(collection, id, document, expires_at_ms, Value::Null)
    }

    pub fn replace<T: Serialize>(
        &mut self,
        collection: &str,
        id: &str,
        document: &T,
        expires_at_ms: Option<i64>,
        update_time: Option<&str>,
    ) -> Result<&mut Self, FirestoreError> {
        let precondition = match update_time {
            Some(update_time) => json!({ "updateTime": update_time }),
            None => json!({ "exists": true }),
        };
        self.update(collection, id, document, expires_at_ms, precondition)
    }

    pub fn delete(
        &mut self,
        collection: &str,
        id: &str,
        update_time: Option<&str>,
    ) -> Result<&mut Self, FirestoreError> {
        let name = self.document_name(collection, id)?;
        let mut write = json!({ "delete": name });
        if let Some(update_time) = update_time {
            write["currentDocument"] = json!({ "updateTime": update_time });
        }
        self.push(name, write)
    }

    fn update<T: Serialize>(
        &mut self,
        collection: &str,
        id: &str,
        document: &T,
        expires_at_ms: Option<i64>,
        precondition: Value,
    ) -> Result<&mut Self, FirestoreError> {
        let name = self.document_name(collection, id)?;
        let mut fields = to_fields(document)?;
        set_expires_at(&mut fields, expires_at_ms)?;
        let mut write = json!({ "update": { "name": name, "fields": fields } });
        if !precondition.is_null() {
            write["currentDocument"] = precondition;
        }
        self.push(name, write)
    }

    fn push(&mut self, name: String, write: Value) -> Result<&mut Self, FirestoreError> {
        // Firestore rejects a commit that touches the same document twice, and
        // the rejection names neither document. Catching it here turns a
        // production INVALID_ARGUMENT into a message that says which key the
        // caller queued twice.
        if self.names.iter().any(|existing| existing == &name) {
            return Err(FirestoreError::Configuration(format!(
                "a Firestore commit may contain at most one write per document, but {name} \
                 appears twice"
            )));
        }
        if self.writes.len() >= MAX_WRITES_PER_COMMIT {
            return Err(FirestoreError::Configuration(format!(
                "a Firestore commit accepts at most {MAX_WRITES_PER_COMMIT} writes"
            )));
        }
        self.names.push(name);
        self.writes.push(write);
        Ok(self)
    }

    fn document_name(&self, collection: &str, id: &str) -> Result<String, FirestoreError> {
        let collection = format!("{}{collection}", self.collection_prefix);
        validate_collection_id(&collection)?;
        validate_document_id(id)?;
        // A resource name inside a JSON body is not a URL, so the segments stay
        // unescaped. `validate_document_id` is what guarantees no segment can
        // introduce another path component.
        Ok(format!("{}/{collection}/{id}", self.documents_root))
    }
}

impl Debug for WriteBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriteBatch")
            .field("writes", &self.writes.len())
            .finish_non_exhaustive()
    }
}

// ---------------------------------------------------------------------------
// Typed value encoding
//
// Firestore stores every field as a tagged union, and the tag is on the wire.
// A hand-rolled client gets this wrong in exactly four ways: it writes int64 as
// a JSON number and loses precision past 2^53, it assumes an empty array or map
// round-trips with its `values`/`fields` key present, it lets a nested array
// through and gets an opaque INVALID_ARGUMENT, and it writes a timestamp as a
// string so the TTL policy silently never fires. The pair below closes all four
// and is the most heavily tested code in this file for that reason.
// ---------------------------------------------------------------------------

/// Encode a serializable document into a Firestore `fields` map.
pub fn to_fields<T: Serialize>(document: &T) -> Result<Value, FirestoreError> {
    let json = serde_json::to_value(document)
        .map_err(|error| FirestoreError::Serialization(error.to_string()))?;
    let Value::Object(map) = json else {
        return Err(FirestoreError::Serialization(
            "a Firestore document must serialize to a JSON object".to_string(),
        ));
    };

    let mut fields = Map::with_capacity(map.len());
    for (key, value) in map {
        validate_field_name(&key)?;
        fields.insert(key, encode_value(&value, false)?);
    }
    Ok(Value::Object(fields))
}

/// Decode a Firestore `fields` map back into a caller type.
pub fn from_fields<T: DeserializeOwned>(fields: &Value) -> Result<T, FirestoreError> {
    let json = fields_to_json(fields)?;
    serde_json::from_value(json).map_err(|error| FirestoreError::Serialization(error.to_string()))
}

fn fields_to_json(fields: &Value) -> Result<Value, FirestoreError> {
    let Value::Object(map) = fields else {
        return Err(FirestoreError::Serialization(
            "Firestore returned a fields map that was not an object".to_string(),
        ));
    };
    let mut decoded = Map::with_capacity(map.len());
    for (key, value) in map {
        decoded.insert(key.clone(), decode_value(value)?);
    }
    Ok(Value::Object(decoded))
}

/// Encode one JSON value as a Firestore `Value`.
pub fn encode_value(value: &Value, inside_array: bool) -> Result<Value, FirestoreError> {
    Ok(match value {
        Value::Null => json!({ "nullValue": null }),
        Value::Bool(value) => json!({ "booleanValue": value }),
        Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                // int64 travels as a *string*. A JSON number would be parsed as
                // a double by any conforming reader and would quietly round any
                // value above 2^53 — which is where millisecond timestamps and
                // monotonic sequence numbers live.
                json!({ "integerValue": value.to_string() })
            } else if let Some(value) = number.as_f64() {
                json!({ "doubleValue": value })
            } else {
                return Err(FirestoreError::Serialization(format!(
                    "{number} exceeds the int64 range Firestore stores"
                )));
            }
        }
        Value::String(value) => json!({ "stringValue": value }),
        Value::Array(items) => {
            // An array value cannot *directly* contain another array. Rejecting
            // here names the value that is wrong; letting it through produces an
            // INVALID_ARGUMENT that names nothing.
            if inside_array {
                return Err(FirestoreError::Serialization(
                    "Firestore cannot store an array directly inside an array".to_string(),
                ));
            }
            let values = items
                .iter()
                .map(|item| encode_value(item, true))
                .collect::<Result<Vec<_>, _>>()?;
            json!({ "arrayValue": { "values": values } })
        }
        Value::Object(map) => {
            let mut fields = Map::with_capacity(map.len());
            for (key, value) in map {
                validate_field_name(key)?;
                // The restriction is on *direct* nesting only: a map inside an
                // array may hold arrays of its own, so the flag resets here.
                fields.insert(key.clone(), encode_value(value, false)?);
            }
            json!({ "mapValue": { "fields": fields } })
        }
    })
}

/// Decode one Firestore `Value` into plain JSON.
pub fn decode_value(value: &Value) -> Result<Value, FirestoreError> {
    let Value::Object(map) = value else {
        return Err(FirestoreError::Serialization(
            "a Firestore value must be a tagged object".to_string(),
        ));
    };
    let Some((tag, inner)) = map.iter().next() else {
        return Err(FirestoreError::Serialization(
            "a Firestore value carried no type tag".to_string(),
        ));
    };

    Ok(match tag.as_str() {
        "nullValue" => Value::Null,
        "booleanValue" => Value::Bool(inner.as_bool().ok_or_else(|| tag_error(tag))?),
        "integerValue" => {
            // Accepts both encodings: Firestore always emits a string, but a
            // fixture or a proxy may hand back a number.
            let parsed = match inner {
                Value::String(text) => text.parse::<i64>().ok(),
                Value::Number(number) => number.as_i64(),
                _ => None,
            };
            Value::Number(parsed.ok_or_else(|| tag_error(tag))?.into())
        }
        // Firestore expresses the non-finite doubles as the strings "NaN",
        // "Infinity" and "-Infinity". JSON has no representation for them, so
        // they surface as an error rather than as a silently wrong number.
        "doubleValue" => {
            let parsed = match inner {
                Value::Number(number) => number.as_f64(),
                Value::String(text) => text.parse::<f64>().ok(),
                _ => None,
            }
            .and_then(serde_json::Number::from_f64)
            .ok_or_else(|| tag_error(tag))?;
            Value::Number(parsed)
        }
        // A timestamp, a reference and a byte string all decode to the string
        // form Firestore transmits (RFC 3339, a resource path, and standard
        // base64). Re-encoding is asymmetric on purpose: nothing in a JSON
        // string says which of the three it was, so the typed constructors below
        // are the only way to write one back.
        "timestampValue" | "stringValue" | "referenceValue" | "bytesValue" => {
            Value::String(inner.as_str().ok_or_else(|| tag_error(tag))?.to_string())
        }
        "geoPointValue" => inner.clone(),
        "arrayValue" => {
            // An empty array arrives as `{"arrayValue": {}}` with no `values`
            // key at all.
            let items = inner
                .get("values")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            Value::Array(
                items
                    .iter()
                    .map(decode_value)
                    .collect::<Result<Vec<_>, _>>()?,
            )
        }
        "mapValue" => match inner.get("fields") {
            // Likewise, an empty map arrives as `{"mapValue": {}}`.
            Some(fields) => fields_to_json(fields)?,
            None => Value::Object(Map::new()),
        },
        other => {
            return Err(FirestoreError::Serialization(format!(
                "unsupported Firestore value type '{other}'"
            )));
        }
    })
}

/// Build a `timestampValue` from epoch milliseconds.
pub fn timestamp_value(epoch_millis: i64) -> Result<Value, FirestoreError> {
    let timestamp = chrono::DateTime::from_timestamp_millis(epoch_millis).ok_or_else(|| {
        FirestoreError::Serialization(format!(
            "{epoch_millis} is not a representable Firestore timestamp"
        ))
    })?;
    Ok(json!({
        "timestampValue": timestamp.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }))
}

/// Build a `bytesValue`. Firestore expects standard base64 with padding.
pub fn bytes_value(bytes: &[u8]) -> Value {
    json!({ "bytesValue": base64::engine::general_purpose::STANDARD.encode(bytes) })
}

/// Write (or clear) the TTL field on an encoded `fields` map.
///
/// The Firestore TTL policy is configured in Terraform against one named field
/// and only acts on a `timestampValue`. A string in that field is not an error,
/// it is simply never collected, so every expiring document has to come through
/// here rather than through the document's own `serde` shape.
///
/// [`EXPIRES_AT_FIELD`] is a sibling of, not a replacement for, whatever integer
/// `expires_at` the document serialized: the record's own field is left untouched
/// and stays the value readers compare against, this one exists only for the
/// policy. Callers must pass the same milliseconds the record carries, so the pair
/// cannot drift — the two are written in one mutation from one value.
pub fn set_expires_at(
    fields: &mut Value,
    expires_at_ms: Option<i64>,
) -> Result<(), FirestoreError> {
    let Value::Object(map) = fields else {
        return Err(FirestoreError::Serialization(
            "a Firestore fields map must be an object".to_string(),
        ));
    };
    match expires_at_ms {
        Some(expires_at_ms) => {
            map.insert(
                EXPIRES_AT_FIELD.to_string(),
                timestamp_value(expires_at_ms)?,
            );
        }
        None => {
            map.remove(EXPIRES_AT_FIELD);
        }
    }
    Ok(())
}

fn tag_error(tag: &str) -> FirestoreError {
    FirestoreError::Serialization(format!("Firestore {tag} had an unusable payload"))
}

// ---------------------------------------------------------------------------
// Cursors
// ---------------------------------------------------------------------------

fn effective_order(order_by: &[QueryOrder]) -> Vec<QueryOrder> {
    let mut order: Vec<QueryOrder> = order_by.to_vec();
    // Firestore appends `__name__` itself, taking the direction of the last
    // explicit sort key. Appending it here makes the same thing happen, but
    // visibly, so the cursor values line up with the server's sort.
    if !order.iter().any(|entry| entry.field == DOCUMENT_NAME_FIELD) {
        let direction = order
            .last()
            .map(|entry| entry.direction)
            .unwrap_or(SortDirection::Ascending);
        order.push(QueryOrder::new(DOCUMENT_NAME_FIELD, direction));
    }
    order
}

fn encode_cursor(
    document: &RawDocument,
    order_by: &[QueryOrder],
) -> Result<String, FirestoreError> {
    let mut values = Vec::with_capacity(order_by.len());
    for order in order_by {
        if order.field == DOCUMENT_NAME_FIELD {
            values.push(json!({ "referenceValue": document.name }));
            continue;
        }
        let value = document.fields.get(&order.field).ok_or_else(|| {
            FirestoreError::Serialization(format!(
                "document {} has no '{}' field to continue the sort from",
                document.name, order.field
            ))
        })?;
        values.push(value.clone());
    }

    // `before: false` means "resume just after this position", which is what
    // makes the next page start at the document following the one encoded here.
    let cursor = json!({ "values": values, "before": false });
    let bytes = serde_json::to_vec(&cursor)
        .map_err(|error| FirestoreError::Serialization(error.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn decode_cursor(continuation: &str) -> Result<Value, FirestoreError> {
    let bytes = URL_SAFE_NO_PAD.decode(continuation).map_err(|error| {
        FirestoreError::Serialization(format!("continuation token is not valid base64: {error}"))
    })?;
    let cursor: Value = serde_json::from_slice(&bytes).map_err(|error| {
        FirestoreError::Serialization(format!("continuation token is not a cursor: {error}"))
    })?;
    if !cursor.get("values").is_some_and(Value::is_array) {
        return Err(FirestoreError::Serialization(
            "continuation token carried no cursor values".to_string(),
        ));
    }
    Ok(cursor)
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Reject a collection ID that could escape its collection.
///
/// Collection IDs reach this client from deployment configuration and from the
/// collection-prefix environment variable, so the check is strict on charset as
/// well as on structure: `/` would append a path segment, `..` would walk up
/// one, and `__x__` is reserved by Firestore for its own internal collections.
pub fn validate_collection_id(value: &str) -> Result<(), FirestoreError> {
    if value.is_empty() {
        return Err(FirestoreError::Configuration(
            "Firestore collection ID must not be empty".to_string(),
        ));
    }
    if value.len() > MAX_ID_BYTES {
        return Err(FirestoreError::Configuration(format!(
            "Firestore collection ID exceeds {MAX_ID_BYTES} bytes"
        )));
    }
    if value == "." || value == ".." || is_reserved_id(value) {
        return Err(FirestoreError::Configuration(format!(
            "'{value}' is a reserved Firestore collection ID"
        )));
    }
    let first_is_alphanumeric = value
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric());
    let charset_is_safe = value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'));
    if !first_is_alphanumeric || !charset_is_safe {
        return Err(FirestoreError::Configuration(format!(
            "'{value}' is not a valid Firestore collection ID; expected alphanumerics, '-', '_' \
             and '.' starting with an alphanumeric"
        )));
    }
    Ok(())
}

/// Document IDs carry caller data — a cache key is `<app id>:<entry key>` — so
/// the charset stays open. Only the characters that change the meaning of a path
/// are refused, plus the IDs Firestore reserves.
fn validate_document_id(id: &str) -> Result<(), FirestoreError> {
    if id.is_empty() {
        return Err(FirestoreError::Configuration(
            "Firestore document ID must not be empty".to_string(),
        ));
    }
    if id.len() > MAX_ID_BYTES {
        return Err(FirestoreError::Configuration(format!(
            "Firestore document ID exceeds {MAX_ID_BYTES} bytes"
        )));
    }
    if id == "." || id == ".." || is_reserved_id(id) {
        return Err(FirestoreError::Configuration(format!(
            "'{id}' is a reserved Firestore document ID"
        )));
    }
    if id
        .chars()
        .any(|character| matches!(character, '/' | '\\') || character.is_control())
    {
        return Err(FirestoreError::Configuration(
            "Firestore document ID must not contain a path separator or a control character"
                .to_string(),
        ));
    }
    Ok(())
}

fn is_reserved_id(value: &str) -> bool {
    value.len() >= 4 && value.starts_with("__") && value.ends_with("__")
}

/// Field names are written into a `fields` map, where a dot or a backtick would
/// be read as a path expression by every query that later touches the document.
fn validate_field_name(name: &str) -> Result<(), FirestoreError> {
    if name.is_empty() {
        return Err(FirestoreError::Serialization(
            "a Firestore field name must not be empty".to_string(),
        ));
    }
    if name.contains('.') || name.contains('`') || name.contains('/') {
        return Err(FirestoreError::Serialization(format!(
            "field name '{name}' contains a character Firestore reads as a field path"
        )));
    }
    Ok(())
}

fn validate_field_path(path: &str) -> Result<(), FirestoreError> {
    if path.is_empty() {
        return Err(FirestoreError::Configuration(
            "a Firestore field path must not be empty".to_string(),
        ));
    }
    if path != DOCUMENT_NAME_FIELD
        && path
            .split('.')
            .any(|segment| segment.is_empty() || validate_field_name(segment).is_err())
    {
        return Err(FirestoreError::Configuration(format!(
            "'{path}' is not a valid Firestore field path"
        )));
    }
    Ok(())
}

fn validate_project_id(value: &str) -> Result<(), FirestoreError> {
    let valid = (6..=30).contains(&value.len())
        && value.starts_with(|character: char| character.is_ascii_lowercase())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.ends_with('-');
    if !valid {
        return Err(FirestoreError::Configuration(format!(
            "'{value}' is not a valid GCP project ID"
        )));
    }
    Ok(())
}

/// The database ID is interpolated into the URL without escaping, so it is
/// checked against the two shapes Firestore actually allows.
fn validate_database_id(value: &str) -> Result<(), FirestoreError> {
    if value == DEFAULT_DATABASE {
        return Ok(());
    }
    let valid = (4..=63).contains(&value.len())
        && value.starts_with(|character: char| character.is_ascii_lowercase())
        && value.ends_with(|character: char| character.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid {
        return Err(FirestoreError::Configuration(format!(
            "'{value}' is not a valid Firestore database ID; expected '{DEFAULT_DATABASE}' or a \
             lowercase name of 4 to 63 characters"
        )));
    }
    Ok(())
}

fn validate_collection_prefix(value: &str) -> Result<(), FirestoreError> {
    if value.is_empty() {
        return Ok(());
    }
    let valid = value.len() <= 64
        && value.starts_with(|character: char| character.is_ascii_alphanumeric())
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte == b'.'
        });
    if !valid {
        return Err(FirestoreError::Configuration(format!(
            "'{value}' is not a valid FIRESTORE_COLLECTION_PREFIX"
        )));
    }
    Ok(())
}

fn required_env(name: &str) -> Result<String, FirestoreError> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| FirestoreError::Configuration(format!("{name} is required")))
}

// ---------------------------------------------------------------------------
// Response handling
// ---------------------------------------------------------------------------

fn document_from_raw<T: DeserializeOwned>(raw: RawDocument) -> Result<Document<T>, FirestoreError> {
    // A resource name is not a URL: Firestore returns the document ID exactly as
    // it was stored. Percent-decoding it here would silently rewrite any ID that
    // legitimately contains a '%'.
    let id = raw.name.rsplit('/').next().unwrap_or_default().to_string();
    let value = from_fields(&Value::Object(raw.fields))?;
    Ok(Document {
        id,
        create_time: raw.create_time,
        update_time: raw.update_time,
        value,
    })
}

/// Turn a mutation response into an outcome, or into an error when it is not one
/// of the four states a caller is expected to handle.
fn classify(response: RawResponse) -> Result<MutationOutcome, FirestoreError> {
    if response.status == StatusCode::OK {
        return Ok(MutationOutcome::Applied);
    }
    let outcome = mutation_outcome(&response);
    match outcome {
        Some(outcome) => Ok(outcome),
        None => Err(service_error(response)),
    }
}

/// Transient answers worth another attempt. The canonical status is consulted
/// because ABORTED shares HTTP 409 with ALREADY_EXISTS; only the body tells a
/// contention abort from a genuine ID collision, and only the former is retried.
fn is_retryable(status: StatusCode, body: &[u8]) -> bool {
    matches!(
        status,
        StatusCode::TOO_MANY_REQUESTS
            | StatusCode::INTERNAL_SERVER_ERROR
            | StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT
    ) || google_status(body).as_deref() == Some("ABORTED")
}

/// Map a failed mutation onto a `MutationOutcome`.
///
/// The HTTP status alone cannot do this. Firestore's canonical codes transcode
/// so that both a malformed request and a lost `updateTime` race arrive as HTTP
/// 400, and both an ID collision and a contention abort arrive as HTTP 409. The
/// `error.status` string is the only field that separates them, so it is read
/// first and the HTTP code is only a fallback.
///
/// ABORTED is deliberately not an outcome. It is transient contention, retried
/// by `send`; if it survives every attempt it is a service error, because
/// telling a caller "the document already exists" or "you lost the lease" about
/// a write that was never applied is worse than telling it Firestore is busy.
/// `ALREADY_EXISTS` is therefore the only canonical source of `Conflict`.
///
/// This is applied to mutations only. `FAILED_PRECONDITION` also means "this
/// query needs an index you have not created", and on a read path that is a
/// deployment fault that must surface as an error rather than as a quiet
/// "somebody else won".
fn mutation_outcome(response: &RawResponse) -> Option<MutationOutcome> {
    let canonical = google_status(&response.body);
    match canonical.as_deref() {
        Some("NOT_FOUND") => return Some(MutationOutcome::NotFound),
        Some("ALREADY_EXISTS") => return Some(MutationOutcome::Conflict),
        Some("FAILED_PRECONDITION") => return Some(MutationOutcome::PreconditionFailed),
        Some(_) => return None,
        None => {}
    }

    match response.status {
        StatusCode::NOT_FOUND => Some(MutationOutcome::NotFound),
        StatusCode::CONFLICT => Some(MutationOutcome::Conflict),
        StatusCode::PRECONDITION_FAILED => Some(MutationOutcome::PreconditionFailed),
        _ => None,
    }
}

fn error_object(body: &[u8]) -> Option<Value> {
    let body = &body[..body.len().min(MAX_ERROR_BODY_BYTES)];
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| value.get("error").cloned())
}

fn google_status(body: &[u8]) -> Option<String> {
    error_object(body)?
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// Honour a server-supplied `RetryInfo` when there is one. Guessing at the
/// backoff for a hot Firestore key is strictly worse than being told.
fn retry_delay_ms(body: &[u8]) -> Option<u64> {
    let details = error_object(body)?.get("details")?.as_array()?.clone();
    for detail in details {
        if detail.get("@type").and_then(Value::as_str)
            != Some("type.googleapis.com/google.rpc.RetryInfo")
        {
            continue;
        }
        let delay = detail.get("retryDelay")?.as_str()?;
        let seconds = delay.trim_end_matches('s').parse::<f64>().ok()?;
        if seconds.is_finite() && seconds >= 0.0 {
            return Some((seconds * 1_000.0) as u64);
        }
    }
    None
}

/// Full jitter over an exponential ceiling. Replicas that back off in lockstep
/// turn one hot Firestore key into a self-sustaining retry storm, and the UUID
/// generator is already a dependency, so no RNG crate is pulled in for it.
fn jittered_delay_ms(attempt: u32) -> u64 {
    let ceiling = 100_u64
        .saturating_mul(1_u64 << attempt.min(8))
        .min(MAX_RETRY_DELAY_MS);
    let entropy = (uuid::Uuid::new_v4().as_u128() % u128::from(ceiling)) as u64;
    entropy.max(1)
}

fn deserialize<T: DeserializeOwned>(body: &[u8]) -> Result<T, FirestoreError> {
    serde_json::from_slice(body).map_err(|error| FirestoreError::Serialization(error.to_string()))
}

fn service_error(response: RawResponse) -> FirestoreError {
    let error = error_object(&response.body);
    let code = error
        .as_ref()
        .and_then(|error| error.get("status"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let reason = error
        .as_ref()
        .and_then(|error| error.get("details"))
        .and_then(Value::as_array)
        .and_then(|details| {
            details
                .iter()
                .find_map(|detail| detail.get("reason").and_then(Value::as_str))
        })
        .map(str::to_owned);
    let message = error
        .as_ref()
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let body = &response.body[..response.body.len().min(MAX_ERROR_BODY_BYTES)];
            String::from_utf8_lossy(body).into_owned()
        });

    FirestoreError::Service {
        status: response.status,
        code,
        reason,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Sample {
        app_id: String,
        sequence: i64,
        ratio: f64,
        active: bool,
        tags: Vec<String>,
        nested: Nested,
        absent: Option<String>,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Nested {
        label: String,
        counters: Vec<i64>,
    }

    fn sample() -> Sample {
        Sample {
            app_id: "app-1".to_string(),
            // Above 2^53: this is the value that a JSON-number encoding loses.
            sequence: 9_007_199_254_740_993,
            ratio: 0.5,
            active: true,
            tags: vec!["a".to_string(), "b".to_string()],
            nested: Nested {
                label: "inner".to_string(),
                counters: vec![1, 2, 3],
            },
            absent: None,
        }
    }

    #[test]
    fn typed_values_round_trip_without_losing_precision() {
        let fields = to_fields(&sample()).expect("sample must encode");
        assert_eq!(
            fields["sequence"],
            json!({ "integerValue": "9007199254740993" })
        );
        assert_eq!(fields["ratio"], json!({ "doubleValue": 0.5 }));
        assert_eq!(fields["active"], json!({ "booleanValue": true }));
        assert_eq!(fields["absent"], json!({ "nullValue": null }));
        assert_eq!(
            fields["tags"],
            json!({ "arrayValue": { "values": [
                { "stringValue": "a" },
                { "stringValue": "b" }
            ] } })
        );

        let decoded: Sample = from_fields(&fields).expect("sample must decode");
        assert_eq!(decoded, sample());
    }

    #[test]
    fn empty_arrays_and_maps_decode_from_their_omitted_wire_form() {
        let fields = json!({
            "tags": { "arrayValue": {} },
            "nested": { "mapValue": {} },
        });
        let decoded = fields_to_json(&fields).expect("omitted keys must decode");
        assert_eq!(decoded["tags"], json!([]));
        assert_eq!(decoded["nested"], json!({}));
    }

    #[test]
    fn directly_nested_arrays_are_refused_before_they_reach_firestore() {
        assert!(matches!(
            encode_value(&json!([[1, 2]]), false),
            Err(FirestoreError::Serialization(_))
        ));
        // Firestore forbids only *direct* nesting: a map inside an array may
        // hold arrays of its own, and rejecting that would refuse documents the
        // service accepts.
        assert!(encode_value(&json!([{ "a": 1 }]), false).is_ok());
        assert!(encode_value(&json!([{ "a": [1] }]), false).is_ok());
        assert!(encode_value(&json!([{ "a": [[1]] }]), false).is_err());
    }

    #[test]
    fn integers_decode_from_the_string_form_firestore_emits() {
        let decoded = decode_value(&json!({ "integerValue": "-42" })).expect("string int decodes");
        assert_eq!(decoded, json!(-42));
        let decoded = decode_value(&json!({ "integerValue": 7 })).expect("numeric int decodes");
        assert_eq!(decoded, json!(7));
    }

    #[test]
    fn timestamps_and_bytes_decode_to_their_transmitted_strings() {
        assert_eq!(
            decode_value(&json!({ "timestampValue": "2026-08-15T10:11:12.500Z" }))
                .expect("timestamp decodes"),
            json!("2026-08-15T10:11:12.500Z")
        );
        assert_eq!(
            decode_value(&bytes_value(b"hi")).expect("bytes decode"),
            json!("aGk=")
        );
    }

    #[test]
    fn ttl_is_written_as_a_timestamp_and_cleared_on_none() {
        let mut fields = to_fields(&sample()).expect("sample must encode");
        set_expires_at(&mut fields, Some(1_771_150_272_500)).expect("ttl must be writable");
        assert_eq!(
            fields[EXPIRES_AT_FIELD],
            json!({ "timestampValue": "2026-02-15T10:11:12.500Z" })
        );

        set_expires_at(&mut fields, None).expect("ttl must be clearable");
        assert!(fields.get(EXPIRES_AT_FIELD).is_none());
    }

    /// Mirrors `local.ttl_field` in `modules/gcp/state/main.tf` and the `ttl_fields`
    /// output. Firestore collects nothing when the writer's name and the policy's
    /// differ, so the literal is asserted here rather than left to the constant.
    #[test]
    fn ttl_field_name_matches_the_terraform_policy_and_spares_the_record_integer() {
        assert_eq!(EXPIRES_AT_FIELD, "expires_at_ts");

        let mut fields = to_fields(&json!({ "id": "run-1", "expires_at": 1_771_150_272_500_i64 }))
            .expect("record must encode");
        set_expires_at(&mut fields, Some(1_771_150_272_500)).expect("ttl must be writable");
        assert_eq!(
            fields["expires_at"],
            json!({ "integerValue": "1771150272500" }),
            "the record's epoch-milliseconds integer is a sibling the TTL write must not touch"
        );
        assert_eq!(
            fields["expires_at_ts"],
            json!({ "timestampValue": "2026-02-15T10:11:12.500Z" })
        );
    }

    #[test]
    fn field_names_that_read_as_paths_are_refused() {
        let value = json!({ "a.b": 1 });
        assert!(encode_value(&value, false).is_err());
    }

    #[test]
    fn collection_ids_reject_traversal_and_reserved_names() {
        for id in ["a/b", "..", ".", "__internal__", "-leading", "a b", ""] {
            assert!(
                validate_collection_id(id).is_err(),
                "collection should be rejected: {id:?}"
            );
        }
        assert!(validate_collection_id("execution-runs").is_ok());
        assert!(validate_collection_id("flowlike.cache_v2").is_ok());
    }

    #[test]
    fn document_ids_allow_cache_keys_but_not_path_separators() {
        for id in ["a/b", "a\\b", "..", "__name__", ""] {
            assert!(
                validate_document_id(id).is_err(),
                "document ID should be rejected: {id:?}"
            );
        }
        // Cache keys are caller data and routinely carry punctuation. They are
        // percent-encoded into the URL and JSON-escaped into a commit body, so
        // only genuine separators need to be refused.
        assert!(validate_document_id("app-1:entry#42?x").is_ok());
        assert!(validate_document_id("blake3:9f8e..7a").is_ok());
    }

    #[test]
    fn database_ids_accept_the_default_and_reject_url_bending_values() {
        assert!(validate_database_id("(default)").is_ok());
        assert!(validate_database_id("flowlike-state").is_ok());
        for id in ["(other)", "../admin", "Flowlike", "ab", "trailing-"] {
            assert!(
                validate_database_id(id).is_err(),
                "database should be rejected: {id:?}"
            );
        }
    }

    #[test]
    fn queries_are_always_totally_ordered() {
        let order = effective_order(&[QueryOrder::new("createdAt", SortDirection::Descending)]);
        assert_eq!(order.len(), 2);
        assert_eq!(order[1].field, DOCUMENT_NAME_FIELD);
        // The implicit key follows the direction of the last explicit key, the
        // same rule the server applies.
        assert_eq!(order[1].direction, SortDirection::Descending);

        let order = effective_order(&[]);
        assert_eq!(order[0].field, DOCUMENT_NAME_FIELD);
        assert_eq!(order[0].direction, SortDirection::Ascending);
    }

    #[test]
    fn cursors_resume_after_the_last_document_of_the_page() {
        let raw = RawDocument {
            name: "projects/p/databases/(default)/documents/runs/run-9".to_string(),
            fields: match json!({ "createdAt": { "integerValue": "17" } }) {
                Value::Object(map) => map,
                _ => unreachable!(),
            },
            create_time: None,
            update_time: None,
        };
        let order = effective_order(&[QueryOrder::new("createdAt", SortDirection::Descending)]);
        let token = encode_cursor(&raw, &order).expect("cursor must encode");
        let cursor = decode_cursor(&token).expect("cursor must decode");

        assert_eq!(cursor["before"], json!(false));
        assert_eq!(cursor["values"][0], json!({ "integerValue": "17" }));
        assert_eq!(cursor["values"][1], json!({ "referenceValue": raw.name }));
    }

    #[test]
    fn cursors_reject_tampered_tokens() {
        assert!(decode_cursor("not base64!!").is_err());
        assert!(decode_cursor(&URL_SAFE_NO_PAD.encode(b"{\"before\":false}")).is_err());
    }

    #[test]
    fn null_comparisons_become_unary_filters() {
        let filter = QueryFilter::new("claimed_by", FilterOperator::Equal, Value::Null);
        let wire = filter.to_wire().expect("null equality must encode");
        assert_eq!(wire["unaryFilter"]["op"], json!("IS_NULL"));

        let filter = QueryFilter::new("claimed_by", FilterOperator::LessThan, Value::Null);
        assert!(filter.to_wire().is_err());
    }

    #[test]
    fn membership_operators_require_an_array_operand() {
        let filter = QueryFilter::new("status", FilterOperator::In, json!("running"));
        assert!(filter.to_wire().is_err());
        let filter = QueryFilter::new("status", FilterOperator::In, json!(["running", "queued"]));
        assert!(filter.to_wire().is_ok());
    }

    #[test]
    fn timestamp_filters_carry_a_timestamp_value() {
        let filter = QueryFilter::timestamp(
            EXPIRES_AT_FIELD,
            FilterOperator::LessThan,
            1_771_150_272_500,
        )
        .expect("timestamp filter must build");
        let wire = filter.to_wire().expect("timestamp filter must encode");
        assert_eq!(
            wire["fieldFilter"]["value"],
            json!({ "timestampValue": "2026-02-15T10:11:12.500Z" })
        );
    }

    #[test]
    fn precondition_failures_are_classified_by_canonical_status_not_http_code() {
        let body = br#"{"error":{"code":400,"status":"FAILED_PRECONDITION","message":"stale"}}"#;
        let response = RawResponse {
            status: StatusCode::BAD_REQUEST,
            body: bytes::Bytes::from_static(body),
        };
        assert_eq!(
            mutation_outcome(&response),
            Some(MutationOutcome::PreconditionFailed)
        );

        let body = br#"{"error":{"code":409,"status":"ALREADY_EXISTS","message":"exists"}}"#;
        let response = RawResponse {
            status: StatusCode::CONFLICT,
            body: bytes::Bytes::from_static(body),
        };
        assert_eq!(mutation_outcome(&response), Some(MutationOutcome::Conflict));

        // A genuinely malformed request must not be mistaken for a lost race.
        let body = br#"{"error":{"code":400,"status":"INVALID_ARGUMENT","message":"bad"}}"#;
        let response = RawResponse {
            status: StatusCode::BAD_REQUEST,
            body: bytes::Bytes::from_static(body),
        };
        assert_eq!(mutation_outcome(&response), None);
    }

    #[test]
    fn contention_aborts_are_retried_not_reported_as_conflicts() {
        let aborted = br#"{"error":{"code":409,"status":"ABORTED","message":"contention"}}"#;
        assert!(is_retryable(StatusCode::CONFLICT, aborted));
        let response = RawResponse {
            status: StatusCode::CONFLICT,
            body: bytes::Bytes::from_static(aborted),
        };
        assert_eq!(mutation_outcome(&response), None);

        let exists = br#"{"error":{"code":409,"status":"ALREADY_EXISTS","message":"exists"}}"#;
        assert!(!is_retryable(StatusCode::CONFLICT, exists));

        assert!(is_retryable(StatusCode::BAD_GATEWAY, b""));
        assert!(is_retryable(StatusCode::GATEWAY_TIMEOUT, b""));
        assert!(!is_retryable(StatusCode::BAD_REQUEST, b""));
    }

    #[test]
    fn server_supplied_retry_delays_win_over_local_backoff() {
        let body = br#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[
            {"@type":"type.googleapis.com/google.rpc.RetryInfo","retryDelay":"1.5s"}]}}"#;
        assert_eq!(retry_delay_ms(body), Some(1_500));
        assert_eq!(retry_delay_ms(b"{}"), None);
    }

    #[test]
    fn backoff_stays_inside_its_ceiling() {
        for attempt in 0..12 {
            let delay = jittered_delay_ms(attempt);
            assert!(delay >= 1);
            assert!(delay <= MAX_RETRY_DELAY_MS);
        }
    }

    #[test]
    fn batches_refuse_two_writes_to_one_document() {
        let mut batch = WriteBatch {
            documents_root: "projects/p/databases/(default)/documents".to_string(),
            collection_prefix: "flowlike-".to_string(),
            writes: Vec::new(),
            names: Vec::new(),
        };
        batch
            .upsert("cache", "app-1:key", &sample(), Some(1_771_150_272_500))
            .expect("first write is accepted");
        assert!(batch.delete("cache", "app-1:key", None).is_err());
        assert_eq!(batch.len(), 1);
        assert_eq!(
            batch.writes[0]["update"]["name"],
            json!("projects/p/databases/(default)/documents/flowlike-cache/app-1:key")
        );
        assert_eq!(
            batch.writes[0]["update"]["fields"][EXPIRES_AT_FIELD],
            json!({ "timestampValue": "2026-02-15T10:11:12.500Z" })
        );
    }

    #[test]
    fn batch_creates_carry_the_non_existence_precondition() {
        let mut batch = WriteBatch {
            documents_root: "projects/p/databases/(default)/documents".to_string(),
            collection_prefix: String::new(),
            writes: Vec::new(),
            names: Vec::new(),
        };
        batch
            .create("cache", "app-1:key", &sample(), None)
            .expect("create is accepted");
        assert_eq!(
            batch.writes[0]["currentDocument"],
            json!({ "exists": false })
        );
    }

    #[test]
    fn existence_preconditions_fold_the_generic_failure_code() {
        assert_eq!(
            Precondition::MustNotExist.normalize(MutationOutcome::PreconditionFailed),
            MutationOutcome::Conflict
        );
        assert_eq!(
            Precondition::MustExist.normalize(MutationOutcome::PreconditionFailed),
            MutationOutcome::NotFound
        );
        // An updateTime race keeps its own identity: a compare-and-swap caller
        // has to tell "someone else wrote" apart from "it was deleted".
        assert_eq!(
            Precondition::UpdateTime("t").normalize(MutationOutcome::PreconditionFailed),
            MutationOutcome::PreconditionFailed
        );
        assert_eq!(
            Precondition::UpdateTime("t").normalize(MutationOutcome::NotFound),
            MutationOutcome::NotFound
        );
    }

    #[test]
    fn preconditions_render_as_document_query_parameters() {
        assert!(Precondition::None.query().is_empty());
        assert_eq!(
            Precondition::MustNotExist.query(),
            vec![("currentDocument.exists".to_string(), "false".to_string())]
        );
        assert_eq!(
            Precondition::UpdateTime("2026-08-15T10:11:12.345678Z").query(),
            vec![(
                "currentDocument.updateTime".to_string(),
                "2026-08-15T10:11:12.345678Z".to_string()
            )]
        );
    }

    #[test]
    fn run_query_bookkeeping_entries_are_not_results() {
        let body = br#"[{"readTime":"2026-08-15T10:11:12Z"},
            {"document":{"name":"projects/p/databases/(default)/documents/runs/r1",
             "fields":{"app_id":{"stringValue":"app-1"}},
             "updateTime":"2026-08-15T10:11:12.345678Z"}},
            {"readTime":"2026-08-15T10:11:13Z"}]"#;
        let entries: Vec<RunQueryEntry> = deserialize(body).expect("run query decodes");
        let documents: Vec<RawDocument> = entries
            .into_iter()
            .filter_map(|entry| entry.document)
            .collect();
        assert_eq!(documents.len(), 1);

        #[derive(Debug, Deserialize, PartialEq)]
        struct Run {
            app_id: String,
        }
        let document: Document<Run> =
            document_from_raw(documents.into_iter().next().expect("one document"))
                .expect("document decodes");
        assert_eq!(document.id, "r1");
        assert_eq!(
            document.update_time.as_deref(),
            Some("2026-08-15T10:11:12.345678Z")
        );
        assert_eq!(
            document.value,
            Run {
                app_id: "app-1".to_string()
            }
        );
    }
}

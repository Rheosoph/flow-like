//! BigQuery tables as DataFusion providers. A scan runs one query job (jobs.query, then
//! getQueryResults pages) holding the projection, the exact filters and the limit, and streams
//! the pages as they are fetched. Schemas come from tables.get, read directly so collations
//! (which decide whether text comparisons push down) and types newer than the client crate's
//! model survive. Tokens carry only the `cloud-platform.read-only` scope, and every request
//! goes to Google's public endpoints through the egress-guarded HTTP client.

use super::connectors::{BigquerySource, ConnectPolicy, Provider, ensure_connection_allowed};
use super::remote_table::{
    DEFAULT_STATEMENT_TIMEOUT, Dialect, Filtering, RemoteColumn, RemoteSource, RemoteTable, Select,
    Statement, ValueKind, text_batch,
};
use flow_like::flow::execution::egress;
use flow_like_storage::arrow::record_batch::RecordBatch;
use flow_like_storage::datafusion::common::{DataFusionError, Result as DFResult};
use flow_like_types::json::DeserializeOwned;
use flow_like_types::reqwest::{self, Url};
use flow_like_types::{Result, Value, anyhow};
use futures::StreamExt;
use futures::stream::{self, BoxStream};
use gcp_bigquery_client::Client;
use gcp_bigquery_client::auth::Authenticator;
use gcp_bigquery_client::error::BQError;
use gcp_bigquery_client::model::get_query_results_parameters::GetQueryResultsParameters;
use gcp_bigquery_client::model::query_request::QueryRequest;
use gcp_bigquery_client::model::table_row::TableRow;
use gcp_bigquery_client::yup_oauth2::{self, ServiceAccountKey};
use serde::Deserialize;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

const API_HOST: &str = "bigquery.googleapis.com";
const API_BASE: &str = "https://bigquery.googleapis.com/bigquery/v2/";
const READ_ONLY_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform.read-only";
/// Token endpoints of Google service account keys.
const TOKEN_HOSTS: [&str; 3] = [
    "oauth2.googleapis.com",
    "accounts.google.com",
    "www.googleapis.com",
];
const PAGE_ROWS: i32 = 10_000;
const LIST_PAGE_TABLES: &str = "1000";
const MAX_LIST_PAGES: usize = 1_000;
/// Longest server-side wait of one jobs.query / getQueryResults call.
const WAIT_SLICE: Duration = Duration::from_secs(10);
const RETRY_PAUSE: Duration = Duration::from_millis(250);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const TOKEN_TIMEOUT: Duration = Duration::from_secs(30);

type TokenSource = yup_oauth2::authenticator::Authenticator<
    <yup_oauth2::DefaultHyperClientBuilder as yup_oauth2::HyperClientBuilder>::Connector,
>;

/// Service account tokens limited to read-only access, whatever roles the account holds.
#[derive(Clone)]
struct ReadOnlyKey(TokenSource);

#[async_trait::async_trait]
impl Authenticator for ReadOnlyKey {
    async fn access_token(&self) -> std::result::Result<String, BQError> {
        let token = self.0.token(&[READ_ONLY_SCOPE]).await?;
        token.token().map(str::to_string).ok_or(BQError::NoToken)
    }
}

/// One BigQuery dataset; the tables it opens share its client and token.
pub struct BigqueryDatabase {
    client: Client,
    http: reqwest::Client,
    auth: ReadOnlyKey,
    project_id: String,
    dataset: String,
    location: Option<String>,
    deadline: Option<Instant>,
}

impl fmt::Debug for BigqueryDatabase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BigqueryDatabase")
            .field("project_id", &self.project_id)
            .field("dataset", &self.dataset)
            .field("location", &self.location)
            .finish_non_exhaustive()
    }
}

impl BigqueryDatabase {
    /// Checks the endpoints against `policy`, then builds the client. Nothing is sent to
    /// BigQuery until a table is opened.
    pub async fn open(
        source: &BigquerySource,
        policy: impl Into<ConnectPolicy>,
    ) -> Result<Arc<Self>> {
        let policy = policy.into();
        let key = service_account_key(&source.service_account_json)?;
        let token_host = token_host(&key.token_uri)?;
        ensure_connection_allowed(policy, "BigQuery", &token_host, 443).await?;
        ensure_connection_allowed(policy, "BigQuery", API_HOST, 443).await?;

        let http = egress::client_builder(policy.environment)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| anyhow!("BigQuery HTTP client could not be built: {error}"))?;
        let tokens = yup_oauth2::ServiceAccountAuthenticator::with_client(
            key,
            yup_oauth2::DefaultHyperClientBuilder::default().with_timeout(TOKEN_TIMEOUT),
        )
        .build()
        .await
        .map_err(|error| anyhow!("BigQuery service account key was rejected: {error}"))?;
        let auth = ReadOnlyKey(tokens);
        let client =
            Client::from_authenticator_and_client(Arc::new(auth.clone()), Some(http.clone()))
                .await
                .map_err(|error| anyhow!("BigQuery client could not be built: {error}"))?;
        Ok(Arc::new(Self {
            client,
            http,
            auth,
            project_id: source.project_id.clone(),
            dataset: source.dataset.clone(),
            location: source
                .location
                .clone()
                .filter(|location| !location.is_empty()),
            deadline: source.deadline,
        }))
    }

    /// A provider for one table or view of the dataset.
    pub async fn table(self: &Arc<Self>, table: &str) -> Result<Provider> {
        let url = self.url(&[
            "projects",
            &self.project_id,
            "datasets",
            &self.dataset,
            "tables",
            table,
        ])?;
        let resource: TableResource = self
            .get(
                url,
                &format!("Reading BigQuery table '{}.{table}'", self.dataset),
            )
            .await?;
        let table_collated = resource
            .default_collation
            .as_deref()
            .is_some_and(|collation| !collation.is_empty());
        let fields = resource
            .schema
            .map(|schema| schema.fields)
            .unwrap_or_default();
        if fields.is_empty() {
            return Err(anyhow!(
                "BigQuery table '{}.{table}' has no columns",
                self.dataset
            ));
        }
        let columns = fields
            .iter()
            .map(|field| bigquery_column(field, table_collated))
            .collect();
        let relation = Dialect::BigQuery.relation(&[&self.project_id, &self.dataset, table])?;
        let source: Arc<dyn RemoteSource> = self.clone();
        Ok(Arc::new(RemoteTable::new(
            Dialect::BigQuery,
            relation,
            columns,
            source,
        )))
    }

    /// The tables, views and materialized views of the dataset, sorted.
    pub async fn tables(&self) -> Result<Vec<String>> {
        let mut tables = Vec::new();
        let mut page_token: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let mut url = self.url(&[
                "projects",
                &self.project_id,
                "datasets",
                &self.dataset,
                "tables",
            ])?;
            url.query_pairs_mut()
                .append_pair("maxResults", LIST_PAGE_TABLES);
            if let Some(token) = &page_token {
                url.query_pairs_mut().append_pair("pageToken", token);
            }
            let page: TableList = self
                .get(
                    url,
                    &format!("Listing the tables of BigQuery dataset '{}'", self.dataset),
                )
                .await?;
            tables.extend(
                page.tables
                    .into_iter()
                    .map(|entry| entry.table_reference.table_id),
            );
            match page.next_page_token {
                Some(token) if !token.is_empty() => page_token = Some(token),
                _ => break,
            }
        }
        tables.sort();
        tables.dedup();
        Ok(tables)
    }

    fn url(&self, segments: &[&str]) -> Result<Url> {
        let mut url = Url::parse(API_BASE)?;
        url.path_segments_mut()
            .map_err(|()| anyhow!("BigQuery API URL cannot carry a path"))?
            .pop_if_empty()
            .extend(segments);
        Ok(url)
    }

    async fn get<T: DeserializeOwned>(&self, url: Url, what: &str) -> Result<T> {
        let token =
            self.auth.access_token().await.map_err(|error| {
                anyhow!("{what} failed: BigQuery authentication failed: {error}")
            })?;
        let response = self
            .http
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|error| anyhow!("{what} failed: {error}"))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| anyhow!("{what} failed: {error}"))?;
        if !status.is_success() {
            return Err(anyhow!(
                "{what} failed with HTTP {}: {}",
                status.as_u16(),
                google_error(&body)
            ));
        }
        flow_like_types::json::from_slice(&body)
            .map_err(|error| anyhow!("{what} returned an unexpected response: {error}"))
    }

    async fn page(&self, sql: &str, cursor: Cursor, deadline: Instant) -> Result<Page> {
        match cursor {
            Cursor::Start => self.start(sql, deadline).await,
            Cursor::Next {
                job_id,
                location,
                token,
            } => self.results(job_id, location, Some(token), deadline).await,
        }
    }

    async fn start(&self, sql: &str, deadline: Instant) -> Result<Page> {
        let mut request = QueryRequest::new(sql);
        request.location = self.location.clone();
        request.max_results = Some(PAGE_ROWS);
        request.timeout_ms = Some(wait_ms(deadline)?);
        request.use_query_cache = Some(true);
        let response = self
            .client
            .job()
            .query(&self.project_id, request)
            .await
            .map_err(|error| bigquery_error("BigQuery query", error))?;
        let job = response.job_reference.as_ref();
        let job_id = job.and_then(|job| job.job_id.clone());
        let location = job
            .and_then(|job| job.location.clone())
            .or_else(|| self.location.clone());
        if response.job_complete == Some(true) {
            return Ok(Page {
                rows: text_rows(response.rows),
                next: next_cursor(job_id, location, response.page_token),
            });
        }
        let job_id = job_id
            .ok_or_else(|| anyhow!("BigQuery accepted the query but named no job to wait for"))?;
        self.results(job_id, location, None, deadline).await
    }

    /// One page of a job's result, waiting (server-side, in slices) until the job is done.
    async fn results(
        &self,
        job_id: String,
        location: Option<String>,
        page_token: Option<String>,
        deadline: Instant,
    ) -> Result<Page> {
        loop {
            let parameters = GetQueryResultsParameters {
                format_options: None,
                location: location.clone(),
                max_results: Some(PAGE_ROWS),
                page_token: page_token.clone(),
                start_index: None,
                timeout_ms: Some(wait_ms(deadline)?),
            };
            let response = self
                .client
                .job()
                .get_query_results(&self.project_id, &job_id, parameters)
                .await
                .map_err(|error| bigquery_error("Reading the BigQuery result", error))?;
            if response.job_complete == Some(true) {
                return Ok(Page {
                    rows: text_rows(response.rows),
                    next: next_cursor(Some(job_id), location, response.page_token),
                });
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            flow_like_types::tokio::time::sleep(RETRY_PAUSE.min(remaining)).await;
        }
    }
}

impl RemoteSource for BigqueryDatabase {
    fn execute(
        self: Arc<Self>,
        statement: Arc<Statement>,
    ) -> BoxStream<'static, DFResult<RecordBatch>> {
        let deadline = self
            .deadline
            .unwrap_or_else(|| Instant::now() + DEFAULT_STATEMENT_TIMEOUT);
        stream::try_unfold(Some(Cursor::Start), move |cursor| {
            let database = self.clone();
            let statement = statement.clone();
            async move {
                let Some(cursor) = cursor else {
                    return Ok::<_, DataFusionError>(None);
                };
                let page = database
                    .page(&statement.sql, cursor, deadline)
                    .await
                    .map_err(|error| DataFusionError::External(error.into()))?;
                let batch =
                    text_batch("BigQuery", &statement.schema, &statement.kinds, &page.rows)?;
                Ok(Some((batch, page.next)))
            }
        })
        .boxed()
    }
}

enum Cursor {
    Start,
    Next {
        job_id: String,
        location: Option<String>,
        token: String,
    },
}

struct Page {
    rows: Vec<Vec<Option<String>>>,
    next: Option<Cursor>,
}

fn next_cursor(
    job_id: Option<String>,
    location: Option<String>,
    token: Option<String>,
) -> Option<Cursor> {
    match (job_id, token) {
        (Some(job_id), Some(token)) if !token.is_empty() => Some(Cursor::Next {
            job_id,
            location,
            token,
        }),
        _ => None,
    }
}

/// Milliseconds the next call may wait server-side; fails once the deadline has passed.
fn wait_ms(deadline: Instant) -> Result<i32> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(anyhow!(
            "BigQuery query timed out before its result was ready"
        ));
    }
    Ok(remaining.min(WAIT_SLICE).as_millis().max(1) as i32)
}

/// Cells as text: BigQuery's JSON rows carry every scalar as a string.
fn text_rows(rows: Option<Vec<TableRow>>) -> Vec<Vec<Option<String>>> {
    rows.unwrap_or_default()
        .into_iter()
        .map(|row| {
            row.columns
                .unwrap_or_default()
                .into_iter()
                .map(|cell| match cell.value {
                    None | Some(Value::Null) => None,
                    Some(Value::String(text)) => Some(text),
                    Some(other) => Some(other.to_string()),
                })
                .collect()
        })
        .collect()
}

fn bigquery_error(context: &str, error: BQError) -> flow_like_types::Error {
    match error {
        BQError::ResponseError { error } => anyhow!(
            "{context} failed: {} ({})",
            error.error.message,
            error.error.status
        ),
        other => anyhow!("{context} failed: {other}"),
    }
}

fn google_error(body: &[u8]) -> String {
    #[derive(Deserialize)]
    struct Envelope {
        error: Detail,
    }
    #[derive(Deserialize)]
    struct Detail {
        #[serde(default)]
        message: String,
        #[serde(default)]
        status: String,
    }
    match flow_like_types::json::from_slice::<Envelope>(body) {
        Ok(envelope) => format!("{} ({})", envelope.error.message, envelope.error.status),
        Err(_) => "no error details".to_string(),
    }
}

/// Parse errors give the position only: a key's values must not reach logs.
fn service_account_key(json: &str) -> Result<ServiceAccountKey> {
    if json.trim().is_empty() {
        return Err(anyhow!(
            "BigQuery source has no service_account_json credential"
        ));
    }
    flow_like_types::json::from_str(json).map_err(|error| {
        anyhow!(
            "BigQuery service_account_json is not a Google service account key ({:?} error at line {}, column {})",
            error.classify(),
            error.line(),
            error.column()
        )
    })
}

/// The host of a key's token endpoint, which must be one of Google's.
fn token_host(token_uri: &str) -> Result<String> {
    let refused = || {
        anyhow!(
            "BigQuery service account token_uri must be an https URL on {}",
            TOKEN_HOSTS.join(", ")
        )
    };
    let url = Url::parse(token_uri).map_err(|_| refused())?;
    let host = url.host_str().ok_or_else(refused)?.to_ascii_lowercase();
    let default_port = url.port().is_none_or(|port| port == 443);
    if url.scheme() != "https" || !default_port || !TOKEN_HOSTS.contains(&host.as_str()) {
        return Err(refused());
    }
    Ok(host)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TableResource {
    #[serde(default)]
    schema: Option<SchemaResource>,
    #[serde(default)]
    default_collation: Option<String>,
}

#[derive(Deserialize)]
struct SchemaResource {
    #[serde(default)]
    fields: Vec<FieldResource>,
}

#[derive(Deserialize)]
struct FieldResource {
    name: String,
    #[serde(rename = "type")]
    field_type: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    collation: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TableList {
    #[serde(default)]
    tables: Vec<TableListEntry>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TableListEntry {
    table_reference: TableReferenceResource,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TableReferenceResource {
    table_id: String,
}

/// The Arrow mapping of a BigQuery column. NUMERIC → Decimal128(38, 9), BIGNUMERIC →
/// Decimal256(76, 38) (values of 39 or more integer digits do not fit), TIMESTAMP → UTC
/// microseconds, DATETIME → zone-less microseconds, BYTES → Binary. GEOGRAPHY (WKT), JSON and
/// INTERVAL arrive as their text; RECORD and REPEATED columns as JSON text; RANGE and types
/// added later as BigQuery's text cast.
fn bigquery_column(field: &FieldResource, table_collated: bool) -> RemoteColumn {
    let name = field.name.as_str();
    let repeated = field
        .mode
        .as_deref()
        .is_some_and(|mode| mode.eq_ignore_ascii_case("REPEATED"));
    if repeated {
        return RemoteColumn::new(name, ValueKind::Text, Filtering::Never)
            .selected_as(Select::Json);
    }
    match field.field_type.to_ascii_uppercase().as_str() {
        "STRING" => {
            let collated = match field.collation.as_deref() {
                Some(collation) => !collation.is_empty(),
                None => table_collated,
            };
            let filtering = if collated {
                Filtering::NullOnly
            } else {
                Filtering::Exact
            };
            RemoteColumn::new(name, ValueKind::Text, filtering)
        }
        "INTEGER" | "INT64" => RemoteColumn::new(name, ValueKind::Int64, Filtering::Exact),
        "FLOAT" | "FLOAT64" => RemoteColumn::new(name, ValueKind::Float64, Filtering::Float),
        "BOOLEAN" | "BOOL" => RemoteColumn::new(name, ValueKind::Boolean, Filtering::Exact),
        "NUMERIC" => RemoteColumn::new(
            name,
            ValueKind::Decimal {
                precision: 38,
                scale: 9,
            },
            Filtering::Exact,
        ),
        "BIGNUMERIC" => RemoteColumn::new(
            name,
            ValueKind::Decimal {
                precision: 76,
                scale: 38,
            },
            Filtering::Exact,
        ),
        "DATE" => RemoteColumn::new(name, ValueKind::Date, Filtering::Exact),
        "TIME" => RemoteColumn::new(name, ValueKind::Time, Filtering::Exact),
        "DATETIME" => {
            RemoteColumn::new(name, ValueKind::Timestamp { utc: false }, Filtering::Exact)
        }
        "TIMESTAMP" => RemoteColumn::new(name, ValueKind::EpochMicros, Filtering::Exact)
            .selected_as(Select::EpochMicros),
        "BYTES" => RemoteColumn::new(name, ValueKind::Binary, Filtering::NullOnly),
        "GEOGRAPHY" | "JSON" | "INTERVAL" => {
            RemoteColumn::new(name, ValueKind::Text, Filtering::NullOnly)
        }
        "RECORD" | "STRUCT" => {
            RemoteColumn::new(name, ValueKind::Text, Filtering::Never).selected_as(Select::Json)
        }
        _ => {
            RemoteColumn::new(name, ValueKind::Text, Filtering::NullOnly).selected_as(Select::Text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gcp_bigquery_client::model::table_cell::TableCell;

    fn field(json: &str) -> FieldResource {
        flow_like_types::json::from_str(json).unwrap()
    }

    fn mapped(json: &str) -> (ValueKind, Filtering, Select) {
        let column = bigquery_column(&field(json), false);
        (column.kind, column.filtering, column.select)
    }

    #[test]
    fn column_types_map_to_arrow_and_pushdown_rules() {
        use Filtering::*;
        use Select::*;
        let cases = [
            (
                r#"{"name":"a","type":"STRING"}"#,
                (ValueKind::Text, Exact, Column),
            ),
            (
                r#"{"name":"a","type":"INTEGER"}"#,
                (ValueKind::Int64, Exact, Column),
            ),
            (
                r#"{"name":"a","type":"INT64","mode":"REQUIRED"}"#,
                (ValueKind::Int64, Exact, Column),
            ),
            (
                r#"{"name":"a","type":"FLOAT"}"#,
                (ValueKind::Float64, Float, Column),
            ),
            (
                r#"{"name":"a","type":"BOOL"}"#,
                (ValueKind::Boolean, Exact, Column),
            ),
            (
                r#"{"name":"a","type":"NUMERIC"}"#,
                (
                    ValueKind::Decimal {
                        precision: 38,
                        scale: 9,
                    },
                    Exact,
                    Column,
                ),
            ),
            (
                r#"{"name":"a","type":"BIGNUMERIC"}"#,
                (
                    ValueKind::Decimal {
                        precision: 76,
                        scale: 38,
                    },
                    Exact,
                    Column,
                ),
            ),
            (
                r#"{"name":"a","type":"DATE"}"#,
                (ValueKind::Date, Exact, Column),
            ),
            (
                r#"{"name":"a","type":"TIME"}"#,
                (ValueKind::Time, Exact, Column),
            ),
            (
                r#"{"name":"a","type":"DATETIME"}"#,
                (ValueKind::Timestamp { utc: false }, Exact, Column),
            ),
            (
                r#"{"name":"a","type":"TIMESTAMP"}"#,
                (ValueKind::EpochMicros, Exact, EpochMicros),
            ),
            (
                r#"{"name":"a","type":"BYTES"}"#,
                (ValueKind::Binary, NullOnly, Column),
            ),
            (
                r#"{"name":"a","type":"GEOGRAPHY"}"#,
                (ValueKind::Text, NullOnly, Column),
            ),
            (
                r#"{"name":"a","type":"JSON"}"#,
                (ValueKind::Text, NullOnly, Column),
            ),
            (
                r#"{"name":"a","type":"RECORD","fields":[{"name":"b","type":"INT64"}]}"#,
                (ValueKind::Text, Never, Json),
            ),
            (
                r#"{"name":"a","type":"INT64","mode":"REPEATED"}"#,
                (ValueKind::Text, Never, Json),
            ),
            (
                r#"{"name":"a","type":"RANGE","rangeElementType":{"type":"DATE"}}"#,
                (ValueKind::Text, NullOnly, Text),
            ),
        ];
        for (json, expected) in cases {
            assert_eq!(mapped(json), expected, "{json}");
        }
    }

    #[test]
    fn collated_text_only_pushes_null_checks() {
        let explicit = field(r#"{"name":"a","type":"STRING","collation":"und:ci"}"#);
        assert_eq!(
            bigquery_column(&explicit, false).filtering,
            Filtering::NullOnly
        );
        let inherited = field(r#"{"name":"a","type":"STRING"}"#);
        assert_eq!(
            bigquery_column(&inherited, true).filtering,
            Filtering::NullOnly
        );
        let binary = field(r#"{"name":"a","type":"STRING","collation":""}"#);
        assert_eq!(bigquery_column(&binary, true).filtering, Filtering::Exact);
    }

    #[test]
    fn table_resources_with_unknown_types_still_parse() {
        let resource: TableResource = flow_like_types::json::from_str(
            r#"{"kind":"bigquery#table","type":"TABLE","defaultCollation":"und:ci",
                "schema":{"fields":[{"name":"r","type":"RANGE","rangeElementType":{"type":"DATE"}},
                                    {"name":"s","type":"STRING","maxLength":"10"}]}}"#,
        )
        .unwrap();
        assert_eq!(resource.default_collation.as_deref(), Some("und:ci"));
        assert_eq!(resource.schema.unwrap().fields.len(), 2);
        let list: TableList = flow_like_types::json::from_str(
            r#"{"tables":[{"tableReference":{"projectId":"p","datasetId":"d","tableId":"t"},"type":"VIEW"}],
                "nextPageToken":"n"}"#,
        )
        .unwrap();
        assert_eq!(list.tables[0].table_reference.table_id, "t");
        assert_eq!(list.next_page_token.as_deref(), Some("n"));
    }

    #[test]
    fn token_endpoints_must_be_googles() {
        assert_eq!(
            token_host("https://oauth2.googleapis.com/token").unwrap(),
            "oauth2.googleapis.com"
        );
        assert_eq!(
            token_host("https://accounts.google.com/o/oauth2/token").unwrap(),
            "accounts.google.com"
        );
        for refused in [
            "http://oauth2.googleapis.com/token",
            "https://oauth2.googleapis.com:8443/token",
            "https://evil.example.com/token",
            "https://oauth2.googleapis.com.evil.example/token",
            "https://169.254.169.254/token",
            "not a url",
        ] {
            assert!(token_host(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn invalid_keys_are_reported_without_their_content() {
        let error = service_account_key(r#"{"private_key": "-----BEGIN SECRET"#)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("BEGIN SECRET"), "{error}");
        assert!(service_account_key("  ").is_err());
        let key = service_account_key(
            r#"{"type":"service_account","private_key":"k","client_email":"a@b.iam.gserviceaccount.com",
                "token_uri":"https://oauth2.googleapis.com/token"}"#,
        )
        .unwrap();
        assert_eq!(key.token_uri, "https://oauth2.googleapis.com/token");
    }

    #[tokio::test]
    async fn sources_without_a_usable_key_fail_before_any_network_call() {
        use flow_like::flow::execution::ExecutionEnvironment;
        let source = |key: &str| BigquerySource {
            project_id: "acme".to_string(),
            dataset: "sales".to_string(),
            location: None,
            service_account_json: key.to_string(),
            table: "orders".to_string(),
            deadline: None,
        };
        let refused = |source: BigquerySource| async move {
            BigqueryDatabase::open(&source, ExecutionEnvironment::Server)
                .await
                .unwrap_err()
                .to_string()
        };
        assert!(refused(source("")).await.contains("service_account_json"));
        let foreign = refused(source(
            r#"{"private_key":"k","client_email":"a@b.c","token_uri":"https://169.254.169.254/token"}"#,
        ))
        .await;
        assert!(foreign.contains("token_uri"), "{foreign}");
    }

    #[test]
    fn json_rows_become_text_cells() {
        let cell = |value: Option<Value>| TableCell { value };
        let rows = vec![TableRow {
            columns: Some(vec![
                cell(Some(Value::String("7".to_string()))),
                cell(Some(Value::Null)),
                cell(None),
                cell(Some(flow_like_types::json::json!(true))),
            ]),
        }];
        assert_eq!(
            text_rows(Some(rows)),
            vec![vec![
                Some("7".to_string()),
                None,
                None,
                Some("true".to_string())
            ]]
        );
        assert!(text_rows(None).is_empty());
    }

    #[test]
    fn page_cursors_need_a_job_and_a_token() {
        assert!(next_cursor(Some("job".into()), None, Some("t".into())).is_some());
        assert!(next_cursor(Some("job".into()), None, Some(String::new())).is_none());
        assert!(next_cursor(None, None, Some("t".into())).is_none());
        assert!(next_cursor(Some("job".into()), None, None).is_none());
    }

    #[test]
    fn waits_end_at_the_deadline() {
        assert!(wait_ms(Instant::now() - Duration::from_millis(1)).is_err());
        let slice = wait_ms(Instant::now() + Duration::from_secs(3600)).unwrap();
        assert_eq!(slice, WAIT_SLICE.as_millis() as i32);
        let error = wait_ms(Instant::now()).unwrap_err().to_string();
        assert!(error.contains("timed out"), "{error}");
    }
}

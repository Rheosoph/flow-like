//! The request the API sends and the trailer and errors the service answers with. Rows travel
//! as Arrow IPC in frames (see [`crate::frames`]); everything else is JSON.

pub mod forecast;
pub mod graph;
pub mod table;

use flow_like_runtime::credentials::SharedCredentials;
use flow_like_storage::databases::graph::lancegraph::GraphOverlayDef;
use flow_like_storage::databases::workbench::WorkbenchView;
use flow_like_storage_contracts::column_metadata::{ColumnReader, DataSensitivity};
use flow_like_types::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

pub const PROTOCOL_VERSION: u16 = 1;
pub const QUERY_PATH: &str = "/v1/query";
pub const HEALTH_PATH: &str = "/health";
pub const CONTENT_TYPE: &str = "application/vnd.flow-like.query.v1";
/// JWT audience and JOSE type of the tokens the API signs for the service.
pub const AUDIENCE: &str = "flow-like-query";
pub const JOSE_TYPE: &str = "flow-like-query+jwt";
/// Carries the API's token, so `Authorization` stays free for platform IAM such as a Cloud Run
/// ID token. `Authorization: Bearer <api token>` is accepted where no platform needs it.
pub const TOKEN_HEADER: &str = "x-flow-like-query-token";
/// Requests carry SQL, definitions and credentials, never rows.
pub const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRequest {
    pub version: u16,
    pub request_id: String,
    /// Epoch milliseconds after which the caller stops waiting.
    pub deadline_ms: i64,
    #[serde(default)]
    pub sources: Vec<SourceSpec>,
    pub limits: RequestLimits,
    pub operation: Operation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLimits {
    pub rows: usize,
    #[serde(default)]
    pub memory_bytes: Option<usize>,
    pub timeout_ms: u64,
}

/// One set of tables in the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSpec {
    /// SQL schema the tables are addressed under; `None` registers them bare, as a single-app
    /// workbench surface does.
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub reader: ReaderSpec,
    pub location: SourceLocation,
}

/// [`ColumnReader`] on the wire.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReaderSpec {
    #[default]
    Unrestricted,
    Attributes {
        attributes: Vec<String>,
    },
}

impl From<&ColumnReader> for ReaderSpec {
    fn from(reader: &ColumnReader) -> Self {
        match reader {
            ColumnReader::Unrestricted => Self::Unrestricted,
            ColumnReader::Attributes(attributes) => Self::Attributes {
                attributes: attributes.iter().cloned().collect(),
            },
        }
    }
}

impl ReaderSpec {
    pub fn to_reader(&self) -> ColumnReader {
        match self {
            Self::Unrestricted => ColumnReader::Unrestricted,
            Self::Attributes { attributes } => ColumnReader::with_attributes(attributes),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceLocation {
    /// An app's LanceDB: the project database, or `owner`'s personal one.
    Lance {
        app_id: String,
        #[serde(default)]
        owner: Option<String>,
        /// Least-privilege, short-lived credentials minted by the API for this request.
        credentials: SharedCredentials,
    },
    External(ExternalSource),
}

impl fmt::Debug for SourceLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lance { app_id, owner, .. } => f
                .debug_struct("Lance")
                .field("app_id", app_id)
                .field("owner", owner)
                .finish_non_exhaustive(),
            Self::External(source) => f.debug_tuple("External").field(source).finish(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalSource {
    pub source_id: String,
    pub app_id: String,
    pub connection: ExternalConnection,
    pub secret: SecretMaterial,
}

/// The non-secret half of an external database connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "driver", rename_all = "snake_case")]
pub enum ExternalConnection {
    Postgres {
        host: String,
        port: u16,
        database: String,
        #[serde(default = "default_postgres_schema")]
        schema: String,
        /// `disable`, `prefer`, `require`, `verify-ca` or `verify-full`.
        #[serde(default = "default_postgres_ssl")]
        ssl_mode: String,
    },
    Mysql {
        host: String,
        port: u16,
        database: String,
        /// `disabled`, `preferred` or `required`.
        #[serde(default = "default_mysql_ssl")]
        ssl_mode: String,
    },
    Clickhouse {
        host: String,
        port: u16,
        database: String,
    },
    Bigquery {
        project_id: String,
        dataset: String,
        #[serde(default)]
        location: Option<String>,
    },
    Athena {
        region: String,
        database: String,
        #[serde(default)]
        catalog: Option<String>,
        #[serde(default)]
        workgroup: Option<String>,
        #[serde(default)]
        output_location: Option<String>,
    },
}

fn default_postgres_schema() -> String {
    "public".to_string()
}

fn default_postgres_ssl() -> String {
    "require".to_string()
}

fn default_mysql_ssl() -> String {
    "required".to_string()
}

impl ExternalConnection {
    pub fn driver(&self) -> &'static str {
        match self {
            Self::Postgres { .. } => "postgres",
            Self::Mysql { .. } => "mysql",
            Self::Clickhouse { .. } => "clickhouse",
            Self::Bigquery { .. } => "bigquery",
            Self::Athena { .. } => "athena",
        }
    }
}

/// The secret half of an external connection.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SecretMaterial {
    None,
    /// Opened by the API, sent only over the authenticated channel, wiped on drop.
    Plain {
        fields: SecretFields,
    },
    /// Sealed for the service to open itself (a key service the API cannot decrypt with).
    Sealed {
        sealed: SealedSecret,
    },
}

impl fmt::Debug for SecretMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::Plain { fields } => write!(f, "Plain({fields:?})"),
            Self::Sealed { sealed } => write!(f, "Sealed({})", sealed.provider),
        }
    }
}

/// Named secret values such as `username`, `password` or `service_account_json`.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SecretFields(pub BTreeMap<String, String>);

impl SecretFields {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }
}

impl fmt::Debug for SecretFields {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.0.keys()).finish()
    }
}

impl Drop for SecretFields {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        for value in self.0.values_mut() {
            value.zeroize();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedSecret {
    /// Which sealer wrote it, such as `aws-kms`.
    pub provider: String,
    #[serde(default)]
    pub key_id: Option<String>,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewSpec {
    pub name: String,
    pub sql: String,
}

impl From<&ViewSpec> for WorkbenchView {
    fn from(view: &ViewSpec) -> Self {
        Self {
            name: view.name.clone(),
            sql: view.sql.clone(),
        }
    }
}

/// The tables a single-app workbench statement sees.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SurfaceSpec {
    Native,
    Overlay { overlay: Box<GraphOverlayDef> },
    RemoteOverlay { overlay: Box<GraphOverlayDef> },
}

/// How tables are addressed: `schema.table` across apps, or one app's workbench surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QueryShape {
    CrossApp,
    Workbench {
        surface: SurfaceSpec,
        #[serde(default)]
        views: Vec<ViewSpec>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Operation {
    /// Rows of one read-only statement. With `reuse_if_unchanged`, the service first compares
    /// those `schema.table` versions and answers `unchanged` without running when all match.
    Sql {
        sql: String,
        #[serde(default)]
        params: Value,
        shape: QueryShape,
        #[serde(default)]
        reuse_if_unchanged: Option<BTreeMap<String, u64>>,
    },
    /// Output columns and `$parameters` of a statement, without running it.
    Describe { sql: String, shape: QueryShape },
    /// Values a parameter bound to `table.column` can take: distinct values (optionally
    /// containing `search`), or the minimum and maximum when `range` is set.
    ParameterOptions {
        table: String,
        column: String,
        #[serde(default)]
        search: Option<String>,
        #[serde(default)]
        range: bool,
        shape: QueryShape,
    },
    /// Table names of the external sources in `sources`.
    ExternalTables,
    /// Connects to every external source in `sources` and lists its tables.
    ExternalTest,
    /// Graph reads on one ontology overlay.
    Graph { graph: graph::GraphOperation },
    /// Table browsing reads.
    Table { table: table::TableOperation },
    /// Forecasts over the rows of one statement.
    Forecast {
        forecast: forecast::ForecastOperation,
    },
}

/// The last frame of every successful response.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Trailer {
    pub row_count: usize,
    pub truncated: bool,
    #[serde(default)]
    pub tables: Vec<String>,
    #[serde(default)]
    pub versions: BTreeMap<String, u64>,
    #[serde(default)]
    pub unversioned_tables: Vec<String>,
    #[serde(default)]
    pub sensitivity: DataSensitivity,
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Answers `reuse_if_unchanged`: nothing ran, the caller's copy is still current.
    #[serde(default)]
    pub unchanged: bool,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    QueryFailed,
    ResourcesExhausted,
    Timeout,
    Unauthorized,
    Unavailable,
    Internal,
}

impl ErrorCode {
    pub fn http_status(self) -> u16 {
        match self {
            Self::InvalidRequest | Self::QueryFailed => 400,
            Self::Unauthorized => 401,
            Self::ResourcesExhausted => 413,
            Self::Unavailable => 503,
            Self::Timeout => 504,
            Self::Internal => 500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct WireError {
    pub code: ErrorCode,
    pub message: String,
}

impl WireError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Sorts an execution failure into the code a caller can act on.
    pub fn from_query_error(error: &flow_like_types::Error) -> Self {
        let message = format!("{error:#}");
        let lower = message.to_ascii_lowercase();
        let code = if lower.contains("timed out") {
            ErrorCode::Timeout
        } else if lower.contains("resources exhausted") {
            ErrorCode::ResourcesExhausted
        } else {
            ErrorCode::QueryFailed
        };
        Self { code, message }
    }
}

/// Claims of the API's ES256 token for one request. `body_sha256` binds the token to the exact
/// request body, so a captured token cannot carry another query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryClaims {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
    pub request_id: String,
    pub body_sha256: String,
}

/// Base64url SHA-256 of a request body, as [`QueryClaims::body_sha256`] carries it.
pub fn body_digest(body: &[u8]) -> String {
    use base64::Engine;
    use sha2::Digest;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(body))
}

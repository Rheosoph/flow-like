mod query;
mod session;
mod sources;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::files::FileRef;
use crate::spec::{Approval, ToolSpec, schema_of};

pub use query::data_query;
pub use sources::data_sources;

pub const DATA_SOURCES: &str = "data_sources";
pub const DATA_QUERY: &str = "data_query";

const CAPABILITY: &str = "data_studio";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceOperation {
    List,
    Describe,
    Attach,
    Detach,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Csv,
    Excel,
    Postgres,
    Mysql,
    Clickhouse,
}

impl SourceKind {
    pub fn is_network(self) -> bool {
        matches!(self, Self::Postgres | Self::Mysql | Self::Clickhouse)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Excel => "excel",
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
            Self::Clickhouse => "clickhouse",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum AttachMode {
    #[default]
    Snapshot,
    Live,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExcelSpec {
    /// `detect_tables` (default) finds the tables on each sheet; `sheet_as_table` takes each sheet whole.
    #[serde(default)]
    pub mode: Option<String>,
    /// Only this sheet.
    #[serde(default)]
    pub sheet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CsvSpec {
    /// A single character; ',' by default.
    #[serde(default)]
    pub delimiter: Option<String>,
    #[serde(default)]
    pub has_header: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConnectionSpec {
    /// The name of the bot secret that holds the password; never the password itself.
    pub secret: String,
    pub username: String,
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    pub database: String,
    #[serde(default)]
    pub schema: Option<String>,
    /// The tables to copy.
    pub tables: Vec<String>,
    #[serde(default)]
    pub ssl_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AttachSpec {
    /// Lowercase letters, digits and '_'; tables are stored as `ws.ext_<name>_<table>`.
    pub name: String,
    pub kind: SourceKind,
    /// The file for csv and excel.
    #[serde(default)]
    pub file: Option<FileRef>,
    #[serde(default)]
    pub csv: Option<CsvSpec>,
    #[serde(default)]
    pub excel: Option<ExcelSpec>,
    /// The database for network kinds; needs the data_connections capability.
    #[serde(default)]
    pub connection: Option<ConnectionSpec>,
    /// `snapshot` (default) copies the data into the workspace; `live` is not available yet.
    #[serde(default)]
    pub mode: AttachMode,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DataSourcesArgs {
    pub operation: SourceOperation,
    /// `ws`, an app alias or an app id (describe).
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub table: Option<String>,
    /// Add up to 5 sample rows to describe.
    #[serde(default)]
    pub include_sample: bool,
    #[serde(default)]
    pub attach: Option<AttachSpec>,
    /// The attached source for detach and refresh.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SaveFile {
    /// A workspace path.
    pub file: String,
    /// `csv` or `json`.
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SaveTable {
    pub table: String,
    /// `replace` (default) or `append`.
    #[serde(default)]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SaveAs {
    Table(SaveTable),
    File(SaveFile),
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DataQueryArgs {
    /// One SELECT or WITH statement. Tables are `ws.<table>` (the workspace), `<app_alias>.<table>` (apps) and
    /// attached sources (`ws.ext_<name>_<table>`).
    pub sql: String,
    /// Named parameters, referenced as `$name`.
    #[serde(default)]
    pub params: Option<Value>,
    /// Rows shown (default 50, at most 200).
    #[serde(default)]
    pub max_rows: Option<usize>,
    /// Save the whole result as a workspace table or file instead of only showing it.
    #[serde(default)]
    pub save_as: Option<SaveAs>,
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: DATA_SOURCES,
            description: "List and describe the tables the bot can query (workspace, apps, attached files and databases), and attach CSV or Excel files or databases as workspace tables. Attaching a network database needs the data_connections capability.",
            capability: Some(CAPABILITY),
            module: "data",
            approval: Approval::None,
            approval_args: &[],
            mutating: true,
            input_schema: schema_of::<DataSourcesArgs>(),
        },
        ToolSpec {
            name: DATA_QUERY,
            description: "Run a read-only SQL query across workspace tables, app tables and attached sources, and optionally save the result as a workspace table or file.",
            capability: Some(CAPABILITY),
            module: "data",
            approval: Approval::None,
            approval_args: &[],
            mutating: false,
            input_schema: schema_of::<DataQueryArgs>(),
        },
    ]
}

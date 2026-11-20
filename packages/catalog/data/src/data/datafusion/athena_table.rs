//! Athena tables as DataFusion providers. A scan starts one query holding the projection, the
//! exact filters and the limit, polls it with backoff until it finishes or the deadline passes,
//! and streams GetQueryResults pages. A query whose scan goes away early is stopped, since
//! Athena bills the data a query scans. Only the source's own keys sign requests: the service's
//! AWS identity is never used.
//!
//! Results are paged through GetQueryResults rather than read from the CSV Athena writes to
//! `output_location`: paging works with managed query results and with credentials that may not
//! read the result bucket, keeps NULL apart from the empty string (CSV cannot), and the scans
//! this serves (pushed-down filters and limits, a bounded result) fit in a few 1,000-row pages.

use super::connectors::{AthenaSource, ConnectPolicy, Provider, ensure_connection_allowed};
use super::remote_table::{
    DEFAULT_STATEMENT_TIMEOUT, Dialect, Filtering, RemoteColumn, RemoteSource, RemoteTable, Select,
    Statement, ValueKind, text_batch,
};
use aws_sdk_athena::Client;
use aws_sdk_athena::config::timeout::TimeoutConfig;
use aws_sdk_athena::config::{BehaviorVersion, Credentials, Region};
use aws_sdk_athena::error::DisplayErrorContext;
use aws_sdk_athena::types::{Column, QueryExecutionContext, ResultConfiguration};
use flow_like_storage::arrow::record_batch::RecordBatch;
use flow_like_storage::datafusion::common::{DataFusionError, Result as DFResult};
use flow_like_types::{Result, anyhow};
use futures::StreamExt;
use futures::stream::{self, BoxStream};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEFAULT_CATALOG: &str = "AwsDataCatalog";
const PAGE_ROWS: i32 = 1_000;
const LIST_PAGE_TABLES: i32 = 50;
const MAX_LIST_PAGES: usize = 1_000;
const FIRST_POLL: Duration = Duration::from_millis(250);
const MAX_POLL: Duration = Duration::from_secs(2);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);

/// One Athena database; the tables it opens share its client.
pub struct AthenaDatabase {
    client: Client,
    catalog: String,
    database: String,
    workgroup: Option<String>,
    output_location: Option<String>,
    deadline: Option<Instant>,
}

impl fmt::Debug for AthenaDatabase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AthenaDatabase")
            .field("catalog", &self.catalog)
            .field("database", &self.database)
            .field("workgroup", &self.workgroup)
            .finish_non_exhaustive()
    }
}

impl AthenaDatabase {
    /// Checks the regional endpoint against `policy`, then builds a client signed with the
    /// source's keys. Nothing is sent to Athena until a table is opened.
    pub async fn open(
        source: &AthenaSource,
        policy: impl Into<ConnectPolicy>,
    ) -> Result<Arc<Self>> {
        let policy = policy.into();
        ensure_region(&source.region)?;
        if source.access_key_id.trim().is_empty() || source.secret_access_key.is_empty() {
            return Err(anyhow!(
                "Athena source needs an access_key_id and a secret_access_key"
            ));
        }
        let output_location = non_empty(&source.output_location);
        if let Some(location) = &output_location
            && !location.starts_with("s3://")
        {
            return Err(anyhow!(
                "Athena output_location must be an s3:// URL, got '{location}'"
            ));
        }
        ensure_connection_allowed(policy, "Athena", &athena_host(&source.region), 443).await?;
        let credentials = Credentials::new(
            source.access_key_id.trim(),
            source.secret_access_key.clone(),
            non_empty(&source.session_token),
            None,
            "flow-like-external-source",
        );
        let config = aws_sdk_athena::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(source.region.clone()))
            .credentials_provider(credentials)
            .timeout_config(
                TimeoutConfig::builder()
                    .connect_timeout(CONNECT_TIMEOUT)
                    .operation_attempt_timeout(ATTEMPT_TIMEOUT)
                    .build(),
            )
            .build();
        Ok(Arc::new(Self {
            client: Client::from_conf(config),
            catalog: non_empty(&source.catalog).unwrap_or_else(|| DEFAULT_CATALOG.to_string()),
            database: source.database.clone(),
            workgroup: non_empty(&source.workgroup),
            output_location,
            deadline: source.deadline,
        }))
    }

    /// A provider for one table or view; partition columns follow the data columns.
    pub async fn table(self: &Arc<Self>, table: &str) -> Result<Provider> {
        let mut request = self
            .client
            .get_table_metadata()
            .catalog_name(&self.catalog)
            .database_name(&self.database)
            .table_name(table);
        if let Some(workgroup) = &self.workgroup {
            request = request.work_group(workgroup);
        }
        let output = request.send().await.map_err(|error| {
            anyhow!(
                "Reading Athena table '{}.{table}' failed: {}",
                self.database,
                DisplayErrorContext(&error)
            )
        })?;
        let metadata = output.table_metadata.ok_or_else(|| {
            anyhow!(
                "Athena returned no metadata for table '{}.{table}'",
                self.database
            )
        })?;
        let columns: Vec<RemoteColumn> = metadata
            .columns
            .unwrap_or_default()
            .into_iter()
            .chain(metadata.partition_keys.unwrap_or_default())
            .map(athena_column)
            .collect();
        if columns.is_empty() {
            return Err(anyhow!(
                "Athena table '{}.{table}' has no columns",
                self.database
            ));
        }
        let relation = Dialect::Trino.relation(&[&self.catalog, &self.database, table])?;
        let source: Arc<dyn RemoteSource> = self.clone();
        Ok(Arc::new(RemoteTable::new(
            Dialect::Trino,
            relation,
            columns,
            source,
        )))
    }

    /// The tables and views of the database, sorted.
    pub async fn tables(&self) -> Result<Vec<String>> {
        let mut tables = Vec::new();
        let mut next_token: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let mut request = self
                .client
                .list_table_metadata()
                .catalog_name(&self.catalog)
                .database_name(&self.database)
                .max_results(LIST_PAGE_TABLES)
                .set_next_token(next_token.take());
            if let Some(workgroup) = &self.workgroup {
                request = request.work_group(workgroup);
            }
            let output = request.send().await.map_err(|error| {
                anyhow!(
                    "Listing the tables of Athena database '{}' failed: {}",
                    self.database,
                    DisplayErrorContext(&error)
                )
            })?;
            tables.extend(
                output
                    .table_metadata_list
                    .unwrap_or_default()
                    .into_iter()
                    .map(|table| table.name),
            );
            match output.next_token {
                Some(token) if !token.is_empty() => next_token = Some(token),
                _ => break,
            }
        }
        tables.sort();
        tables.dedup();
        Ok(tables)
    }

    /// Starts the statement and waits until it succeeded; the id of the finished query.
    async fn run(&self, sql: &str, deadline: Instant) -> Result<String> {
        let mut request = self
            .client
            .start_query_execution()
            .query_string(sql)
            .query_execution_context(
                QueryExecutionContext::builder()
                    .catalog(&self.catalog)
                    .database(&self.database)
                    .build(),
            );
        if let Some(location) = &self.output_location {
            request = request.result_configuration(
                ResultConfiguration::builder()
                    .output_location(location)
                    .build(),
            );
        }
        if let Some(workgroup) = &self.workgroup {
            request = request.work_group(workgroup);
        }
        let output = request.send().await.map_err(|error| {
            anyhow!(
                "Starting the Athena query failed: {}",
                DisplayErrorContext(&error)
            )
        })?;
        let query_id = output
            .query_execution_id
            .ok_or_else(|| anyhow!("Athena started the query but returned no execution id"))?;
        let running = RunningQuery {
            client: self.client.clone(),
            query_id: Some(query_id),
        };
        match self.wait(running.id(), deadline).await {
            Ok(()) => Ok(running.finished()),
            Err(error) => {
                running.stop().await;
                Err(error)
            }
        }
    }

    async fn wait(&self, query_id: &str, deadline: Instant) -> Result<()> {
        let mut delay = FIRST_POLL;
        loop {
            let output = self
                .client
                .get_query_execution()
                .query_execution_id(query_id)
                .send()
                .await
                .map_err(|error| {
                    anyhow!(
                        "Checking the Athena query failed: {}",
                        DisplayErrorContext(&error)
                    )
                })?;
            let status = output
                .query_execution
                .and_then(|execution| execution.status);
            let state = status
                .as_ref()
                .and_then(|status| status.state.as_ref())
                .map(|state| state.as_str().to_string())
                .unwrap_or_default();
            match state.as_str() {
                "SUCCEEDED" => return Ok(()),
                "FAILED" | "CANCELLED" => {
                    let reason = status
                        .and_then(|status| {
                            status
                                .athena_error
                                .and_then(|error| error.error_message)
                                .or(status.state_change_reason)
                        })
                        .unwrap_or_else(|| "no reason given".to_string());
                    return Err(anyhow!(
                        "Athena query {}: {reason}",
                        state.to_ascii_lowercase()
                    ));
                }
                _ => {}
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(anyhow!("Athena query timed out before it finished"));
            }
            flow_like_types::tokio::time::sleep(delay.min(remaining)).await;
            delay = (delay * 3 / 2).min(MAX_POLL);
        }
    }

    async fn page(
        &self,
        query_id: &str,
        next_token: Option<String>,
        first: bool,
    ) -> Result<AthenaPage> {
        let output = self
            .client
            .get_query_results()
            .query_execution_id(query_id)
            .max_results(PAGE_ROWS)
            .set_next_token(next_token)
            .send()
            .await
            .map_err(|error| {
                anyhow!(
                    "Reading the Athena result failed: {}",
                    DisplayErrorContext(&error)
                )
            })?;
        let next = output.next_token.filter(|token| !token.is_empty());
        let Some(result_set) = output.result_set else {
            return Ok(AthenaPage {
                rows: Vec::new(),
                next,
            });
        };
        let labels: Vec<String> = result_set
            .result_set_metadata
            .and_then(|metadata| metadata.column_info)
            .unwrap_or_default()
            .into_iter()
            .map(|column| column.label.unwrap_or(column.name))
            .collect();
        let mut rows: Vec<Vec<Option<String>>> = result_set
            .rows
            .unwrap_or_default()
            .into_iter()
            .map(|row| {
                row.data
                    .unwrap_or_default()
                    .into_iter()
                    .map(|datum| datum.var_char_value)
                    .collect()
            })
            .collect();
        if first && rows.first().is_some_and(|row| is_header(row, &labels)) {
            rows.remove(0);
        }
        Ok(AthenaPage { rows, next })
    }
}

impl RemoteSource for AthenaDatabase {
    fn execute(
        self: Arc<Self>,
        statement: Arc<Statement>,
    ) -> BoxStream<'static, DFResult<RecordBatch>> {
        let deadline = self
            .deadline
            .unwrap_or_else(|| Instant::now() + DEFAULT_STATEMENT_TIMEOUT);
        stream::try_unfold(Step::Start, move |step| {
            let database = self.clone();
            let statement = statement.clone();
            async move {
                let failed =
                    |error: flow_like_types::Error| DataFusionError::External(error.into());
                let (query_id, next_token, first) = match step {
                    Step::Start => (
                        database
                            .run(&statement.sql, deadline)
                            .await
                            .map_err(failed)?,
                        None,
                        true,
                    ),
                    Step::Next {
                        query_id,
                        next_token,
                    } => (query_id, Some(next_token), false),
                    Step::Done => return Ok::<_, DataFusionError>(None),
                };
                let page = database
                    .page(&query_id, next_token, first)
                    .await
                    .map_err(failed)?;
                let batch = text_batch("Athena", &statement.schema, &statement.kinds, &page.rows)?;
                let next = match page.next {
                    Some(next_token) => Step::Next {
                        query_id,
                        next_token,
                    },
                    None => Step::Done,
                };
                Ok(Some((batch, next)))
            }
        })
        .boxed()
    }
}

enum Step {
    Start,
    Next {
        query_id: String,
        next_token: String,
    },
    Done,
}

struct AthenaPage {
    rows: Vec<Vec<Option<String>>>,
    next: Option<String>,
}

/// A started query. Failed waits stop it in place; dropped mid-wait (the caller gave up), it
/// is stopped from a spawned task, which a frozen Lambda may never run.
struct RunningQuery {
    client: Client,
    query_id: Option<String>,
}

impl RunningQuery {
    fn id(&self) -> &str {
        self.query_id.as_deref().unwrap_or_default()
    }

    fn finished(mut self) -> String {
        self.query_id.take().unwrap_or_default()
    }

    async fn stop(mut self) {
        if let Some(query_id) = self.query_id.take() {
            stop_query(self.client.clone(), query_id).await;
        }
    }
}

impl Drop for RunningQuery {
    fn drop(&mut self) {
        let Some(query_id) = self.query_id.take() else {
            return;
        };
        if let Ok(runtime) = flow_like_types::tokio::runtime::Handle::try_current() {
            runtime.spawn(stop_query(self.client.clone(), query_id));
        }
    }
}

async fn stop_query(client: Client, query_id: String) {
    if let Err(error) = client
        .stop_query_execution()
        .query_execution_id(query_id)
        .send()
        .await
    {
        tracing::debug!(
            "Stopping an Athena query failed: {}",
            DisplayErrorContext(&error)
        );
    }
}

/// The first row of a SELECT's first page repeats the column labels.
fn is_header(row: &[Option<String>], labels: &[String]) -> bool {
    !labels.is_empty()
        && row.len() == labels.len()
        && row
            .iter()
            .zip(labels)
            .all(|(cell, label)| cell.as_deref() == Some(label.as_str()))
}

fn non_empty(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Region names such as `eu-central-1` or `us-gov-west-1`; anything else could rewrite the
/// endpoint host the SDK derives from it.
fn ensure_region(region: &str) -> Result<()> {
    let parts: Vec<&str> = region.split('-').collect();
    let letters = |part: &&str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_lowercase());
    let valid = parts.len() >= 3
        && parts[0].len() == 2
        && letters(&parts[0])
        && parts[1..parts.len() - 1].iter().all(letters)
        && parts.last().is_some_and(|last| {
            (1..=2).contains(&last.len()) && last.bytes().all(|b| b.is_ascii_digit())
        });
    if valid {
        Ok(())
    } else {
        Err(anyhow!(
            "Athena region '{region}' is not an AWS region name such as eu-central-1"
        ))
    }
}

fn athena_host(region: &str) -> String {
    if region.starts_with("cn-") {
        format!("athena.{region}.amazonaws.com.cn")
    } else {
        format!("athena.{region}.amazonaws.com")
    }
}

fn athena_column(column: Column) -> RemoteColumn {
    let (kind, filtering, select) = hive_type(column.r#type.as_deref().unwrap_or_default());
    RemoteColumn::new(column.name, kind, filtering).selected_as(select)
}

/// The Arrow mapping of a Glue/Hive column type. `float` is single precision, `decimal(p,s)`
/// (p ≤ 38) Decimal128, `timestamp` zone-less microseconds, `timestamp with time zone` UTC.
/// `char(n)` keeps its padding (only NULL checks push down), binary arrives as base64,
/// `array`, `map`, `struct` and `row` as JSON text, other types as Athena's VARCHAR cast.
fn hive_type(declared: &str) -> (ValueKind, Filtering, Select) {
    let lowered = declared.trim().to_ascii_lowercase();
    let base = lowered.split(['(', '<']).next().unwrap_or_default().trim();
    match base {
        "boolean" => (ValueKind::Boolean, Filtering::Exact, Select::Column),
        "tinyint" => (ValueKind::Int8, Filtering::Exact, Select::Column),
        "smallint" => (ValueKind::Int16, Filtering::Exact, Select::Column),
        "int" | "integer" => (ValueKind::Int32, Filtering::Exact, Select::Column),
        "bigint" => (ValueKind::Int64, Filtering::Exact, Select::Column),
        "float" | "real" => (ValueKind::Float32, Filtering::Float, Select::Column),
        "double" | "double precision" => (ValueKind::Float64, Filtering::Float, Select::Column),
        "decimal" | "numeric" => match decimal_parameters(&lowered) {
            Some((precision, scale)) => (
                ValueKind::Decimal { precision, scale },
                Filtering::Exact,
                Select::Column,
            ),
            None => (ValueKind::Text, Filtering::NullOnly, Select::Text),
        },
        "string" | "varchar" => (ValueKind::Text, Filtering::Exact, Select::Column),
        "char" => (ValueKind::Text, Filtering::NullOnly, Select::Column),
        "date" => (ValueKind::Date, Filtering::Exact, Select::Column),
        "timestamp" if lowered.ends_with("with time zone") => (
            ValueKind::Timestamp { utc: true },
            Filtering::Exact,
            Select::Column,
        ),
        "timestamp" => (
            ValueKind::Timestamp { utc: false },
            Filtering::Exact,
            Select::Column,
        ),
        "timestamptz" => (
            ValueKind::Timestamp { utc: true },
            Filtering::Exact,
            Select::Column,
        ),
        "binary" | "varbinary" => (ValueKind::Binary, Filtering::NullOnly, Select::Base64),
        "array" | "map" | "struct" | "row" => (ValueKind::Text, Filtering::Never, Select::Json),
        "json" => (ValueKind::Text, Filtering::NullOnly, Select::Column),
        _ => (ValueKind::Text, Filtering::NullOnly, Select::Text),
    }
}

/// `(precision, scale)` of `decimal(p,s)`; Hive's bare `decimal` is `decimal(10,0)`.
fn decimal_parameters(declared: &str) -> Option<(u8, i8)> {
    let Some(open) = declared.find('(') else {
        return Some((10, 0));
    };
    let close = declared[open..].find(')')? + open;
    let mut parts = declared[open + 1..close].split(',').map(str::trim);
    let precision: u8 = parts.next()?.parse().ok()?;
    let scale: i8 = match parts.next() {
        Some(scale) => scale.parse().ok()?,
        None => 0,
    };
    ((1..=38).contains(&precision) && (0..=precision as i8).contains(&scale))
        .then_some((precision, scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hive_types_map_to_arrow_and_pushdown_rules() {
        use Filtering::*;
        let cases = [
            ("boolean", ValueKind::Boolean, Exact, Select::Column),
            ("tinyint", ValueKind::Int8, Exact, Select::Column),
            ("smallint", ValueKind::Int16, Exact, Select::Column),
            ("int", ValueKind::Int32, Exact, Select::Column),
            ("INTEGER", ValueKind::Int32, Exact, Select::Column),
            ("bigint", ValueKind::Int64, Exact, Select::Column),
            ("float", ValueKind::Float32, Float, Select::Column),
            ("double", ValueKind::Float64, Float, Select::Column),
            (
                "decimal(10,2)",
                ValueKind::Decimal {
                    precision: 10,
                    scale: 2,
                },
                Exact,
                Select::Column,
            ),
            (
                "decimal",
                ValueKind::Decimal {
                    precision: 10,
                    scale: 0,
                },
                Exact,
                Select::Column,
            ),
            ("decimal(40,2)", ValueKind::Text, NullOnly, Select::Text),
            ("string", ValueKind::Text, Exact, Select::Column),
            ("varchar(255)", ValueKind::Text, Exact, Select::Column),
            ("char(3)", ValueKind::Text, NullOnly, Select::Column),
            ("date", ValueKind::Date, Exact, Select::Column),
            (
                "timestamp",
                ValueKind::Timestamp { utc: false },
                Exact,
                Select::Column,
            ),
            (
                "timestamp(6) with time zone",
                ValueKind::Timestamp { utc: true },
                Exact,
                Select::Column,
            ),
            ("binary", ValueKind::Binary, NullOnly, Select::Base64),
            ("array<string>", ValueKind::Text, Never, Select::Json),
            ("map<string,int>", ValueKind::Text, Never, Select::Json),
            (
                "struct<a:int,b:array<string>>",
                ValueKind::Text,
                Never,
                Select::Json,
            ),
            (
                "uniontype<int,string>",
                ValueKind::Text,
                NullOnly,
                Select::Text,
            ),
            ("", ValueKind::Text, NullOnly, Select::Text),
        ];
        for (declared, kind, filtering, select) in cases {
            assert_eq!(hive_type(declared), (kind, filtering, select), "{declared}");
        }
    }

    #[test]
    fn decimal_parameters_are_validated() {
        assert_eq!(decimal_parameters("decimal(38,9)"), Some((38, 9)));
        assert_eq!(decimal_parameters("decimal( 5 )"), Some((5, 0)));
        assert_eq!(decimal_parameters("decimal(3,4)"), None);
        assert_eq!(decimal_parameters("decimal(0,0)"), None);
        assert_eq!(decimal_parameters("decimal(x,1)"), None);
    }

    #[test]
    fn regions_must_be_aws_region_names() {
        for region in [
            "us-east-1",
            "eu-central-2",
            "us-gov-west-1",
            "cn-north-1",
            "ap-southeast-5",
        ] {
            assert!(ensure_region(region).is_ok(), "{region}");
        }
        for region in [
            "",
            "us-east",
            "US-EAST-1",
            "us-east-1.evil.example",
            "evil.example#",
            "us-east-123",
            "us--1",
        ] {
            assert!(ensure_region(region).is_err(), "{region}");
        }
        assert_eq!(athena_host("eu-west-1"), "athena.eu-west-1.amazonaws.com");
        assert_eq!(
            athena_host("cn-north-1"),
            "athena.cn-north-1.amazonaws.com.cn"
        );
    }

    #[test]
    fn the_header_row_is_recognised_by_its_labels() {
        let labels = vec!["id".to_string(), "name".to_string()];
        let header = vec![Some("id".to_string()), Some("name".to_string())];
        let data = vec![Some("1".to_string()), None];
        assert!(is_header(&header, &labels));
        assert!(!is_header(&data, &labels));
        assert!(!is_header(&header[..1], &labels));
        assert!(!is_header(&[], &[]));
    }

    #[test]
    fn columns_keep_their_names_and_partitions_follow() {
        let column = |name: &str, declared: &str| {
            Column::builder()
                .name(name)
                .r#type(declared)
                .build()
                .unwrap()
        };
        let mapped = athena_column(column("dt", "string"));
        assert_eq!(mapped.name, "dt");
        assert_eq!(mapped.kind, ValueKind::Text);
        let nested = athena_column(column("tags", "array<string>"));
        assert_eq!(nested.select, Select::Json);
    }

    #[tokio::test]
    async fn invalid_sources_fail_before_any_network_call() {
        use flow_like::flow::execution::ExecutionEnvironment;
        let source = AthenaSource {
            region: "eu-central-1".to_string(),
            database: "logs".to_string(),
            catalog: None,
            workgroup: None,
            output_location: None,
            access_key_id: String::new(),
            secret_access_key: String::new(),
            session_token: None,
            table: "events".to_string(),
            deadline: None,
        };
        let refused = |source: AthenaSource| async move {
            AthenaDatabase::open(&source, ExecutionEnvironment::Server)
                .await
                .unwrap_err()
                .to_string()
        };
        assert!(refused(source.clone()).await.contains("access_key_id"));
        let keyed = AthenaSource {
            access_key_id: "AKIAEXAMPLE".to_string(),
            secret_access_key: "secret".to_string(),
            ..source
        };
        let region = refused(AthenaSource {
            region: "evil.example#".to_string(),
            ..keyed.clone()
        })
        .await;
        assert!(region.contains("region"), "{region}");
        let output = refused(AthenaSource {
            output_location: Some("https://bucket.example".to_string()),
            ..keyed
        })
        .await;
        assert!(output.contains("s3://"), "{output}");
        assert!(!output.contains("secret"), "{output}");
    }

    #[test]
    fn blank_options_count_as_unset() {
        assert_eq!(non_empty(&Some("  ".to_string())), None);
        assert_eq!(non_empty(&None), None);
        assert_eq!(non_empty(&Some(" wg ".to_string())), Some("wg".to_string()));
    }
}

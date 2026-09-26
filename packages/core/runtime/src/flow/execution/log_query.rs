//! Structured queries over one run's log table.
//!
//! Callers describe what they want as a [`LogQuery`]; this module turns it into
//! a Lance predicate with every value bound through
//! [`bind_filter_params`], so no caller ever writes filter syntax. Pages come
//! back oldest first: row ids are sorted by `start` on a scan that reads only
//! the `start` column plus the filtered columns, then just the page's rows are
//! taken whole.

use flow_like_storage::arrow_array::{Array, UInt64Array};
use flow_like_storage::databases::lance_filter_params::bind_filter_params;
use flow_like_storage::lance::dataset::scanner::ColumnOrdering;
use flow_like_storage::lancedb::Table;
use flow_like_storage::lancedb::query::{ExecutableQuery, QueryBase};
use flow_like_storage::serde_arrow;
use flow_like_types::{Result, Value, anyhow};
use futures::TryStreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::log::{LogMessage, StoredLogMessage};
use super::log_summary::{LogSummary, LogSummaryBuilder, fingerprint};

pub const MAX_PAGE_SIZE: usize = 1_000;
const MAX_LIST_ENTRIES: usize = 256;
const MAX_PHRASE_CHARS: usize = 512;
const LEGACY_SUMMARY_ROW_CAP: usize = 250_000;
const FINGERPRINT_COLUMN: &str = "fingerprint";
const ROW_ID_COLUMN: &str = "_rowid";

fn null_as_empty<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LogFold {
    pub fingerprint: String,
    pub first_start: u64,
}

/// Mirrors `ILogQuery` in `packages/ui/lib/schema/flow/log-query.ts`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(default)]
pub struct LogQuery {
    #[serde(deserialize_with = "null_as_empty")]
    pub levels: Vec<u8>,
    #[serde(deserialize_with = "null_as_empty")]
    pub exclude_levels: Vec<u8>,
    #[serde(deserialize_with = "null_as_empty")]
    pub nodes: Vec<String>,
    #[serde(deserialize_with = "null_as_empty")]
    pub exclude_nodes: Vec<String>,
    #[serde(deserialize_with = "null_as_empty")]
    pub text: Vec<String>,
    #[serde(deserialize_with = "null_as_empty")]
    pub exclude_text: Vec<String>,
    #[serde(deserialize_with = "null_as_empty")]
    pub fingerprints: Vec<String>,
    #[serde(deserialize_with = "null_as_empty")]
    pub exclude_fingerprints: Vec<String>,
    pub from: Option<u64>,
    pub to: Option<u64>,
    #[serde(deserialize_with = "null_as_empty")]
    pub fold: Vec<LogFold>,
}

struct FilterBuilder {
    clauses: Vec<String>,
    params: Vec<(String, Value)>,
}

impl FilterBuilder {
    fn param(&mut self, value: Value) -> String {
        let name = format!("p{}", self.params.len());
        self.params.push((name.clone(), value));
        format!("${name}")
    }

    fn list<T: Into<Value> + Clone>(&mut self, values: &[T]) -> String {
        self.param(Value::Array(
            values.iter().cloned().map(Into::into).collect(),
        ))
    }
}

/// A `LIKE` pattern matching `phrase` anywhere. `%`, `_` and `\` in the phrase are
/// escaped for the pattern matcher; quoting is left to the binder.
fn contains_pattern(phrase: &str) -> String {
    let mut pattern = String::with_capacity(phrase.len() + 2);
    pattern.push('%');
    for c in phrase.chars() {
        if matches!(c, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

fn check_list<T>(field: &str, values: &[T]) -> Result<()> {
    if values.len() > MAX_LIST_ENTRIES {
        return Err(anyhow!(
            "Log query field `{field}` has {} entries; at most {MAX_LIST_ENTRIES} are allowed",
            values.len()
        ));
    }
    Ok(())
}

impl LogQuery {
    fn validate(&self) -> Result<()> {
        check_list("levels", &self.levels)?;
        check_list("exclude_levels", &self.exclude_levels)?;
        check_list("nodes", &self.nodes)?;
        check_list("exclude_nodes", &self.exclude_nodes)?;
        check_list("text", &self.text)?;
        check_list("exclude_text", &self.exclude_text)?;
        check_list("fingerprints", &self.fingerprints)?;
        check_list("exclude_fingerprints", &self.exclude_fingerprints)?;
        check_list("fold", &self.fold)?;
        if let Some(level) = self
            .levels
            .iter()
            .chain(&self.exclude_levels)
            .find(|level| **level > 4)
        {
            return Err(anyhow!(
                "Log query level {level} is out of range; levels are 0 (Debug) to 4 (Fatal)"
            ));
        }
        if let Some(phrase) = self
            .text
            .iter()
            .chain(&self.exclude_text)
            .find(|phrase| phrase.chars().count() > MAX_PHRASE_CHARS)
        {
            return Err(anyhow!(
                "Log query phrase starting with {:?} is longer than {MAX_PHRASE_CHARS} characters",
                phrase.chars().take(40).collect::<String>()
            ));
        }
        Ok(())
    }

    /// Whether the query can only match nothing on a table without fingerprints.
    fn requires_fingerprints(&self) -> bool {
        !self.fingerprints.is_empty()
    }

    /// The bound Lance predicate, or `None` when the query matches every row.
    /// Without a fingerprint column (runs recorded before fingerprints), the
    /// fingerprint exclusions and folds are dropped.
    pub fn to_filter(&self, has_fingerprints: bool) -> Result<Option<String>> {
        self.validate()?;
        let mut builder = FilterBuilder {
            clauses: Vec::new(),
            params: Vec::new(),
        };

        if !self.levels.is_empty() {
            let list = builder.list(&self.levels);
            builder.clauses.push(format!("log_level IN ({list})"));
        }
        if !self.exclude_levels.is_empty() {
            let list = builder.list(&self.exclude_levels);
            builder.clauses.push(format!("log_level NOT IN ({list})"));
        }

        let (named, without_node): (Vec<&String>, Vec<&String>) =
            self.nodes.iter().partition(|node| !node.is_empty());
        if !named.is_empty() || !without_node.is_empty() {
            let mut alternatives = Vec::new();
            if !named.is_empty() {
                let named = named.into_iter().cloned().collect::<Vec<_>>();
                let list = builder.list(&named);
                alternatives.push(format!("node_id IN ({list})"));
            }
            if !without_node.is_empty() {
                alternatives.push("node_id IS NULL".to_string());
            }
            builder
                .clauses
                .push(format!("({})", alternatives.join(" OR ")));
        }
        let (named, without_node): (Vec<&String>, Vec<&String>) =
            self.exclude_nodes.iter().partition(|node| !node.is_empty());
        if !named.is_empty() {
            let named = named.into_iter().cloned().collect::<Vec<_>>();
            let list = builder.list(&named);
            let keep_unassigned = if without_node.is_empty() {
                "node_id IS NULL OR "
            } else {
                ""
            };
            builder
                .clauses
                .push(format!("({keep_unassigned}node_id NOT IN ({list}))"));
        } else if !without_node.is_empty() {
            builder.clauses.push("node_id IS NOT NULL".to_string());
        }

        for phrase in &self.text {
            let pattern = builder.param(Value::String(contains_pattern(phrase)));
            builder.clauses.push(format!("message ILIKE {pattern}"));
        }
        for phrase in &self.exclude_text {
            let pattern = builder.param(Value::String(contains_pattern(phrase)));
            builder
                .clauses
                .push(format!("NOT (message ILIKE {pattern})"));
        }

        if let Some(from) = self.from {
            let from = builder.param(Value::from(from));
            builder.clauses.push(format!("start >= {from}"));
        }
        if let Some(to) = self.to {
            let to = builder.param(Value::from(to));
            builder.clauses.push(format!("start <= {to}"));
        }

        if has_fingerprints {
            if !self.fingerprints.is_empty() {
                let list = builder.list(&self.fingerprints);
                builder
                    .clauses
                    .push(format!("{FINGERPRINT_COLUMN} IN ({list})"));
            }
            if !self.exclude_fingerprints.is_empty() {
                let list = builder.list(&self.exclude_fingerprints);
                builder.clauses.push(format!(
                    "({FINGERPRINT_COLUMN} IS NULL OR {FINGERPRINT_COLUMN} NOT IN ({list}))"
                ));
            }
            if !self.fold.is_empty() {
                let folded = self
                    .fold
                    .iter()
                    .map(|fold| fold.fingerprint.clone())
                    .collect::<Vec<_>>();
                let list = builder.list(&folded);
                let mut alternatives = vec![format!(
                    "{FINGERPRINT_COLUMN} IS NULL OR {FINGERPRINT_COLUMN} NOT IN ({list})"
                )];
                for fold in &self.fold {
                    let fingerprint = builder.param(Value::String(fold.fingerprint.clone()));
                    let first = builder.param(Value::from(fold.first_start));
                    alternatives.push(format!(
                        "({FINGERPRINT_COLUMN} = {fingerprint} AND start = {first})"
                    ));
                }
                builder
                    .clauses
                    .push(format!("({})", alternatives.join(" OR ")));
            }
        }

        if builder.clauses.is_empty() {
            return Ok(None);
        }
        let filter = builder
            .clauses
            .iter()
            .map(|clause| format!("({clause})"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let bound = bind_filter_params(&filter, &builder.params)
            .map_err(|e| anyhow!("Failed to bind the log filter {filter:?}: {e}"))?;
        Ok(Some(bound))
    }
}

pub async fn has_fingerprint_column(table: &Table) -> Result<bool> {
    let schema = table.schema().await?;
    Ok(schema.field_with_name(FINGERPRINT_COLUMN).is_ok())
}

fn stored_to_messages(stored: Vec<StoredLogMessage>) -> Vec<LogMessage> {
    stored.into_iter().map(LogMessage::from).collect()
}

/// One page of logs, oldest first.
pub async fn query_log_page(
    table: &Table,
    query: &LogQuery,
    offset: usize,
    limit: usize,
) -> Result<Vec<LogMessage>> {
    let limit = limit.clamp(1, MAX_PAGE_SIZE);
    let has_fingerprints = has_fingerprint_column(table).await?;
    if query.requires_fingerprints() && !has_fingerprints {
        return Ok(Vec::new());
    }
    let filter = query.to_filter(has_fingerprints)?;

    let Some(wrapper) = table.dataset() else {
        let mut q = table.query();
        if let Some(filter) = &filter {
            q = q.only_if(filter);
        }
        let batches = q
            .offset(offset)
            .limit(limit)
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        let mut stored = Vec::new();
        for batch in batches {
            stored.extend(serde_arrow::from_record_batch::<Vec<StoredLogMessage>>(
                &batch,
            )?);
        }
        stored.sort_by_key(|log| log.start);
        return Ok(stored_to_messages(stored));
    };

    let dataset = wrapper.get().await?;
    let mut scanner = dataset.scan();
    scanner.project(&["start"])?;
    scanner.with_row_id();
    if let Some(filter) = &filter {
        scanner.filter(filter)?;
    }
    scanner.order_by(Some(vec![ColumnOrdering::asc_nulls_first(
        "start".to_string(),
    )]))?;
    scanner.limit(Some(limit as i64), Some(offset as i64))?;
    let ids_batch = scanner.try_into_batch().await?;
    let ids = ids_batch
        .column_by_name(ROW_ID_COLUMN)
        .and_then(|column| column.as_any().downcast_ref::<UInt64Array>())
        .ok_or_else(|| anyhow!("The log scan returned no row id column"))?;
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids = ids.values().to_vec();

    let rows = dataset
        .take_rows(&ids, dataset.schema().clone())
        .await
        .map_err(|e| anyhow!("Failed to read {} log rows by id: {e}", ids.len()))?;
    let mut stored = serde_arrow::from_record_batch::<Vec<StoredLogMessage>>(&rows)?;
    stored.sort_by_key(|log| log.start);
    Ok(stored_to_messages(stored))
}

pub async fn count_logs(table: &Table, query: &LogQuery) -> Result<usize> {
    let has_fingerprints = has_fingerprint_column(table).await?;
    if query.requires_fingerprints() && !has_fingerprints {
        return Ok(0);
    }
    Ok(table.count_rows(query.to_filter(has_fingerprints)?).await?)
}

#[derive(Deserialize)]
struct SummaryRow {
    message: String,
    node_id: Option<String>,
    log_level: u8,
    start: u64,
}

/// A summary computed from the table, for runs recorded without a summary
/// sidecar. Reads at most [`LEGACY_SUMMARY_ROW_CAP`] rows; beyond that the
/// counts are lower bounds and `partial` is set.
pub async fn scan_log_summary(table: &Table, visited: Option<Vec<String>>) -> Result<LogSummary> {
    let fingerprinted = has_fingerprint_column(table).await?;
    let total = table.count_rows(None).await?;
    let mut stream = table
        .query()
        .select(flow_like_storage::lancedb::query::Select::columns(&[
            "message",
            "node_id",
            "log_level",
            "start",
        ]))
        .limit(LEGACY_SUMMARY_ROW_CAP)
        .execute()
        .await?;

    let mut builder = LogSummaryBuilder::default();
    while let Some(batch) = stream.try_next().await? {
        for row in serde_arrow::from_record_batch::<Vec<SummaryRow>>(&batch)? {
            let fp = fingerprint(row.node_id.as_deref(), row.log_level, &row.message);
            builder.record(
                row.node_id.as_deref(),
                row.log_level,
                row.start,
                &row.message,
                Some(&fp),
            );
        }
    }

    let mut summary = builder.finish(fingerprinted, visited);
    summary.partial = total > LEGACY_SUMMARY_ROW_CAP;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::execution::LogLevel;
    use flow_like_storage::arrow_array::{RecordBatchIterator, RecordBatchReader};
    use flow_like_storage::lancedb;
    use std::time::{Duration, SystemTime};

    fn log(node: &str, level: LogLevel, message: &str, micros: u64) -> LogMessage {
        let mut log = LogMessage::new(message, level, None);
        log.node_id = Some(node.to_string());
        log.start = SystemTime::UNIX_EPOCH + Duration::from_micros(micros);
        log.end = log.start;
        log
    }

    fn sample_logs() -> Vec<LogMessage> {
        let mut logs = vec![
            log("event", LogLevel::Info, "payload received", 10),
            log("http", LogLevel::Info, "status: 200 OK\nbody: 50% done", 20),
            log("print", LogLevel::Warn, "it's quoted", 30),
        ];
        for i in 0..5u64 {
            logs.push(log(
                "loop",
                LogLevel::Error,
                &format!("Error: failed in iteration {i}"),
                1_000 + i * 10,
            ));
        }
        logs.push(log("print", LogLevel::Debug, "under_score and 50% off", 5));
        logs
    }

    async fn table_with(batches: Vec<Vec<LogMessage>>) -> Table {
        let db = lancedb::connect(&format!("memory://logs-{}", flow_like_types::create_id()))
            .execute()
            .await
            .unwrap();
        let mut table: Option<Table> = None;
        for logs in batches {
            let batch = LogMessage::into_arrow(logs).unwrap();
            let schema = batch.schema();
            let reader: Box<dyn RecordBatchReader + Send> =
                Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema));
            match &table {
                None => {
                    table = Some(db.create_table("run", reader).execute().await.unwrap());
                }
                Some(table) => {
                    table.add(reader).execute().await.unwrap();
                }
            }
        }
        table.unwrap()
    }

    fn messages(logs: &[LogMessage]) -> Vec<&str> {
        logs.iter().map(|log| log.message.as_str()).collect()
    }

    #[test]
    fn empty_query_matches_everything() {
        assert_eq!(LogQuery::default().to_filter(true).unwrap(), None);
    }

    #[test]
    fn values_are_bound_as_literals() {
        let query = LogQuery {
            nodes: vec!["a' OR true --".into()],
            ..LogQuery::default()
        };
        let filter = query.to_filter(true).unwrap().unwrap();
        assert!(filter.contains("'a'' OR true --'"), "{filter}");
    }

    #[test]
    fn rejects_out_of_range_levels_and_oversized_lists() {
        let bad_level = LogQuery {
            levels: vec![9],
            ..LogQuery::default()
        };
        assert!(bad_level.to_filter(true).is_err());
        let too_many = LogQuery {
            nodes: (0..300).map(|i| i.to_string()).collect(),
            ..LogQuery::default()
        };
        assert!(too_many.to_filter(true).is_err());
    }

    #[test]
    fn null_lists_deserialize_as_empty() {
        let query: LogQuery =
            flow_like_types::json::from_str(r#"{"levels":null,"text":["x"],"from":null}"#).unwrap();
        assert!(query.levels.is_empty());
        assert_eq!(query.text, vec!["x".to_string()]);
    }

    #[tokio::test]
    async fn pages_come_back_oldest_first_across_flushes() {
        let logs = sample_logs();
        let (late, early) = logs.split_at(4);
        let table = table_with(vec![late.to_vec(), early.to_vec()]).await;

        let page = query_log_page(&table, &LogQuery::default(), 0, 4)
            .await
            .unwrap();
        assert_eq!(
            messages(&page),
            vec![
                "under_score and 50% off",
                "payload received",
                "status: 200 OK\nbody: 50% done",
                "it's quoted"
            ]
        );
        let next = query_log_page(&table, &LogQuery::default(), 4, 100)
            .await
            .unwrap();
        assert_eq!(next.len(), 5);
        assert!(next.iter().all(|log| log.fingerprint.is_some()));
    }

    #[tokio::test]
    async fn filters_levels_nodes_text_and_time() {
        let table = table_with(vec![sample_logs()]).await;

        let errors = LogQuery {
            levels: vec![3, 4],
            ..LogQuery::default()
        };
        assert_eq!(count_logs(&table, &errors).await.unwrap(), 5);

        let not_loop = LogQuery {
            exclude_nodes: vec!["loop".into()],
            ..LogQuery::default()
        };
        assert_eq!(count_logs(&table, &not_loop).await.unwrap(), 4);

        let quoted = LogQuery {
            text: vec!["IT'S".into()],
            ..LogQuery::default()
        };
        let page = query_log_page(&table, &quoted, 0, 10).await.unwrap();
        assert_eq!(messages(&page), vec!["it's quoted"]);

        let literal_percent = LogQuery {
            text: vec!["50%".into()],
            ..LogQuery::default()
        };
        assert_eq!(count_logs(&table, &literal_percent).await.unwrap(), 2);

        let literal_underscore = LogQuery {
            text: vec!["r_s".into()],
            ..LogQuery::default()
        };
        assert_eq!(count_logs(&table, &literal_underscore).await.unwrap(), 1);
        let wildcard_underscore = LogQuery {
            text: vec!["d_r".into()],
            ..LogQuery::default()
        };
        assert_eq!(count_logs(&table, &wildcard_underscore).await.unwrap(), 0);

        let hide_iterations = LogQuery {
            exclude_text: vec!["in iteration".into()],
            ..LogQuery::default()
        };
        assert_eq!(count_logs(&table, &hide_iterations).await.unwrap(), 4);

        let window = LogQuery {
            from: Some(20),
            to: Some(1_010),
            ..LogQuery::default()
        };
        assert_eq!(count_logs(&table, &window).await.unwrap(), 4);
    }

    #[tokio::test]
    async fn folding_keeps_the_first_occurrence_of_each_group() {
        let table = table_with(vec![sample_logs()]).await;
        let summary = scan_log_summary(&table, None).await.unwrap();
        let group = summary
            .groups
            .iter()
            .find(|group| group.count == 5)
            .unwrap()
            .clone();
        assert_eq!(group.template, "Error: failed in iteration ⟨n⟩");

        let folded = LogQuery {
            fold: vec![LogFold {
                fingerprint: group.fingerprint.clone(),
                first_start: group.first_start,
            }],
            ..LogQuery::default()
        };
        let page = query_log_page(&table, &folded, 0, 100).await.unwrap();
        assert_eq!(page.len(), 5);
        assert_eq!(page.last().unwrap().message, "Error: failed in iteration 0");
        assert_eq!(count_logs(&table, &folded).await.unwrap(), 5);

        let only_group = LogQuery {
            fingerprints: vec![group.fingerprint.clone()],
            ..LogQuery::default()
        };
        assert_eq!(count_logs(&table, &only_group).await.unwrap(), 5);
        let hide_group = LogQuery {
            exclude_fingerprints: vec![group.fingerprint],
            ..LogQuery::default()
        };
        assert_eq!(count_logs(&table, &hide_group).await.unwrap(), 4);
    }

    #[tokio::test]
    async fn summary_scan_counts_levels_nodes_and_first_error() {
        let table = table_with(vec![sample_logs()]).await;
        let summary = scan_log_summary(&table, Some(vec!["db".into()]))
            .await
            .unwrap();
        assert!(summary.fingerprinted);
        assert!(!summary.partial);
        assert_eq!(summary.total, 9);
        assert_eq!(summary.levels, [1, 2, 1, 5, 0]);
        assert_eq!(summary.nodes["loop"], [0, 0, 0, 5, 0]);
        assert_eq!(summary.first_error.unwrap().start, 1_000);
        assert_eq!(summary.visited, Some(vec!["db".to_string()]));
    }

    #[tokio::test]
    async fn tables_without_a_fingerprint_column_still_read() {
        #[derive(Serialize)]
        struct LegacyRow {
            message: String,
            operation_id: Option<String>,
            node_id: Option<String>,
            log_level: u8,
            token_in: Option<u64>,
            token_out: Option<u64>,
            bit_ids: Option<Vec<String>>,
            start: u64,
            end: u64,
        }
        use flow_like_storage::serde_arrow::schema::{SchemaLike, TracingOptions};
        let rows = (0..3u64)
            .map(|i| LegacyRow {
                message: format!("legacy {i}"),
                operation_id: None,
                node_id: Some("n".into()),
                log_level: 3,
                token_in: None,
                token_out: None,
                bit_ids: None,
                start: 100 - i,
                end: 100 - i,
            })
            .collect::<Vec<_>>();
        let fields =
            Vec::<flow_like_storage::lancedb::arrow::arrow_schema::FieldRef>::from_samples(
                &rows,
                TracingOptions::default().allow_null_fields(true),
            )
            .unwrap();
        let batch = serde_arrow::to_record_batch(&fields, &rows).unwrap();
        let schema = batch.schema();
        let db = lancedb::connect(&format!("memory://legacy-{}", flow_like_types::create_id()))
            .execute()
            .await
            .unwrap();
        let reader: Box<dyn RecordBatchReader + Send> =
            Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema));
        let table = db.create_table("run", reader).execute().await.unwrap();

        assert!(!has_fingerprint_column(&table).await.unwrap());
        let page = query_log_page(&table, &LogQuery::default(), 0, 10)
            .await
            .unwrap();
        assert_eq!(messages(&page), vec!["legacy 2", "legacy 1", "legacy 0"]);
        assert!(page.iter().all(|log| log.fingerprint.is_none()));

        let folded = LogQuery {
            fold: vec![LogFold {
                fingerprint: "ffff".into(),
                first_start: 0,
            }],
            ..LogQuery::default()
        };
        assert_eq!(count_logs(&table, &folded).await.unwrap(), 3);
        let only_group = LogQuery {
            fingerprints: vec!["ffff".into()],
            ..LogQuery::default()
        };
        assert_eq!(count_logs(&table, &only_group).await.unwrap(), 0);

        let summary = scan_log_summary(&table, None).await.unwrap();
        assert!(!summary.fingerprinted);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.groups[0].count, 3);
    }
}

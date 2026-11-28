//! Home data widgets over the tables of several local-only apps. Online apps run the same
//! query on the hub (`POST /user/data/query`); this is its local counterpart, with the
//! same row, size and cache rules and an in-process result cache.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use flow_like::flow_like_storage::{
    contracts::column_metadata::ColumnReader,
    databases::workbench::{
        CachedCrossAppResult, CrossAppSources, SqlQueryResult, WorkbenchLimits, WorkbenchSchema,
        describe_cross_app_query, execute_readonly_sql_across_apps,
    },
};
use flow_like_types::Value;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::AppHandle;

use super::graph::graph_connection;
use crate::functions::TauriFunctionError;
use flow_like::flow_like_storage::databases::workbench::{
    parameter_options_answer, parameter_options_query,
};

const SOFT_SOURCES: usize = 8;
const MAX_SOURCES: usize = 32;
const MAX_SQL_BYTES: usize = 128 * 1024;
const MAX_PARAMS: usize = 128;
const DEFAULT_ROWS: usize = 500;
const MAX_ROWS: usize = 5_000;
const MAX_CACHE_SECONDS: u64 = 86_400;
const MAX_CACHED_RESULTS: usize = 64;
const MEMORY_BYTES: usize = 256 * 1024 * 1024;

static RESULTS: LazyLock<Mutex<HashMap<String, CachedCrossAppResult>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Deserialize)]
pub struct CrossAppSourceInput {
    pub alias: String,
    pub app_id: String,
    #[serde(default)]
    pub scope: Option<String>,
    /// External databases are read by the hub's query service, never by a local-only app.
    #[serde(default)]
    pub source_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CrossAppQueryInput {
    pub sql: String,
    pub sources: Vec<CrossAppSourceInput>,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub cache_seconds: Option<u64>,
    #[serde(default)]
    pub fresh: bool,
}

#[derive(Debug, Deserialize)]
pub struct CrossAppDescribeInput {
    pub sql: String,
    pub sources: Vec<CrossAppSourceInput>,
}

#[derive(Debug, Deserialize)]
pub struct CrossAppOptionsInput {
    pub sources: Vec<CrossAppSourceInput>,
    /// `alias.table` the parameter is compared with.
    pub table: String,
    pub column: String,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub range: bool,
}

#[derive(Debug, Serialize)]
struct CrossAppQueryOutput {
    #[serde(flatten)]
    result: SqlQueryResult,
    tables: Vec<String>,
    computed_at: i64,
    cached: bool,
    expires_at: Option<i64>,
    warnings: Vec<String>,
}

impl CrossAppQueryOutput {
    fn new(
        entry: CachedCrossAppResult,
        cached: bool,
        ttl: Duration,
        warnings: Vec<String>,
    ) -> Self {
        Self {
            result: entry.result,
            tables: entry.tables,
            computed_at: entry.computed_at,
            cached,
            expires_at: (!ttl.is_zero()).then_some(entry.expires_at),
            warnings,
        }
    }
}

fn personal(source: &CrossAppSourceInput) -> flow_like_types::Result<bool> {
    match source.scope.as_deref() {
        None | Some("project") => Ok(false),
        Some("personal") => Ok(true),
        Some(other) => Err(flow_like_types::anyhow!(
            "Unknown scope '{other}' for app source '{}'; use 'project' or 'personal'",
            source.alias
        )),
    }
}

fn validate_sources(
    sources: &[CrossAppSourceInput],
    warnings: &mut Vec<String>,
) -> flow_like_types::Result<()> {
    if sources.is_empty() || sources.len() > MAX_SOURCES {
        return Err(flow_like_types::anyhow!(
            "A query reads between 1 and {MAX_SOURCES} app sources, not {}",
            sources.len()
        ));
    }
    for source in sources {
        personal(source)?;
        if source.source_id.is_some() {
            return Err(flow_like_types::anyhow!(
                "App source '{}' reads an external database, which only online apps can query",
                source.alias
            ));
        }
    }
    if sources.len() > SOFT_SOURCES {
        warnings.push(format!(
            "This query reads {} apps; queries over more than {SOFT_SOURCES} apps are slower and time out sooner",
            sources.len()
        ));
    }
    Ok(())
}

fn validate_sql(sql: &str) -> flow_like_types::Result<()> {
    if sql.len() > MAX_SQL_BYTES {
        return Err(flow_like_types::anyhow!(
            "The SQL is larger than {} KiB",
            MAX_SQL_BYTES / 1024
        ));
    }
    Ok(())
}

/// The hard limits of a request; returns its row limit and the soft limits it passed.
fn validate(input: &CrossAppQueryInput) -> flow_like_types::Result<(usize, Vec<String>)> {
    let mut warnings = Vec::new();
    validate_sources(&input.sources, &mut warnings)?;
    validate_sql(&input.sql)?;
    if let Some(Value::Object(params)) = &input.params
        && params.len() > MAX_PARAMS
    {
        return Err(flow_like_types::anyhow!(
            "A query takes at most {MAX_PARAMS} parameters, not {}",
            params.len()
        ));
    }
    let rows = input.limit.unwrap_or(DEFAULT_ROWS).max(1);
    if rows > MAX_ROWS {
        return Err(flow_like_types::anyhow!(
            "A query returns at most {MAX_ROWS} rows, not {rows}"
        ));
    }
    if rows > DEFAULT_ROWS {
        warnings.push(format!(
            "This query may return up to {rows} rows; widgets load faster with at most {DEFAULT_ROWS}"
        ));
    }
    Ok((rows, warnings))
}

async fn open_schemas<'a>(
    app_handle: &AppHandle,
    sources: impl IntoIterator<Item = &'a CrossAppSourceInput>,
) -> flow_like_types::Result<Vec<WorkbenchSchema>> {
    let mut schemas = Vec::new();
    for source in sources {
        schemas.push(WorkbenchSchema {
            name: source.alias.clone(),
            connection: graph_connection(app_handle, &source.app_id, personal(source)?).await?,
            reader: ColumnReader::Unrestricted,
        });
    }
    Ok(schemas)
}

fn cache_key(input: &CrossAppQueryInput, params: &Value, limit: usize) -> String {
    let mut sources = input
        .sources
        .iter()
        .map(|source| {
            json!({
                "alias": source.alias,
                "app_id": source.app_id,
                "personal": source.scope.as_deref() == Some("personal"),
            })
        })
        .collect::<Vec<_>>();
    sources.sort_by(|a, b| a["alias"].as_str().cmp(&b["alias"].as_str()));
    let request = json!({ "sql": input.sql, "params": params, "limit": limit, "sources": sources });
    blake3::hash(request.to_string().as_bytes())
        .to_hex()
        .to_string()
}

fn cached(key: &str) -> Option<CachedCrossAppResult> {
    RESULTS.lock().ok()?.get(key).cloned()
}

fn remember(key: String, entry: CachedCrossAppResult) {
    let Ok(mut results) = RESULTS.lock() else {
        return;
    };
    let now = chrono::Utc::now().timestamp_millis();
    results.retain(|_, held| held.expires_at > now);
    while results.len() >= MAX_CACHED_RESULTS {
        let Some(oldest) = results
            .iter()
            .min_by_key(|(_, held)| held.computed_at)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        results.remove(&oldest);
    }
    results.insert(key, entry);
}

#[tauri::command(async)]
pub async fn query_execute_cross_app(
    app_handle: AppHandle,
    payload: CrossAppQueryInput,
) -> Result<serde_json::Value, TauriFunctionError> {
    let (limit, warnings) = validate(&payload)?;
    let schemas = open_schemas(&app_handle, &payload.sources).await?;

    let params = payload.params.clone().unwrap_or(Value::Null);
    let ttl = Duration::from_secs(payload.cache_seconds.unwrap_or(0).min(MAX_CACHE_SECONDS));
    let key = cache_key(&payload, &params, limit);

    if !ttl.is_zero() && !payload.fresh {
        let now = chrono::Utc::now().timestamp_millis();
        if let Some(entry) = cached(&key)
            && entry.is_current(&schemas, now).await
        {
            return Ok(serde_json::to_value(CrossAppQueryOutput::new(
                entry, true, ttl, warnings,
            ))?);
        }
    }

    let computed_at = chrono::Utc::now().timestamp_millis();
    let outcome = execute_readonly_sql_across_apps(
        &schemas,
        &payload.sql,
        &params,
        WorkbenchLimits {
            rows: Some(limit),
            memory_bytes: Some(MEMORY_BYTES),
        },
    )
    .await?;
    let entry = CachedCrossAppResult::new(outcome, computed_at, ttl);
    if !ttl.is_zero() {
        remember(key, entry.clone());
    }
    Ok(serde_json::to_value(CrossAppQueryOutput::new(
        entry, false, ttl, warnings,
    ))?)
}

/// Output columns and `$parameters` of cross-app SQL over local-only apps, without running it.
#[tauri::command(async)]
pub async fn query_describe_cross_app(
    app_handle: AppHandle,
    payload: CrossAppDescribeInput,
) -> Result<serde_json::Value, TauriFunctionError> {
    validate_sources(&payload.sources, &mut Vec::new())?;
    validate_sql(&payload.sql)?;
    let schemas = open_schemas(&app_handle, &payload.sources).await?;
    let description = describe_cross_app_query(
        CrossAppSources {
            lance: &schemas,
            external: &[],
        },
        &payload.sql,
    )
    .await?;
    Ok(serde_json::to_value(description)?)
}

/// The source an options request reads: the one whose alias qualifies its table.
fn option_source<'a>(
    sources: &'a [CrossAppSourceInput],
    table: &str,
) -> flow_like_types::Result<&'a CrossAppSourceInput> {
    let (alias, _) = table.split_once('.').ok_or_else(|| {
        flow_like_types::anyhow!("Name the table as <alias>.<table>, not '{table}'")
    })?;
    sources
        .iter()
        .find(|source| source.alias == alias)
        .ok_or_else(|| flow_like_types::anyhow!("No app source is named '{alias}'"))
}

/// Values (or the range) a cross-app `$parameter` bound to `alias.table.column` can take.
#[tauri::command(async)]
pub async fn query_cross_app_parameter_options(
    app_handle: AppHandle,
    payload: CrossAppOptionsInput,
) -> Result<serde_json::Value, TauriFunctionError> {
    validate_sources(&payload.sources, &mut Vec::new())?;
    let source = option_source(&payload.sources, &payload.table)?;
    let options = parameter_options_query(
        &payload.table,
        &payload.column,
        payload.search.as_deref(),
        payload.range,
    )?;
    let schemas = open_schemas(&app_handle, [source]).await?;
    let outcome = execute_readonly_sql_across_apps(
        &schemas,
        &options.sql,
        &options.params,
        WorkbenchLimits {
            rows: Some(options.rows),
            memory_bytes: Some(MEMORY_BYTES),
        },
    )
    .await?;
    Ok(parameter_options_answer(outcome.result, payload.range))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(sources: Vec<(&str, &str, Option<&str>)>) -> CrossAppQueryInput {
        CrossAppQueryInput {
            sql: "SELECT 1".to_string(),
            sources: sources
                .into_iter()
                .map(|(alias, app_id, scope)| CrossAppSourceInput {
                    alias: alias.to_string(),
                    app_id: app_id.to_string(),
                    scope: scope.map(str::to_string),
                    source_id: None,
                })
                .collect(),
            params: None,
            limit: None,
            cache_seconds: None,
            fresh: false,
        }
    }

    #[test]
    fn validates_source_count_scope_and_length() {
        assert!(validate(&input(vec![("crm", "a", Some("personal"))])).is_ok());
        assert!(validate(&input(vec![])).is_err());
        assert!(validate(&input(vec![("crm", "a", Some("team"))])).is_err());
        let mut long = input(vec![("crm", "a", None)]);
        long.sql = "x".repeat(MAX_SQL_BYTES + 1);
        assert!(validate(&long).is_err());
        let mut external = input(vec![("warehouse", "a", None)]);
        external.sources[0].source_id = Some("source-1".to_string());
        let error = validate(&external).unwrap_err().to_string();
        assert!(error.contains("only online apps"), "{error}");
    }

    #[test]
    fn limits_match_the_hub() {
        let aliases: Vec<String> = (0..=MAX_SOURCES).map(|i| format!("s{i}")).collect();
        let sources = |count: usize| {
            aliases[..count]
                .iter()
                .map(|alias| (alias.as_str(), "a", None))
                .collect::<Vec<_>>()
        };
        let (rows, warnings) = validate(&input(sources(SOFT_SOURCES))).unwrap();
        assert_eq!(rows, DEFAULT_ROWS);
        assert!(warnings.is_empty());
        assert_eq!(
            validate(&input(sources(SOFT_SOURCES + 1))).unwrap().1.len(),
            1
        );
        assert!(validate(&input(sources(MAX_SOURCES + 1))).is_err());

        let mut tall = input(sources(1));
        tall.limit = Some(MAX_ROWS);
        let (rows, warnings) = validate(&tall).unwrap();
        assert_eq!(rows, MAX_ROWS);
        assert_eq!(warnings.len(), 1);
        tall.limit = Some(MAX_ROWS + 1);
        assert!(validate(&tall).is_err());

        let mut params = input(sources(1));
        params.params = Some(Value::Object(
            (0..=MAX_PARAMS)
                .map(|i| (format!("p{i}"), json!(i)))
                .collect(),
        ));
        assert!(validate(&params).is_err());
    }

    #[test]
    fn options_read_only_the_source_their_table_names() {
        let sources = input(vec![("crm", "a", None), ("shop", "b", None)]).sources;
        assert_eq!(option_source(&sources, "shop.orders").unwrap().app_id, "b");
        assert!(option_source(&sources, "orders").is_err());
        assert!(option_source(&sources, "hr.people").is_err());
    }

    #[test]
    fn cache_keys_ignore_source_order_but_not_scope() {
        let params = Value::Null;
        let forward = input(vec![("crm", "a", None), ("shop", "b", None)]);
        let reverse = input(vec![("shop", "b", None), ("crm", "a", None)]);
        assert_eq!(
            cache_key(&forward, &params, 10),
            cache_key(&reverse, &params, 10)
        );
        let personal = input(vec![("crm", "a", Some("personal")), ("shop", "b", None)]);
        assert_ne!(
            cache_key(&forward, &params, 10),
            cache_key(&personal, &params, 10)
        );
        assert_ne!(
            cache_key(&forward, &params, 10),
            cache_key(&forward, &params, 11)
        );
    }
}

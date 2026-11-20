//! One read-only SQL statement over the tables of several apps, for Home data widgets.
//!
//! Every source app is authorized on every request, before any cache lookup, and keeps its own
//! column restrictions. The query service runs the statement with read-only credentials minted
//! per source. Results are shared by identical requests from callers with identical access:
//! a fresh result is served as it is, an expired one is reused only after the service confirmed
//! that none of the tables it read changed, and a failed refresh falls back to the last result.

mod cache;
pub mod parameters;

use std::collections::HashSet;
use std::time::Duration;

use axum::{Extension, Json, extract::State};
use flow_like_query::{Operation, QueryShape, SourceSpec};
use flow_like_storage::contracts::column_metadata::ColumnReader;
use flow_like_storage::databases::workbench::SqlQueryResult;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;

use crate::{
    cache::best_effort,
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    query::{self, LanceRead},
    routes::app::db::ScopeParams,
    state::AppState,
};
use cache::{CachedQuery, Lookup, STALE_IF_ERROR, Settled};

const CACHE_NAMESPACE: &str = "home_data_query";
const CACHE_KEY_VERSION: u32 = 2;
/// Rows returned when a request names no limit; larger requests are allowed with a warning.
pub const DEFAULT_ROWS: usize = 500;
pub const MAX_ROWS: usize = 5_000;
/// Apps one query may read before it gets a warning, and at most.
pub const SOFT_SOURCES: usize = 8;
pub const MAX_SOURCES: usize = 32;
pub const MAX_SQL_BYTES: usize = 128 * 1024;
pub const MAX_PARAMS: usize = 128;
pub const MAX_CACHE_SECONDS: u64 = 86_400;
const RESERVED_ALIASES: [&str; 2] = ["public", "information_schema"];
const TOO_LARGE_TO_CACHE: &str = "The result is too large to cache, every load runs the query";

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DataQuerySource {
    /// Schema name the SQL uses for this app's tables, as in `crm.customers`.
    pub alias: String,
    pub app_id: String,
    /// `project` (default) or `personal` for the caller's own data in the app.
    #[serde(default)]
    pub scope: Option<String>,
    /// An external data source of the app to read instead of the app's own tables.
    #[serde(default)]
    pub source_id: Option<String>,
}

impl DataQuerySource {
    fn personal(&self) -> Result<bool, ApiError> {
        match self.scope.as_deref() {
            None | Some("project") => Ok(false),
            Some("personal") => Ok(true),
            Some(other) => Err(ApiError::bad_request(format!(
                "Unknown scope '{other}' for app source '{}'; use 'project' or 'personal'",
                self.alias
            ))),
        }
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DataQueryPayload {
    /// One read-only SELECT or WITH statement. Tables are addressed as `<alias>.<table>`.
    pub sql: String,
    pub sources: Vec<DataQuerySource>,
    /// Values for `$name` placeholders, keyed by name without the `$`.
    #[serde(default)]
    pub params: Option<Value>,
    /// Maximum rows returned: 500 when unset, at most 5,000.
    #[serde(default)]
    pub limit: Option<usize>,
    /// How long identical requests may reuse this result, at most one day. 0 disables caching.
    #[serde(default)]
    pub cache_seconds: Option<u64>,
    /// Skip any cached result and store the new one.
    #[serde(default)]
    pub fresh: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DataQueryResponse {
    #[serde(flatten)]
    #[schema(value_type = Object)]
    pub result: SqlQueryResult,
    /// `alias.table` of every scanned table.
    pub tables: Vec<String>,
    /// When the rows were computed, or last confirmed current, in epoch milliseconds.
    pub computed_at: i64,
    /// Whether these rows come from an earlier identical request.
    pub cached: bool,
    /// When a cached copy stops being reused, in epoch milliseconds.
    pub expires_at: Option<i64>,
    /// Soft limits this request passed and limits the query service lowered.
    pub warnings: Vec<String>,
    /// The refresh failed and these are the last rows; `warnings` says why.
    pub stale: bool,
}

/// The hard limits of a request, with the soft ones it passed as warnings.
#[derive(Debug, PartialEq, Eq)]
struct Checked {
    rows: usize,
    ttl: Duration,
    warnings: Vec<String>,
}

fn is_valid_alias(alias: &str) -> bool {
    alias.len() <= 63
        && alias
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first == '_')
        && alias
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !RESERVED_ALIASES.contains(&alias)
}

fn validate_sql(sql: &str) -> Result<(), ApiError> {
    if sql.trim().is_empty() {
        return Err(ApiError::bad_request("The SQL is empty"));
    }
    if sql.len() > MAX_SQL_BYTES {
        return Err(ApiError::bad_request(format!(
            "The SQL is larger than {} KiB",
            MAX_SQL_BYTES / 1024
        )));
    }
    Ok(())
}

/// Checks the sources every cross-app route takes and notes a soft limit they pass.
fn validate_sources(
    sources: &[DataQuerySource],
    warnings: &mut Vec<String>,
) -> Result<(), ApiError> {
    if sources.is_empty() || sources.len() > MAX_SOURCES {
        return Err(ApiError::bad_request(format!(
            "A query reads between 1 and {MAX_SOURCES} app sources, not {}",
            sources.len()
        )));
    }
    let mut aliases = HashSet::new();
    for source in sources {
        if !is_valid_alias(&source.alias) {
            return Err(ApiError::bad_request(format!(
                "'{}' is not a valid schema name: use at most 63 lowercase letters, digits and _, \
                 starting with a letter or _, and not 'public' or 'information_schema'",
                source.alias
            )));
        }
        if !aliases.insert(source.alias.as_str()) {
            return Err(ApiError::bad_request(format!(
                "The schema name '{}' is given to more than one app source",
                source.alias
            )));
        }
        if source.app_id.trim().is_empty() {
            return Err(ApiError::bad_request(format!(
                "App source '{}' has no app id",
                source.alias
            )));
        }
        source.personal()?;
        if source.source_id.is_some() {
            return Err(ApiError::bad_request("External sources are not wired yet"));
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

fn validate(payload: &DataQueryPayload) -> Result<Checked, ApiError> {
    let mut warnings = Vec::new();
    validate_sql(&payload.sql)?;
    validate_sources(&payload.sources, &mut warnings)?;
    match &payload.params {
        None | Some(Value::Null) => {}
        Some(Value::Object(params)) if params.len() <= MAX_PARAMS => {}
        Some(Value::Object(params)) => {
            return Err(ApiError::bad_request(format!(
                "A query takes at most {MAX_PARAMS} parameters, not {}",
                params.len()
            )));
        }
        Some(_) => {
            return Err(ApiError::bad_request(
                "Parameters must be an object of named values",
            ));
        }
    }
    let rows = payload.limit.unwrap_or(DEFAULT_ROWS).max(1);
    if rows > MAX_ROWS {
        return Err(ApiError::bad_request(format!(
            "A query returns at most {MAX_ROWS} rows, not {rows}"
        )));
    }
    if rows > DEFAULT_ROWS {
        warnings.push(format!(
            "This query may return up to {rows} rows; widgets load faster with at most {DEFAULT_ROWS}"
        ));
    }
    Ok(Checked {
        rows,
        ttl: Duration::from_secs(payload.cache_seconds.unwrap_or(0).min(MAX_CACHE_SECONDS)),
        warnings,
    })
}

fn reader_identity(reader: &ColumnReader) -> Value {
    match reader {
        ColumnReader::Unrestricted => Value::Null,
        ColumnReader::Attributes(attributes) => json!(attributes),
    }
}

/// One source after its permission check.
struct AuthorizedSource {
    alias: String,
    read: LanceRead,
}

impl AuthorizedSource {
    /// What a shared cache key needs: the same app, data owner and column visibility.
    fn identity(&self) -> Value {
        json!({
            "alias": self.alias,
            "app_id": self.read.app_id,
            "owner": self.read.owner,
            "reader": reader_identity(&self.read.reader),
        })
    }
}

/// Checks the caller may read the app, and which of its columns.
async fn authorize_source(
    state: &AppState,
    user: &AppUser,
    source: &DataQuerySource,
) -> Result<AuthorizedSource, ApiError> {
    let scope = ScopeParams {
        scope: source.personal()?.then(|| "user".to_string()),
    };
    let permission = ensure_any_permission!(
        user,
        &source.app_id,
        state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );
    Ok(AuthorizedSource {
        alias: source.alias.clone(),
        read: LanceRead::scoped(user, &permission, &source.app_id, &scope)?,
    })
}

async fn authorize_sources(
    state: &AppState,
    user: &AppUser,
    sources: &[DataQuerySource],
) -> Result<Vec<AuthorizedSource>, ApiError> {
    futures::future::try_join_all(
        sources
            .iter()
            .map(|source| authorize_source(state, user, source)),
    )
    .await
}

/// Every source as the query service receives it, each under its alias.
async fn source_specs(
    state: &AppState,
    sources: &[AuthorizedSource],
) -> Result<Vec<SourceSpec>, ApiError> {
    futures::future::try_join_all(sources.iter().map(|source| {
        state
            .query
            .lance_source(state, &source.read, Some(source.alias.clone()))
    }))
    .await
}

fn cache_key(sql: &str, params: &Value, limit: usize, sources: &[AuthorizedSource]) -> String {
    let mut identities: Vec<Value> = sources.iter().map(AuthorizedSource::identity).collect();
    identities.sort_by(|a, b| a["alias"].as_str().cmp(&b["alias"].as_str()));
    let request = json!({
        "v": CACHE_KEY_VERSION,
        "sql": sql,
        "params": params,
        "limit": limit,
        "sources": identities,
    });
    blake3::hash(request.to_string().as_bytes())
        .to_hex()
        .to_string()
}

fn respond(settled: Settled, ttl: Duration, mut warnings: Vec<String>) -> DataQueryResponse {
    let Settled {
        entry,
        cached,
        failure,
        ..
    } = settled;
    warnings.extend(entry.warnings);
    let stale = failure.is_some();
    warnings.extend(failure);
    DataQueryResponse {
        result: entry.result,
        tables: entry.tables,
        computed_at: entry.computed_at,
        cached,
        expires_at: (!ttl.is_zero()).then_some(entry.expires_at),
        warnings,
        stale,
    }
}

#[utoipa::path(
    post,
    path = "/user/data/query",
    tag = "user",
    description = "Run one read-only SQL query across the tables of several of your apps, as used by Home data widgets. Results may be reused for identical requests until they expire; an expired result is reused only while none of the tables it read changed, and stands in with a warning when a refresh fails.",
    request_body = DataQueryPayload,
    responses(
        (status = 200, description = "Query results with when they were computed, whether they came from the cache and any limit warnings", body = DataQueryResponse),
        (status = 400, description = "The query or its sources are invalid, or the query failed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "You cannot read one of the apps"),
        (status = 413, description = "The query needs more memory than the query service allows"),
        (status = 503, description = "The query service is unavailable"),
        (status = 504, description = "The query ran out of time")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "POST /user/data/query", skip_all)]
pub async fn data_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<DataQueryPayload>,
) -> Result<Json<DataQueryResponse>, ApiError> {
    let Checked {
        rows,
        ttl,
        mut warnings,
    } = validate(&payload)?;
    let sources = authorize_sources(&state, &user, &payload.sources).await?;
    let params = payload.params.clone().unwrap_or(Value::Null);
    let key = cache_key(&payload.sql, &params, rows, &sources);

    let now = chrono::Utc::now().timestamp_millis();
    let held = if ttl.is_zero() {
        None
    } else {
        best_effort::get::<CachedQuery>(&state.cache, CACHE_NAMESPACE, &key).await
    };
    let (held, reuse) = match cache::lookup(held, payload.fresh, now) {
        Lookup::Fresh(entry) => {
            let settled = Settled {
                entry,
                cached: true,
                failure: None,
                store: false,
            };
            return Ok(Json(respond(settled, ttl, warnings)));
        }
        Lookup::Refresh { held, reuse } => (held, reuse),
    };

    let outcome = match source_specs(&state, &sources).await {
        Ok(specs) => {
            let request = query::request(
                specs,
                rows,
                Operation::Sql {
                    sql: payload.sql,
                    params,
                    shape: QueryShape::CrossApp,
                    reuse_if_unchanged: reuse,
                },
            );
            state.query.run(&query::subject(&user), &request).await
        }
        Err(error) => Err(error),
    };
    let settled = cache::settle(held, outcome, now, ttl)?;
    if settled.store && !ttl.is_zero() && cacheable(&settled.entry, &mut warnings) {
        best_effort::set(
            &state.cache,
            CACHE_NAMESPACE,
            &key,
            &settled.entry,
            ttl + STALE_IF_ERROR,
        )
        .await;
    }
    Ok(Json(respond(settled, ttl, warnings)))
}

/// Whether `entry` fits the shared cache; a result too large for it is still answered.
fn cacheable(entry: &CachedQuery, warnings: &mut Vec<String>) -> bool {
    let fits = best_effort::fits(entry);
    if !fits {
        warnings.push(TOO_LARGE_TO_CACHE.to_string());
    }
    fits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(alias: &str, app_id: &str, scope: Option<&str>) -> DataQuerySource {
        DataQuerySource {
            alias: alias.to_string(),
            app_id: app_id.to_string(),
            scope: scope.map(str::to_string),
            source_id: None,
        }
    }

    fn payload(sources: Vec<DataQuerySource>) -> DataQueryPayload {
        DataQueryPayload {
            sql: "SELECT * FROM crm.customers".to_string(),
            sources,
            params: None,
            limit: None,
            cache_seconds: None,
            fresh: false,
        }
    }

    fn rejection(payload: &DataQueryPayload) -> String {
        format!(
            "{:?}",
            validate(payload).expect_err("payload must be rejected")
        )
    }

    fn many(count: usize) -> Vec<DataQuerySource> {
        (0..count)
            .map(|i| source(&format!("s{i}"), "a", None))
            .collect()
    }

    #[test]
    fn validates_sources_sql_and_params() {
        assert!(
            validate(&payload(vec![
                source("crm", "a", None),
                source("shop", "b", Some("personal")),
                source("_x9", "a", Some("project")),
            ]))
            .is_ok()
        );
        assert!(rejection(&payload(vec![])).contains("between 1 and 32"));
        assert!(rejection(&payload(many(MAX_SOURCES + 1))).contains("between 1 and 32"));
        for alias in ["CRM", "1crm", "crm-eu", "", "public", "information_schema"] {
            assert!(
                rejection(&payload(vec![source(alias, "a", None)]))
                    .contains("not a valid schema name"),
                "{alias}"
            );
        }
        assert!(
            rejection(&payload(vec![
                source("crm", "a", None),
                source("crm", "b", None)
            ]))
            .contains("more than one")
        );
        assert!(rejection(&payload(vec![source("crm", " ", None)])).contains("no app id"));
        assert!(
            rejection(&payload(vec![source("crm", "a", Some("team"))])).contains("Unknown scope")
        );
        let mut external = source("warehouse", "a", None);
        external.source_id = Some("source-1".to_string());
        assert!(rejection(&payload(vec![external])).contains("External sources are not wired yet"));

        let mut empty = payload(vec![source("crm", "a", None)]);
        empty.sql = "  ".to_string();
        assert!(rejection(&empty).contains("empty"));
        let mut long = payload(vec![source("crm", "a", None)]);
        long.sql = "x".repeat(MAX_SQL_BYTES + 1);
        assert!(rejection(&long).contains("128 KiB"));
        long.sql = "x".repeat(MAX_SQL_BYTES);
        assert!(validate(&long).is_ok());
        let mut listed = payload(vec![source("crm", "a", None)]);
        listed.params = Some(json!([1]));
        assert!(rejection(&listed).contains("object"));
        let mut params = payload(vec![source("crm", "a", None)]);
        params.params = Some(Value::Object(
            (0..=MAX_PARAMS)
                .map(|i| (format!("p{i}"), json!(i)))
                .collect(),
        ));
        assert!(rejection(&params).contains("at most 128"));
    }

    #[test]
    fn soft_limits_warn_and_hard_limits_refuse() {
        let quiet = validate(&payload(many(SOFT_SOURCES))).unwrap();
        assert_eq!(quiet.rows, DEFAULT_ROWS);
        assert!(quiet.warnings.is_empty(), "{:?}", quiet.warnings);

        let wide = validate(&payload(many(SOFT_SOURCES + 1))).unwrap();
        assert_eq!(wide.warnings.len(), 1);
        assert!(wide.warnings[0].contains("9 apps"), "{:?}", wide.warnings);

        let mut tall = payload(many(1));
        tall.limit = Some(MAX_ROWS);
        let checked = validate(&tall).unwrap();
        assert_eq!(checked.rows, MAX_ROWS);
        assert!(
            checked.warnings[0].contains("5000 rows"),
            "{:?}",
            checked.warnings
        );
        tall.limit = Some(MAX_ROWS + 1);
        assert!(rejection(&tall).contains("at most 5000 rows"));
        tall.limit = Some(0);
        assert_eq!(validate(&tall).unwrap().rows, 1);

        let mut cached = payload(many(1));
        cached.cache_seconds = Some(MAX_CACHE_SECONDS * 7);
        assert_eq!(
            validate(&cached).unwrap().ttl,
            Duration::from_secs(MAX_CACHE_SECONDS)
        );
    }

    fn authorized(alias: &str, reader: ColumnReader, owner: Option<&str>) -> AuthorizedSource {
        AuthorizedSource {
            alias: alias.to_string(),
            read: LanceRead {
                app_id: "a".to_string(),
                principal: "anyone".to_string(),
                owner: owner.map(str::to_string),
                reader,
            },
        }
    }

    #[test]
    fn cache_keys_separate_access_scope_and_inputs() {
        let key = |sql: &str, params: Value, limit: usize, source: AuthorizedSource| {
            cache_key(sql, &params, limit, &[source])
        };
        let base = || authorized("crm", ColumnReader::Unrestricted, None);
        let reference = key("SELECT 1", json!({ "a": 1 }), 10, base());
        assert_eq!(
            reference,
            key("SELECT 1", json!({ "a": 1 }), 10, base()),
            "identical requests from callers with identical access share a key"
        );
        let mut other_principal = base();
        other_principal.read.principal = "someone-else".to_string();
        assert_eq!(
            reference,
            key("SELECT 1", json!({ "a": 1 }), 10, other_principal),
            "who mints the credentials does not split the cache"
        );
        for other in [
            key("SELECT 2", json!({ "a": 1 }), 10, base()),
            key("SELECT 1", json!({ "a": 2 }), 10, base()),
            key("SELECT 1", json!({ "a": 1 }), 11, base()),
            key(
                "SELECT 1",
                json!({ "a": 1 }),
                10,
                authorized("crm", ColumnReader::with_attributes(["sales"]), None),
            ),
            key(
                "SELECT 1",
                json!({ "a": 1 }),
                10,
                authorized("crm", ColumnReader::Unrestricted, Some("user-1")),
            ),
        ] {
            assert_ne!(reference, other);
        }
        let order = |first: &str, second: &str| {
            cache_key(
                "SELECT 1",
                &Value::Null,
                10,
                &[
                    authorized(first, ColumnReader::Unrestricted, None),
                    authorized(second, ColumnReader::Unrestricted, None),
                ],
            )
        };
        assert_eq!(order("crm", "shop"), order("shop", "crm"));
    }

    fn settled(failure: Option<&str>, warnings: Vec<&str>) -> Settled {
        Settled {
            entry: CachedQuery {
                result: SqlQueryResult {
                    columns: Vec::new(),
                    rows: vec![json!({ "n": 1 })],
                    row_count: 1,
                    truncated: false,
                },
                tables: vec!["crm.customers".to_string()],
                versions: Default::default(),
                unversioned_tables: Vec::new(),
                warnings: warnings.into_iter().map(str::to_string).collect(),
                computed_at: 1_000,
                expires_at: 61_000,
            },
            cached: true,
            failure: failure.map(str::to_string),
            store: false,
        }
    }

    #[test]
    fn responses_carry_every_warning_and_the_stale_flag() {
        let response = respond(
            settled(None, vec!["lowered"]),
            Duration::from_secs(60),
            vec!["wide".to_string()],
        );
        assert_eq!(response.warnings, vec!["wide", "lowered"]);
        assert!(!response.stale && response.cached);
        assert_eq!(response.expires_at, Some(61_000));

        let stale = respond(
            settled(
                Some("The refresh failed (x); showing the last result"),
                Vec::new(),
            ),
            Duration::ZERO,
            Vec::new(),
        );
        assert!(stale.stale);
        assert_eq!(stale.expires_at, None);
        assert_eq!(stale.warnings.len(), 1);

        let wire = serde_json::to_value(&stale).unwrap();
        for field in [
            "columns",
            "rows",
            "row_count",
            "truncated",
            "tables",
            "computed_at",
            "cached",
            "expires_at",
            "warnings",
            "stale",
        ] {
            assert!(wire.get(field).is_some(), "{field} is part of the contract");
        }
    }

    #[test]
    fn oversized_results_are_answered_but_not_cached() {
        let mut warnings = Vec::new();
        assert!(cacheable(&settled(None, Vec::new()).entry, &mut warnings));
        assert!(warnings.is_empty());

        let mut large = settled(None, Vec::new()).entry;
        large.result.rows = vec![json!({ "text": "x".repeat(best_effort::MAX_ENTRY_BYTES) })];
        assert!(!cacheable(&large, &mut warnings));
        assert_eq!(warnings, vec![TOO_LARGE_TO_CACHE]);
    }
}

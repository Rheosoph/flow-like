//! What a cross-app statement returns and asks for, and the values its `$parameters` can take,
//! so Home widgets can offer viewers matching controls.

use axum::{Extension, Json, extract::State};
use flow_like_query::{Operation, QueryShape};
use serde::Deserialize;
use serde_json::Value;
use utoipa::ToSchema;

use super::{DataQuerySource, authorize_sources, source_specs, validate_sources, validate_sql};
use crate::{error::ApiError, middleware::jwt::AppUser, query, state::AppState};

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DataQueryDescribePayload {
    /// One read-only SELECT or WITH statement. Tables are addressed as `<alias>.<table>`.
    pub sql: String,
    pub sources: Vec<DataQuerySource>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DataQueryOptionsPayload {
    pub sources: Vec<DataQuerySource>,
    /// `alias.table` the parameter is compared with.
    pub table: String,
    pub column: String,
    /// Only values containing this text, ignoring case.
    #[serde(default)]
    pub search: Option<String>,
    /// Answer `{min, max}` instead of distinct values.
    #[serde(default)]
    pub range: bool,
}

/// The position of the source an options request reads: the one whose alias qualifies `table`.
fn option_source(sources: &[DataQuerySource], table: &str) -> Result<usize, ApiError> {
    let (alias, _) = table.split_once('.').ok_or_else(|| {
        ApiError::bad_request(format!("Name the table as <alias>.<table>, not '{table}'"))
    })?;
    sources
        .iter()
        .position(|source| source.alias == alias)
        .ok_or_else(|| ApiError::bad_request(format!("No app source is named '{alias}'")))
}

#[utoipa::path(
    post,
    path = "/user/data/query/describe",
    tag = "user",
    description = "Describe a cross-app SQL query without running it: the columns it returns and the `$parameters` it takes, with the table columns each parameter is compared with.",
    request_body = DataQueryDescribePayload,
    responses(
        (status = 200, description = "The query's output columns and parameters", body = Object),
        (status = 400, description = "The query or its sources are invalid"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "You cannot read one of the apps"),
        (status = 503, description = "The query service is unavailable")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "POST /user/data/query/describe", skip_all)]
pub async fn describe_data_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<DataQueryDescribePayload>,
) -> Result<Json<Value>, ApiError> {
    validate_sql(&payload.sql)?;
    validate_sources(&payload.sources, &mut Vec::new())?;
    let sources = authorize_sources(&state, &user, &payload.sources).await?;
    let request = query::request(
        source_specs(&state, &sources).await?,
        1,
        Operation::Describe {
            sql: payload.sql,
            shape: QueryShape::CrossApp,
        },
    );
    let response = state.query.run(&query::subject(&user), &request).await?;
    Ok(Json(query::json_answer(response)?))
}

#[utoipa::path(
    post,
    path = "/user/data/query/options",
    tag = "user",
    description = "List the values a query parameter compared with `alias.table.column` can take: up to 200 distinct values, optionally only those containing a search text, or the smallest and largest value.",
    request_body = DataQueryOptionsPayload,
    responses(
        (status = 200, description = "`{values, truncated}`, or `{min, max}` for a range", body = Object),
        (status = 400, description = "The table, column or sources are invalid"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "You cannot read one of the apps"),
        (status = 503, description = "The query service is unavailable")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "POST /user/data/query/options", skip_all)]
pub async fn data_query_options(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<DataQueryOptionsPayload>,
) -> Result<Json<Value>, ApiError> {
    validate_sources(&payload.sources, &mut Vec::new())?;
    query::validate_option_target(&payload.table, &payload.column, payload.search.as_deref())?;
    let read = option_source(&payload.sources, &payload.table)?;
    let sources = authorize_sources(&state, &user, &payload.sources).await?;
    let request = query::request(
        source_specs(&state, std::slice::from_ref(&sources[read])).await?,
        1,
        Operation::ParameterOptions {
            table: payload.table,
            column: payload.column,
            search: payload.search,
            range: payload.range,
            shape: QueryShape::CrossApp,
        },
    );
    let response = state.query.run(&query::subject(&user), &request).await?;
    Ok(Json(query::json_answer(response)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(alias: &str) -> DataQuerySource {
        DataQuerySource {
            alias: alias.to_string(),
            app_id: "a".to_string(),
            scope: None,
            source_id: None,
        }
    }

    #[test]
    fn options_read_only_the_source_their_table_names() {
        let sources = [source("crm"), source("shop")];
        assert_eq!(option_source(&sources, "shop.orders").unwrap(), 1);
        assert!(option_source(&sources, "orders").is_err());
        assert!(option_source(&sources, "hr.people").is_err());
    }

    #[test]
    fn payloads_match_the_frontend_contract() {
        let describe: DataQueryDescribePayload = serde_json::from_value(serde_json::json!({
            "sql": "SELECT 1",
            "sources": [{ "alias": "crm", "app_id": "a", "scope": "project" }],
        }))
        .unwrap();
        assert_eq!(describe.sources[0].alias, "crm");
        let options: DataQueryOptionsPayload = serde_json::from_value(serde_json::json!({
            "sources": [{ "alias": "crm", "app_id": "a", "scope": "personal" }],
            "table": "crm.customers",
            "column": "region",
        }))
        .unwrap();
        assert!(!options.range);
        assert_eq!(options.search, None);
    }
}

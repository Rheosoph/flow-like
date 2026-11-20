//! What a Data Studio statement returns and asks for, and the values its `$parameters` can take,
//! so viewers get controls that match each parameter.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_query::Operation;
use serde_json::Value;
use utoipa::ToSchema;

use super::{
    ScopeParams,
    saved_queries::{SurfaceTarget, run_on_surface},
};
use crate::{error::ApiError, middleware::jwt::AppUser, query, state::AppState};

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct DescribeQueryPayload {
    pub sql: String,
    /// `native` or `overlay`.
    pub surface: String,
    #[serde(default)]
    pub overlay_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, ToSchema)]
pub struct QueryOptionsPayload {
    /// The table the parameter is compared with, as the statement names it.
    pub table: String,
    pub column: String,
    /// Only values containing this text, ignoring case.
    #[serde(default)]
    pub search: Option<String>,
    /// Answer `{min, max}` instead of distinct values.
    #[serde(default)]
    pub range: bool,
    /// `native` or `overlay`.
    pub surface: String,
    #[serde(default)]
    pub overlay_id: Option<String>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/queries/describe",
    tag = "data-studio",
    description = "Describe a read-only SQL query without running it: the columns it returns and the `$parameters` it takes, with the table columns each parameter is compared with.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = DescribeQueryPayload,
    responses(
        (status = 200, description = "The query's output columns and parameters", body = Object),
        (status = 400, description = "The query is invalid"),
        (status = 403, description = "Forbidden"),
        (status = 503, description = "The query service is unavailable")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "POST /apps/{app_id}/db/queries/describe", skip_all)]
pub async fn describe_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<DescribeQueryPayload>,
) -> Result<Json<Value>, ApiError> {
    let target = SurfaceTarget {
        app_id: &app_id,
        scope: &scope,
        surface: &payload.surface,
        overlay_id: payload.overlay_id.as_deref(),
    };
    let response = run_on_surface(&state, &user, target, 1, |shape| Operation::Describe {
        sql: payload.sql,
        shape,
    })
    .await?;
    Ok(Json(query::json_answer(response)?))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/queries/options",
    tag = "data-studio",
    description = "List the values a query parameter compared with `table.column` can take: up to 200 distinct values, optionally only those containing a search text, or the smallest and largest value.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("scope" = Option<String>, Query, description = "Scope: 'user' or omit for project")
    ),
    request_body = QueryOptionsPayload,
    responses(
        (status = 200, description = "`{values, truncated}`, or `{min, max}` for a range", body = Object),
        (status = 400, description = "The table or column is invalid"),
        (status = 403, description = "Forbidden"),
        (status = 503, description = "The query service is unavailable")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "POST /apps/{app_id}/db/queries/options", skip_all)]
pub async fn query_options(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<QueryOptionsPayload>,
) -> Result<Json<Value>, ApiError> {
    query::validate_option_target(&payload.table, &payload.column, payload.search.as_deref())?;
    let target = SurfaceTarget {
        app_id: &app_id,
        scope: &scope,
        surface: &payload.surface,
        overlay_id: payload.overlay_id.as_deref(),
    };
    let response = run_on_surface(&state, &user, target, 1, |shape| {
        Operation::ParameterOptions {
            table: payload.table,
            column: payload.column,
            search: payload.search,
            range: payload.range,
            shape,
        }
    })
    .await?;
    Ok(Json(query::json_answer(response)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn payloads_match_the_frontend_contract() {
        let describe: DescribeQueryPayload = serde_json::from_value(json!({
            "sql": "SELECT * FROM orders WHERE region = $region",
            "surface": "native",
        }))
        .unwrap();
        assert_eq!(describe.overlay_id, None);
        let options: QueryOptionsPayload = serde_json::from_value(json!({
            "table": "orders",
            "column": "region",
            "search": "we",
            "surface": "overlay",
            "overlay_id": "overlay-1",
        }))
        .unwrap();
        assert_eq!(options.search.as_deref(), Some("we"));
        assert!(!options.range);
    }
}

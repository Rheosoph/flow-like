use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::{
    contracts::database::{
        DatabaseAction, DatabaseActionResult, DatabaseDiff, DatabaseHistory, DatabaseSelector,
    },
    databases::vector::lancedb::LanceDBVectorStore,
};

use crate::{
    audit_branch, ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{
        ScopeParams, resolve_connection, resolve_write_connection, validate_table_name,
    },
    state::AppState,
};

#[utoipa::path(
    get,
    path = "/apps/{app_id}/db/{table}/references",
    tag = "database",
    description = "Inspect a selected table reference and its versions, branches and tags.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name"),
        ("scope" = Option<String>, Query, description = "Use user for a user-scoped database"),
        ("branch" = Option<String>, Query, description = "Branch, defaults to main"),
        ("version" = Option<u64>, Query, description = "Read-only historical version"),
        ("tag" = Option<String>, Query, description = "Named snapshot, resolves its own branch"),
        ("read_only" = Option<bool>, Query, description = "Disable writes and reference management")
    ),
    responses(
        (status = 200, description = "Database reference result", body = Object),
        (status = 400, description = "Invalid reference or operation"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/db/{table}/references",
    skip(state, user, scope)
)]
pub async fn history(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Query(selector): Query<DatabaseSelector>,
) -> Result<Json<DatabaseHistory>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );
    validate_table_name(&table)?;
    selector
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let db = LanceDBVectorStore::from_connection_with_selector(connection, table, selector).await?;
    Ok(Json(db.history().await?))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/{table}/references",
    tag = "database",
    description = "Create, move or delete table references, restore a version, snapshot, clone or clean up retained history.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name"),
        ("scope" = Option<String>, Query, description = "Use user for a user-scoped database"),
        ("branch" = Option<String>, Query, description = "Branch, defaults to main"),
        ("version" = Option<u64>, Query, description = "Read-only historical version"),
        ("tag" = Option<String>, Query, description = "Named snapshot, resolves its own branch"),
        ("read_only" = Option<bool>, Query, description = "Disable writes and reference management")
    ),
    request_body = Object,
    responses(
        (status = 200, description = "Database reference result", body = Object),
        (status = 400, description = "Invalid reference or operation"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/db/{table}/references",
    skip(state, user, scope, action)
)]
pub async fn reference_action(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Query(selector): Query<DatabaseSelector>,
    Json(action): Json<DatabaseAction>,
) -> Result<Json<DatabaseActionResult>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    validate_table_name(&table)?;
    selector
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if let DatabaseAction::Clone { name } = &action {
        validate_table_name(name)?;
    }
    let details = serde_json::json!({
        "operation": action,
        "selector": selector,
        "user_scoped": scope.is_user_scoped(),
    });
    let connection = resolve_write_connection(&state, &user, &app_id, &scope).await?;
    let db = LanceDBVectorStore::from_connection_with_selector(connection, table.clone(), selector)
        .await?;
    let result = db.reference_action(action).await?;
    audit_branch!(
        state,
        user,
        app_id,
        "database.reference.change",
        "DatabaseTable",
        table,
        details
    );
    Ok(Json(result))
}

#[derive(Debug, serde::Deserialize)]
pub struct ComparePayload {
    pub other: DatabaseSelector,
    pub key: String,
    pub limit: Option<usize>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/{table}/compare",
    tag = "database",
    description = "Compare two table references using a unique application key. Row previews are bounded to 1000.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name"),
        ("scope" = Option<String>, Query, description = "Use user for a user-scoped database"),
        ("branch" = Option<String>, Query, description = "Branch, defaults to main"),
        ("version" = Option<u64>, Query, description = "Read-only historical version"),
        ("tag" = Option<String>, Query, description = "Named snapshot, resolves its own branch"),
        ("read_only" = Option<bool>, Query, description = "Disable writes and reference management")
    ),
    request_body = Object,
    responses(
        (status = 200, description = "Database reference result", body = Object),
        (status = 400, description = "Invalid reference or operation"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/db/{table}/compare",
    skip(state, user, scope, payload)
)]
pub async fn compare(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Query(selector): Query<DatabaseSelector>,
    Json(payload): Json<ComparePayload>,
) -> Result<Json<DatabaseDiff>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );
    validate_table_name(&table)?;
    selector
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    payload
        .other
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let source = LanceDBVectorStore::from_connection_with_selector(
        connection.clone(),
        table.clone(),
        selector,
    )
    .await?;
    let target =
        LanceDBVectorStore::from_connection_with_selector(connection, table, payload.other).await?;
    Ok(Json(
        source
            .compare(
                &target,
                &payload.key,
                payload.limit.unwrap_or(100).min(1000),
            )
            .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_parse_alongside_scope_and_pagination() {
        let uri = "/?scope=user&offset=25&branch=experiment%2Fa&version=7&read_only=true"
            .parse()
            .unwrap();
        let Query(selector) = Query::<DatabaseSelector>::try_from_uri(&uri).unwrap();
        assert_eq!(selector.branch, "experiment/a");
        assert_eq!(selector.version, Some(7));
        assert!(selector.read_only);
        let Query(scope) = Query::<ScopeParams>::try_from_uri(&uri).unwrap();
        assert!(scope.is_user_scoped());
    }

    #[test]
    fn absent_selector_opens_main_and_snapshots_reject_writes() {
        let uri = "/?scope=user".parse().unwrap();
        let Query(selector) = Query::<DatabaseSelector>::try_from_uri(&uri).unwrap();
        assert_eq!(selector, DatabaseSelector::default());
        super::super::validate_writable_selector(&selector).unwrap();
        for selector in [
            DatabaseSelector {
                version: Some(1),
                ..Default::default()
            },
            DatabaseSelector {
                tag: Some("training".into()),
                ..Default::default()
            },
            DatabaseSelector {
                read_only: true,
                ..Default::default()
            },
        ] {
            let error = super::super::validate_writable_selector(&selector).unwrap_err();
            assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
        }
    }
}

use axum::{
    Router,
    routing::{delete, get, post, put},
};
use flow_like_storage::lancedb::Connection;

use crate::{
    credentials::CredentialsAccess, error::ApiError, middleware::jwt::AppUser, state::AppState,
};

pub mod add_column;
pub mod alter_column;
pub mod build_index;
pub mod create_table;
pub mod db_add;
pub mod db_count;
pub mod db_delete;
pub mod db_list;
pub mod db_query;
pub mod db_update;
pub mod drop_columns;
pub mod drop_index;
pub mod drop_table;
pub mod get_db_schema;
pub mod get_indices;
pub mod list_tables;
pub mod list_tables_user;
pub mod optimize;
pub mod presign_db_access;
pub mod saved_queries;
pub mod table_view;

#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct ScopeParams {
    pub scope: Option<String>,
}

impl ScopeParams {
    pub fn is_user_scoped(&self) -> bool {
        self.scope.as_deref() == Some("user")
    }
}

#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct ScopedPaginationParams {
    pub limit: Option<u64>,
    pub offset: Option<u64>,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionAccess {
    Read,
    Write,
}

impl ConnectionAccess {
    fn credentials_access(self) -> CredentialsAccess {
        match self {
            Self::Read => CredentialsAccess::InvokeRead,
            Self::Write => CredentialsAccess::InvokeWrite,
        }
    }
}

impl ScopedPaginationParams {
    pub fn scope_params(&self) -> ScopeParams {
        ScopeParams {
            scope: self.scope.clone(),
        }
    }
}

/// Validates a table name: alphanumeric, hyphens, underscores, dots only; no path traversal.
/// Also rejects reserved table names (e.g. `__graph_overlays__`).
pub fn validate_table_name(name: &str) -> Result<(), ApiError> {
    if name.is_empty() || name.len() > 256 {
        return Err(ApiError::bad_request(
            "Table name must be 1-256 characters".to_string(),
        ));
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(ApiError::bad_request(
            "Table name contains forbidden characters".to_string(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(ApiError::bad_request(
            "Table name contains invalid characters".to_string(),
        ));
    }
    if flow_like_catalog_core::is_reserved_table(name) {
        return Err(ApiError::bad_request(
            "Table name is reserved for internal use".to_string(),
        ));
    }
    Ok(())
}

pub async fn resolve_connection(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    scope: &ScopeParams,
) -> Result<Connection, ApiError> {
    resolve_connection_with_access(state, user, app_id, scope, ConnectionAccess::Read).await
}

/// Resolve an app database connection for a mutating operation. Project-scoped
/// connections use the same master database, while user-scoped connections
/// receive write-capable temporary credentials rather than the read-only
/// credentials used by [`resolve_connection`].
pub async fn resolve_write_connection(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    scope: &ScopeParams,
) -> Result<Connection, ApiError> {
    resolve_connection_with_access(state, user, app_id, scope, ConnectionAccess::Write).await
}

async fn resolve_connection_with_access(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    scope: &ScopeParams,
    access: ConnectionAccess,
) -> Result<Connection, ApiError> {
    if scope.is_user_scoped() {
        let sub = user.sub()?;
        let credentials = state
            .scoped_credentials(&sub, app_id, access.credentials_access())
            .await?;
        let builder = credentials.to_db_scoped(&sub, app_id).await?;
        Ok(builder.execute().await?)
    } else {
        let credentials = state.master_credentials().await?;
        let builder = credentials.to_db(app_id).await?;
        Ok(builder.execute().await?)
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_tables::list_tables))
        .route("/user", get(list_tables_user::list_tables_user))
        .route(
            "/queries",
            get(saved_queries::list_saved_queries).post(saved_queries::create_saved_query),
        )
        .route("/queries/execute", post(saved_queries::execute_query))
        .route(
            "/queries/{query_id}",
            get(saved_queries::get_saved_query)
                .put(saved_queries::update_saved_query)
                .delete(saved_queries::delete_saved_query),
        )
        .route("/presign", post(presign_db_access::presign_db_access))
        .route(
            "/presign/project",
            post(presign_db_access::presign_project_db_access),
        )
        .route(
            "/{table}",
            post(create_table::create_table)
                .put(db_add::add_to_table)
                .delete(db_delete::delete_from_table)
                .get(db_list::list_items),
        )
        .route("/{table}/table", delete(drop_table::drop_table))
        .route("/{table}/update", put(db_update::update_table))
        .route("/{table}/optimize", post(optimize::optimize_table))
        .route(
            "/{table}/columns",
            post(add_column::add_column)
                .put(alter_column::alter_column)
                .delete(drop_columns::drop_columns),
        )
        .route("/{table}/index", post(build_index::build_index))
        .route(
            "/{table}/index/{index_name}",
            delete(drop_index::drop_index),
        )
        .route("/{table}/query", post(db_query::query_table))
        .route("/{table}/schema", get(get_db_schema::get_db_schema))
        .route("/{table}/count", get(db_count::db_count))
        .route("/{table}/indices", get(get_indices::get_db_indices))
        .route("/{table}/view", get(table_view::table_view))
}

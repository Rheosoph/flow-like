use axum::{
    Router,
    routing::{delete, get, post, put},
};
use flow_like_storage::lancedb::Connection;

use crate::{
    credentials::CredentialsAccess,
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};

pub mod add_column;
pub mod alter_column;
pub mod build_index;
pub mod db_add;
pub mod db_count;
pub mod db_delete;
pub mod db_list;
pub mod db_query;
pub mod db_update;
pub mod drop_columns;
pub mod drop_index;
pub mod get_db_schema;
pub mod get_indices;
pub mod list_tables;
pub mod list_tables_user;
pub mod optimize;
pub mod presign_db_access;
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

impl ScopedPaginationParams {
    pub fn scope_params(&self) -> ScopeParams {
        ScopeParams {
            scope: self.scope.clone(),
        }
    }
}

/// Validates a table name: alphanumeric, hyphens, underscores, dots only; no path traversal.
pub fn validate_table_name(name: &str) -> Result<(), ApiError> {
    if name.is_empty() || name.len() > 256 {
        return Err(ApiError::bad_request("Table name must be 1-256 characters".to_string()));
    }
    if name.contains("..") || name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(ApiError::bad_request("Table name contains forbidden characters".to_string()));
    }
    if !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.') {
        return Err(ApiError::bad_request("Table name contains invalid characters".to_string()));
    }
    Ok(())
}

pub async fn resolve_connection(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    scope: &ScopeParams,
) -> Result<Connection, ApiError> {
    if scope.is_user_scoped() {
        let sub = user.sub()?;
        let credentials = state
            .scoped_credentials(&sub, app_id, CredentialsAccess::InvokeRead)
            .await?;
        Ok(credentials.to_db_scoped(&sub, app_id).await?.execute().await?)
    } else {
        let credentials = state.master_credentials().await?;
        Ok(credentials.to_db(app_id).await?.execute().await?)
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list_tables::list_tables))
        .route("/user", get(list_tables_user::list_tables_user))
        .route("/presign", post(presign_db_access::presign_db_access))
        .route(
            "/{table}",
            put(db_add::add_to_table)
                .delete(db_delete::delete_from_table)
                .get(db_list::list_items),
        )
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

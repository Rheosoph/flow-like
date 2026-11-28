use axum::{
    Router,
    routing::{delete, get, post, put},
};
use flow_like_query::{Operation, table::TableOperation};
use flow_like_storage::contracts::column_metadata::ColumnReader;
use flow_like_storage::databases::vector::schema::{
    ColumnAccessRejected, PrimaryKeyRejected, TableInputRejected,
};
use flow_like_storage::lancedb::Connection;

use crate::{
    credentials::CredentialsAccess,
    error::ApiError,
    middleware::jwt::{AppPermissionResponse, AppUser},
    permission::role_permission::RolePermissions,
    query::{self, LanceRead},
    state::AppState,
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
pub mod forecasting;
pub mod get_db_schema;
pub mod get_indices;
pub mod list_tables;
pub mod list_tables_user;
pub mod optimize;
pub mod presign_db_access;
pub mod query_parameters;
pub mod references;
pub mod saved_forecasts;
pub mod saved_queries;
pub mod set_column_metadata;
pub mod set_primary_key;
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
pub struct TableListParams {
    /// `summary` swaps the bare name list for a full per-table summary.
    pub detail: Option<String>,
}

impl TableListParams {
    pub fn wants_summary(&self) -> bool {
        self.detail
            .as_deref()
            .is_some_and(|detail| detail.eq_ignore_ascii_case("summary"))
    }
}

/// Sends one table read to the query service for the database `scope` names, after the
/// caller's read check. Answers the JSON the route returns.
pub(crate) async fn read_table(
    state: &AppState,
    user: &AppUser,
    permission: &AppPermissionResponse,
    app_id: &str,
    scope: &ScopeParams,
    rows: usize,
    table: TableOperation,
) -> Result<flow_like_types::Value, ApiError> {
    let read = LanceRead::scoped(user, permission, app_id, scope)?;
    let source = state.query.lance_source(state, &read, None).await?;
    let request = query::request(vec![source], rows, Operation::Table { table });
    let response = state.query.run(&query::subject(user), &request).await?;
    query::json_answer(response)
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

pub fn validate_writable_selector(
    selector: &flow_like_storage::contracts::database::DatabaseSelector,
) -> Result<(), ApiError> {
    selector
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if selector.is_read_only() {
        return Err(ApiError::bad_request(
            "This reference is read-only. Select the latest version of a branch before editing it."
                .to_string(),
        ));
    }
    Ok(())
}

/// The caller-facing message of the first table input or table key rejection among `causes`.
pub fn table_rejection_message<'a>(
    mut causes: impl Iterator<Item = &'a (dyn std::error::Error + 'static)>,
) -> Option<String> {
    causes.find_map(|cause| {
        cause
            .downcast_ref::<TableInputRejected>()
            .map(ToString::to_string)
            .or_else(|| {
                cause
                    .downcast_ref::<PrimaryKeyRejected>()
                    .map(ToString::to_string)
            })
    })
}

/// Rejected row values, column names and table key rules are the caller's to fix, wherever
/// they sit in the error chain; a column access change the role may not make is forbidden;
/// any other failure stays internal.
pub fn table_input_error(error: flow_like_types::Error) -> ApiError {
    if let Some(message) = error.chain().find_map(|cause| {
        cause
            .downcast_ref::<ColumnAccessRejected>()
            .map(ToString::to_string)
    }) {
        return ApiError::forbidden(message);
    }
    match table_rejection_message(error.chain()) {
        Some(message) => ApiError::bad_request(message),
        None => ApiError::from(error),
    }
}

/// Whose column read restrictions apply to a request. Admins and owners read every column,
/// and so does everyone in their own user-scoped database; any other role sees a
/// restricted column only through one of its attributes.
pub fn column_reader(permission: &AppPermissionResponse, scope: &ScopeParams) -> ColumnReader {
    if scope.is_user_scoped() || permission.has_permission(RolePermissions::Admin) {
        return ColumnReader::Unrestricted;
    }
    ColumnReader::with_attributes(
        permission
            .role
            .attributes
            .iter()
            .flat_map(|list| list.iter()),
    )
}

pub fn table_key_error(error: flow_like_types::Error) -> ApiError {
    table_input_error(error)
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
        .route("/queries/describe", post(query_parameters::describe_query))
        .route("/queries/options", post(query_parameters::query_options))
        .route("/queries/forecast", post(forecasting::forecast_query))
        .route("/forecasts", saved_forecasts::collection_routes())
        .route(
            "/forecasts/{forecast_id}",
            get(saved_forecasts::get_saved_forecast).delete(saved_forecasts::delete_saved_forecast),
        )
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
        .route("/{table}/compare", post(references::compare))
        .route(
            "/{table}/references",
            get(references::history).post(references::reference_action),
        )
        .route("/{table}/update", put(db_update::update_table))
        .route("/{table}/optimize", post(optimize::optimize_table))
        .route(
            "/{table}/columns",
            post(add_column::add_column)
                .put(alter_column::alter_column)
                .delete(drop_columns::drop_columns),
        )
        .route(
            "/{table}/columns/{column}/metadata",
            put(set_column_metadata::set_column_metadata),
        )
        .route(
            "/{table}/primary-key",
            put(set_primary_key::set_primary_key),
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

#[cfg(test)]
mod tests {
    use super::{table_input_error, table_key_error};
    use axum::http::StatusCode;
    use flow_like_storage::databases::vector::schema::{PrimaryKeyRejected, TableInputRejected};
    use flow_like_types::{Context, anyhow};

    const REJECTION: &str =
        "Column 'geometry' row 3: Unknown geometry kind: Feature. Store the Feature's geometry.";

    #[test]
    fn table_input_rejection_behind_context_is_a_bad_request_with_its_own_message() {
        let error = Err::<(), _>(TableInputRejected(REJECTION.to_string()))
            .context("Failed to convert rows for table 'sites'")
            .context("Insert into table 'sites' failed")
            .unwrap_err();

        let api_error = table_input_error(error);

        assert_eq!(api_error.status(), StatusCode::BAD_REQUEST);
        assert_eq!(api_error.public_message(), Some(REJECTION));
    }

    #[test]
    fn table_input_rejection_raised_as_anyhow_error_is_a_bad_request() {
        let error = flow_like_types::Error::new(TableInputRejected(REJECTION.to_string()))
            .context("Insert into table 'sites' failed");

        let api_error = table_input_error(error);

        assert_eq!(api_error.status(), StatusCode::BAD_REQUEST);
        assert_eq!(api_error.public_message(), Some(REJECTION));
    }

    #[test]
    fn table_input_error_keeps_primary_key_rejections_as_bad_requests() {
        let message = "Column 'feature_id' is the table key and cannot be null";
        let error = flow_like_types::Error::new(PrimaryKeyRejected(message.to_string()))
            .context("Upsert into table 'sites' failed");

        for api_error in [
            table_input_error(anyhow!(PrimaryKeyRejected(message.to_string()))),
            table_key_error(error),
        ] {
            assert_eq!(api_error.status(), StatusCode::BAD_REQUEST);
            assert_eq!(api_error.public_message(), Some(message));
        }
    }

    #[test]
    fn table_input_error_keeps_unrelated_failures_internal() {
        let error = anyhow!("object store request to bucket 'apps-prod' timed out")
            .context("Insert into table 'sites' failed");

        let api_error = table_input_error(error);

        assert_eq!(api_error.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(api_error.public_message(), None);
    }
}

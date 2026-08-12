use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, resolve_write_connection, validate_table_name},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::vector::{
    lancedb::LanceDBVectorStore,
    schema::{DatabaseSchemaField, database_fields_to_arrow_schema},
};

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateTableFieldPayload {
    /// Column name. Use ASCII letters, numbers, and underscores; do not start with a number.
    pub name: String,
    /// string, boolean, int8/int16/int32/int64, uint8/uint16/uint32/uint64,
    /// float32/float64, binary, date32, timestamp:ms:UTC, or vector. Use
    /// timestamp:ms:UTC for FlowLike Date/date-time instant fields; date32 is calendar-only.
    #[serde(rename = "type")]
    pub data_type: String,
    /// Whether the column accepts null values. Defaults to true.
    pub nullable: Option<bool>,
    /// Required for vector fields; the number of float32 values in each vector.
    pub vector_size: Option<u32>,
}

impl From<CreateTableFieldPayload> for DatabaseSchemaField {
    fn from(field: CreateTableFieldPayload) -> Self {
        Self {
            name: field.name,
            data_type: field.data_type,
            nullable: field.nullable.unwrap_or(true),
            vector_size: field.vector_size,
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateTablePayload {
    /// Explicit schema for the empty table. No seed rows are inserted.
    pub fields: Vec<CreateTableFieldPayload>,
    /// Return success when the table already exists. Defaults to true.
    pub if_not_exists: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct CreateTableResponse {
    pub table_name: String,
    pub created: bool,
    pub if_not_exists: bool,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/{table}",
    tag = "database",
    description = "Create an empty LanceDB table from an explicit schema without inserting seed rows.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name"),
        ("scope" = Option<String>, Query, description = "Use 'user' for a user-scoped database")
    ),
    request_body = CreateTablePayload,
    responses(
        (status = 200, description = "Table created or already present", body = CreateTableResponse),
        (status = 400, description = "Invalid table name or schema"),
        (status = 409, description = "Table exists with an incompatible schema or if_not_exists is false"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/db/{table}",
    skip(state, user, scope, payload)
)]
pub async fn create_table(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<CreateTablePayload>,
) -> Result<Json<CreateTableResponse>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    validate_table_name(&table)?;

    let if_not_exists = payload.if_not_exists.unwrap_or(true);
    let fields = payload
        .fields
        .into_iter()
        .map(DatabaseSchemaField::from)
        .collect::<Vec<_>>();
    let schema = database_fields_to_arrow_schema(&fields)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;

    let connection = resolve_write_connection(&state, &user, &app_id, &scope).await?;
    let mut db = LanceDBVectorStore::from_connection(connection, table.clone()).await;
    let created = db
        .create_empty_table(schema, if_not_exists)
        .await
        .map_err(|error| {
            let message = error.to_string();
            if message.contains("already exists") {
                ApiError::conflict(message)
            } else {
                ApiError::from(error)
            }
        })?;

    Ok(Json(CreateTableResponse {
        table_name: table,
        created,
        if_not_exists,
    }))
}

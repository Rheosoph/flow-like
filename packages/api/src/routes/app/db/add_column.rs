use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{
        ScopeParams, resolve_write_connection, table_input_error, validate_table_name,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::contracts::database::DatabaseSelector;
use flow_like_storage::databases::vector::{
    lancedb::LanceDBVectorStore,
    schema::{DatabaseSchemaField, database_fields_to_arrow_schema},
};
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct AddColumnPayload {
    pub name: String,
    /// Computes the column from existing columns. Mutually exclusive with `type`.
    #[serde(default)]
    pub sql_expression: Option<String>,
    /// A table column type (for example `geometry`) for a column that starts empty.
    #[serde(default, rename = "type")]
    pub data_type: Option<String>,
    /// Required when `type` is `vector`.
    #[serde(default)]
    pub vector_size: Option<u32>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/{table}/columns",
    tag = "database",
    description = "Add a column to a table, computed from an SQL expression or typed and initially empty.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    request_body = AddColumnPayload,
    responses(
        (status = 200, description = "Column added", body = ()),
        (status = 400, description = "Invalid column name or type, or not exactly one of sql_expression and type given"),
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
    name = "POST /apps/{app_id}/db/{table}/columns",
    skip(state, user, scope, payload)
)]
pub async fn add_column(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Query(selector): Query<DatabaseSelector>,
    Json(payload): Json<AddColumnPayload>,
) -> Result<Json<()>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    validate_table_name(&table)?;
    super::validate_writable_selector(&selector)?;
    validate_column_definition(&payload)?;

    let connection = resolve_write_connection(&state, &user, &app_id, &scope).await?;
    let db = LanceDBVectorStore::from_connection_with_selector(connection, table, selector).await?;

    db.add_column_definition(
        &payload.name,
        payload.sql_expression.as_deref(),
        payload.data_type.as_deref(),
        payload.vector_size,
    )
    .await
    .map_err(table_input_error)?;

    Ok(Json(()))
}

fn validate_column_definition(payload: &AddColumnPayload) -> Result<(), ApiError> {
    match (&payload.sql_expression, &payload.data_type) {
        (Some(_), None) => Ok(()),
        (None, Some(data_type)) => database_fields_to_arrow_schema(&[DatabaseSchemaField {
            name: payload.name.clone(),
            data_type: data_type.clone(),
            nullable: true,
            vector_size: payload.vector_size,
            primary_key: false,
        }])
        .map(drop)
        .map_err(|error| ApiError::bad_request(error.to_string())),
        _ => Err(ApiError::bad_request(format!(
            "Column '{}' needs exactly one of sql_expression or type",
            payload.name
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{AddColumnPayload, validate_column_definition};
    use axum::http::StatusCode;

    fn payload(
        name: &str,
        sql_expression: Option<&str>,
        data_type: Option<&str>,
    ) -> AddColumnPayload {
        AddColumnPayload {
            name: name.to_string(),
            sql_expression: sql_expression.map(str::to_string),
            data_type: data_type.map(str::to_string),
            vector_size: None,
        }
    }

    #[test]
    fn column_definitions_the_table_cannot_take_are_bad_requests() {
        for (definition, expected) in [
            (payload("city", None, Some("geo")), "Unsupported type 'geo'"),
            (
                payload("addr:city", None, Some("string")),
                "'addr:city' is invalid",
            ),
            (
                payload("area", Some("1.0"), Some("float64")),
                "exactly one of",
            ),
            (payload("area", None, None), "exactly one of"),
        ] {
            let error = validate_column_definition(&definition).unwrap_err();

            assert_eq!(error.status(), StatusCode::BAD_REQUEST);
            let message = error.public_message().unwrap_or_default();
            assert!(message.contains(expected), "{message}");
        }
    }

    #[test]
    fn typed_and_computed_column_definitions_pass() {
        for definition in [
            payload("footprint", None, Some("geometry")),
            payload("area_km2", Some("area_m2 / 1000000.0"), None),
        ] {
            assert!(validate_column_definition(&definition).is_ok());
        }
    }
}

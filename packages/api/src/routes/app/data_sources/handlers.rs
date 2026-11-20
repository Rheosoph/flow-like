use super::{
    model::{CreateDataSourceRequest, ExternalDataSource, UpdateDataSourceRequest},
    query_source::{driver_of, load_source, sealers},
    validation,
};
use crate::{
    audit_branch, data_source_secrets::SecretScope, ensure_permission, entity::app_data_source,
    error::ApiError, middleware::jwt::AppUser, permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_query::SecretFields;
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DbErr, EntityTrait, ModelTrait, QueryFilter,
    QueryOrder, SqlErr,
};

struct SealedColumns {
    secret: Option<String>,
    sealed_by: Option<String>,
    fields: Vec<String>,
}

/// Credentials of another app's databases are managed by its own people, never by an app
/// acting through a connection token.
fn deny_connected_app(user: &AppUser) -> Result<(), ApiError> {
    if user.is_connected_app() {
        return Err(ApiError::forbidden(
            "Connected apps cannot manage another app's external databases",
        ));
    }
    Ok(())
}

async fn seal(
    state: &AppState,
    app_id: &str,
    source_id: &str,
    secret: &SecretFields,
) -> Result<SealedColumns, ApiError> {
    if secret.0.is_empty() {
        return Ok(SealedColumns {
            secret: None,
            sealed_by: None,
            fields: Vec::new(),
        });
    }
    let record = sealers(state)
        .seal_fields(SecretScope { app_id, source_id }, secret)
        .await
        .map_err(ApiError::internal_error)?;
    Ok(SealedColumns {
        secret: Some(record.encode()),
        sealed_by: Some(record.sealed_by()),
        fields: secret.0.keys().cloned().collect(),
    })
}

async fn ensure_alias_free(
    state: &AppState,
    app_id: &str,
    alias: &str,
    except: Option<&str>,
) -> Result<(), ApiError> {
    let mut query = app_data_source::Entity::find()
        .filter(app_data_source::Column::AppId.eq(app_id))
        .filter(app_data_source::Column::Alias.eq(alias));
    if let Some(source_id) = except {
        query = query.filter(app_data_source::Column::Id.ne(source_id));
    }
    if query.one(&state.db).await?.is_some() {
        return Err(alias_taken(alias));
    }
    Ok(())
}

fn alias_taken(alias: &str) -> ApiError {
    ApiError::conflict(format!(
        "This app already has an external database with the schema name '{alias}'"
    ))
}

/// A concurrent writer can take the alias between the check and the write.
fn write_error(error: DbErr, alias: &str) -> ApiError {
    if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) {
        alias_taken(alias)
    } else {
        error.into()
    }
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/data-sources",
    tag = "database",
    description = "List the external databases this app's queries can read as `<alias>.<table>`. Credentials are never returned, only the names of the stored fields.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "External databases of the app", body = Vec<ExternalDataSource>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "GET /apps/{app_id}/data-sources", skip(state, user))]
pub async fn list_data_sources(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<ExternalDataSource>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadDatabase);
    let rows = app_data_source::Entity::find()
        .filter(app_data_source::Column::AppId.eq(&app_id))
        .order_by_asc(app_data_source::Column::Alias)
        .all(&state.db)
        .await?;
    rows.into_iter()
        .map(ExternalDataSource::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map(Json)
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/data-sources/{source_id}",
    tag = "database",
    description = "Get one external database of this app. Credentials are never returned, only the names of the stored fields.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("source_id" = String, Path, description = "Data source ID")
    ),
    responses(
        (status = 200, description = "The external database", body = ExternalDataSource),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/data-sources/{source_id}",
    skip(state, user)
)]
pub async fn get_data_source(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, source_id)): Path<(String, String)>,
) -> Result<Json<ExternalDataSource>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadDatabase);
    let row = load_source(&state.db, &app_id, &source_id).await?;
    Ok(Json(row.try_into()?))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/data-sources",
    tag = "database",
    description = "Connect an external database (PostgreSQL or CockroachDB, MySQL or MariaDB, ClickHouse, BigQuery, Athena) so this app's queries can read its tables as `<alias>.<table>`. Credentials are stored encrypted and are never returned. Requires admin access.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = CreateDataSourceRequest,
    responses(
        (status = 200, description = "The stored external database, without credentials", body = ExternalDataSource),
        (status = 400, description = "Invalid name, schema name, settings or credentials"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 409, description = "The schema name is already used in this app")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "POST /apps/{app_id}/data-sources", skip(state, user, payload))]
pub async fn create_data_source(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<CreateDataSourceRequest>,
) -> Result<Json<ExternalDataSource>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Admin);
    deny_connected_app(&user)?;
    let name = validation::name(&payload.name).map_err(ApiError::bad_request)?;
    validation::alias(&payload.alias).map_err(ApiError::bad_request)?;
    let connection =
        validation::connection(payload.driver, &payload.config).map_err(ApiError::bad_request)?;
    let secret = payload.secret.unwrap_or_default();
    validation::secret(payload.driver, &secret).map_err(ApiError::bad_request)?;
    ensure_alias_free(&state, &app_id, &payload.alias, None).await?;

    let source_id = create_id();
    let sealed = seal(&state, &app_id, &source_id, &secret).await?;
    let created_by = permission
        .sub
        .clone()
        .or_else(|| permission.effective_user_id.clone())
        .unwrap_or_else(|| permission.identifier.clone());
    let now = chrono::Utc::now().fixed_offset();
    let model = app_data_source::ActiveModel {
        id: Set(source_id.clone()),
        app_id: Set(app_id.clone()),
        name: Set(name),
        alias: Set(payload.alias.clone()),
        driver: Set(payload.driver.as_str().to_string()),
        config: Set(validation::stored_config(&connection)),
        secret: Set(sealed.secret),
        sealed_by: Set(sealed.sealed_by),
        secret_fields: Set(sealed.fields.into()),
        created_by: Set(created_by),
        last_checked_at: Set(None),
        last_check_status: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await
    .map_err(|error| write_error(error, &payload.alias))?;

    audit_branch!(
        state,
        user,
        app_id,
        "data_source.create",
        "AppDataSource",
        source_id
    );
    Ok(Json(model.try_into()?))
}

#[utoipa::path(
    patch,
    path = "/apps/{app_id}/data-sources/{source_id}",
    tag = "database",
    description = "Change an external database of this app. Fields left out keep their value; without `secret` the stored credentials stay, and `secret: {}` removes them. Requires admin access.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("source_id" = String, Path, description = "Data source ID")
    ),
    request_body = UpdateDataSourceRequest,
    responses(
        (status = 200, description = "The updated external database, without credentials", body = ExternalDataSource),
        (status = 400, description = "Invalid name, schema name, settings or credentials"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
        (status = 409, description = "The schema name is already used in this app")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/data-sources/{source_id}",
    skip(state, user, payload)
)]
pub async fn update_data_source(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, source_id)): Path<(String, String)>,
    Json(payload): Json<UpdateDataSourceRequest>,
) -> Result<Json<ExternalDataSource>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);
    deny_connected_app(&user)?;
    let row = load_source(&state.db, &app_id, &source_id).await?;
    let stored_driver = driver_of(&row)?;
    let driver = payload.driver.unwrap_or(stored_driver);
    let mut active: app_data_source::ActiveModel = row.clone().into();

    if let Some(name) = &payload.name {
        active.name = Set(validation::name(name).map_err(ApiError::bad_request)?);
    }
    let alias = payload.alias.as_deref().unwrap_or(&row.alias);
    if alias != row.alias {
        validation::alias(alias).map_err(ApiError::bad_request)?;
        ensure_alias_free(&state, &app_id, alias, Some(&source_id)).await?;
        active.alias = Set(alias.to_string());
    }

    let config = match &payload.config {
        Some(config) => config.clone(),
        None => row.config.as_object().cloned().unwrap_or_default(),
    };
    let connection = validation::connection(driver, &config).map_err(ApiError::bad_request)?;
    let config = validation::stored_config(&connection);
    let reconnects = driver != stored_driver || config != row.config || payload.secret.is_some();
    active.driver = Set(driver.as_str().to_string());
    active.config = Set(config);

    match &payload.secret {
        Some(secret) => {
            validation::secret(driver, secret).map_err(ApiError::bad_request)?;
            let sealed = seal(&state, &app_id, &source_id, secret).await?;
            active.secret = Set(sealed.secret);
            active.sealed_by = Set(sealed.sealed_by);
            active.secret_fields = Set(sealed.fields.into());
        }
        None => {
            let foreign = validation::foreign_secret_fields(driver, &row.secret_fields);
            if !foreign.is_empty() {
                return Err(ApiError::bad_request(format!(
                    "The stored credentials ({}) do not fit {} sources: send new credentials with the driver change",
                    foreign.join(", "),
                    driver.as_str()
                )));
            }
        }
    }
    if reconnects {
        active.last_checked_at = Set(None);
        active.last_check_status = Set(None);
    }
    active.updated_at = Set(chrono::Utc::now().fixed_offset());
    let model = active
        .update(&state.db)
        .await
        .map_err(|error| write_error(error, alias))?;

    audit_branch!(
        state,
        user,
        app_id,
        "data_source.update",
        "AppDataSource",
        source_id
    );
    Ok(Json(model.try_into()?))
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/data-sources/{source_id}",
    tag = "database",
    description = "Remove an external database from this app, including its stored credentials. Queries that address its schema name stop working. Requires admin access.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("source_id" = String, Path, description = "Data source ID")
    ),
    responses(
        (status = 200, description = "Removed", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "DELETE /apps/{app_id}/data-sources/{source_id}",
    skip(state, user)
)]
pub async fn delete_data_source(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, source_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);
    deny_connected_app(&user)?;
    let row = load_source(&state.db, &app_id, &source_id).await?;
    row.delete(&state.db).await?;

    audit_branch!(
        state,
        user,
        app_id,
        "data_source.delete",
        "AppDataSource",
        source_id
    );
    Ok(Json(()))
}

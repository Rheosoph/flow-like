use crate::{
    audit_branch,
    entity::app_process_note,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::{global_permission::GlobalPermission, role_permission::RolePermissions},
    routes::app::connection::graph::ProcessNoteInfo,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, ModelTrait, QueryFilter,
    QueryOrder,
};
use serde::Deserialize;
use utoipa::ToSchema;

const MAX_NOTE_LENGTH: usize = 4096;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpsertProcessNoteRequest {
    /// The note content (markdown, max 4096 characters)
    pub content: String,
}

fn validate_content(content: &str) -> Result<(), ApiError> {
    let content = content.trim();
    if content.is_empty() {
        return Err(ApiError::bad_request("Note content must not be empty"));
    }
    if content.len() > MAX_NOTE_LENGTH {
        return Err(ApiError::bad_request(format!(
            "Note content must be at most {} characters",
            MAX_NOTE_LENGTH
        )));
    }
    Ok(())
}

/// Notes can be written by app owners/admins, or by platform admins (who
/// annotate apps through the global process graph without being members).
/// Returns the author user id for attribution.
async fn ensure_note_admin(
    user: &AppUser,
    app_id: &str,
    state: &AppState,
) -> Result<Option<String>, ApiError> {
    crate::routes::app::connection::deny_connected_app(user)?;

    if let Ok(permission) = user.app_permission(app_id, state).await
        && permission.has_permission(RolePermissions::Admin)
    {
        return Ok(permission.effective_user_id().ok());
    }

    user.check_global_permission(state, GlobalPermission::Admin)
        .await?;
    Ok(user.effective_user_id().ok())
}

/// Notes can be read by any member of the app, or by platform admins.
async fn ensure_note_reader(
    user: &AppUser,
    app_id: &str,
    state: &AppState,
) -> Result<(), ApiError> {
    if user.app_permission(app_id, state).await.is_ok() {
        return Ok(());
    }
    user.check_global_permission(state, GlobalPermission::Admin)
        .await?;
    Ok(())
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/connections/notes",
    tag = "team",
    description = "List the process notes of this app. Notes document what the app does inside cross-app process chains.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Process notes", body = Vec<ProcessNoteInfo>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/connections/notes", skip(state, user))]
pub async fn list_notes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<ProcessNoteInfo>>, ApiError> {
    ensure_note_reader(&user, &app_id, &state).await?;

    let notes = app_process_note::Entity::find()
        .filter(app_process_note::Column::AppId.eq(&app_id))
        .order_by_asc(app_process_note::Column::CreatedAt)
        .all(&state.db)
        .await?;

    Ok(Json(notes.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/connections/notes",
    tag = "team",
    description = "Add a process note to this app. Only app owners and admins can annotate.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = UpsertProcessNoteRequest,
    responses(
        (status = 200, description = "Created process note", body = ProcessNoteInfo),
        (status = 400, description = "Invalid content"),
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
    name = "PUT /apps/{app_id}/connections/notes",
    skip(state, user, payload)
)]
pub async fn create_note(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<UpsertProcessNoteRequest>,
) -> Result<Json<ProcessNoteInfo>, ApiError> {
    let author_user_id = ensure_note_admin(&user, &app_id, &state).await?;

    validate_content(&payload.content)?;

    let note = app_process_note::ActiveModel {
        id: Set(create_id()),
        app_id: Set(app_id.clone()),
        author_user_id: Set(author_user_id),
        content: Set(payload.content.trim().to_string()),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
    };
    let note = note.insert(&state.db).await?;
    let note_id = note.id.clone();

    audit_branch!(
        state,
        user,
        app_id,
        "process_note.create",
        "AppProcessNote",
        note_id,
        "Process note created"
    );

    Ok(Json(note.into()))
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/connections/notes/{note_id}",
    tag = "team",
    description = "Update a process note of this app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("note_id" = String, Path, description = "Note ID")
    ),
    request_body = UpsertProcessNoteRequest,
    responses(
        (status = 200, description = "Updated process note", body = ProcessNoteInfo),
        (status = 400, description = "Invalid content"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/connections/notes/{note_id}",
    skip(state, user, payload)
)]
pub async fn update_note(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, note_id)): Path<(String, String)>,
    Json(payload): Json<UpsertProcessNoteRequest>,
) -> Result<Json<ProcessNoteInfo>, ApiError> {
    ensure_note_admin(&user, &app_id, &state).await?;

    validate_content(&payload.content)?;

    let note = app_process_note::Entity::find()
        .filter(
            app_process_note::Column::Id
                .eq(&note_id)
                .and(app_process_note::Column::AppId.eq(&app_id)),
        )
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut active: app_process_note::ActiveModel = note.into();
    active.content = Set(payload.content.trim().to_string());
    active.updated_at = Set(chrono::Utc::now().naive_utc());
    let note = active.update(&state.db).await?;

    audit_branch!(
        state,
        user,
        app_id,
        "process_note.update",
        "AppProcessNote",
        note_id,
        "Process note updated"
    );

    Ok(Json(note.into()))
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/connections/notes/{note_id}",
    tag = "team",
    description = "Delete a process note of this app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("note_id" = String, Path, description = "Note ID")
    ),
    responses(
        (status = 200, description = "Note deleted", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "DELETE /apps/{app_id}/connections/notes/{note_id}",
    skip(state, user)
)]
pub async fn delete_note(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, note_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    ensure_note_admin(&user, &app_id, &state).await?;

    let note = app_process_note::Entity::find()
        .filter(
            app_process_note::Column::Id
                .eq(&note_id)
                .and(app_process_note::Column::AppId.eq(&app_id)),
        )
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    note.delete(&state.db).await?;

    audit_branch!(
        state,
        user,
        app_id,
        "process_note.delete",
        "AppProcessNote",
        note_id,
        "Process note deleted"
    );

    Ok(Json(()))
}

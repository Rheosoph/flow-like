use crate::{
    deletion::{self, AcceptedDeletion, Deleted, DeletionRoot},
    entity::course_module,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    routes::course::access::{ensure_course_exists, ensure_module_in_course},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait, IntoActiveModel};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct ModuleUpsertBody {
    pub title: String,
    pub description: Option<String>,
    pub position: Option<i32>,
}

#[utoipa::path(
    put,
    path = "/courses/{course_id}/modules/{module_id}",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("module_id" = String, Path, description = "Module identifier")
    ),
    request_body = ModuleUpsertBody,
    responses(
        (status = 200, description = "Module created or updated", body = Object),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "PUT /courses/{course_id}/modules/{module_id}",
    skip(state, user, body)
)]
pub async fn upsert_module(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, module_id)): Path<(String, String)>,
    Json(body): Json<ModuleUpsertBody>,
) -> Result<Json<course_module::Model>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;

    let now = chrono::Utc::now().naive_utc();
    let existing = course_module::Entity::find_by_id(&module_id)
        .one(&state.db)
        .await?;

    let saved = if let Some(m) = existing {
        if m.course_id != course_id {
            return Err(ApiError::NOT_FOUND);
        }
        let mut active = m.into_active_model();
        active.title = Set(body.title);
        active.description = Set(body.description);
        active.position = Set(body.position.unwrap_or(0));
        active.updated_at = Set(now);
        active.update(&state.db).await?
    } else {
        ensure_course_exists(&state, &course_id).await?;
        let active = course_module::ActiveModel {
            id: Set(module_id),
            course_id: Set(course_id),
            title: Set(body.title),
            description: Set(body.description),
            position: Set(body.position.unwrap_or(0)),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&state.db).await?
    };

    Ok(Json(saved))
}

#[utoipa::path(
    delete,
    path = "/courses/{course_id}/modules/{module_id}",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("module_id" = String, Path, description = "Module identifier")
    ),
    responses(
        (status = 200, description = "Module deleted"),
        (status = 202, description = "Queued for deletion; follow the job on `GET /admin/deletions/{job_id}`", body = AcceptedDeletion),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "DELETE /courses/{course_id}/modules/{module_id}",
    skip(state, user)
)]
pub async fn delete_module(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, module_id)): Path<(String, String)>,
) -> Result<Deleted<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;
    ensure_module_in_course(&state, &course_id, &module_id).await?;
    let requested_by = user.sub().ok();
    deletion::delete_now(
        &state,
        DeletionRoot::CourseModule,
        &module_id,
        requested_by.as_deref(),
        (),
    )
    .await
}

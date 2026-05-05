use crate::{
    entity::{lesson_app_ref, sea_orm_active_enums::LessonAppRefKind},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::{global_permission::GlobalPermission, role_permission::RolePermissions},
    routes::course::access::{ensure_app_ref_in_lesson, ensure_lesson_in_course},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::Value;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait, IntoActiveModel};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AppRefUpsertBody {
    pub kind: String,
    #[schema(value_type = Object)]
    pub target: Value,
    pub app_alias: Option<String>,
    pub app_id: Option<String>,
    pub label: Option<String>,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct AppRefView {
    pub id: String,
    pub lesson_id: String,
    pub app_alias: Option<String>,
    pub app_id: Option<String>,
    pub kind: String,
    #[schema(value_type = Object)]
    pub target: Value,
    pub label: Option<String>,
}

pub fn parse_ref_kind(s: &str) -> LessonAppRefKind {
    match s.to_uppercase().as_str() {
        "FOCUS_NODE" => LessonAppRefKind::FocusNode,
        "ADD_NODE" => LessonAppRefKind::AddNode,
        "CREATE_EVENT" => LessonAppRefKind::CreateEvent,
        "OPEN_OR_CLONE_APP" => LessonAppRefKind::OpenOrCloneApp,
        _ => LessonAppRefKind::Navigate,
    }
}

pub fn ref_kind_to_string(k: &LessonAppRefKind) -> &'static str {
    match k {
        LessonAppRefKind::Navigate => "NAVIGATE",
        LessonAppRefKind::FocusNode => "FOCUS_NODE",
        LessonAppRefKind::AddNode => "ADD_NODE",
        LessonAppRefKind::CreateEvent => "CREATE_EVENT",
        LessonAppRefKind::OpenOrCloneApp => "OPEN_OR_CLONE_APP",
    }
}

async fn ensure_app_admin(state: &AppState, user: &AppUser, app_id: &str) -> Result<(), ApiError> {
    let app_id = app_id.trim();
    if app_id.is_empty() {
        return Err(ApiError::bad_request("Invalid app id"));
    }

    let app_permission = user.app_permission(app_id, state).await?;
    if app_permission.has_permission(RolePermissions::Admin) {
        Ok(())
    } else {
        Err(ApiError::FORBIDDEN)
    }
}

async fn ensure_app_ref_targets_allowed(
    state: &AppState,
    user: &AppUser,
    kind: &LessonAppRefKind,
    body: &AppRefUpsertBody,
) -> Result<(), ApiError> {
    if let Some(app_id) = body.app_id.as_deref() {
        ensure_app_admin(state, user, app_id).await?;
    }

    if matches!(kind, LessonAppRefKind::OpenOrCloneApp) {
        for key in ["sharedAppId", "shared_app_id"] {
            if let Some(app_id) = body.target.get(key).and_then(|value| value.as_str()) {
                ensure_app_admin(state, user, app_id).await?;
            }
        }
    }

    Ok(())
}

impl From<lesson_app_ref::Model> for AppRefView {
    fn from(value: lesson_app_ref::Model) -> Self {
        Self {
            id: value.id,
            lesson_id: value.lesson_id,
            app_alias: value.app_alias,
            app_id: value.app_id,
            kind: ref_kind_to_string(&value.kind).to_string(),
            target: value.target,
            label: value.label,
        }
    }
}

#[utoipa::path(
    put,
    path = "/courses/{course_id}/lessons/{lesson_id}/refs/{ref_id}",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("lesson_id" = String, Path, description = "Lesson identifier"),
        ("ref_id" = String, Path, description = "App reference identifier")
    ),
    request_body = AppRefUpsertBody,
    responses(
        (status = 200, description = "App reference created or updated", body = AppRefView),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "PUT /courses/{course_id}/lessons/{lesson_id}/refs/{ref_id}",
    skip(state, user, body)
)]
pub async fn upsert_app_ref(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, lesson_id, ref_id)): Path<(String, String, String)>,
    Json(body): Json<AppRefUpsertBody>,
) -> Result<Json<AppRefView>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;

    let now = chrono::Utc::now().naive_utc();
    let kind = parse_ref_kind(&body.kind);
    ensure_app_ref_targets_allowed(&state, &user, &kind, &body).await?;
    let existing = lesson_app_ref::Entity::find_by_id(&ref_id)
        .one(&state.db)
        .await?;

    let saved = if let Some(m) = existing {
        ensure_app_ref_in_lesson(&state, &course_id, &lesson_id, &ref_id).await?;
        let mut active = m.into_active_model();
        active.kind = Set(kind);
        active.target = Set(body.target);
        active.app_alias = Set(body.app_alias);
        active.app_id = Set(body.app_id);
        active.label = Set(body.label);
        active.updated_at = Set(now);
        active.update(&state.db).await?
    } else {
        ensure_lesson_in_course(&state, &course_id, &lesson_id).await?;
        let active = lesson_app_ref::ActiveModel {
            id: Set(ref_id),
            lesson_id: Set(lesson_id),
            kind: Set(kind),
            target: Set(body.target),
            app_alias: Set(body.app_alias),
            app_id: Set(body.app_id),
            label: Set(body.label),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&state.db).await?
    };

    Ok(Json(saved.into()))
}

#[utoipa::path(
    delete,
    path = "/courses/{course_id}/lessons/{lesson_id}/refs/{ref_id}",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("lesson_id" = String, Path, description = "Lesson identifier"),
        ("ref_id" = String, Path, description = "App reference identifier")
    ),
    responses(
        (status = 200, description = "App reference deleted"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "DELETE /courses/{course_id}/lessons/{lesson_id}/refs/{ref_id}",
    skip(state, user)
)]
pub async fn delete_app_ref(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, lesson_id, ref_id)): Path<(String, String, String)>,
) -> Result<Json<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;
    ensure_app_ref_in_lesson(&state, &course_id, &lesson_id, &ref_id).await?;
    lesson_app_ref::Entity::delete_by_id(ref_id)
        .exec(&state.db)
        .await?;
    Ok(Json(()))
}

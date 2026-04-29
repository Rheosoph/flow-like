use crate::{
    entity::{challenge, course_module, lesson, lesson_app_ref, user_challenge_attempt},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    routes::course::{
        access::{
            ensure_course_readable, ensure_lesson_in_module, ensure_module_in_course,
            has_course_read_grant,
        },
        app_refs::AppRefView,
        attempts::ChallengeAttemptView,
        challenges::ChallengeView,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct LessonUpsertBody {
    pub title: String,
    pub language: Option<String>,
    pub content: String,
    pub video_url: Option<String>,
    pub estimated_minutes: Option<i32>,
    pub position: Option<i32>,
    pub is_optional: Option<bool>,
}

#[derive(Clone, Serialize, ToSchema)]
pub struct LessonWithChildren {
    #[schema(value_type = Object)]
    pub lesson: lesson::Model,
    pub challenges: Vec<ChallengeView>,
    pub app_refs: Vec<AppRefView>,
    pub attempts: Vec<ChallengeAttemptView>,
}

#[utoipa::path(
    get,
    path = "/courses/{course_id}/modules/{module_id}/lessons/{lesson_id}",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("module_id" = String, Path, description = "Module identifier"),
        ("lesson_id" = String, Path, description = "Lesson identifier")
    ),
    responses(
        (status = 200, description = "Lesson with challenges and app references", body = LessonWithChildren),
        (status = 404, description = "Lesson not found")
    )
)]
#[tracing::instrument(
    name = "GET /courses/{course_id}/modules/{module_id}/lessons/{lesson_id}",
    skip(state, user)
)]
pub async fn get_lesson(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, module_id, lesson_id)): Path<(String, String, String)>,
) -> Result<Json<LessonWithChildren>, ApiError> {
    let sub = user.sub()?;

    let module = course_module::Entity::find_by_id(&module_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if module.course_id != course_id {
        return Err(ApiError::NOT_FOUND);
    }
    ensure_course_readable(&state, &user, &course_id).await?;
    let include_solution_payload = has_course_read_grant(&state, &user).await;

    let lesson = lesson::Entity::find_by_id(&lesson_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if lesson.module_id != module_id {
        return Err(ApiError::NOT_FOUND);
    }

    let challenges = challenge::Entity::find()
        .filter(challenge::Column::LessonId.eq(&lesson_id))
        .order_by_asc(challenge::Column::Position)
        .all(&state.db)
        .await?;
    let challenge_ids = challenges
        .iter()
        .map(|challenge| challenge.id.clone())
        .collect::<Vec<_>>();

    let attempts = if challenge_ids.is_empty() {
        vec![]
    } else {
        user_challenge_attempt::Entity::find()
            .filter(user_challenge_attempt::Column::UserId.eq(&sub))
            .filter(user_challenge_attempt::Column::ChallengeId.is_in(challenge_ids))
            .order_by_desc(user_challenge_attempt::Column::AttemptedAt)
            .all(&state.db)
            .await?
    };

    let app_refs = lesson_app_ref::Entity::find()
        .filter(lesson_app_ref::Column::LessonId.eq(&lesson_id))
        .all(&state.db)
        .await?;

    Ok(Json(LessonWithChildren {
        lesson,
        challenges: challenges
            .into_iter()
            .map(|challenge| ChallengeView::from_model(challenge, include_solution_payload))
            .collect(),
        app_refs: app_refs.into_iter().map(Into::into).collect(),
        attempts: attempts.into_iter().map(Into::into).collect(),
    }))
}

#[utoipa::path(
    put,
    path = "/courses/{course_id}/modules/{module_id}/lessons/{lesson_id}",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("module_id" = String, Path, description = "Module identifier"),
        ("lesson_id" = String, Path, description = "Lesson identifier")
    ),
    request_body = LessonUpsertBody,
    responses(
        (status = 200, description = "Lesson created or updated", body = Object),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "PUT /courses/{course_id}/modules/{module_id}/lessons/{lesson_id}",
    skip(state, user, body)
)]
pub async fn upsert_lesson(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, module_id, lesson_id)): Path<(String, String, String)>,
    Json(body): Json<LessonUpsertBody>,
) -> Result<Json<lesson::Model>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;

    let now = chrono::Utc::now().naive_utc();
    let existing = lesson::Entity::find_by_id(&lesson_id)
        .one(&state.db)
        .await?;

    let saved = if let Some(m) = existing {
        ensure_lesson_in_module(&state, &course_id, &module_id, &lesson_id).await?;
        let mut active = m.into_active_model();
        active.title = Set(body.title);
        active.language = Set(body.language.unwrap_or_else(|| "en".to_string()));
        active.content = Set(body.content);
        active.video_url = Set(body.video_url);
        active.estimated_minutes = Set(body.estimated_minutes.unwrap_or(5));
        active.position = Set(body.position.unwrap_or(0));
        active.is_optional = Set(body.is_optional.unwrap_or(false));
        active.updated_at = Set(now);
        active.update(&state.db).await?
    } else {
        ensure_module_in_course(&state, &course_id, &module_id).await?;
        let active = lesson::ActiveModel {
            id: Set(lesson_id),
            module_id: Set(module_id),
            title: Set(body.title),
            language: Set(body.language.unwrap_or_else(|| "en".to_string())),
            content: Set(body.content),
            video_url: Set(body.video_url),
            estimated_minutes: Set(body.estimated_minutes.unwrap_or(5)),
            position: Set(body.position.unwrap_or(0)),
            is_optional: Set(body.is_optional.unwrap_or(false)),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&state.db).await?
    };

    Ok(Json(saved))
}

#[utoipa::path(
    delete,
    path = "/courses/{course_id}/modules/{module_id}/lessons/{lesson_id}",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("module_id" = String, Path, description = "Module identifier"),
        ("lesson_id" = String, Path, description = "Lesson identifier")
    ),
    responses(
        (status = 200, description = "Lesson deleted"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "DELETE /courses/{course_id}/modules/{module_id}/lessons/{lesson_id}",
    skip(state, user)
)]
pub async fn delete_lesson(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, module_id, lesson_id)): Path<(String, String, String)>,
) -> Result<Json<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;
    ensure_lesson_in_module(&state, &course_id, &module_id, &lesson_id).await?;
    lesson::Entity::delete_by_id(lesson_id)
        .exec(&state.db)
        .await?;
    Ok(Json(()))
}

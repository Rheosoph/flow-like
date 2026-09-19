use crate::{
    entity::{
        challenge, course_module, lesson, sea_orm_active_enums::LessonStatus,
        user_challenge_attempt, user_course_enrollment, user_lesson_progress,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    routes::course::access::{ensure_course_readable, ensure_lesson_course_readable},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, JoinType,
    QueryFilter, QuerySelect, RelationTrait,
};
use serde::Serialize;
use serde_json::json;
use std::collections::HashSet;
use utoipa::ToSchema;

#[derive(Clone, Serialize, ToSchema)]
pub struct UserLessonProgressView {
    pub id: String,
    pub user_id: String,
    pub lesson_id: String,
    pub status: String,
    pub completed_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub created_at: chrono::DateTime<chrono::FixedOffset>,
    pub updated_at: chrono::DateTime<chrono::FixedOffset>,
}

fn lesson_status_to_string(status: &LessonStatus) -> &'static str {
    match status {
        LessonStatus::NotStarted => "NOT_STARTED",
        LessonStatus::InProgress => "IN_PROGRESS",
        LessonStatus::Completed => "COMPLETED",
    }
}

impl From<user_lesson_progress::Model> for UserLessonProgressView {
    fn from(value: user_lesson_progress::Model) -> Self {
        Self {
            id: value.id,
            user_id: value.user_id,
            lesson_id: value.lesson_id,
            status: lesson_status_to_string(&value.status).to_string(),
            completed_at: value.completed_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

async fn lessons_challenges_completed(
    state: &AppState,
    user_id: &str,
    lesson_ids: &[String],
) -> Result<bool, ApiError> {
    if lesson_ids.is_empty() {
        return Ok(true);
    }

    let challenge_ids: Vec<String> = challenge::Entity::find()
        .select_only()
        .column(challenge::Column::Id)
        .filter(challenge::Column::LessonId.is_in(lesson_ids.iter().cloned()))
        .into_tuple()
        .all(&state.db)
        .await?;

    if challenge_ids.is_empty() {
        return Ok(true);
    }

    let completed_challenge_ids: HashSet<String> = user_challenge_attempt::Entity::find()
        .select_only()
        .column(user_challenge_attempt::Column::ChallengeId)
        .distinct()
        .filter(user_challenge_attempt::Column::UserId.eq(user_id))
        .filter(user_challenge_attempt::Column::ChallengeId.is_in(challenge_ids.iter().cloned()))
        .filter(user_challenge_attempt::Column::IsCorrect.eq(true))
        .into_tuple::<String>()
        .all(&state.db)
        .await?
        .into_iter()
        .collect();

    Ok(challenge_ids
        .iter()
        .all(|challenge_id| completed_challenge_ids.contains(challenge_id)))
}

pub async fn lesson_challenges_completed(
    state: &AppState,
    user_id: &str,
    lesson_id: &str,
) -> Result<bool, ApiError> {
    lessons_challenges_completed(state, user_id, &[lesson_id.to_string()]).await
}

pub async fn required_lessons_completed(
    state: &AppState,
    user_id: &str,
    course_id: &str,
) -> Result<bool, ApiError> {
    let module_ids: Vec<String> = course_module::Entity::find()
        .select_only()
        .column(course_module::Column::Id)
        .filter(course_module::Column::CourseId.eq(course_id))
        .into_tuple()
        .all(&state.db)
        .await?;

    if module_ids.is_empty() {
        return Ok(false);
    }

    let required_lesson_ids: Vec<String> = lesson::Entity::find()
        .select_only()
        .column(lesson::Column::Id)
        .filter(lesson::Column::ModuleId.is_in(module_ids))
        .filter(lesson::Column::IsOptional.eq(false))
        .into_tuple()
        .all(&state.db)
        .await?;

    if required_lesson_ids.is_empty() {
        return Ok(true);
    }

    let completed_lesson_ids: HashSet<String> = user_lesson_progress::Entity::find()
        .select_only()
        .column(user_lesson_progress::Column::LessonId)
        .filter(user_lesson_progress::Column::UserId.eq(user_id))
        .filter(user_lesson_progress::Column::LessonId.is_in(required_lesson_ids.iter().cloned()))
        .filter(user_lesson_progress::Column::Status.eq(LessonStatus::Completed))
        .into_tuple::<String>()
        .all(&state.db)
        .await?
        .into_iter()
        .collect();

    if !required_lesson_ids
        .iter()
        .all(|lesson_id| completed_lesson_ids.contains(lesson_id))
    {
        return Ok(false);
    }

    lessons_challenges_completed(state, user_id, &required_lesson_ids).await
}

async fn refresh_course_enrollment_completion(
    state: &AppState,
    user_id: &str,
    course_id: &str,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<(), ApiError> {
    let is_complete = required_lessons_completed(state, user_id, course_id).await?;
    let existing = user_course_enrollment::Entity::find()
        .filter(user_course_enrollment::Column::UserId.eq(user_id))
        .filter(user_course_enrollment::Column::CourseId.eq(course_id))
        .one(&state.db)
        .await?;

    if let Some(enrollment) = existing {
        let already_completed = enrollment.completed_at.is_some();
        let mut active = enrollment.into_active_model();
        active.last_seen_at = Set(now);
        if is_complete && !already_completed {
            active.completed_at = Set(Some(now));
        }
        active.update(&state.db).await?;
    } else {
        let active = user_course_enrollment::ActiveModel {
            id: Set(create_id()),
            user_id: Set(user_id.to_string()),
            course_id: Set(course_id.to_string()),
            linked_app_ids: Set(json!({})),
            id_maps: Set(json!({})),
            started_at: Set(now),
            last_seen_at: Set(now),
            completed_at: Set(is_complete.then_some(now)),
        };
        active.insert(&state.db).await?;
    }

    Ok(())
}

#[utoipa::path(
    post,
    path = "/courses/lessons/{lesson_id}/complete",
    tag = "courses",
    params(("lesson_id" = String, Path, description = "Lesson identifier")),
    responses(
        (status = 200, description = "Lesson marked complete", body = UserLessonProgressView)
    )
)]
#[tracing::instrument(name = "POST /courses/lessons/{lesson_id}/complete", skip(state, user))]
pub async fn mark_lesson_complete(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(lesson_id): Path<String>,
) -> Result<Json<UserLessonProgressView>, ApiError> {
    let sub = user.sub()?;
    let now = chrono::Utc::now().fixed_offset();

    let (_lesson, module) = ensure_lesson_course_readable(&state, &user, &lesson_id).await?;
    if !lesson_challenges_completed(&state, &sub, &lesson_id).await? {
        return Err(ApiError::forbidden(
            "All lesson challenges must be completed successfully before marking the lesson complete",
        ));
    }

    let existing = user_lesson_progress::Entity::find()
        .filter(user_lesson_progress::Column::UserId.eq(&sub))
        .filter(user_lesson_progress::Column::LessonId.eq(&lesson_id))
        .one(&state.db)
        .await?;

    let saved = if let Some(p) = existing {
        let mut active = p.into_active_model();
        active.status = Set(LessonStatus::Completed);
        active.completed_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(&state.db).await?
    } else {
        let active = user_lesson_progress::ActiveModel {
            id: Set(create_id()),
            user_id: Set(sub.clone()),
            lesson_id: Set(lesson_id),
            status: Set(LessonStatus::Completed),
            completed_at: Set(Some(now)),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&state.db).await?
    };

    refresh_course_enrollment_completion(&state, &sub, &module.course_id, now).await?;

    Ok(Json(saved.into()))
}

#[utoipa::path(
    get,
    path = "/courses/{course_id}/progress/me",
    tag = "courses",
    params(("course_id" = String, Path, description = "Course identifier")),
    responses(
        (status = 200, description = "Returns the current user's progress for the course", body = Vec<UserLessonProgressView>)
    )
)]
#[tracing::instrument(name = "GET /courses/{course_id}/progress/me", skip(state, user))]
pub async fn get_my_course_progress(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(course_id): Path<String>,
) -> Result<Json<Vec<UserLessonProgressView>>, ApiError> {
    let sub = user.sub()?;
    ensure_course_readable(&state, &user, &course_id).await?;

    let rows = user_lesson_progress::Entity::find()
        .join(
            JoinType::InnerJoin,
            user_lesson_progress::Relation::Lesson.def(),
        )
        .join(JoinType::InnerJoin, lesson::Relation::CourseModule.def())
        .filter(course_module::Column::CourseId.eq(&course_id))
        .filter(user_lesson_progress::Column::UserId.eq(sub))
        .all(&state.db)
        .await?;

    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

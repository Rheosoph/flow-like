use crate::{
    entity::user_course_enrollment, error::ApiError, middleware::jwt::AppUser,
    routes::course::access::ensure_course_readable, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
};
use serde_json::json;

#[utoipa::path(
    post,
    path = "/courses/{course_id}/enroll",
    tag = "courses",
    params(("course_id" = String, Path, description = "Course identifier")),
    responses(
        (status = 200, description = "Enrollment created or refreshed", body = Object)
    )
)]
#[tracing::instrument(name = "POST /courses/{course_id}/enroll", skip(state, user))]
pub async fn enroll(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(course_id): Path<String>,
) -> Result<Json<user_course_enrollment::Model>, ApiError> {
    let sub = user.sub()?;
    let now = chrono::Utc::now().naive_utc();
    ensure_course_readable(&state, &user, &course_id).await?;

    let existing = user_course_enrollment::Entity::find()
        .filter(user_course_enrollment::Column::UserId.eq(&sub))
        .filter(user_course_enrollment::Column::CourseId.eq(&course_id))
        .one(&state.db)
        .await?;

    let saved = if let Some(e) = existing {
        let mut active = e.into_active_model();
        active.last_seen_at = Set(now);
        active.update(&state.db).await?
    } else {
        let active = user_course_enrollment::ActiveModel {
            id: Set(create_id()),
            user_id: Set(sub),
            course_id: Set(course_id),
            linked_app_ids: Set(json!({})),
            id_maps: Set(json!({})),
            started_at: Set(now),
            last_seen_at: Set(now),
            completed_at: Set(None),
        };
        active.insert(&state.db).await?
    };

    Ok(Json(saved))
}

#[utoipa::path(
    get,
    path = "/courses/enrollments/me",
    tag = "courses",
    responses(
        (status = 200, description = "Returns the current user's course enrollments", body = Vec<Object>)
    )
)]
#[tracing::instrument(name = "GET /courses/enrollments/me", skip(state, user))]
pub async fn get_my_enrollments(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Vec<user_course_enrollment::Model>>, ApiError> {
    let sub = user.sub()?;
    let rows = user_course_enrollment::Entity::find()
        .filter(user_course_enrollment::Column::UserId.eq(sub))
        .all(&state.db)
        .await?;
    Ok(Json(rows))
}

use crate::{
    entity::{challenge, course, course_module, lesson, weekly_challenge},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    routes::course::{access::has_course_read_grant, challenges::ChallengeView},
    state::AppState,
};
use axum::{Extension, Json, extract::State};
use chrono::Datelike;
use flow_like_types::create_id;
use rand::seq::IndexedRandom;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct CurrentWeekly {
    pub week_iso: String,
    #[schema(value_type = Option<Object>)]
    pub challenge: Option<ChallengeView>,
    pub expires_at: Option<String>,
}

fn iso_week(now: chrono::DateTime<chrono::Utc>) -> String {
    let iso = now.iso_week();
    format!("{}-W{:02}", iso.year(), iso.week())
}

#[utoipa::path(
    get,
    path = "/courses/weekly",
    tag = "courses",
    responses(
        (status = 200, description = "Returns the active weekly challenge if one is set", body = CurrentWeekly)
    )
)]
#[tracing::instrument(name = "GET /courses/weekly", skip(state, user))]
pub async fn get_current_weekly(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<CurrentWeekly>, ApiError> {
    let now_utc = chrono::Utc::now();
    let week = iso_week(now_utc);

    let entry = weekly_challenge::Entity::find()
        .filter(weekly_challenge::Column::WeekIso.eq(&week))
        .one(&state.db)
        .await?;
    let include_solution_payload = has_course_read_grant(&state, &user).await;

    let (challenge_opt, expires_at) = if let Some(e) = entry {
        let c = challenge::Entity::find_by_id(&e.challenge_id)
            .one(&state.db)
            .await?;
        let c = if let Some(challenge) = c {
            let lesson = lesson::Entity::find_by_id(&challenge.lesson_id)
                .one(&state.db)
                .await?;
            let module = match lesson {
                Some(lesson) => {
                    course_module::Entity::find_by_id(&lesson.module_id)
                        .one(&state.db)
                        .await?
                }
                None => None,
            };
            let readable = match module {
                Some(module) => {
                    let course = course::Entity::find_by_id(&module.course_id)
                        .one(&state.db)
                        .await?;
                    course.map(|course| course.is_published).unwrap_or(false)
                        || include_solution_payload
                }
                None => false,
            };
            readable.then(|| ChallengeView::from_model(challenge, include_solution_payload))
        } else {
            None
        };
        (c, Some(e.expires_at.to_rfc3339()))
    } else {
        (None, None)
    };

    Ok(Json(CurrentWeekly {
        week_iso: week,
        challenge: challenge_opt,
        expires_at,
    }))
}

#[utoipa::path(
    post,
    path = "/courses/weekly/rotate",
    tag = "courses",
    responses(
        (status = 200, description = "Rotates the weekly challenge (admin / scheduler)", body = CurrentWeekly),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "POST /courses/weekly/rotate", skip(state, user))]
pub async fn rotate_weekly(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<CurrentWeekly>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;
    let now_utc = chrono::Utc::now();
    let week = iso_week(now_utc);

    let candidate_rows = challenge::Entity::find()
        .find_also_related(lesson::Entity)
        .order_by_desc(challenge::Column::UpdatedAt)
        .limit(Some(50))
        .all(&state.db)
        .await?;
    let mut candidates = Vec::new();
    for (challenge, lesson) in candidate_rows {
        let Some(lesson) = lesson else {
            continue;
        };
        let Some(module) = course_module::Entity::find_by_id(&lesson.module_id)
            .one(&state.db)
            .await?
        else {
            continue;
        };
        let Some(course) = course::Entity::find_by_id(&module.course_id)
            .one(&state.db)
            .await?
        else {
            continue;
        };
        if course.is_published {
            candidates.push(challenge);
        }
    }
    let chosen = {
        let mut rng = rand::rng();
        candidates.choose(&mut rng).cloned()
    };

    let now = now_utc.fixed_offset();
    let expires = (now_utc + chrono::Duration::days(7)).fixed_offset();

    let existing = weekly_challenge::Entity::find()
        .filter(weekly_challenge::Column::WeekIso.eq(&week))
        .one(&state.db)
        .await?;

    if let Some(e) = existing {
        weekly_challenge::Entity::delete_by_id(e.id)
            .exec(&state.db)
            .await?;
    }

    let challenge_opt = if let Some(c) = chosen {
        let active = weekly_challenge::ActiveModel {
            id: Set(create_id()),
            week_iso: Set(week.clone()),
            challenge_id: Set(c.id.clone()),
            expires_at: Set(expires),
            created_at: Set(now),
        };
        active.insert(&state.db).await?;
        Some(ChallengeView::from_model(c, true))
    } else {
        None
    };

    Ok(Json(CurrentWeekly {
        week_iso: week,
        challenge: challenge_opt,
        expires_at: Some(expires.to_rfc3339()),
    }))
}

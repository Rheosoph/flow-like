use crate::{
    entity::{leaderboard_opt_in, user},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::{PaginationParams, user::sign_avatar},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct LeaderboardEntry {
    pub user_id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub total_points: i32,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct OptInBody {
    pub display_name: String,
    pub is_opted_in: bool,
}

fn verified_display_name(user: Option<&user::Model>) -> String {
    user.and_then(|user| {
        user.name
            .as_deref()
            .or(user.preferred_username.as_deref())
            .or(user.username.as_deref())
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string())
    })
    .unwrap_or_else(|| "Anonymous".to_string())
}

async fn verified_user_snapshot(
    state: &AppState,
    user_id: &str,
) -> Result<(String, Option<String>), ApiError> {
    let user = user::Entity::find_by_id(user_id).one(&state.db).await?;
    let display_name = verified_display_name(user.as_ref());
    let avatar_url = match user.as_ref().and_then(|user| user.avatar.as_deref()) {
        Some(avatar_id) => sign_avatar(user_id, avatar_id, state).await.ok(),
        None => None,
    };
    Ok((display_name, avatar_url))
}

#[utoipa::path(
    get,
    path = "/courses/leaderboard",
    tag = "courses",
    params(
        ("limit" = Option<u64>, Query, description = "Maximum entries (max 100)"),
        ("offset" = Option<u64>, Query, description = "Offset for pagination")
    ),
    responses(
        (status = 200, description = "Returns the public leaderboard (opted-in users only)", body = Vec<LeaderboardEntry>)
    )
)]
#[tracing::instrument(name = "GET /courses/leaderboard", skip_all)]
pub async fn get_leaderboard(
    State(state): State<AppState>,
    Extension(_user): Extension<AppUser>,
    Query(q): Query<PaginationParams>,
) -> Result<Json<Vec<LeaderboardEntry>>, ApiError> {
    let limit = q.limit.unwrap_or(100).min(100);

    let rows = leaderboard_opt_in::Entity::find()
        .filter(leaderboard_opt_in::Column::IsOptedIn.eq(true))
        .order_by_desc(leaderboard_opt_in::Column::TotalPoints)
        .limit(Some(limit))
        .offset(q.offset)
        .all(&state.db)
        .await?;
    let user_ids = rows
        .iter()
        .map(|row| row.user_id.clone())
        .collect::<Vec<_>>();
    let users = if user_ids.is_empty() {
        HashMap::new()
    } else {
        user::Entity::find()
            .filter(user::Column::Id.is_in(user_ids))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|user| (user.id.clone(), user))
            .collect::<HashMap<_, _>>()
    };

    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let user = users.get(&row.user_id);
        let avatar_url = match user.and_then(|user| user.avatar.as_deref()) {
            Some(avatar_id) => sign_avatar(&row.user_id, avatar_id, &state).await.ok(),
            None => None,
        };
        entries.push(LeaderboardEntry {
            user_id: row.user_id,
            display_name: verified_display_name(user),
            avatar_url,
            total_points: row.total_points,
        });
    }

    Ok(Json(entries))
}

#[utoipa::path(
    get,
    path = "/courses/leaderboard/me",
    tag = "courses",
    responses(
        (status = 200, description = "Returns the current user's leaderboard opt-in record", body = Object)
    )
)]
#[tracing::instrument(name = "GET /courses/leaderboard/me", skip(state, user))]
pub async fn get_my_opt_in(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Option<leaderboard_opt_in::Model>>, ApiError> {
    let sub = user.sub()?;
    let row = leaderboard_opt_in::Entity::find_by_id(&sub)
        .one(&state.db)
        .await?;
    let Some(row) = row else {
        return Ok(Json(None));
    };
    let (display_name, _) = verified_user_snapshot(&state, &sub).await?;
    if row.display_name == display_name {
        return Ok(Json(Some(row)));
    }
    let mut active = row.into_active_model();
    active.display_name = Set(display_name);
    active.updated_at = Set(chrono::Utc::now().naive_utc());
    let row = active.update(&state.db).await?;
    Ok(Json(Some(row)))
}

#[utoipa::path(
    put,
    path = "/courses/leaderboard/me",
    tag = "courses",
    request_body = OptInBody,
    responses(
        (status = 200, description = "Updates leaderboard opt-in preference", body = Object)
    )
)]
#[tracing::instrument(name = "PUT /courses/leaderboard/me", skip(state, user, body))]
pub async fn update_my_opt_in(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<OptInBody>,
) -> Result<Json<leaderboard_opt_in::Model>, ApiError> {
    let sub = user.sub()?;
    let now = chrono::Utc::now().naive_utc();
    let (display_name, _) = verified_user_snapshot(&state, &sub).await?;
    let existing = leaderboard_opt_in::Entity::find_by_id(&sub)
        .one(&state.db)
        .await?;

    let saved = if let Some(e) = existing {
        let mut active = e.into_active_model();
        active.display_name = Set(display_name);
        active.is_opted_in = Set(body.is_opted_in);
        active.updated_at = Set(now);
        active.update(&state.db).await?
    } else {
        let active = leaderboard_opt_in::ActiveModel {
            user_id: Set(sub),
            display_name: Set(display_name),
            is_opted_in: Set(body.is_opted_in),
            total_points: Set(0),
            updated_at: Set(now),
        };
        active.insert(&state.db).await?
    };

    Ok(Json(saved))
}

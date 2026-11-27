//! A lead offered by an admin becomes the lead once the bot's owner agrees; the lead's owner may
//! step it down again.

use super::{
    access::{active, forbidden, is_active_in, load_team, org_of},
    effects::{Effects, settle},
    lock_org_scope, lock_scope,
    room::sync_room,
    teams::clear_offer,
    threads::sync_team_threads,
    view::team_view,
};
use crate::{
    entity::{bot_team, bot_team_bot},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::{bots::owned_bot, inbox::now_millis},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::bot_team::Team;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ConnectionTrait, EntityTrait};

fn no_offer(team: &bot_team::Model, bot_id: &str) -> ApiError {
    ApiError::not_found(format!(
        "Team '{}' has no lead offer for bot '{bot_id}'",
        team.name
    ))
}

async fn offered_team<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    bot_id: &str,
) -> Result<bot_team::Model, ApiError> {
    owned_bot(db, sub, bot_id).await?;
    let team = load_team(db, team_id).await?;
    if team.lead_offer_bot_id.as_deref() != Some(bot_id) {
        return Err(no_offer(&team, bot_id));
    }
    Ok(team)
}

async fn store_lead<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    lead: Option<String>,
    now: DateTime<FixedOffset>,
) -> Result<bot_team::Model, ApiError> {
    let mut update = bot_team::ActiveModel {
        id: Set(team.id.clone()),
        lead_bot_id: Set(lead),
        updated_at: Set(now),
        ..Default::default()
    };
    clear_offer(&mut update);
    Ok(update.update(db).await?)
}

pub(crate) async fn accept_lead_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    bot_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    let team = offered_team(db, sub, team_id, bot_id).await?;
    let row = bot_team_bot::Entity::find_by_id((team.id.clone(), bot_id.to_string()))
        .one(db)
        .await?;
    if row.is_none_or(|row| row.status != active()) {
        return Err(ApiError::bad_request(format!(
            "Bot '{bot_id}' is not an active bot of team '{}' and can't lead it",
            team.name
        )));
    }
    let org = org_of(db, &team).await?;
    if !is_active_in(db, &org.ids(), sub).await? {
        return Err(forbidden(format!(
            "You are no longer a person of '{}' and can't lead its team '{}' with your bot",
            org.root.name, team.name
        )));
    }
    let team = store_lead(db, &team, Some(bot_id.to_string()), now).await?;
    let mut effects = Effects::default();
    sync_room(db, &team, sub, now, &mut effects).await?;
    sync_team_threads(db, &team, sub, now, &mut effects).await?;
    Ok(effects)
}

pub(crate) async fn decline_lead_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    bot_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    let team = offered_team(db, sub, team_id, bot_id).await?;
    store_lead(db, &team, team.lead_bot_id.clone(), now).await?;
    Ok(())
}

pub(crate) async fn resign_lead_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    bot_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    owned_bot(db, sub, bot_id).await?;
    let team = load_team(db, team_id).await?;
    if team.lead_bot_id.as_deref() != Some(bot_id) {
        return Err(ApiError::not_found(format!(
            "Bot '{bot_id}' is not the lead of team '{}'",
            team.name
        )));
    }
    let team = bot_team::ActiveModel {
        id: Set(team.id.clone()),
        lead_bot_id: Set(None),
        updated_at: Set(now),
        ..Default::default()
    }
    .update(db)
    .await?;
    let mut effects = Effects::default();
    sync_room(db, &team, sub, now, &mut effects).await?;
    sync_team_threads(db, &team, sub, now, &mut effects).await?;
    Ok(effects)
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/bots/{bot_id}/lead/accept",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("bot_id" = String, Path, description = "Your bot, offered as the team's lead")
    ),
    responses(
        (status = 200, description = "Your bot leads the team now: it reads the team room along and people of the organization reach it through their thread with the team.", body = Object),
        (status = 400, description = "The bot is no longer an active bot of the team"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "You are no longer a person of the organization"),
        (status = 404, description = "The bot is not yours, or the team holds no lead offer for it")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bot-teams/{team_id}/bots/{bot_id}/lead/accept",
    skip(state, user)
)]
pub async fn accept_lead(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, bot_id)): Path<(String, String)>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, team_id, bot_id) = (sub.clone(), team_id.clone(), bot_id.clone());
            Box::pin(async move {
                lock_scope(txn, &[team_id.as_str()], &[bot_id.as_str()]).await?;
                accept_lead_in(txn, &sub, &team_id, &bot_id, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(Json(team_view(&state, &sub, &team_id).await?))
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/bots/{bot_id}/lead/decline",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("bot_id" = String, Path, description = "Your bot, offered as the team's lead")
    ),
    responses(
        (status = 204, description = "You declined the offer; the team keeps its current lead."),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "The bot is not yours, or the team holds no lead offer for it")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bot-teams/{team_id}/bots/{bot_id}/lead/decline",
    skip(state, user)
)]
pub async fn decline_lead(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, bot_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    state
        .transaction(|txn| {
            let (sub, team_id, bot_id) = (sub.clone(), team_id.clone(), bot_id.clone());
            Box::pin(async move {
                lock_scope(txn, &[team_id.as_str()], &[bot_id.as_str()]).await?;
                decline_lead_in(txn, &sub, &team_id, &bot_id, now_millis()).await
            })
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/bots/{bot_id}/lead/resign",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("bot_id" = String, Path, description = "Your bot, the team's lead")
    ),
    responses(
        (status = 200, description = "Your bot no longer leads the team; the team has no lead until an admin offers one, and people can't talk to it as a team meanwhile.", body = Object),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only the lead's owner steps it down"),
        (status = 404, description = "The team has no lead, or the bot is not the lead")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bot-teams/{team_id}/bots/{bot_id}/lead/resign",
    skip(state, user)
)]
pub async fn resign_lead(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, bot_id)): Path<(String, String)>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, team_id, bot_id) = (sub.clone(), team_id.clone(), bot_id.clone());
            Box::pin(async move {
                lock_org_scope(txn, &[team_id.as_str()]).await?;
                resign_lead_in(txn, &sub, &team_id, &bot_id, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(Json(team_view(&state, &sub, &team_id).await?))
}

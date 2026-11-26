//! Bots in a team: own bots join at once, another person's bot is invited and its owner decides.

use super::{
    access::{
        active, admin_team, limit, load_team, member_team, not_found, org_of, pending, visible_team,
    },
    effects::{Effects, settle},
    lock_scope,
    model::{AcceptBotRequest, AddBotRequest, BriefingResponse, DirectAgreementRequest},
    room::{bot as bot_principal, leave, sync_room},
    view::team_view,
};
use crate::{
    bot_host::teams::{active_team_of, team_brief_for},
    entity::{bot, bot_team, bot_team_bot, bot_team_person},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::{
        bots::owned_bot,
        inbox::{invitations::validate_limits, now_millis},
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    bot_effective::BotRoomLimits,
    bot_team::{MAX_TEAM_BOTS, Team, team_brief_text},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, sea_query::Expr,
};

const FLUX: &str = "flux";

async fn live_bot<C: ConnectionTrait>(db: &C, bot_id: &str) -> Result<bot::Model, ApiError> {
    bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Bot '{bot_id}' not found")))
}

fn limits_value(
    limits: Option<&BotRoomLimits>,
) -> Result<Option<flow_like_types::Value>, ApiError> {
    limits
        .map(|limits| {
            validate_limits(limits)?;
            serde_json::to_value(limits).map_err(|error| {
                ApiError::bad_request(format!("Room limits are not valid: {error}"))
            })
        })
        .transpose()
}

async fn row_of<C: ConnectionTrait>(
    db: &C,
    team_id: &str,
    bot_id: &str,
) -> Result<Option<bot_team_bot::Model>, sea_orm::DbErr> {
    bot_team_bot::Entity::find_by_id((team_id.to_string(), bot_id.to_string()))
        .one(db)
        .await
}

/// Detaches every bot of the org under `root_id` whose owner is not an active person of one of
/// its teams, so no bot reads an org its owner has left. Rows of deleted bots are left to
/// retirement.
pub(crate) async fn prune_outsiders<C: ConnectionTrait>(
    db: &C,
    root_id: &str,
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    let Some(root) = bot_team::Entity::find_by_id(root_id).one(db).await? else {
        return Ok(());
    };
    let org = org_of(db, &root).await?;
    let ids = org.ids();
    let insiders: std::collections::HashSet<String> = bot_team_person::Entity::find()
        .filter(bot_team_person::Column::TeamId.is_in(ids.clone()))
        .filter(bot_team_person::Column::Status.eq(active()))
        .all(db)
        .await?
        .into_iter()
        .map(|row| row.user_id)
        .collect();
    let rows = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::TeamId.is_in(ids))
        .all(db)
        .await?;
    let owners: std::collections::HashMap<String, String> = bot::Entity::find()
        .filter(bot::Column::Id.is_in(rows.iter().map(|row| row.bot_id.clone())))
        .filter(bot::Column::DeletedAt.is_null())
        .all(db)
        .await?
        .into_iter()
        .map(|row| (row.id, row.owner_id))
        .collect();
    for row in rows {
        let outsider = owners
            .get(&row.bot_id)
            .is_some_and(|owner| !insiders.contains(owner));
        if let (true, Some(team)) = (outsider, org.teams.iter().find(|t| t.id == row.team_id)) {
            detach_bot_in(db, team, &row.bot_id, by, now, effects).await?;
        }
    }
    Ok(())
}

/// Takes the bot out of the team: its row, a lead role it held and its place in the team room.
pub(crate) async fn detach_bot_in<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    bot_id: &str,
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    bot_team_bot::Entity::delete_by_id((team.id.clone(), bot_id.to_string()))
        .exec(db)
        .await?;
    if team.lead_bot_id.as_deref() == Some(bot_id) {
        bot_team::Entity::update_many()
            .col_expr(bot_team::Column::LeadBotId, Expr::value(None::<String>))
            .col_expr(bot_team::Column::UpdatedAt, Expr::value(now))
            .filter(bot_team::Column::Id.eq(&team.id))
            .exec(db)
            .await?;
    }
    leave(db, team, by, &[bot_principal(bot_id)], now, effects).await
}

/// Makes the bot active in `team` and out of any other team, with the limits its owner chose.
async fn activate_in<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    bot_id: &str,
    existing: Option<bot_team_bot::Model>,
    limits: Option<&BotRoomLimits>,
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    let limits = limits_value(limits)?;
    let elsewhere = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::BotId.eq(bot_id))
        .filter(bot_team_bot::Column::Status.eq(active()))
        .filter(bot_team_bot::Column::TeamId.ne(&team.id))
        .all(db)
        .await?;
    for row in elsewhere {
        let previous = load_team(db, &row.team_id).await?;
        detach_bot_in(db, &previous, bot_id, by, now, effects).await?;
    }
    match existing {
        Some(row) => {
            let mut update: bot_team_bot::ActiveModel = row.into();
            update.status = Set(active().to_string());
            update.direct_agreed = Set(team.direct_asks);
            update.limits = Set(limits);
            update.accepted_at = Set(Some(now));
            update.update(db).await?;
        }
        None => {
            bot_team_bot::ActiveModel {
                team_id: Set(team.id.clone()),
                bot_id: Set(bot_id.to_string()),
                status: Set(active().to_string()),
                direct_agreed: Set(team.direct_asks),
                limits: Set(limits),
                invited_by: Set(Some(by.to_string())),
                created_at: Set(now),
                accepted_at: Set(Some(now)),
            }
            .insert(db)
            .await?;
        }
    }
    sync_room(db, team, by, now, effects).await
}

async fn ensure_bot_capacity<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
) -> Result<(), ApiError> {
    let count = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::TeamId.eq(&team.id))
        .count(db)
        .await?;
    if count as usize >= MAX_TEAM_BOTS {
        return Err(limit(format!(
            "A team has at most {MAX_TEAM_BOTS} bots; '{}' has {count}",
            team.name
        )));
    }
    Ok(())
}

/// Adds the caller's own bot at once, or invites another person's bot for its owner to accept.
pub(crate) async fn add_bot_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    request: &AddBotRequest,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    let (team, org) = visible_team(db, sub, team_id).await?;
    let target = live_bot(db, &request.bot_id).await?;
    if target
        .system_key
        .as_deref()
        .is_some_and(|key| key.eq_ignore_ascii_case(FLUX))
    {
        return Err(ApiError::bad_request(
            "Flux is a built-in bot and can't join a team",
        ));
    }
    let existing = row_of(db, team_id, &target.id).await?;
    let mut effects = Effects::default();
    if target.owner_id == sub {
        member_team(db, sub, team_id).await?;
        if existing.as_ref().is_some_and(|row| row.status == active()) {
            return Ok(effects);
        }
        if existing.is_none() {
            ensure_bot_capacity(db, &team).await?;
        }
        activate_in(
            db,
            &team,
            &target.id,
            existing,
            request.limits.as_ref(),
            sub,
            now,
            &mut effects,
        )
        .await?;
        return Ok(effects);
    }
    admin_team(db, sub, team_id).await?;
    if request.limits.is_some() {
        return Err(ApiError::bad_request(
            "Room limits can only be set up front on your own bots; the owner of an invited bot sets them when accepting",
        ));
    }
    if !super::access::is_active_in(db, &org.ids(), &target.owner_id).await? {
        return Err(ApiError::bad_request(format!(
            "The owner of bot '{}' is not a person of an organization team of '{}'",
            target.id, org.root.name
        )));
    }
    if existing.is_some() {
        return Ok(effects);
    }
    ensure_bot_capacity(db, &team).await?;
    bot_team_bot::ActiveModel {
        team_id: Set(team.id.clone()),
        bot_id: Set(target.id.clone()),
        status: Set(pending().to_string()),
        direct_agreed: Set(false),
        limits: Set(None),
        invited_by: Set(Some(sub.to_string())),
        created_at: Set(now),
        accepted_at: Set(None),
    }
    .insert(db)
    .await?;
    effects.notice(
        &target.owner_id,
        sub,
        format!(
            "wants to add your bot {} to the team {}",
            crate::bot_notify::display_name(&target),
            team.name
        ),
    );
    Ok(effects)
}

fn lock_ids<'a>(team_id: &'a str, bot_id: &'a str) -> ([&'a str; 1], [&'a str; 1]) {
    ([team_id], [bot_id])
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/bots",
    tag = "Bot teams",
    params(("team_id" = String, Path, description = "The team")),
    request_body = AddBotRequest,
    responses(
        (status = 200, description = "The team. Your own bot joins at once and leaves the team it was in; as a team admin you invite another member's bot and its owner accepts.", body = Object),
        (status = 400, description = "Flux can't join a team, or the limits are not valid"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: add your own bots as a person of the team, or invite others' bots as its admin"),
        (status = 404, description = "TEAM_NOT_FOUND, or the bot does not exist"),
        (status = 409, description = "TEAM_LIMIT: a team has at most 30 bots")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bot-teams/{team_id}/bots", skip(state, user, body))]
pub async fn add_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(team_id): Path<String>,
    Json(body): Json<AddBotRequest>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, team_id, body) = (sub.clone(), team_id.clone(), body.clone());
            Box::pin(async move {
                let (teams, bots) = lock_ids(&team_id, &body.bot_id);
                lock_scope(txn, &teams, &bots).await?;
                add_bot_in(txn, &sub, &team_id, &body, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(Json(team_view(&state.db, &team_id).await?))
}

pub(crate) async fn remove_bot_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    bot_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    let team = load_team(db, team_id).await?;
    row_of(db, team_id, bot_id)
        .await?
        .ok_or_else(|| not_found(team_id))?;
    let owner = bot::Entity::find_by_id(bot_id).one(db).await?;
    if owner.is_none_or(|row| row.owner_id != sub) {
        admin_team(db, sub, team_id).await?;
    }
    let mut effects = Effects::default();
    detach_bot_in(db, &team, bot_id, sub, now, &mut effects).await?;
    Ok(effects)
}

#[utoipa::path(
    delete,
    path = "/bot-teams/{team_id}/bots/{bot_id}",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("bot_id" = String, Path, description = "The bot")
    ),
    responses(
        (status = 204, description = "The bot left the team and its room, or its pending invitation is withdrawn. If it led the team, the team has no lead now."),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only a team admin or the bot's owner removes it"),
        (status = 404, description = "TEAM_NOT_FOUND, or the bot is not in the team")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bot-teams/{team_id}/bots/{bot_id}", skip(state, user))]
pub async fn remove_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, bot_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, team_id, bot_id) = (sub.clone(), team_id.clone(), bot_id.clone());
            Box::pin(async move {
                let (teams, bots) = lock_ids(&team_id, &bot_id);
                lock_scope(txn, &teams, &bots).await?;
                remove_bot_in(txn, &sub, &team_id, &bot_id, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn accept_bot_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    bot_id: &str,
    request: &AcceptBotRequest,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    owned_bot(db, sub, bot_id).await?;
    let team = load_team(db, team_id).await?;
    let org = org_of(db, &team).await?;
    if !super::access::is_active_in(db, &org.ids(), sub).await? {
        return Err(super::access::forbidden(format!(
            "You are no longer a person of '{}' and can't add your bot to it",
            org.root.name
        )));
    }
    let row = row_of(db, team_id, bot_id)
        .await?
        .ok_or_else(|| not_found(team_id))?;
    let mut effects = Effects::default();
    if row.status == active() {
        return Ok(effects);
    }
    activate_in(
        db,
        &team,
        bot_id,
        Some(row),
        request.limits.as_ref(),
        sub,
        now,
        &mut effects,
    )
    .await?;
    Ok(effects)
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/bots/{bot_id}/accept",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("bot_id" = String, Path, description = "Your bot")
    ),
    request_body = AcceptBotRequest,
    responses(
        (status = 200, description = "Your bot joined the team and its room with the limits you set, and left the team it was in.", body = Object),
        (status = 400, description = "The limits are not valid"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "The bot is not yours, or holds no invitation to this team")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bot-teams/{team_id}/bots/{bot_id}/accept",
    skip(state, user, body)
)]
pub async fn accept_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, bot_id)): Path<(String, String)>,
    body: Option<Json<AcceptBotRequest>>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    let body = body.map(|Json(body)| body).unwrap_or_default();
    let effects = state
        .transaction(|txn| {
            let (sub, team_id, bot_id, body) =
                (sub.clone(), team_id.clone(), bot_id.clone(), body.clone());
            Box::pin(async move {
                let (teams, bots) = lock_ids(&team_id, &bot_id);
                lock_scope(txn, &teams, &bots).await?;
                accept_bot_in(txn, &sub, &team_id, &bot_id, &body, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(Json(team_view(&state.db, &team_id).await?))
}

pub(crate) async fn decline_bot_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    bot_id: &str,
) -> Result<(), ApiError> {
    owned_bot(db, sub, bot_id).await?;
    let deleted = bot_team_bot::Entity::delete_many()
        .filter(bot_team_bot::Column::TeamId.eq(team_id))
        .filter(bot_team_bot::Column::BotId.eq(bot_id))
        .filter(bot_team_bot::Column::Status.eq(pending()))
        .exec(db)
        .await?;
    if deleted.rows_affected == 0 {
        return Err(not_found(team_id));
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/bots/{bot_id}/decline",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("bot_id" = String, Path, description = "Your bot")
    ),
    responses(
        (status = 204, description = "You declined the invitation for your bot."),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "The bot is not yours, or holds no invitation to this team")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bot-teams/{team_id}/bots/{bot_id}/decline",
    skip(state, user)
)]
pub async fn decline_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, bot_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    decline_bot_in(&state.db, &sub, &team_id, &bot_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn set_direct_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    bot_id: &str,
    agreed: bool,
) -> Result<(), ApiError> {
    owned_bot(db, sub, bot_id).await?;
    let updated = bot_team_bot::Entity::update_many()
        .col_expr(bot_team_bot::Column::DirectAgreed, Expr::value(agreed))
        .filter(bot_team_bot::Column::TeamId.eq(team_id))
        .filter(bot_team_bot::Column::BotId.eq(bot_id))
        .filter(bot_team_bot::Column::Status.eq(active()))
        .exec(db)
        .await?;
    if updated.rows_affected == 0 {
        return Err(not_found(team_id));
    }
    Ok(())
}

#[utoipa::path(
    put,
    path = "/bot-teams/{team_id}/bots/{bot_id}/direct",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("bot_id" = String, Path, description = "Your bot")
    ),
    request_body = DirectAgreementRequest,
    responses(
        (status = 200, description = "The team with your agreement to direct asks for this bot.", body = Object),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "The bot is not yours, or is not an active bot of this team")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PUT /bot-teams/{team_id}/bots/{bot_id}/direct",
    skip(state, user, body)
)]
pub async fn set_direct(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, bot_id)): Path<(String, String)>,
    Json(body): Json<DirectAgreementRequest>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    set_direct_in(&state.db, &sub, &team_id, &bot_id, body.agreed).await?;
    Ok(Json(team_view(&state.db, &team_id).await?))
}

/// Whether `user_id` may read what `bot_id` knows of its team: its owner or an active person of
/// its org.
pub(crate) async fn may_read_briefing<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    bot_row: &bot::Model,
) -> Result<bool, ApiError> {
    if bot_row.owner_id == user_id {
        return Ok(true);
    }
    let Some(team) = active_team_of(db, &bot_row.id).await? else {
        return Ok(false);
    };
    let org = org_of(db, &team).await?;
    Ok(super::access::is_active_in(db, &org.ids(), user_id).await?)
}

#[utoipa::path(
    get,
    path = "/bot-teams/bots/{bot_id}/briefing",
    tag = "Bot teams",
    params(("bot_id" = String, Path, description = "The bot")),
    responses(
        (status = 200, description = "What the bot reads about its team and organization before every turn.", body = BriefingResponse),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "The bot is in no team, or you may not read its briefing")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bot-teams/bots/{bot_id}/briefing", skip(state, user))]
pub async fn briefing(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<BriefingResponse>, ApiError> {
    let sub = user.sub()?;
    let missing = || ApiError::not_found(format!("Bot '{bot_id}' is in no team"));
    let bot_row = live_bot(&state.db, &bot_id).await.map_err(|_| missing())?;
    if !may_read_briefing(&state.db, &sub, &bot_row).await? {
        return Err(missing());
    }
    let brief = team_brief_for(&state, &bot_id).await?.ok_or_else(missing)?;
    Ok(Json(BriefingResponse {
        bot_id,
        text: team_brief_text(&brief),
    }))
}

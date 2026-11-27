//! Apps pinned to a team: only someone who owns or administers an app pins it. An admin's pin is
//! active at once, a member's waits for a team admin.

use super::{
    access::{
        active, admin_team, forbidden, is_active_in, limit, load_team, member_team, not_found,
        org_of, pending, person_row,
    },
    effects::{Effects, settle},
    lock_scope,
    model::PinAppRequest,
    org_apps::{AppFacts, is_admin_role, load_facts, sign_media},
    view::team_view,
};
use crate::{
    entity::{bot_team, bot_team_app, bot_team_person, membership, role},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::now_millis,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::bot_team::{MAX_TEAM_APPS, PinnableApp, Team, TeamMemberStatus, TeamRole};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect,
};
use std::collections::HashMap;

const PINNABLE_LIMIT: u64 = 200;

async fn pin_row<C: ConnectionTrait>(
    db: &C,
    team_id: &str,
    app_id: &str,
) -> Result<Option<bot_team_app::Model>, sea_orm::DbErr> {
    bot_team_app::Entity::find_by_id((team_id.to_string(), app_id.to_string()))
        .one(db)
        .await
}

async fn is_team_admin<C: ConnectionTrait>(
    db: &C,
    team_id: &str,
    user_id: &str,
) -> Result<bool, sea_orm::DbErr> {
    Ok(person_row(db, team_id, user_id).await?.is_some_and(|row| {
        row.status == active() && TeamRole::parse(&row.role) == Some(TeamRole::Admin)
    }))
}

async fn active_admins<C: ConnectionTrait>(
    db: &C,
    team_id: &str,
) -> Result<Vec<String>, sea_orm::DbErr> {
    Ok(bot_team_person::Entity::find()
        .filter(bot_team_person::Column::TeamId.eq(team_id))
        .filter(bot_team_person::Column::Status.eq(active()))
        .filter(bot_team_person::Column::Role.eq(TeamRole::Admin.as_str()))
        .all(db)
        .await?
        .into_iter()
        .map(|row| row.user_id)
        .collect())
}

/// Whether `user_id` owns or administers the app, whatever its state.
async fn administers_app<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    app_id: &str,
) -> Result<bool, sea_orm::DbErr> {
    let Some(member) = membership::Entity::find()
        .filter(membership::Column::UserId.eq(user_id))
        .filter(membership::Column::AppId.eq(app_id))
        .one(db)
        .await?
    else {
        return Ok(false);
    };
    Ok(role::Entity::find_by_id(member.role_id)
        .one(db)
        .await?
        .is_some_and(|row| is_admin_role(row.permissions)))
}

async fn administered_app<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    app_id: &str,
) -> Result<AppFacts, ApiError> {
    let facts = load_facts(db, sub, &[app_id.to_string()])
        .await?
        .remove(app_id)
        .ok_or_else(|| ApiError::not_found(format!("App '{app_id}' not found")))?;
    if !facts.can_open() {
        return Err(ApiError::not_found(format!("App '{app_id}' not found")));
    }
    if !facts.reader_admin() {
        return Err(forbidden(
            "Only an owner or admin of the app may pin it to a team",
        ));
    }
    Ok(facts)
}

fn activate(
    row: bot_team_app::Model,
    by: &str,
    now: DateTime<FixedOffset>,
) -> bot_team_app::ActiveModel {
    let mut update: bot_team_app::ActiveModel = row.into();
    update.status = Set(active().to_string());
    update.approved_by = Set(Some(by.to_string()));
    update.approved_at = Set(Some(now));
    update
}

async fn ensure_pin_capacity<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
) -> Result<(), ApiError> {
    let count = bot_team_app::Entity::find()
        .filter(bot_team_app::Column::TeamId.eq(&team.id))
        .count(db)
        .await?;
    if count as usize >= MAX_TEAM_APPS {
        return Err(limit(format!(
            "A team has at most {MAX_TEAM_APPS} apps; '{}' has {count}",
            team.name
        )));
    }
    Ok(())
}

/// Pins the app for the caller: active at once as a team admin, pending for a member, whose
/// team admins are told. A team admin pinning an app that waits approves it.
pub(crate) async fn pin_app_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    app_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    let (team, _) = member_team(db, sub, team_id).await?;
    let facts = administered_app(db, sub, app_id).await?;
    let admin = is_team_admin(db, team_id, sub).await?;
    let mut effects = Effects::default();
    if let Some(row) = pin_row(db, team_id, app_id).await? {
        if admin && row.status == pending() {
            let pinner = row.pinned_by.clone();
            activate(row, sub, now).update(db).await?;
            effects.wake(vec![pinner]);
        }
        return Ok(effects);
    }
    ensure_pin_capacity(db, &team).await?;
    bot_team_app::ActiveModel {
        team_id: Set(team.id.clone()),
        app_id: Set(app_id.to_string()),
        status: Set(if admin { active() } else { pending() }.to_string()),
        pinned_by: Set(sub.to_string()),
        approved_by: Set(None),
        created_at: Set(now),
        approved_at: Set(None),
    }
    .insert(db)
    .await?;
    if !admin {
        let admins: Vec<String> = active_admins(db, team_id)
            .await?
            .into_iter()
            .filter(|admin| admin != sub)
            .collect();
        for admin in &admins {
            effects.notice(
                admin,
                sub,
                format!(
                    "wants to pin the app {} to the team {}",
                    facts.name(),
                    team.name
                ),
            );
        }
        effects.wake(admins);
    }
    Ok(effects)
}

/// Activates a member's pending pin.
pub(crate) async fn approve_pin_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    app_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    admin_team(db, sub, team_id).await?;
    let row = pin_row(db, team_id, app_id)
        .await?
        .ok_or_else(|| not_found(team_id))?;
    let mut effects = Effects::default();
    if row.status == pending() {
        effects.wake(vec![row.pinned_by.clone()]);
        activate(row, sub, now).update(db).await?;
    }
    Ok(effects)
}

/// Deletes a member's pending pin.
pub(crate) async fn decline_pin_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    app_id: &str,
) -> Result<Effects, ApiError> {
    admin_team(db, sub, team_id).await?;
    let row = pin_row(db, team_id, app_id)
        .await?
        .filter(|row| row.status == pending())
        .ok_or_else(|| not_found(team_id))?;
    bot_team_app::Entity::delete_by_id((row.team_id, row.app_id))
        .exec(db)
        .await?;
    let mut effects = Effects::default();
    effects.wake(vec![row.pinned_by]);
    Ok(effects)
}

/// Removes a pin: its pinner, an active team admin or anyone who administers the app may.
pub(crate) async fn unpin_app_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    app_id: &str,
) -> Result<(), ApiError> {
    let team = load_team(db, team_id).await?;
    let row = pin_row(db, team_id, app_id)
        .await?
        .ok_or_else(|| not_found(team_id))?;
    let allowed = row.pinned_by == sub
        || is_team_admin(db, team_id, sub).await?
        || administers_app(db, sub, app_id).await?;
    if !allowed {
        let org = org_of(db, &team).await?;
        return Err(if is_active_in(db, &org.ids(), sub).await? {
            forbidden("Only the pinner, a team admin or an admin of the app may unpin it")
        } else {
            not_found(team_id)
        });
    }
    bot_team_app::Entity::delete_by_id((row.team_id, row.app_id))
        .exec(db)
        .await?;
    Ok(())
}

fn status_of(row: &bot_team_app::Model) -> Option<TeamMemberStatus> {
    TeamMemberStatus::parse(&row.status)
}

/// The apps the caller is a member of with this team's pin on each, owners and admins first.
pub(crate) async fn list_pinnable_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
) -> Result<Vec<PinnableApp>, ApiError> {
    let (_, org) = member_team(db, sub, team_id).await?;
    let ids: Vec<String> = membership::Entity::find()
        .select_only()
        .column(membership::Column::AppId)
        .filter(membership::Column::UserId.eq(sub))
        .order_by_desc(membership::Column::CreatedAt)
        .order_by_asc(membership::Column::AppId)
        .limit(PINNABLE_LIMIT)
        .into_tuple::<String>()
        .all(db)
        .await?;
    let facts = load_facts(db, sub, &ids).await?;
    let pins = bot_team_app::Entity::find()
        .filter(bot_team_app::Column::AppId.is_in(ids))
        .filter(bot_team_app::Column::TeamId.is_in(org.ids()))
        .all(db)
        .await?;
    let mut here: HashMap<&str, TeamMemberStatus> = HashMap::new();
    let mut elsewhere: HashMap<&str, Vec<String>> = HashMap::new();
    for pin in &pins {
        if pin.team_id == team_id {
            if let Some(status) = status_of(pin) {
                here.insert(pin.app_id.as_str(), status);
            }
        } else if pin.status == active() {
            elsewhere
                .entry(pin.app_id.as_str())
                .or_default()
                .push(pin.team_id.clone());
        }
    }
    let mut apps: Vec<PinnableApp> = facts
        .values()
        .map(|facts| PinnableApp {
            id: facts.app.id.clone(),
            name: facts.name(),
            description: facts.description(),
            icon: facts.meta.as_ref().and_then(|meta| meta.icon.clone()),
            app_type: facts.app_type(),
            reader_admin: facts.reader_admin(),
            owner_id: facts.owner_id.clone(),
            pinned: here.get(facts.app.id.as_str()).copied(),
            pinned_elsewhere: {
                let mut teams = elsewhere.remove(facts.app.id.as_str()).unwrap_or_default();
                teams.sort();
                teams
            },
        })
        .collect();
    apps.sort_by(|a, b| {
        b.reader_admin
            .cmp(&a.reader_admin)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(apps)
}

#[utoipa::path(
    get,
    path = "/bot-teams/{team_id}/apps/pinnable",
    tag = "Bot teams",
    params(("team_id" = String, Path, description = "The team")),
    responses(
        (status = 200, description = "Your apps, up to 200, with their pin in this team; only the ones you own or administer can be pinned.", body = Vec<Object>),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: you are not an active person of the team"),
        (status = 404, description = "TEAM_NOT_FOUND")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bot-teams/{team_id}/apps/pinnable", skip(state, user))]
pub async fn list_pinnable(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(team_id): Path<String>,
) -> Result<Json<Vec<PinnableApp>>, ApiError> {
    let sub = user.sub()?;
    let mut apps = list_pinnable_in(&state.db, &sub, &team_id).await?;
    let items = apps
        .iter_mut()
        .map(|app| (app.id.as_str(), &mut app.icon, None))
        .collect();
    sign_media(&state, items).await;
    Ok(Json(apps))
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/apps",
    tag = "Bot teams",
    params(("team_id" = String, Path, description = "The team")),
    request_body = PinAppRequest,
    responses(
        (status = 200, description = "The team with the app pinned: active at once as a team admin, waiting for a team admin's approval as a member.", body = Object),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: you are not an active person of the team, or you do not own or administer the app"),
        (status = 404, description = "TEAM_NOT_FOUND, or the app does not exist, is archived or is a bot's workspace"),
        (status = 409, description = "TEAM_LIMIT: a team has at most 24 apps")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bot-teams/{team_id}/apps", skip(state, user, body))]
pub async fn pin_app(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(team_id): Path<String>,
    Json(body): Json<PinAppRequest>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, team_id, body) = (sub.clone(), team_id.clone(), body.clone());
            Box::pin(async move {
                lock_scope(txn, &[team_id.as_str()], &[]).await?;
                pin_app_in(txn, &sub, &team_id, &body.app_id, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(Json(team_view(&state, &sub, &team_id).await?))
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/apps/{app_id}/approve",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("app_id" = String, Path, description = "The pinned app")
    ),
    responses(
        (status = 200, description = "The team with the member's pin approved.", body = Object),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only a team admin approves a pin"),
        (status = 404, description = "TEAM_NOT_FOUND, or the app is not pinned to the team")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bot-teams/{team_id}/apps/{app_id}/approve",
    skip(state, user)
)]
pub async fn approve_pin(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, app_id)): Path<(String, String)>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, team_id, app_id) = (sub.clone(), team_id.clone(), app_id.clone());
            Box::pin(async move {
                lock_scope(txn, &[team_id.as_str()], &[]).await?;
                approve_pin_in(txn, &sub, &team_id, &app_id, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(Json(team_view(&state, &sub, &team_id).await?))
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/apps/{app_id}/decline",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("app_id" = String, Path, description = "The pinned app")
    ),
    responses(
        (status = 204, description = "The member's pending pin is gone."),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only a team admin declines a pin"),
        (status = 404, description = "TEAM_NOT_FOUND, or the app has no pending pin in the team")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bot-teams/{team_id}/apps/{app_id}/decline",
    skip(state, user)
)]
pub async fn decline_pin(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, app_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, team_id, app_id) = (sub.clone(), team_id.clone(), app_id.clone());
            Box::pin(async move {
                lock_scope(txn, &[team_id.as_str()], &[]).await?;
                decline_pin_in(txn, &sub, &team_id, &app_id).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/bot-teams/{team_id}/apps/{app_id}",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("app_id" = String, Path, description = "The pinned app")
    ),
    responses(
        (status = 204, description = "The app is no longer pinned to the team."),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only the pinner, a team admin or an admin of the app unpins it"),
        (status = 404, description = "TEAM_NOT_FOUND, or the app is not pinned to the team")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bot-teams/{team_id}/apps/{app_id}", skip(state, user))]
pub async fn unpin_app(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, app_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    state
        .transaction(|txn| {
            let (sub, team_id, app_id) = (sub.clone(), team_id.clone(), app_id.clone());
            Box::pin(async move {
                lock_scope(txn, &[team_id.as_str()], &[]).await?;
                unpin_app_in(txn, &sub, &team_id, &app_id).await
            })
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

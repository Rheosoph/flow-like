//! Creating, changing and deleting teams.

use super::{
    access::{Org, TEAM_CYCLE, active, admin_team, limit, load_team, org_of},
    bots::prune_outsiders,
    chats::sync_chats_of,
    classification::{drop_classifications, prune_classifications},
    effects::{Effects, settle},
    lead::{audit_lead, lead_events},
    lock_org_scope, lock_scope,
    model::{CreateTeamRequest, PatchTeamRequest},
    org_apps::sign_org_apps,
    room::sync_room,
    threads::{release_team_threads, sync_team_threads},
    view::{org_view, sign_team_images, team_view},
};
use crate::{
    entity::{bot, bot_team, bot_team_app, bot_team_bot, bot_team_person},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::{now_millis, team_room},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    bot_team::{
        MAX_ORG_TEAMS, MAX_TEAM_DEPTH, MAX_TEAM_NAME_CHARS, MAX_TEAM_PURPOSE_CHARS, OrgView, Team,
        TeamRole,
    },
    create_id,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter,
    sea_query::Expr,
};

fn clean_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    let chars = name.chars().count();
    if chars == 0 || chars > MAX_TEAM_NAME_CHARS {
        return Err(ApiError::bad_request(format!(
            "A team name has 1 to {MAX_TEAM_NAME_CHARS} characters; this one has {chars}"
        )));
    }
    Ok(name.to_string())
}

fn clean_purpose(purpose: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(purpose) = purpose.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    let chars = purpose.chars().count();
    if chars > MAX_TEAM_PURPOSE_CHARS {
        return Err(ApiError::bad_request(format!(
            "A team purpose has at most {MAX_TEAM_PURPOSE_CHARS} characters; this one has {chars}"
        )));
    }
    Ok(Some(purpose.to_string()))
}

fn too_deep(team_name: &str) -> ApiError {
    ApiError::bad_request(format!(
        "Teams nest at most {MAX_TEAM_DEPTH} levels below their top-level team; '{team_name}' would sit deeper"
    ))
}

/// Creates the team with its room; the creator becomes its active admin. Returns the team id.
pub(crate) async fn create_team_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    request: &CreateTeamRequest,
    now: DateTime<FixedOffset>,
) -> Result<(String, Effects), ApiError> {
    let name = clean_name(&request.name)?;
    let purpose = clean_purpose(request.purpose.as_deref())?;
    if let Some(parent_id) = request.parent_id.as_deref() {
        let (parent, org) = admin_team(db, sub, parent_id).await?;
        if org.depth_of(&parent.id).unwrap_or(0) + 1 > MAX_TEAM_DEPTH {
            return Err(too_deep(&name));
        }
        if org.teams.len() >= MAX_ORG_TEAMS {
            return Err(limit(format!(
                "An organization has at most {MAX_ORG_TEAMS} teams; '{}' has {}",
                org.root.name,
                org.teams.len()
            )));
        }
    }
    let (team_id, room_id) = (create_id(), create_id());
    let mut effects = Effects::default();
    effects.wake(team_room::create_room(db, &room_id, sub, &name, now).await?);
    bot_team::ActiveModel {
        id: Set(team_id.clone()),
        name: Set(name),
        purpose: Set(purpose),
        parent_id: Set(request.parent_id.clone()),
        lead_bot_id: Set(None),
        lead_offer_bot_id: Set(None),
        lead_offer_by: Set(None),
        lead_offer_at: Set(None),
        direct_asks: Set(false),
        conversation_id: Set(Some(room_id)),
        created_by: Set(sub.to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await?;
    bot_team_person::ActiveModel {
        team_id: Set(team_id.clone()),
        user_id: Set(sub.to_string()),
        role: Set(TeamRole::Admin.as_str().to_string()),
        status: Set(active().to_string()),
        invited_by: Set(None),
        created_at: Set(now),
        accepted_at: Set(Some(now)),
    }
    .insert(db)
    .await?;
    Ok((team_id, effects))
}

#[utoipa::path(
    post,
    path = "/bot-teams",
    tag = "Bot teams",
    request_body = CreateTeamRequest,
    responses(
        (status = 200, description = "The new team with you as its admin and a team room for it. Created inside another team, you must be that team's admin.", body = Object),
        (status = 400, description = "The name or purpose is too long, or the team would nest too deep"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only an admin of the parent team creates a team in it"),
        (status = 404, description = "TEAM_NOT_FOUND: the parent team does not exist or you can't see it"),
        (status = 409, description = "TEAM_LIMIT: an organization has at most 100 teams")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bot-teams", skip(state, user, body))]
pub async fn create_team(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<CreateTeamRequest>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    let (team_id, effects) = state
        .transaction(|txn| {
            let (sub, body) = (sub.clone(), body.clone());
            Box::pin(async move {
                let scope: Vec<&str> = body.parent_id.as_deref().into_iter().collect();
                lock_scope(txn, &scope, &[]).await?;
                create_team_in(txn, &sub, &body, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(Json(team_view(&state, &sub, &team_id).await?))
}

#[utoipa::path(
    get,
    path = "/bot-teams",
    tag = "Bot teams",
    responses(
        (status = 200, description = "Every organization you are an active person in: its teams, the bots in them with their job descriptions, and the invitations waiting for your answer.", body = Object),
        (status = 401, description = "Not signed in")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bot-teams", skip(state, user))]
pub async fn list_org(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<OrgView>, ApiError> {
    let sub = user.sub()?;
    let mut view = org_view(&state.db, &sub).await?;
    sign_team_images(&state, &mut view.teams).await?;
    sign_org_apps(&state, &mut view.apps).await;
    Ok(Json(view))
}

async fn set_parent<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team: &bot_team::Model,
    org: &Org,
    parent_id: Option<&str>,
) -> Result<(), ApiError> {
    let Some(parent_id) = parent_id else {
        return Ok(());
    };
    let subtree = org.subtree(&team.id);
    if subtree.iter().any(|below| below.id == parent_id) {
        return Err(ApiError::coded(
            StatusCode::BAD_REQUEST,
            TEAM_CYCLE,
            format!(
                "Team '{}' can't move into '{parent_id}': that team sits inside it",
                team.name
            ),
        ));
    }
    let (parent, parent_org) = admin_team(db, sub, parent_id).await?;
    let own_depth = org.depth_of(&team.id).unwrap_or(0);
    let height = subtree
        .iter()
        .filter_map(|below| org.depth_of(&below.id))
        .max()
        .unwrap_or(own_depth)
        - own_depth;
    if parent_org.depth_of(&parent.id).unwrap_or(0) + 1 + height > MAX_TEAM_DEPTH {
        return Err(too_deep(&team.name));
    }
    if parent_org.root.id != org.root.id && parent_org.teams.len() + subtree.len() > MAX_ORG_TEAMS {
        return Err(limit(format!(
            "An organization has at most {MAX_ORG_TEAMS} teams; '{}' would have {}",
            parent_org.root.name,
            parent_org.teams.len() + subtree.len()
        )));
    }
    Ok(())
}

async fn set_direct_asks<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team: &bot_team::Model,
    on: bool,
) -> Result<(), ApiError> {
    let rows =
        bot_team_bot::Entity::update_many().filter(bot_team_bot::Column::TeamId.eq(&team.id));
    if !on {
        rows.col_expr(bot_team_bot::Column::DirectAgreed, Expr::value(false))
            .exec(db)
            .await?;
        return Ok(());
    }
    let own: Vec<String> = bot::Entity::find()
        .filter(bot::Column::OwnerId.eq(sub))
        .all(db)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect();
    rows.col_expr(bot_team_bot::Column::DirectAgreed, Expr::value(true))
        .filter(bot_team_bot::Column::Status.eq(active()))
        .filter(bot_team_bot::Column::BotId.is_in(own))
        .exec(db)
        .await?;
    Ok(())
}

async fn ensure_lead<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    bot_id: &str,
) -> Result<String, ApiError> {
    let row = bot_team_bot::Entity::find_by_id((team.id.clone(), bot_id.to_string()))
        .one(db)
        .await?;
    let not_active = || {
        ApiError::bad_request(format!(
            "'{bot_id}' is not an active bot of team '{}' and can't lead it",
            team.name
        ))
    };
    if row.is_none_or(|row| row.status != active()) {
        return Err(not_active());
    }
    bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .map(|bot| bot.owner_id)
        .ok_or_else(not_active)
}

pub(crate) fn clear_offer(update: &mut bot_team::ActiveModel) {
    update.lead_offer_bot_id = Set(None);
    update.lead_offer_by = Set(None);
    update.lead_offer_at = Set(None);
}

/// Applies the changes of `request` to the team; the caller must be its admin.
pub(crate) async fn patch_team_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    request: &PatchTeamRequest,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    let (team, org) = admin_team(db, sub, team_id).await?;
    let mut effects = Effects::default();
    let mut update = bot_team::ActiveModel {
        id: Set(team.id.clone()),
        updated_at: Set(now),
        ..Default::default()
    };
    let mut lead_changed = false;
    let mut renamed = None;
    if let Some(name) = request.name.as_deref() {
        let name = clean_name(name)?;
        if name != team.name {
            renamed = Some(name.clone());
            update.name = Set(name);
        }
    }
    if let Some(purpose) = &request.purpose {
        update.purpose = Set(clean_purpose(purpose.as_deref())?);
    }
    if let Some(parent_id) = &request.parent_id {
        set_parent(db, sub, &team, &org, parent_id.as_deref()).await?;
        update.parent_id = Set(parent_id.clone());
    }
    if let Some(lead) = &request.lead_bot_id {
        match lead.as_deref() {
            None => {
                lead_changed = team.lead_bot_id.is_some();
                update.lead_bot_id = Set(None);
                clear_offer(&mut update);
            }
            Some(bot_id) if team.lead_bot_id.as_deref() == Some(bot_id) => {
                ensure_lead(db, &team, bot_id).await?;
            }
            Some(bot_id) => {
                let owner = ensure_lead(db, &team, bot_id).await?;
                if owner == sub {
                    lead_changed = true;
                    update.lead_bot_id = Set(Some(bot_id.to_string()));
                    clear_offer(&mut update);
                } else {
                    update.lead_offer_bot_id = Set(Some(bot_id.to_string()));
                    update.lead_offer_by = Set(Some(sub.to_string()));
                    update.lead_offer_at = Set(Some(now));
                }
            }
        }
    }
    if let Some(on) = request.direct_asks {
        set_direct_asks(db, sub, &team, on).await?;
        update.direct_asks = Set(on);
    }
    let team = update.update(db).await?;
    if request.parent_id.is_some() {
        let new_root = org_of(db, &team).await?.root.id;
        if new_root != org.root.id {
            if org.root.id != team.id {
                prune_outsiders(db, &org.root.id, sub, now, &mut effects).await?;
                prune_classifications(db, &org.root.id).await?;
            }
            prune_outsiders(db, &new_root, sub, now, &mut effects).await?;
            prune_classifications(db, &new_root).await?;
        }
        if org.root.id == team.id && team.parent_id.is_some() {
            drop_classifications(db, &team.id).await?;
        }
    }
    if let (Some(name), Some(room)) = (renamed, team.conversation_id.as_deref()) {
        effects.wake(team_room::retitle_room(db, room, sub, &name, now).await?);
    }
    if lead_changed {
        sync_room(db, &team, sub, now, &mut effects).await?;
        sync_team_threads(db, &team, sub, now, &mut effects).await?;
    }
    Ok(effects)
}

#[utoipa::path(
    patch,
    path = "/bot-teams/{team_id}",
    tag = "Bot teams",
    params(("team_id" = String, Path, description = "The team")),
    request_body = PatchTeamRequest,
    responses(
        (status = 200, description = "The team with your changes. Renaming it renames its room; a new lead reads the room along and the other bots answer when named. Turning direct asks on agrees for your own bots in the team; turning them off withdraws every agreement.", body = Object),
        (status = 400, description = "TEAM_CYCLE: a team can't move into itself or a team below it; or a name, purpose, lead or depth is not valid"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only a team admin changes it, and moving it needs admin of the new parent too"),
        (status = 404, description = "TEAM_NOT_FOUND: the team does not exist or you can't see it")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PATCH /bot-teams/{team_id}", skip(state, user, body))]
pub async fn patch_team(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(team_id): Path<String>,
    Json(body): Json<PatchTeamRequest>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    let (effects, lead_changes) = state
        .transaction(|txn| {
            let (sub, team_id, body) = (sub.clone(), team_id.clone(), body.clone());
            Box::pin(async move {
                let mut scope = vec![team_id.as_str()];
                scope.extend(body.parent_id.iter().flatten().map(String::as_str));
                lock_org_scope(txn, &scope).await?;
                let before = load_team(txn, &team_id).await?;
                let effects = patch_team_in(txn, &sub, &team_id, &body, now_millis()).await?;
                let after = load_team(txn, &team_id).await?;
                Ok::<_, ApiError>((effects, lead_events(&before, &after)))
            })
        })
        .await?;
    settle(&state, effects).await;
    for (action, bot_id) in &lead_changes {
        audit_lead(&state, &user, action, &team_id, bot_id).await;
    }
    Ok(Json(team_view(&state, &sub, &team_id).await?))
}

/// Deletes the team's rows; its child teams move up to its parent.
pub(crate) async fn remove_team_rows<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    release_team_threads(db, team, by, now, effects).await?;
    let team_bots: Vec<String> = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::TeamId.eq(&team.id))
        .all(db)
        .await?
        .into_iter()
        .map(|row| row.bot_id)
        .collect();
    bot_team::Entity::update_many()
        .col_expr(
            bot_team::Column::ParentId,
            Expr::value(team.parent_id.clone()),
        )
        .filter(bot_team::Column::ParentId.eq(&team.id))
        .exec(db)
        .await?;
    bot_team_bot::Entity::delete_many()
        .filter(bot_team_bot::Column::TeamId.eq(&team.id))
        .exec(db)
        .await?;
    bot_team_app::Entity::delete_many()
        .filter(bot_team_app::Column::TeamId.eq(&team.id))
        .exec(db)
        .await?;
    bot_team_person::Entity::delete_many()
        .filter(bot_team_person::Column::TeamId.eq(&team.id))
        .exec(db)
        .await?;
    bot_team::Entity::delete_by_id(team.id.clone())
        .exec(db)
        .await?;
    sync_chats_of(db, &team_bots, &[], by, now, effects).await
}

/// Deletes the team, then detaches the bots whose owners are no longer in the org they sit in:
/// the org above it, or each former child of a top-level team, which is an org of its own now.
pub(crate) async fn remove_team_and_prune<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    let org = org_of(db, team).await?;
    let children: Vec<String> = org
        .teams
        .iter()
        .filter(|child| child.parent_id.as_deref() == Some(team.id.as_str()))
        .map(|child| child.id.clone())
        .collect();
    remove_team_rows(db, team, by, now, effects).await?;
    drop_classifications(db, &team.id).await?;
    let roots = match team.parent_id {
        Some(_) => vec![org.root.id.clone()],
        None => children,
    };
    for root in roots {
        prune_outsiders(db, &root, by, now, effects).await?;
        prune_classifications(db, &root).await?;
    }
    Ok(())
}

pub(crate) async fn delete_team_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    let (team, _) = admin_team(db, sub, team_id).await?;
    let mut effects = Effects::default();
    remove_team_and_prune(db, &team, sub, now, &mut effects).await?;
    Ok(effects)
}

#[utoipa::path(
    delete,
    path = "/bot-teams/{team_id}",
    tag = "Bot teams",
    params(("team_id" = String, Path, description = "The team")),
    responses(
        (status = 204, description = "The team is gone with its people and bot memberships. Teams inside it move up to its parent; its room stays as a plain group chat."),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only a team admin deletes it"),
        (status = 404, description = "TEAM_NOT_FOUND: the team does not exist or you can't see it")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bot-teams/{team_id}", skip(state, user))]
pub async fn delete_team(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(team_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, team_id) = (sub.clone(), team_id.clone());
            Box::pin(async move {
                lock_org_scope(txn, &[team_id.as_str()]).await?;
                delete_team_in(txn, &sub, &team_id, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(StatusCode::NO_CONTENT)
}

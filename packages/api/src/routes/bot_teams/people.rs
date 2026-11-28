//! People in a team: invitations, joining, roles and leaving.

use super::{
    access::{
        TEAM_LAST_ADMIN, active, admin_team, limit, load_team, not_found, org_of, pending,
        person_row,
    },
    bots::prune_outsiders,
    classification::prune_classifications,
    effects::{Effects, settle},
    lock_org_scope, lock_scope,
    model::{AddPeopleRequest, SetRoleRequest},
    room::{leave, person, sync_room},
    teams::remove_team_and_prune,
    view::team_view,
};
use crate::{
    entity::{bot_team_person, user},
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
use flow_like_types::bot_team::{MAX_TEAM_PEOPLE, Team, TeamRole};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, QuerySelect, sea_query::Expr,
};
use std::collections::HashSet;

async fn ensure_users<C: ConnectionTrait>(db: &C, ids: &[String]) -> Result<(), ApiError> {
    let found: HashSet<String> = user::Entity::find()
        .select_only()
        .column(user::Column::Id)
        .filter(user::Column::Id.is_in(ids.iter().cloned()))
        .into_tuple::<String>()
        .all(db)
        .await?
        .into_iter()
        .collect();
    match ids.iter().find(|id| !found.contains(*id)) {
        Some(id) => Err(ApiError::not_found(format!("User '{id}' not found"))),
        None => Ok(()),
    }
}

/// Invites `request.user_ids` as pending people; people already in the team stay as they are.
pub(crate) async fn add_people_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    request: &AddPeopleRequest,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    let (team, _) = admin_team(db, sub, team_id).await?;
    let mut seen = HashSet::new();
    let ids: Vec<String> = request
        .user_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty() && seen.insert(id.clone()))
        .collect();
    ensure_users(db, &ids).await?;
    let rows = bot_team_person::Entity::find()
        .filter(bot_team_person::Column::TeamId.eq(team_id))
        .all(db)
        .await?;
    let fresh: Vec<&String> = ids
        .iter()
        .filter(|id| !rows.iter().any(|row| &row.user_id == *id))
        .collect();
    if rows.len() + fresh.len() > MAX_TEAM_PEOPLE {
        return Err(limit(format!(
            "A team has at most {MAX_TEAM_PEOPLE} people; '{}' has {} and you asked to add {}",
            team.name,
            rows.len(),
            fresh.len()
        )));
    }
    let role = request.role.unwrap_or(TeamRole::Member);
    let mut effects = Effects::default();
    for id in fresh {
        bot_team_person::ActiveModel {
            team_id: Set(team_id.to_string()),
            user_id: Set(id.clone()),
            role: Set(role.as_str().to_string()),
            status: Set(pending().to_string()),
            invited_by: Set(Some(sub.to_string())),
            created_at: Set(now),
            accepted_at: Set(None),
        }
        .insert(db)
        .await?;
        effects.notice(
            id,
            sub,
            format!("invited you to join the team {}", team.name),
        );
    }
    Ok(effects)
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/people",
    tag = "Bot teams",
    params(("team_id" = String, Path, description = "The team")),
    request_body = AddPeopleRequest,
    responses(
        (status = 200, description = "The team with the invited people pending until they accept. People already in the team are unchanged.", body = Object),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only a team admin invites people"),
        (status = 404, description = "TEAM_NOT_FOUND, or the first user that does not exist"),
        (status = 409, description = "TEAM_LIMIT: a team has at most 20 people")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bot-teams/{team_id}/people", skip(state, user, body))]
pub async fn add_people(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(team_id): Path<String>,
    Json(body): Json<AddPeopleRequest>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, team_id, body) = (sub.clone(), team_id.clone(), body.clone());
            Box::pin(async move {
                lock_scope(txn, &[team_id.as_str()], &[]).await?;
                add_people_in(txn, &sub, &team_id, &body, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(Json(team_view(&state, &sub, &team_id).await?))
}

pub(crate) async fn join_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    let row = person_row(db, team_id, sub)
        .await?
        .ok_or_else(|| not_found(team_id))?;
    let mut effects = Effects::default();
    if row.status == active() {
        return Ok(effects);
    }
    bot_team_person::Entity::update_many()
        .col_expr(bot_team_person::Column::Status, Expr::value(active()))
        .col_expr(bot_team_person::Column::AcceptedAt, Expr::value(now))
        .filter(bot_team_person::Column::TeamId.eq(team_id))
        .filter(bot_team_person::Column::UserId.eq(sub))
        .exec(db)
        .await?;
    let team = load_team(db, team_id).await?;
    sync_room(db, &team, sub, now, &mut effects).await?;
    Ok(effects)
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/join",
    tag = "Bot teams",
    params(("team_id" = String, Path, description = "The team")),
    responses(
        (status = 200, description = "You accepted the invitation: you are an active person of the team and in its room.", body = Object),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "TEAM_NOT_FOUND: you hold no invitation to this team")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bot-teams/{team_id}/join", skip(state, user))]
pub async fn join_team(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(team_id): Path<String>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, team_id) = (sub.clone(), team_id.clone());
            Box::pin(async move {
                lock_scope(txn, &[team_id.as_str()], &[]).await?;
                join_in(txn, &sub, &team_id, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(Json(team_view(&state, &sub, &team_id).await?))
}

pub(crate) async fn decline_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
) -> Result<(), ApiError> {
    let deleted = bot_team_person::Entity::delete_many()
        .filter(bot_team_person::Column::TeamId.eq(team_id))
        .filter(bot_team_person::Column::UserId.eq(sub))
        .filter(bot_team_person::Column::Status.eq(pending()))
        .exec(db)
        .await?;
    if deleted.rows_affected == 0 {
        return Err(not_found(team_id));
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/decline",
    tag = "Bot teams",
    params(("team_id" = String, Path, description = "The team")),
    responses(
        (status = 204, description = "You declined the invitation."),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "TEAM_NOT_FOUND: you hold no invitation to this team")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bot-teams/{team_id}/decline", skip(state, user))]
pub async fn decline_team(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(team_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    decline_in(&state.db, &sub, &team_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn other_active_admins<C: ConnectionTrait>(
    db: &C,
    team_id: &str,
    user_id: &str,
) -> Result<(u64, u64), ApiError> {
    let others = bot_team_person::Entity::find()
        .filter(bot_team_person::Column::TeamId.eq(team_id))
        .filter(bot_team_person::Column::UserId.ne(user_id))
        .filter(bot_team_person::Column::Status.eq(active()));
    let admins = others
        .clone()
        .filter(bot_team_person::Column::Role.eq(TeamRole::Admin.as_str()))
        .count(db)
        .await?;
    Ok((others.count(db).await?, admins))
}

fn last_admin(team_name: &str) -> ApiError {
    ApiError::coded(
        StatusCode::CONFLICT,
        TEAM_LAST_ADMIN,
        format!("'{team_name}' needs at least one active admin while it has other people"),
    )
}

pub(crate) async fn set_role_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    user_id: &str,
    role: TeamRole,
) -> Result<(), ApiError> {
    let (team, _) = admin_team(db, sub, team_id).await?;
    let row = person_row(db, team_id, user_id).await?.ok_or_else(|| {
        ApiError::not_found(format!("'{user_id}' is not a person of team '{team_id}'"))
    })?;
    if role == TeamRole::Member && row.status == active() && row.role == TeamRole::Admin.as_str() {
        let (_, admins) = other_active_admins(db, team_id, user_id).await?;
        if admins == 0 {
            return Err(last_admin(&team.name));
        }
    }
    bot_team_person::Entity::update_many()
        .col_expr(bot_team_person::Column::Role, Expr::value(role.as_str()))
        .filter(bot_team_person::Column::TeamId.eq(team_id))
        .filter(bot_team_person::Column::UserId.eq(user_id))
        .exec(db)
        .await?;
    Ok(())
}

#[utoipa::path(
    put,
    path = "/bot-teams/{team_id}/people/{user_id}/role",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("user_id" = String, Path, description = "The person")
    ),
    request_body = SetRoleRequest,
    responses(
        (status = 200, description = "The team with the person's new role.", body = Object),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only a team admin changes roles"),
        (status = 404, description = "TEAM_NOT_FOUND, or the person is not in the team"),
        (status = 409, description = "TEAM_LAST_ADMIN: a team keeps at least one active admin")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PUT /bot-teams/{team_id}/people/{user_id}/role",
    skip(state, user, body)
)]
pub async fn set_role(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, user_id)): Path<(String, String)>,
    Json(body): Json<SetRoleRequest>,
) -> Result<Json<Team>, ApiError> {
    let sub = user.sub()?;
    state
        .transaction(|txn| {
            let (sub, team_id, user_id) = (sub.clone(), team_id.clone(), user_id.clone());
            Box::pin(async move {
                lock_scope(txn, &[team_id.as_str()], &[]).await?;
                set_role_in(txn, &sub, &team_id, &user_id, body.role).await
            })
        })
        .await?;
    Ok(Json(team_view(&state, &sub, &team_id).await?))
}

/// Removes a person from the team: an admin removes anyone, everyone may leave. Their bots in the
/// team go with them unless they stay active elsewhere in the org. A team whose last active
/// person leaves is deleted, since nobody could manage it any more.
pub(crate) async fn remove_person_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    user_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Effects, ApiError> {
    let team = if sub == user_id {
        load_team(db, team_id).await?
    } else {
        admin_team(db, sub, team_id).await?.0
    };
    let row = person_row(db, team_id, user_id).await?.ok_or_else(|| {
        ApiError::not_found(format!("'{user_id}' is not a person of team '{team_id}'"))
    })?;
    let was_active = row.status == active();
    if was_active && row.role == TeamRole::Admin.as_str() {
        let (others, admins) = other_active_admins(db, team_id, user_id).await?;
        if others > 0 && admins == 0 {
            return Err(last_admin(&team.name));
        }
    }
    bot_team_person::Entity::delete_by_id((team_id.to_string(), user_id.to_string()))
        .exec(db)
        .await?;
    let mut effects = Effects::default();
    if !was_active {
        return Ok(effects);
    }
    leave(db, &team, sub, &[person(user_id)], now, &mut effects).await?;
    let remaining = bot_team_person::Entity::find()
        .filter(bot_team_person::Column::TeamId.eq(team_id))
        .filter(bot_team_person::Column::Status.eq(active()))
        .count(db)
        .await?;
    if remaining == 0 {
        remove_team_and_prune(db, &team, sub, now, &mut effects).await?;
    } else {
        let org = org_of(db, &team).await?;
        prune_outsiders(db, &org.root.id, sub, now, &mut effects).await?;
        prune_classifications(db, &org.root.id).await?;
    }
    Ok(effects)
}

#[utoipa::path(
    delete,
    path = "/bot-teams/{team_id}/people/{user_id}",
    tag = "Bot teams",
    params(
        ("team_id" = String, Path, description = "The team"),
        ("user_id" = String, Path, description = "The person; your own id leaves the team")
    ),
    responses(
        (status = 204, description = "The person left the team and its room, and their bots in the team with them unless they stay active elsewhere in the organization."),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_FORBIDDEN: only a team admin removes someone else"),
        (status = 404, description = "TEAM_NOT_FOUND, or the person is not in the team"),
        (status = 409, description = "TEAM_LAST_ADMIN: the last active admin can't leave while others remain")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "DELETE /bot-teams/{team_id}/people/{user_id}",
    skip(state, user)
)]
pub async fn remove_person(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((team_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let effects = state
        .transaction(|txn| {
            let (sub, team_id, user_id) = (sub.clone(), team_id.clone(), user_id.clone());
            Box::pin(async move {
                lock_org_scope(txn, &[team_id.as_str()]).await?;
                remove_person_in(txn, &sub, &team_id, &user_id, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(StatusCode::NO_CONTENT)
}

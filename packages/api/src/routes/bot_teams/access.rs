//! Who may see and change a team. A team the caller can't see is `TEAM_NOT_FOUND`; a visible one
//! where the caller lacks the role is `TEAM_FORBIDDEN`.

use crate::{
    bot_host::teams::{org_teams, root_of},
    entity::{bot_team, bot_team_person},
    error::ApiError,
};
use axum::http::StatusCode;
use flow_like_types::bot_team::{TeamMemberStatus, TeamRole};
use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter};

pub(crate) const TEAM_NOT_FOUND: &str = "TEAM_NOT_FOUND";
pub(crate) const TEAM_FORBIDDEN: &str = "TEAM_FORBIDDEN";
pub(crate) const TEAM_CYCLE: &str = "TEAM_CYCLE";
pub(crate) const TEAM_LAST_ADMIN: &str = "TEAM_LAST_ADMIN";
pub(crate) const TEAM_LIMIT: &str = "TEAM_LIMIT";

pub(crate) fn not_found(team_id: &str) -> ApiError {
    ApiError::coded(
        StatusCode::NOT_FOUND,
        TEAM_NOT_FOUND,
        format!("Team '{team_id}' not found"),
    )
}

pub(crate) fn forbidden(message: impl Into<String>) -> ApiError {
    ApiError::coded(StatusCode::FORBIDDEN, TEAM_FORBIDDEN, message)
}

pub(crate) fn limit(message: impl Into<String>) -> ApiError {
    ApiError::coded(StatusCode::CONFLICT, TEAM_LIMIT, message)
}

pub(crate) fn active() -> &'static str {
    TeamMemberStatus::Active.as_str()
}

pub(crate) fn pending() -> &'static str {
    TeamMemberStatus::Pending.as_str()
}

/// A team's org: its top-level team and every team under it.
pub(crate) struct Org {
    pub root: bot_team::Model,
    pub teams: Vec<bot_team::Model>,
}

impl Org {
    pub fn ids(&self) -> Vec<String> {
        self.teams.iter().map(|team| team.id.clone()).collect()
    }

    /// Levels `team_id` sits below the top-level team; `None` when it isn't in the org.
    pub fn depth_of(&self, team_id: &str) -> Option<usize> {
        let mut depth = 0;
        let mut current = self.teams.iter().find(|team| team.id == team_id)?;
        while let Some(parent) = current
            .parent_id
            .as_deref()
            .and_then(|id| self.teams.iter().find(|team| team.id == id))
        {
            depth += 1;
            if depth > self.teams.len() {
                break;
            }
            current = parent;
        }
        Some(depth)
    }

    /// `team_id` and every team below it, the team first.
    pub fn subtree(&self, team_id: &str) -> Vec<&bot_team::Model> {
        let mut found: Vec<&bot_team::Model> = self
            .teams
            .iter()
            .filter(|team| team.id == team_id)
            .collect();
        let mut index = 0;
        while index < found.len() {
            let id = found[index].id.clone();
            for child in self
                .teams
                .iter()
                .filter(|team| team.parent_id.as_deref() == Some(id.as_str()))
            {
                if !found.iter().any(|seen| seen.id == child.id) {
                    found.push(child);
                }
            }
            index += 1;
        }
        found
    }
}

pub(crate) async fn org_of<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
) -> Result<Org, DbErr> {
    let root = root_of(db, team).await?;
    let teams = org_teams(db, &root).await?;
    Ok(Org { root, teams })
}

pub(crate) async fn load_team<C: ConnectionTrait>(
    db: &C,
    team_id: &str,
) -> Result<bot_team::Model, ApiError> {
    bot_team::Entity::find_by_id(team_id)
        .one(db)
        .await?
        .ok_or_else(|| not_found(team_id))
}

pub(crate) async fn person_row<C: ConnectionTrait>(
    db: &C,
    team_id: &str,
    user_id: &str,
) -> Result<Option<bot_team_person::Model>, DbErr> {
    bot_team_person::Entity::find_by_id((team_id.to_string(), user_id.to_string()))
        .one(db)
        .await
}

/// Whether `user_id` is an active person of any team in `team_ids`.
pub(crate) async fn is_active_in<C: ConnectionTrait>(
    db: &C,
    team_ids: &[String],
    user_id: &str,
) -> Result<bool, DbErr> {
    Ok(bot_team_person::Entity::find()
        .filter(bot_team_person::Column::TeamId.is_in(team_ids.iter().cloned()))
        .filter(bot_team_person::Column::UserId.eq(user_id))
        .filter(bot_team_person::Column::Status.eq(active()))
        .one(db)
        .await?
        .is_some())
}

/// The team and its org when `user_id` is an active person of any team in the org.
pub(crate) async fn visible_team<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    team_id: &str,
) -> Result<(bot_team::Model, Org), ApiError> {
    let team = load_team(db, team_id).await?;
    let org = org_of(db, &team).await?;
    if !is_active_in(db, &org.ids(), user_id).await? {
        return Err(not_found(team_id));
    }
    Ok((team, org))
}

/// [`visible_team`] for an active person of the team itself, any role.
pub(crate) async fn member_team<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    team_id: &str,
) -> Result<(bot_team::Model, Org), ApiError> {
    let (team, org) = visible_team(db, user_id, team_id).await?;
    match person_row(db, team_id, user_id).await? {
        Some(row) if row.status == active() => Ok((team, org)),
        _ => Err(forbidden(format!(
            "You are not an active member of team '{}'",
            team.name
        ))),
    }
}

/// [`visible_team`] for an active admin of the team itself.
pub(crate) async fn admin_team<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    team_id: &str,
) -> Result<(bot_team::Model, Org), ApiError> {
    let (team, org) = visible_team(db, user_id, team_id).await?;
    match person_row(db, team_id, user_id).await? {
        Some(row)
            if row.status == active() && TeamRole::parse(&row.role) == Some(TeamRole::Admin) =>
        {
            Ok((team, org))
        }
        _ => Err(forbidden(format!(
            "Only an admin of team '{}' may do this",
            team.name
        ))),
    }
}

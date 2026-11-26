//! The wire shapes of teams: one team with its rows, and the org a person sees.

use super::access::{active, load_team, org_of, pending};
use crate::{
    bot_host::teams::{org_teams, root_of},
    entity::{bot, bot_team, bot_team_bot, bot_team_person},
    error::ApiError,
};
use flow_like_types::bot_team::{
    OrgBot, OrgView, Team, TeamBot, TeamInvitation, TeamInvitationKind, TeamMemberStatus,
    TeamPerson, TeamRole,
};
use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder};
use std::collections::{HashMap, HashSet};

fn rfc3339(time: &chrono::DateTime<chrono::FixedOffset>) -> String {
    time.to_rfc3339()
}

pub(crate) fn org_bot(row: &bot::Model) -> OrgBot {
    OrgBot {
        id: row.id.clone(),
        owner_id: row.owner_id.clone(),
        system: row.system_key.as_ref().map(|key| key.to_lowercase()),
        name: row.name.clone(),
        description: row.description.clone(),
        appearance: row.appearance.clone(),
        location: row.location.to_lowercase(),
        classification: row
            .classification
            .as_ref()
            .map(|value| value.to_lowercase()),
    }
}

async fn live_bots<C: ConnectionTrait>(
    db: &C,
    ids: impl IntoIterator<Item = String>,
) -> Result<HashMap<String, bot::Model>, DbErr> {
    Ok(bot::Entity::find()
        .filter(bot::Column::Id.is_in(ids))
        .filter(bot::Column::DeletedAt.is_null())
        .all(db)
        .await?
        .into_iter()
        .map(|row| (row.id.clone(), row))
        .collect())
}

/// The wire form of `teams` with their people and bots; rows of deleted bots are left out.
pub(crate) async fn team_views<C: ConnectionTrait>(
    db: &C,
    teams: &[bot_team::Model],
) -> Result<Vec<Team>, DbErr> {
    let ids: Vec<String> = teams.iter().map(|team| team.id.clone()).collect();
    let people = bot_team_person::Entity::find()
        .filter(bot_team_person::Column::TeamId.is_in(ids.clone()))
        .order_by_asc(bot_team_person::Column::CreatedAt)
        .order_by_asc(bot_team_person::Column::UserId)
        .all(db)
        .await?;
    let rows = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::TeamId.is_in(ids))
        .order_by_asc(bot_team_bot::Column::CreatedAt)
        .order_by_asc(bot_team_bot::Column::BotId)
        .all(db)
        .await?;
    let bots = live_bots(db, rows.iter().map(|row| row.bot_id.clone())).await?;
    Ok(teams
        .iter()
        .map(|team| Team {
            id: team.id.clone(),
            name: team.name.clone(),
            purpose: team.purpose.clone(),
            parent_id: team.parent_id.clone(),
            lead_bot_id: team.lead_bot_id.clone(),
            direct_asks: team.direct_asks,
            conversation_id: team.conversation_id.clone(),
            people: people
                .iter()
                .filter(|row| row.team_id == team.id)
                .map(|row| TeamPerson {
                    user_id: row.user_id.clone(),
                    role: TeamRole::parse(&row.role).unwrap_or(TeamRole::Member),
                    status: TeamMemberStatus::parse(&row.status)
                        .unwrap_or(TeamMemberStatus::Pending),
                    invited_by: row.invited_by.clone(),
                })
                .collect(),
            bots: rows
                .iter()
                .filter(|row| row.team_id == team.id)
                .filter_map(|row| {
                    let owner = bots.get(&row.bot_id)?;
                    Some(TeamBot {
                        bot_id: row.bot_id.clone(),
                        owner_id: owner.owner_id.clone(),
                        status: TeamMemberStatus::parse(&row.status)
                            .unwrap_or(TeamMemberStatus::Pending),
                        direct_agreed: row.direct_agreed,
                        invited_by: row.invited_by.clone(),
                    })
                })
                .collect(),
            created_at: rfc3339(&team.created_at),
            updated_at: rfc3339(&team.updated_at),
        })
        .collect())
}

pub(crate) async fn team_view<C: ConnectionTrait>(db: &C, team_id: &str) -> Result<Team, ApiError> {
    let team = load_team(db, team_id).await?;
    let mut views = team_views(db, &[team]).await?;
    views
        .pop()
        .ok_or_else(|| ApiError::internal(format!("Team '{team_id}' vanished while reading it")))
}

/// Every org `user_id` is an active person in: its teams, bots and what waits for the person.
pub(crate) async fn org_view<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
) -> Result<OrgView, ApiError> {
    let rows = bot_team_person::Entity::find()
        .filter(bot_team_person::Column::UserId.eq(user_id))
        .all(db)
        .await?;
    let mut teams: Vec<bot_team::Model> = Vec::new();
    let mut seen_roots = HashSet::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let own_teams = bot_team::Entity::find()
        .filter(bot_team::Column::Id.is_in(rows.iter().map(|row| row.team_id.clone())))
        .all(db)
        .await?;
    for row in &rows {
        let Some(team) = own_teams.iter().find(|team| team.id == row.team_id) else {
            continue;
        };
        let root = root_of(db, team).await?;
        names.insert(team.id.clone(), root.name.clone());
        if row.status == active() && seen_roots.insert(root.id.clone()) {
            teams.extend(org_teams(db, &root).await?);
        }
    }
    let team_ids: Vec<String> = teams.iter().map(|team| team.id.clone()).collect();
    let in_org = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::TeamId.is_in(team_ids))
        .all(db)
        .await?;
    let mut bot_ids: HashSet<String> = in_org.iter().map(|row| row.bot_id.clone()).collect();
    bot_ids.extend(teams.iter().filter_map(|team| team.lead_bot_id.clone()));
    let mut bots: Vec<OrgBot> = live_bots(db, bot_ids)
        .await?
        .values()
        .map(org_bot)
        .collect();
    bots.sort_by(|a, b| a.id.cmp(&b.id));
    let invitations = invitations_for(db, user_id, &rows, &own_teams, &names).await?;
    Ok(OrgView {
        teams: team_views(db, &teams).await?,
        bots,
        invitations,
    })
}

async fn invitations_for<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    person_rows: &[bot_team_person::Model],
    own_teams: &[bot_team::Model],
    root_names: &HashMap<String, String>,
) -> Result<Vec<TeamInvitation>, ApiError> {
    let mut invitations = Vec::new();
    for row in person_rows.iter().filter(|row| row.status == pending()) {
        let Some(team) = own_teams.iter().find(|team| team.id == row.team_id) else {
            continue;
        };
        invitations.push(TeamInvitation {
            kind: TeamInvitationKind::Person,
            team_id: team.id.clone(),
            team_name: team.name.clone(),
            org_name: root_names.get(&team.id).cloned().unwrap_or_default(),
            bot_id: None,
            invited_by: row.invited_by.clone(),
            created_at: rfc3339(&row.created_at),
        });
    }
    let owned: Vec<String> = bot::Entity::find()
        .filter(bot::Column::OwnerId.eq(user_id))
        .filter(bot::Column::DeletedAt.is_null())
        .all(db)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect();
    let bot_rows = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::BotId.is_in(owned))
        .all(db)
        .await?;
    let teams: HashMap<String, bot_team::Model> = bot_team::Entity::find()
        .filter(bot_team::Column::Id.is_in(bot_rows.iter().map(|row| row.team_id.clone())))
        .all(db)
        .await?
        .into_iter()
        .map(|team| (team.id.clone(), team))
        .collect();
    for row in &bot_rows {
        let Some(team) = teams.get(&row.team_id) else {
            continue;
        };
        let (kind, created_at) = if row.status == pending() {
            (TeamInvitationKind::Bot, &row.created_at)
        } else if team.direct_asks && !row.direct_agreed {
            (TeamInvitationKind::DirectAsks, &team.updated_at)
        } else {
            continue;
        };
        let org_name = match root_names.get(&team.id) {
            Some(name) => name.clone(),
            None => org_of(db, team).await?.root.name,
        };
        invitations.push(TeamInvitation {
            kind,
            team_id: team.id.clone(),
            team_name: team.name.clone(),
            org_name,
            bot_id: Some(row.bot_id.clone()),
            invited_by: row.invited_by.clone(),
            created_at: rfc3339(created_at),
        });
    }
    Ok(invitations)
}

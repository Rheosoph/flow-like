//! Reads the bot team tables for the runtime: the team a bot is active in, its org, the brief the
//! bot reads before every turn, the teammates and leads its `ask_bot` roster adds beyond its
//! owner's crew, and whether two bots ask each other without the first-time Allow card.

use crate::{
    bot_notify::display_name,
    entity::{bot, bot_team, bot_team_bot},
    routes::inbox::invitations::person_name,
    state::AppState,
};
use flow_like_types::bot_team::{
    BriefBot, BriefTeam, MAX_ORG_TEAMS, MAX_TEAM_DEPTH, TeamBrief, TeamMemberStatus, team_tag,
};
use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter};
use std::collections::{HashMap, HashSet};

fn active() -> &'static str {
    TeamMemberStatus::Active.as_str()
}

/// The team `bot_id` is active in; a bot is active in at most one.
pub(crate) async fn active_team_of<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
) -> Result<Option<bot_team::Model>, DbErr> {
    let Some(row) = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::BotId.eq(bot_id))
        .filter(bot_team_bot::Column::Status.eq(active()))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    bot_team::Entity::find_by_id(row.team_id).one(db).await
}

/// The top-level team above `team`, or `team` itself. A chain longer than the depth limit, or one
/// that loops, stops at the last team reached.
pub(crate) async fn root_of<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
) -> Result<bot_team::Model, DbErr> {
    let mut current = team.clone();
    let mut seen = HashSet::from([current.id.clone()]);
    while let Some(parent_id) = current.parent_id.clone() {
        if seen.len() > MAX_TEAM_DEPTH + 1 {
            break;
        }
        let Some(parent) = bot_team::Entity::find_by_id(&parent_id).one(db).await? else {
            break;
        };
        if !seen.insert(parent.id.clone()) {
            break;
        }
        current = parent;
    }
    Ok(current)
}

/// Every team of the org under `root`, the root first, then level by level.
pub(crate) async fn org_teams<C: ConnectionTrait>(
    db: &C,
    root: &bot_team::Model,
) -> Result<Vec<bot_team::Model>, DbErr> {
    let mut teams = vec![root.clone()];
    let mut seen = HashSet::from([root.id.clone()]);
    let mut frontier = vec![root.id.clone()];
    while !frontier.is_empty() && teams.len() < MAX_ORG_TEAMS {
        let children = bot_team::Entity::find()
            .filter(bot_team::Column::ParentId.is_in(frontier.clone()))
            .all(db)
            .await?;
        frontier.clear();
        for child in children {
            if seen.insert(child.id.clone()) {
                frontier.push(child.id.clone());
                teams.push(child);
            }
        }
    }
    Ok(teams)
}

/// Whether `a` and `b` are active in the same team, that team has direct asks on, and both bots'
/// owners agreed to them.
pub(crate) async fn direct_between<C: ConnectionTrait>(
    db: &C,
    a: &str,
    b: &str,
) -> Result<bool, DbErr> {
    let rows = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::BotId.is_in([a.to_string(), b.to_string()]))
        .filter(bot_team_bot::Column::Status.eq(active()))
        .all(db)
        .await?;
    let (Some(first), Some(second)) = (
        rows.iter().find(|row| row.bot_id == a),
        rows.iter().find(|row| row.bot_id == b),
    ) else {
        return Ok(false);
    };
    if first.team_id != second.team_id || !first.direct_agreed || !second.direct_agreed {
        return Ok(false);
    }
    Ok(bot_team::Entity::find_by_id(&first.team_id)
        .one(db)
        .await?
        .is_some_and(|team| team.direct_asks))
}

/// Whether `a` and `b` are active in teams of the same org.
pub(crate) async fn same_org<C: ConnectionTrait>(db: &C, a: &str, b: &str) -> Result<bool, DbErr> {
    let (Some(first), Some(second)) = (active_team_of(db, a).await?, active_team_of(db, b).await?)
    else {
        return Ok(false);
    };
    if first.id == second.id {
        return Ok(true);
    }
    Ok(root_of(db, &first).await?.id == root_of(db, &second).await?.id)
}

/// A bot a team member may ask beyond its owner's crew.
#[derive(Debug, Clone)]
pub(crate) struct TeamReach {
    pub bot: bot::Model,
    /// The team the bot is active in.
    pub team_name: String,
    /// The team it leads, when it leads one.
    pub lead_of: Option<String>,
    /// Asked without the first-time Allow card.
    pub direct: bool,
}

/// Who `bot_id` may ask through its team: its active teammates of any owner and the leads of the
/// org's other teams. Live bots only, never `bot_id` itself. Members of other teams are reached
/// through their lead and are not listed.
pub(crate) async fn team_reach<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
) -> Result<Vec<TeamReach>, DbErr> {
    let Some(team) = active_team_of(db, bot_id).await? else {
        return Ok(Vec::new());
    };
    let root = root_of(db, &team).await?;
    let teams = org_teams(db, &root).await?;
    let by_id: HashMap<&str, &bot_team::Model> =
        teams.iter().map(|team| (team.id.as_str(), team)).collect();
    let rows = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::TeamId.is_in(teams.iter().map(|team| team.id.clone())))
        .filter(bot_team_bot::Column::Status.eq(active()))
        .all(db)
        .await?;
    let own_agreed = rows
        .iter()
        .any(|row| row.bot_id == bot_id && row.team_id == team.id && row.direct_agreed);
    let mut wanted: HashMap<String, (String, Option<String>, bool)> = HashMap::new();
    for row in rows.iter().filter(|row| row.bot_id != bot_id) {
        let Some(their_team) = by_id.get(row.team_id.as_str()) else {
            continue;
        };
        let leads = their_team.lead_bot_id.as_deref() == Some(row.bot_id.as_str());
        if row.team_id == team.id {
            let direct = team.direct_asks && own_agreed && row.direct_agreed;
            wanted.insert(
                row.bot_id.clone(),
                (
                    their_team.name.clone(),
                    leads.then(|| their_team.name.clone()),
                    direct,
                ),
            );
        } else if leads {
            wanted.insert(
                row.bot_id.clone(),
                (
                    their_team.name.clone(),
                    Some(their_team.name.clone()),
                    false,
                ),
            );
        }
    }
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let bots = bot::Entity::find()
        .filter(bot::Column::Id.is_in(wanted.keys().cloned()))
        .filter(bot::Column::DeletedAt.is_null())
        .all(db)
        .await?;
    Ok(bots
        .into_iter()
        .filter_map(|row| {
            let (team_name, lead_of, direct) = wanted.remove(&row.id)?;
            Some(TeamReach {
                bot: row,
                team_name,
                lead_of,
                direct,
            })
        })
        .collect())
}

fn description(row: &bot::Model) -> Option<String> {
    row.description
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// The brief `bot_id` reads about its team and org before every turn; `None` while it is in no
/// team or is gone.
pub(crate) async fn team_brief_for(
    state: &AppState,
    bot_id: &str,
) -> Result<Option<TeamBrief>, DbErr> {
    let db = &state.db;
    let Some(team) = active_team_of(db, bot_id).await? else {
        return Ok(None);
    };
    let Some(me) = bot::Entity::find_by_id(bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    let root = root_of(db, &team).await?;
    let teams = org_teams(db, &root).await?;
    let rows = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::TeamId.is_in(teams.iter().map(|team| team.id.clone())))
        .all(db)
        .await?;
    let mut ids: HashSet<String> = rows.iter().map(|row| row.bot_id.clone()).collect();
    ids.extend(teams.iter().filter_map(|team| team.lead_bot_id.clone()));
    ids.insert(me.id.clone());
    let bots: HashMap<String, bot::Model> = bot::Entity::find()
        .filter(bot::Column::Id.is_in(ids))
        .filter(bot::Column::DeletedAt.is_null())
        .all(db)
        .await?
        .into_iter()
        .map(|row| (row.id.clone(), row))
        .collect();
    let mut owners: HashMap<String, String> = HashMap::new();
    for owner_id in bots.values().map(|row| row.owner_id.clone()) {
        if !owners.contains_key(&owner_id) {
            let name = person_name(state, &owner_id).await;
            owners.insert(owner_id, name);
        }
    }
    let brief_bot = |id: &str, joining: bool| -> Option<BriefBot> {
        let row = bots.get(id)?;
        Some(BriefBot {
            id: row.id.clone(),
            name: display_name(row),
            owner_name: owners.get(&row.owner_id).cloned().unwrap_or_default(),
            description: description(row),
            joining,
        })
    };
    let status_of = |row: &bot_team_bot::Model| TeamMemberStatus::parse(&row.status);
    let is_lead = team.lead_bot_id.as_deref() == Some(bot_id);
    let parent = team
        .parent_id
        .as_deref()
        .and_then(|parent_id| teams.iter().find(|candidate| candidate.id == parent_id));
    let teammates: Vec<BriefBot> = rows
        .iter()
        .filter(|row| row.team_id == team.id && row.bot_id != bot_id)
        .filter_map(|row| {
            brief_bot(
                &row.bot_id,
                status_of(row) == Some(TeamMemberStatus::Pending),
            )
        })
        .collect();
    let lead = if is_lead {
        None
    } else {
        team.lead_bot_id
            .as_deref()
            .and_then(|lead| brief_bot(lead, false))
    };
    let reports_to = if is_lead {
        parent
            .and_then(|parent| parent.lead_bot_id.as_deref())
            .filter(|lead| *lead != bot_id)
            .and_then(|lead| brief_bot(lead, false))
    } else {
        None
    };
    let sub_team_leads = if is_lead {
        teams
            .iter()
            .filter(|candidate| candidate.parent_id.as_deref() == Some(team.id.as_str()))
            .filter_map(|candidate| candidate.lead_bot_id.as_deref())
            .filter_map(|lead| brief_bot(lead, false))
            .collect()
    } else {
        Vec::new()
    };
    let agreed = rows.iter().any(|row| {
        row.team_id == team.id
            && row.bot_id == bot_id
            && status_of(row) == Some(TeamMemberStatus::Active)
            && row.direct_agreed
    });
    let mut org = Vec::new();
    let mut stack: Vec<(&bot_team::Model, u32)> = vec![(&root, 0)];
    while let Some((node, depth)) = stack.pop() {
        let lead = node
            .lead_bot_id
            .as_deref()
            .and_then(|lead| bots.get(lead))
            .map(display_name);
        let members = rows
            .iter()
            .filter(|row| {
                row.team_id == node.id
                    && status_of(row) == Some(TeamMemberStatus::Active)
                    && node.lead_bot_id.as_deref() != Some(row.bot_id.as_str())
            })
            .filter_map(|row| brief_bot(&row.bot_id, false))
            .collect();
        org.push(BriefTeam {
            id: node.id.clone(),
            name: node.name.clone(),
            depth,
            lead,
            members,
        });
        let mut children: Vec<&bot_team::Model> = teams
            .iter()
            .filter(|candidate| candidate.parent_id.as_deref() == Some(node.id.as_str()))
            .collect();
        children.sort_by(|a, b| b.name.cmp(&a.name));
        stack.extend(children.into_iter().map(|child| (child, depth + 1)));
    }
    Ok(Some(TeamBrief {
        bot_id: me.id.clone(),
        bot_name: display_name(&me),
        owner_name: owners.get(&me.owner_id).cloned().unwrap_or_default(),
        team_id: team.id.clone(),
        team_name: team.name.clone(),
        purpose: team.purpose.clone(),
        parent_name: parent.map(|parent| parent.name.clone()),
        is_lead,
        lead,
        reports_to,
        sub_team_leads,
        teammates,
        direct_asks: team.direct_asks && agreed,
        room: team.conversation_id.as_ref().map(|_| team_tag(&team.name)),
        org,
    }))
}

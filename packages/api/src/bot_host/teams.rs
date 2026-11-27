//! Reads the bot team tables for the runtime: the team a bot is active in, its org, the brief the
//! bot reads before every turn, the directory its `org_lookup` searches, the teammates and leads
//! its `ask_bot` roster adds beyond its owner's crew, and whether two bots ask each other without
//! the first-time Allow card.

use crate::{
    bot_notify::display_name,
    entity::{bot, bot_team, bot_team_bot, bot_team_person, conversation_member},
    routes::{
        bot_teams::levels::person_levels,
        inbox::{active_member, outreach::person_names},
    },
    state::AppState,
};
use flow_like_types::{
    bot_team::{
        BriefBot, BriefMembership, BriefPerson, BriefTeam, DirectoryBot, DirectoryPerson,
        DirectoryTeam, MAX_ORG_TEAMS, MAX_TEAM_DEPTH, OrgDirectory, OrgLookup, TeamBrief,
        TeamMemberStatus, TeamRole, UNCLASSIFIED_PERSON, org_lookup, team_tag,
    },
    data_classification::DataClassification,
};
use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QuerySelect};
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

/// The active people of `conversation_id`.
async fn conversation_people<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
) -> Result<Vec<String>, DbErr> {
    conversation_member::Entity::find()
        .select_only()
        .column(conversation_member::Column::PrincipalId)
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::PrincipalKind.eq("USER"))
        .filter(active_member())
        .into_tuple()
        .all(db)
        .await
}

/// Whether every one of `people` is an active person of one of `teams`; true for nobody.
async fn all_insiders<C: ConnectionTrait>(
    db: &C,
    teams: &[bot_team::Model],
    people: &[String],
) -> Result<bool, DbErr> {
    if people.is_empty() {
        return Ok(true);
    }
    let insiders: HashSet<String> = bot_team_person::Entity::find()
        .select_only()
        .column(bot_team_person::Column::UserId)
        .filter(bot_team_person::Column::TeamId.is_in(teams.iter().map(|team| team.id.clone())))
        .filter(bot_team_person::Column::UserId.is_in(people.iter().cloned()))
        .filter(bot_team_person::Column::Status.eq(active()))
        .into_tuple()
        .all(db)
        .await?
        .into_iter()
        .collect();
    Ok(people.iter().all(|person| insiders.contains(person)))
}

/// Whether a bot active in `team` may use its team context (brief, lookup, teammates, other
/// leads) in `conversation_id`: always in its owner's own chat (`None`), otherwise only when every
/// active person member of the conversation is an active person of the team's org.
pub(crate) async fn team_context_allowed<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    conversation_id: Option<&str>,
) -> Result<bool, DbErr> {
    let Some(conversation_id) = conversation_id else {
        return Ok(true);
    };
    let people = conversation_people(db, conversation_id).await?;
    if people.is_empty() {
        return Ok(true);
    }
    let root = root_of(db, team).await?;
    let teams = org_teams(db, &root).await?;
    all_insiders(db, &teams, &people).await
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
/// through their lead and are not listed. Nobody is listed in a conversation with people outside
/// the org (see [`team_context_allowed`]).
pub(crate) async fn team_reach<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    conversation_id: Option<&str>,
) -> Result<Vec<TeamReach>, DbErr> {
    let Some(team) = active_team_of(db, bot_id).await? else {
        return Ok(Vec::new());
    };
    if !team_context_allowed(db, &team, conversation_id).await? {
        return Ok(Vec::new());
    }
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

fn role_of(row: &bot_team_person::Model) -> TeamRole {
    TeamRole::parse(&row.role).unwrap_or(TeamRole::Member)
}

async fn live_bots<C: ConnectionTrait>(
    db: &C,
    ids: HashSet<String>,
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

async fn names_of<C: ConnectionTrait>(
    db: &C,
    ids: impl IntoIterator<Item = String>,
) -> Result<HashMap<String, String>, DbErr> {
    let ids: Vec<String> = ids
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    person_names(db, &ids).await.map_err(|error| {
        DbErr::Custom(format!(
            "Reading the names of a team's people failed: {error}"
        ))
    })
}

/// An org's teams and the bots active in them.
struct OrgTables {
    root: bot_team::Model,
    /// The root first, then level by level.
    teams: Vec<bot_team::Model>,
    bots: Vec<bot_team_bot::Model>,
}

impl OrgTables {
    async fn of<C: ConnectionTrait>(db: &C, team: &bot_team::Model) -> Result<Self, DbErr> {
        let root = root_of(db, team).await?;
        let teams = org_teams(db, &root).await?;
        let bots = bot_team_bot::Entity::find()
            .filter(bot_team_bot::Column::TeamId.is_in(teams.iter().map(|team| team.id.clone())))
            .filter(bot_team_bot::Column::Status.eq(active()))
            .all(db)
            .await?;
        Ok(Self { root, teams, bots })
    }

    fn team(&self, id: &str) -> Option<&bot_team::Model> {
        self.teams.iter().find(|team| team.id == id)
    }

    /// The active people of the org's teams; with `user_ids`, only theirs.
    async fn people<C: ConnectionTrait>(
        &self,
        db: &C,
        user_ids: Option<&[String]>,
    ) -> Result<Vec<bot_team_person::Model>, DbErr> {
        let mut query = bot_team_person::Entity::find()
            .filter(
                bot_team_person::Column::TeamId
                    .is_in(self.teams.iter().map(|team| team.id.clone())),
            )
            .filter(bot_team_person::Column::Status.eq(active()));
        if let Some(ids) = user_ids {
            query = query.filter(bot_team_person::Column::UserId.is_in(ids.iter().cloned()));
        }
        query.all(db).await
    }

    /// The teams depth first, children by name, as a brief outlines its org.
    fn outline(&self, bots: &HashMap<String, bot::Model>) -> Vec<BriefTeam> {
        let mut org = Vec::new();
        let mut seen = HashSet::new();
        let mut stack: Vec<(&bot_team::Model, u32)> = vec![(&self.root, 0)];
        while let Some((node, depth)) = stack.pop() {
            if !seen.insert(node.id.as_str()) {
                continue;
            }
            org.push(BriefTeam {
                id: node.id.clone(),
                name: node.name.clone(),
                parent_id: node.parent_id.clone().filter(|_| depth > 0),
                depth,
                purpose: node.purpose.clone(),
                lead: node
                    .lead_bot_id
                    .as_deref()
                    .and_then(|lead| bots.get(lead))
                    .map(display_name),
                bots: self
                    .bots
                    .iter()
                    .filter(|row| row.team_id == node.id)
                    .count() as u32,
            });
            let mut children: Vec<&bot_team::Model> = self
                .teams
                .iter()
                .filter(|child| child.parent_id.as_deref() == Some(node.id.as_str()))
                .collect();
            children.sort_by(|a, b| b.name.cmp(&a.name));
            stack.extend(children.into_iter().map(|child| (child, depth + 1)));
        }
        org
    }

    /// Each person of `rows` once, by name, with their teams in the org's order.
    fn directory_people(
        &self,
        rows: &[bot_team_person::Model],
        names: &HashMap<String, String>,
    ) -> Vec<DirectoryPerson> {
        let order: HashMap<&str, usize> = self
            .teams
            .iter()
            .enumerate()
            .map(|(index, team)| (team.id.as_str(), index))
            .collect();
        let mut by_user: HashMap<&str, Vec<(usize, String, TeamRole)>> = HashMap::new();
        for row in rows {
            let Some(index) = order.get(row.team_id.as_str()) else {
                continue;
            };
            by_user.entry(row.user_id.as_str()).or_default().push((
                *index,
                row.team_id.clone(),
                role_of(row),
            ));
        }
        let mut people: Vec<DirectoryPerson> = by_user
            .into_iter()
            .map(|(user_id, mut teams)| {
                teams.sort_by_key(|(index, ..)| *index);
                DirectoryPerson {
                    name: names.get(user_id).cloned().unwrap_or_default(),
                    teams: teams.into_iter().map(|(_, id, role)| (id, role)).collect(),
                }
            })
            .collect();
        people.sort_by(|a, b| a.name.cmp(&b.name));
        people
    }
}

/// A person in the conversation a turn runs in, before their name is read.
struct Present {
    user_id: String,
    teams: Vec<BriefMembership>,
    classification: DataClassification,
}

/// The people of a conversation with their teams in `org` and their classification there, when
/// someone besides `owner_id` is among them.
async fn present_in<C: ConnectionTrait>(
    db: &C,
    org: &OrgTables,
    owner_id: &str,
    people: &[String],
) -> Result<Vec<Present>, DbErr> {
    if people.iter().all(|person| person == owner_id) {
        return Ok(Vec::new());
    }
    let rows = org.people(db, Some(people)).await?;
    let levels = person_levels(db, &org.root.id, people).await?;
    Ok(people
        .iter()
        .map(|user_id| Present {
            user_id: user_id.clone(),
            teams: org
                .teams
                .iter()
                .filter_map(|team| {
                    let row = rows
                        .iter()
                        .find(|row| row.user_id == *user_id && row.team_id == team.id)?;
                    Some(BriefMembership {
                        team: team.name.clone(),
                        role: role_of(row),
                    })
                })
                .collect(),
            classification: levels.get(user_id).copied().unwrap_or(UNCLASSIFIED_PERSON),
        })
        .collect())
}

/// The brief `bot_id` reads in its owner's own chat.
pub(crate) async fn team_brief_for(
    state: &AppState,
    bot_id: &str,
) -> Result<Option<TeamBrief>, DbErr> {
    team_brief_in(state, bot_id, None).await
}

/// The brief `bot_id` reads about its team, its org and the people of `conversation_id` before
/// every turn there; `None` while it is in no team, is gone, or the conversation holds people
/// outside the org. It names the bots of its own team and every team's lead, never all of a large
/// org's bots: `org_lookup` finds the rest.
pub(crate) async fn team_brief_in(
    state: &AppState,
    bot_id: &str,
    conversation_id: Option<&str>,
) -> Result<Option<TeamBrief>, DbErr> {
    let db = &state.db;
    let Some(team) = active_team_of(db, bot_id).await? else {
        return Ok(None);
    };
    let org = OrgTables::of(db, &team).await?;
    let people = match conversation_id {
        Some(conversation_id) => conversation_people(db, conversation_id).await?,
        None => Vec::new(),
    };
    if !all_insiders(db, &org.teams, &people).await? {
        return Ok(None);
    }
    let mut wanted: HashSet<String> = org
        .bots
        .iter()
        .filter(|row| row.team_id == team.id)
        .map(|row| row.bot_id.clone())
        .collect();
    wanted.extend(org.teams.iter().filter_map(|team| team.lead_bot_id.clone()));
    wanted.insert(bot_id.to_string());
    let bots = live_bots(db, wanted).await?;
    let Some(me) = bots.get(bot_id) else {
        return Ok(None);
    };
    let present = present_in(db, &org, &me.owner_id, &people).await?;
    let names = names_of(
        db,
        bots.values()
            .map(|row| row.owner_id.clone())
            .chain(present.iter().map(|person| person.user_id.clone())),
    )
    .await?;
    let brief_bot = |id: &str| -> Option<BriefBot> {
        let row = bots.get(id)?;
        Some(BriefBot {
            id: row.id.clone(),
            name: display_name(row),
            owner_name: names.get(&row.owner_id).cloned().unwrap_or_default(),
            description: description(row),
        })
    };
    let is_lead = team.lead_bot_id.as_deref() == Some(bot_id);
    let parent = team
        .parent_id
        .as_deref()
        .and_then(|parent_id| org.team(parent_id));
    let teammates: Vec<BriefBot> = org
        .bots
        .iter()
        .filter(|row| row.team_id == team.id && row.bot_id != bot_id)
        .filter_map(|row| brief_bot(&row.bot_id))
        .collect();
    let lead = if is_lead {
        None
    } else {
        team.lead_bot_id.as_deref().and_then(|lead| brief_bot(lead))
    };
    let reports_to = if is_lead {
        parent
            .and_then(|parent| parent.lead_bot_id.as_deref())
            .filter(|lead| *lead != bot_id)
            .and_then(|lead| brief_bot(lead))
    } else {
        None
    };
    let sub_team_leads = if is_lead {
        org.teams
            .iter()
            .filter(|candidate| candidate.parent_id.as_deref() == Some(team.id.as_str()))
            .filter_map(|candidate| candidate.lead_bot_id.as_deref())
            .filter_map(|lead| brief_bot(lead))
            .collect()
    } else {
        Vec::new()
    };
    let agreed = org
        .bots
        .iter()
        .any(|row| row.team_id == team.id && row.bot_id == bot_id && row.direct_agreed);
    let mut present: Vec<BriefPerson> = present
        .into_iter()
        .map(|person| BriefPerson {
            name: names.get(&person.user_id).cloned().unwrap_or_default(),
            owner: person.user_id == me.owner_id,
            teams: person.teams,
            classification: person.classification,
        })
        .collect();
    present.sort_by(|a, b| a.owner.cmp(&b.owner).then_with(|| a.name.cmp(&b.name)));
    Ok(Some(TeamBrief {
        bot_id: me.id.clone(),
        bot_name: display_name(me),
        owner_name: names.get(&me.owner_id).cloned().unwrap_or_default(),
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
        org: org.outline(&bots),
        present,
    }))
}

/// What `bot_id`'s `org_lookup` finds for `query` in its org: what every member of the org sees
/// in the org chart, never anyone's classification. `None` where its brief would be `None`.
pub(crate) async fn org_lookup_in(
    state: &AppState,
    bot_id: &str,
    conversation_id: Option<&str>,
    query: &str,
) -> Result<Option<OrgLookup>, DbErr> {
    let db = &state.db;
    let Some(team) = active_team_of(db, bot_id).await? else {
        return Ok(None);
    };
    let org = OrgTables::of(db, &team).await?;
    let people = match conversation_id {
        Some(conversation_id) => conversation_people(db, conversation_id).await?,
        None => Vec::new(),
    };
    if !all_insiders(db, &org.teams, &people).await? {
        return Ok(None);
    }
    let bots = live_bots(db, org.bots.iter().map(|row| row.bot_id.clone()).collect()).await?;
    let rows = org.people(db, None).await?;
    let names = names_of(
        db,
        bots.values()
            .map(|row| row.owner_id.clone())
            .chain(rows.iter().map(|row| row.user_id.clone())),
    )
    .await?;
    let order: HashMap<&str, usize> = org
        .teams
        .iter()
        .enumerate()
        .map(|(index, team)| (team.id.as_str(), index))
        .collect();
    let mut directory_bots: Vec<DirectoryBot> = org
        .bots
        .iter()
        .filter_map(|row| {
            let bot = bots.get(&row.bot_id)?;
            Some(DirectoryBot {
                id: bot.id.clone(),
                name: display_name(bot),
                owner_name: names.get(&bot.owner_id).cloned().unwrap_or_default(),
                description: description(bot),
                team_id: row.team_id.clone(),
            })
        })
        .collect();
    directory_bots.sort_by(|a, b| {
        order
            .get(a.team_id.as_str())
            .cmp(&order.get(b.team_id.as_str()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let directory = OrgDirectory {
        teams: org
            .teams
            .iter()
            .map(|team| DirectoryTeam {
                id: team.id.clone(),
                name: team.name.clone(),
                parent_id: team.parent_id.clone().filter(|_| team.id != org.root.id),
                purpose: team.purpose.clone(),
                lead_bot_id: team.lead_bot_id.clone(),
            })
            .collect(),
        bots: directory_bots,
        people: org.directory_people(&rows, &names),
    };
    Ok(Some(org_lookup(&directory, query)))
}

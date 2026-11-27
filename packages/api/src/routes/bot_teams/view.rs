//! The wire shapes of teams: one team with its rows, and the org a person sees.

use super::{
    access::{active, load_team, org_of, pending},
    inputs::inputs_of,
    levels::person_levels,
    org_apps::org_apps,
    reach::{ChatFacts, Placement, ReachFacts, reach_of},
};
use crate::{
    bot_host::teams::{org_teams, root_of},
    entity::{
        app, bot, bot_app, bot_org_classification, bot_team, bot_team_app, bot_team_bot,
        bot_team_chat, bot_team_person, bot_team_thread, conversation,
    },
    error::ApiError,
    routes::inbox::{image::image_urls, now_millis},
    state::AppState,
};
use flow_like_types::{
    bot_team::{
        BotChat, BotChatStatus, OrgApp, OrgBot, OrgView, PersonClassification, Team, TeamApp, TeamBot,
        TeamInvitation, TeamInvitationKind, TeamMemberStatus, TeamPerson, TeamRole, TeamThread,
        UNCLASSIFIED_PERSON, person_may_talk,
    },
    data_classification::DataClassification,
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder,
};
use std::collections::{HashMap, HashSet};

use crate::entity::sea_orm_active_enums::Status;

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
        inputs: Default::default(),
        app_ids: Vec::new(),
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

fn parse_level(row: &bot::Model) -> Option<DataClassification> {
    row.classification
        .as_deref()
        .and_then(DataClassification::parse)
}

/// The top-level team of each of `teams`, found inside the slice where possible.
pub(crate) async fn root_ids<C: ConnectionTrait>(
    db: &C,
    teams: &[bot_team::Model],
) -> Result<HashMap<String, String>, DbErr> {
    let by_id: HashMap<&str, &bot_team::Model> =
        teams.iter().map(|team| (team.id.as_str(), team)).collect();
    let mut roots = HashMap::new();
    for team in teams {
        let mut current = team;
        let mut hops = 0;
        while let Some(parent) = current
            .parent_id
            .as_deref()
            .and_then(|id| by_id.get(id))
            .filter(|_| hops <= teams.len())
        {
            current = parent;
            hops += 1;
        }
        let root = match current.parent_id {
            None => current.id.clone(),
            Some(_) => root_of(db, current).await?.id,
        };
        roots.insert(team.id.clone(), root);
    }
    Ok(roots)
}

/// Bots of `team` that stay silent in its room: an active bot above the level of some active
/// person other than its owner.
fn gated_in(
    team_bots: &[&bot_team_bot::Model],
    bots: &HashMap<String, bot::Model>,
    people: &[&bot_team_person::Model],
    levels: &HashMap<String, DataClassification>,
) -> Vec<String> {
    team_bots
        .iter()
        .filter(|row| row.status == active())
        .filter_map(|row| bots.get(&row.bot_id))
        .filter(|bot| {
            people
                .iter()
                .filter(|person| person.status == active() && person.user_id != bot.owner_id)
                .any(|person| {
                    let level = levels.get(&person.user_id).copied();
                    !person_may_talk(level, parse_level(bot))
                })
        })
        .map(|bot| bot.id.clone())
        .collect()
}

/// The wire form of `teams` as `viewer` sees them, with their people and bots. Rows of deleted
/// bots are left out; a pending bot shows only to its owner and the team's admins, and a lead
/// offer only to those admins and the offered bot's owner.
pub(crate) async fn team_views<C: ConnectionTrait>(
    db: &C,
    viewer: &str,
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
    let pins = visible_pin_rows(db, &ids_of(teams)).await?;
    let bots = live_bots(
        db,
        rows.iter().map(|row| row.bot_id.clone()).chain(
            teams
                .iter()
                .filter_map(|team| team.lead_offer_bot_id.clone()),
        ),
    )
    .await?;
    let roots = root_ids(db, teams).await?;
    let mut levels: HashMap<String, HashMap<String, DataClassification>> = HashMap::new();
    for root in roots.values().collect::<HashSet<_>>() {
        let users: Vec<String> = people
            .iter()
            .filter(|row| row.status == active())
            .map(|row| row.user_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        levels.insert(root.clone(), person_levels(db, root, &users).await?);
    }
    let no_levels = HashMap::new();
    Ok(teams
        .iter()
        .map(|team| {
            let team_people: Vec<&bot_team_person::Model> =
                people.iter().filter(|row| row.team_id == team.id).collect();
            let team_rows: Vec<&bot_team_bot::Model> =
                rows.iter().filter(|row| row.team_id == team.id).collect();
            let viewer_admin = team_people.iter().any(|row| {
                row.user_id == viewer
                    && row.status == active()
                    && TeamRole::parse(&row.role) == Some(TeamRole::Admin)
            });
            let owned_by_viewer =
                |bot_id: &str| bots.get(bot_id).is_some_and(|bot| bot.owner_id == viewer);
            let lead_offer_bot_id = team
                .lead_offer_bot_id
                .clone()
                .filter(|id| bots.contains_key(id))
                .filter(|id| viewer_admin || owned_by_viewer(id));
            let team_levels = roots
                .get(&team.id)
                .and_then(|root| levels.get(root))
                .unwrap_or(&no_levels);
            Team {
                id: team.id.clone(),
                name: team.name.clone(),
                purpose: team.purpose.clone(),
                parent_id: team.parent_id.clone(),
                lead_bot_id: team.lead_bot_id.clone(),
                direct_asks: team.direct_asks,
                conversation_id: team.conversation_id.clone(),
                image_url: None,
                lead_offer_bot_id,
                gated_bots: gated_in(&team_rows, &bots, &team_people, team_levels),
                people: team_people
                    .iter()
                    .map(|row| TeamPerson {
                        user_id: row.user_id.clone(),
                        role: TeamRole::parse(&row.role).unwrap_or(TeamRole::Member),
                        status: TeamMemberStatus::parse(&row.status)
                            .unwrap_or(TeamMemberStatus::Pending),
                        invited_by: row.invited_by.clone(),
                    })
                    .collect(),
                bots: team_rows
                    .iter()
                    .filter_map(|row| {
                        let owner = bots.get(&row.bot_id)?;
                        if row.status == pending() && !viewer_admin && owner.owner_id != viewer {
                            return None;
                        }
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
                apps: pins
                    .iter()
                    .filter(|pin| pin.team_id == team.id)
                    .filter(|pin| {
                        pin.status != pending() || viewer_admin || pin.pinned_by == viewer
                    })
                    .map(team_app)
                    .collect(),
                created_at: rfc3339(&team.created_at),
                updated_at: rfc3339(&team.updated_at),
            }
        })
        .collect())
}

fn ids_of(teams: &[bot_team::Model]) -> Vec<String> {
    teams.iter().map(|team| team.id.clone()).collect()
}

fn team_app(pin: &bot_team_app::Model) -> TeamApp {
    TeamApp {
        app_id: pin.app_id.clone(),
        status: TeamMemberStatus::parse(&pin.status).unwrap_or(TeamMemberStatus::Pending),
        pinned_by: pin.pinned_by.clone(),
        approved_by: pin.approved_by.clone(),
        pinned_at: rfc3339(&pin.created_at),
    }
}

/// Every pin of `team_ids` whose app still exists and is neither archived nor a bot's workspace,
/// oldest first.
async fn visible_pin_rows<C: ConnectionTrait>(
    db: &C,
    team_ids: &[String],
) -> Result<Vec<bot_team_app::Model>, DbErr> {
    let pins = bot_team_app::Entity::find()
        .filter(bot_team_app::Column::TeamId.is_in(team_ids.to_vec()))
        .order_by_asc(bot_team_app::Column::CreatedAt)
        .order_by_asc(bot_team_app::Column::AppId)
        .all(db)
        .await?;
    let live = live_app_ids(db, pins.iter().map(|pin| pin.app_id.clone())).await?;
    Ok(pins
        .into_iter()
        .filter(|pin| live.contains(&pin.app_id))
        .collect())
}

async fn live_app_ids<C: ConnectionTrait>(
    db: &C,
    ids: impl IntoIterator<Item = String>,
) -> Result<HashSet<String>, DbErr> {
    let ids: Vec<String> = ids.into_iter().collect();
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    Ok(app::Entity::find()
        .filter(app::Column::Id.is_in(ids))
        .filter(app::Column::BotId.is_null())
        .filter(app::Column::Status.ne(Status::Archived))
        .all(db)
        .await?
        .into_iter()
        .map(|row| row.id)
        .collect())
}

/// Links each team to its room's picture, its icon. Everyone in the org sees it, also people
/// outside the room, whose inbox never lists it.
pub(crate) async fn sign_team_images(state: &AppState, teams: &mut [Team]) -> Result<(), DbErr> {
    let rooms: Vec<String> = teams
        .iter()
        .filter_map(|team| team.conversation_id.clone())
        .collect();
    if rooms.is_empty() {
        return Ok(());
    }
    let pictures: Vec<(String, String)> = conversation::Entity::find()
        .filter(conversation::Column::Id.is_in(rooms))
        .filter(conversation::Column::Image.is_not_null())
        .all(&state.db)
        .await?
        .into_iter()
        .filter_map(|room| Some((room.id, room.image?)))
        .collect();
    let pairs: Vec<(&str, &str)> = pictures
        .iter()
        .map(|(id, image)| (id.as_str(), image.as_str()))
        .collect();
    let urls = image_urls(state, &pairs).await;
    for team in teams.iter_mut() {
        team.image_url = team
            .conversation_id
            .as_ref()
            .and_then(|room| urls.get(room).cloned());
    }
    Ok(())
}

pub(crate) async fn team_view(
    state: &AppState,
    viewer: &str,
    team_id: &str,
) -> Result<Team, ApiError> {
    let team = load_team(&state.db, team_id).await?;
    let mut views = team_views(&state.db, viewer, &[team]).await?;
    sign_team_images(state, &mut views).await?;
    views
        .pop()
        .ok_or_else(|| ApiError::internal(format!("Team '{team_id}' vanished while reading it")))
}

fn level_of_org(
    levels: &HashMap<String, DataClassification>,
    org_id: Option<&String>,
) -> DataClassification {
    org_id
        .and_then(|id| levels.get(id))
        .copied()
        .unwrap_or(UNCLASSIFIED_PERSON)
}

fn chat_view(row: &bot_team_chat::Model) -> Option<BotChat> {
    Some(BotChat {
        bot_id: row.bot_id.clone(),
        user_id: row.user_id.clone(),
        owner_id: row.owner_id.clone(),
        status: BotChatStatus::parse(&row.status)?,
        conversation_id: row.conversation_id.clone(),
        note: row.note.clone(),
        requested_at: rfc3339(&row.requested_at),
        decided_at: row.decided_at.as_ref().map(rfc3339),
    })
}

/// Every org `user_id` is an active person in: its teams, bots and what waits for the person.
pub(crate) async fn org_view<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
) -> Result<OrgView, ApiError> {
    let now = now_millis();
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
    let roots = root_ids(db, &teams).await?;
    let team_ids: Vec<String> = teams.iter().map(|team| team.id.clone()).collect();
    let in_org = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::TeamId.is_in(team_ids))
        .all(db)
        .await?;
    let admin_teams: HashSet<&str> = rows
        .iter()
        .filter(|row| row.status == active() && TeamRole::parse(&row.role) == Some(TeamRole::Admin))
        .map(|row| row.team_id.as_str())
        .collect();
    let mut bot_ids: HashSet<String> = in_org.iter().map(|row| row.bot_id.clone()).collect();
    bot_ids.extend(teams.iter().filter_map(|team| team.lead_bot_id.clone()));
    let mut org_bots = live_bots(db, bot_ids).await?;
    let visible: HashSet<String> = org_bots
        .values()
        .filter(|bot| {
            bot.owner_id == user_id
                || in_org.iter().any(|row| {
                    row.bot_id == bot.id
                        && (row.status == active() || admin_teams.contains(row.team_id.as_str()))
                })
        })
        .map(|bot| bot.id.clone())
        .collect();
    org_bots.retain(|id, _| visible.contains(id));
    let mut inputs = inputs_of(db, &org_bots.values().collect::<Vec<_>>()).await?;
    let pins = visible_pin_rows(db, &ids_of(&teams)).await?;
    let (apps, mut bot_apps) = chart_apps(
        db,
        user_id,
        &pins,
        &admin_teams,
        &in_org,
        &org_bots,
        now,
    )
    .await?;
    let mut bots: Vec<OrgBot> = org_bots
        .values()
        .map(|row| OrgBot {
            inputs: inputs.remove(&row.id).unwrap_or_default(),
            app_ids: bot_apps.remove(&row.id).unwrap_or_default(),
            ..org_bot(row)
        })
        .collect();
    bots.sort_by(|a, b| a.id.cmp(&b.id));
    let pending_pins: Vec<&bot_team_app::Model> = pins
        .iter()
        .filter(|pin| pin.status == pending() && admin_teams.contains(pin.team_id.as_str()))
        .collect();
    let invitations =
        invitations_for(db, user_id, &rows, &own_teams, &names, &pending_pins).await?;
    let threads: Vec<TeamThread> = bot_team_thread::Entity::find()
        .filter(bot_team_thread::Column::UserId.eq(user_id))
        .filter(bot_team_thread::Column::TeamId.is_in(teams.iter().map(|team| team.id.clone())))
        .order_by_asc(bot_team_thread::Column::CreatedAt)
        .all(db)
        .await?
        .into_iter()
        .map(|row| TeamThread {
            team_id: row.team_id,
            conversation_id: row.conversation_id,
        })
        .collect();

    let admin_roots: Vec<String> = roots
        .values()
        .collect::<HashSet<_>>()
        .into_iter()
        .filter(|root| admin_teams.contains(root.as_str()))
        .cloned()
        .collect();
    let all_roots: Vec<String> = roots
        .values()
        .collect::<HashSet<_>>()
        .into_iter()
        .cloned()
        .collect();
    let class_rows = bot_org_classification::Entity::find()
        .filter(
            Condition::any()
                .add(
                    Condition::all()
                        .add(bot_org_classification::Column::UserId.eq(user_id))
                        .add(bot_org_classification::Column::OrgId.is_in(all_roots)),
                )
                .add(bot_org_classification::Column::OrgId.is_in(admin_roots)),
        )
        .all(db)
        .await?;
    let reader_levels: HashMap<String, DataClassification> = class_rows
        .iter()
        .filter(|row| row.user_id == user_id)
        .filter_map(|row| {
            Some((
                row.org_id.clone(),
                DataClassification::parse(&row.classification)?,
            ))
        })
        .collect();
    let mut classifications: Vec<PersonClassification> = class_rows
        .iter()
        .filter_map(|row| {
            Some(PersonClassification {
                org_id: row.org_id.clone(),
                user_id: row.user_id.clone(),
                classification: DataClassification::parse(&row.classification)?,
            })
        })
        .collect();
    classifications.sort_by(|a, b| (&a.org_id, &a.user_id).cmp(&(&b.org_id, &b.user_id)));

    let chat_rows = bot_team_chat::Entity::find()
        .filter(
            Condition::any()
                .add(bot_team_chat::Column::UserId.eq(user_id))
                .add(bot_team_chat::Column::OwnerId.eq(user_id)),
        )
        .filter(bot_team_chat::Column::Status.ne(BotChatStatus::Ended.as_str()))
        .order_by_asc(bot_team_chat::Column::RequestedAt)
        .order_by_asc(bot_team_chat::Column::BotId)
        .order_by_asc(bot_team_chat::Column::UserId)
        .all(db)
        .await?;
    let own_chats: HashMap<&str, &bot_team_chat::Model> = chat_rows
        .iter()
        .filter(|row| row.user_id == user_id)
        .map(|row| (row.bot_id.as_str(), row))
        .collect();
    let chats: Vec<BotChat> = chat_rows.iter().filter_map(chat_view).collect();

    let team_by_id: HashMap<&str, &bot_team::Model> =
        teams.iter().map(|team| (team.id.as_str(), team)).collect();
    let active_in: HashSet<&str> = rows
        .iter()
        .filter(|row| row.status == active())
        .map(|row| row.team_id.as_str())
        .collect();
    let thread_of: HashMap<&str, &str> = threads
        .iter()
        .map(|thread| (thread.team_id.as_str(), thread.conversation_id.as_str()))
        .collect();
    let reach = org_bots
        .values()
        .map(|bot| {
            let row = in_org
                .iter()
                .find(|row| row.bot_id == bot.id && row.status == active())
                .or_else(|| in_org.iter().find(|row| row.bot_id == bot.id));
            let team = row.and_then(|row| team_by_id.get(row.team_id.as_str()));
            let placement = Placement {
                active: row.is_some_and(|row| row.status == active()),
                is_lead: team.is_some_and(|team| team.lead_bot_id.as_deref() == Some(&bot.id)),
                reader_in_team: team.is_some_and(|team| active_in.contains(team.id.as_str())),
                team_room: team.and_then(|team| team.conversation_id.as_deref()),
                reader_thread: team.and_then(|team| thread_of.get(team.id.as_str()).copied()),
                reader_level: level_of_org(
                    &reader_levels,
                    team.and_then(|team| roots.get(&team.id)),
                ),
            };
            let chat = own_chats.get(bot.id.as_str()).and_then(|chat| {
                Some(ChatFacts {
                    status: BotChatStatus::parse(&chat.status)?,
                    conversation_id: chat.conversation_id.as_deref(),
                    decided_at: chat.decided_at,
                })
            });
            reach_of(
                &ReachFacts {
                    bot_id: &bot.id,
                    owned_by_reader: bot.owner_id == user_id,
                    bot_level: parse_level(bot),
                    placement,
                    chat,
                },
                now,
            )
        })
        .collect::<Vec<_>>();
    let mut reach = reach;
    reach.sort_by(|a, b| a.bot_id.cmp(&b.bot_id));
    Ok(OrgView {
        teams: team_views(db, user_id, &teams).await?,
        bots,
        invitations,
        threads,
        reach,
        chats,
        classifications,
        apps,
    })
}

/// The apps the chart shows for `user_id` and each active bot's app ids among them: active pins,
/// pending pins the reader may see, and the selected apps of active bots.
async fn chart_apps<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    pins: &[bot_team_app::Model],
    admin_teams: &HashSet<&str>,
    in_org: &[bot_team_bot::Model],
    org_bots: &HashMap<String, bot::Model>,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<(Vec<OrgApp>, HashMap<String, Vec<String>>), DbErr> {
    let active_bots: Vec<String> = in_org
        .iter()
        .filter(|row| row.status == active() && org_bots.contains_key(&row.bot_id))
        .map(|row| row.bot_id.clone())
        .collect();
    let selected = bot_app::Entity::find()
        .filter(bot_app::Column::BotId.is_in(active_bots))
        .all(db)
        .await?;
    let mut ids: HashSet<String> = pins
        .iter()
        .filter(|pin| {
            pin.status == active()
                || admin_teams.contains(pin.team_id.as_str())
                || pin.pinned_by == user_id
        })
        .map(|pin| pin.app_id.clone())
        .collect();
    ids.extend(selected.iter().map(|row| row.app_id.clone()));
    let ids: Vec<String> = ids.into_iter().collect();
    let cards = org_apps(db, user_id, &ids, now).await?;
    let shown: HashSet<&str> = cards.iter().map(|card| card.id.as_str()).collect();
    let mut by_bot: HashMap<String, Vec<String>> = HashMap::new();
    for row in selected {
        if shown.contains(row.app_id.as_str()) {
            by_bot.entry(row.bot_id).or_default().push(row.app_id);
        }
    }
    by_bot.values_mut().for_each(|ids| ids.sort());
    Ok((cards, by_bot))
}

async fn org_name_of<C: ConnectionTrait>(
    db: &C,
    names: &mut HashMap<String, String>,
    team: &bot_team::Model,
) -> Result<String, DbErr> {
    if let Some(name) = names.get(&team.id) {
        return Ok(name.clone());
    }
    let name = org_of(db, team).await?.root.name;
    names.insert(team.id.clone(), name.clone());
    Ok(name)
}

async fn invitations_for<C: ConnectionTrait>(
    db: &C,
    user_id: &str,
    person_rows: &[bot_team_person::Model],
    own_teams: &[bot_team::Model],
    root_names: &HashMap<String, String>,
    pending_pins: &[&bot_team_app::Model],
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
            user_id: None,
            note: None,
            app_id: None,
            created_at: rfc3339(&row.created_at),
        });
    }
    for pin in pending_pins {
        let Some(team) = own_teams.iter().find(|team| team.id == pin.team_id) else {
            continue;
        };
        invitations.push(TeamInvitation {
            kind: TeamInvitationKind::AppPin,
            team_id: team.id.clone(),
            team_name: team.name.clone(),
            org_name: root_names.get(&team.id).cloned().unwrap_or_default(),
            bot_id: None,
            invited_by: Some(pin.pinned_by.clone()),
            user_id: Some(pin.pinned_by.clone()),
            note: None,
            app_id: Some(pin.app_id.clone()),
            created_at: rfc3339(&pin.created_at),
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
        .filter(bot_team_bot::Column::BotId.is_in(owned.clone()))
        .all(db)
        .await?;
    let offered = bot_team::Entity::find()
        .filter(bot_team::Column::LeadOfferBotId.is_in(owned.clone()))
        .all(db)
        .await?;
    let mut teams: HashMap<String, bot_team::Model> = bot_team::Entity::find()
        .filter(bot_team::Column::Id.is_in(bot_rows.iter().map(|row| row.team_id.clone())))
        .all(db)
        .await?
        .into_iter()
        .map(|team| (team.id.clone(), team))
        .collect();
    let mut org_names: HashMap<String, String> = root_names.clone();
    let chats = bot_team_chat::Entity::find()
        .filter(bot_team_chat::Column::OwnerId.eq(user_id))
        .filter(bot_team_chat::Column::Status.eq(BotChatStatus::Pending.as_str()))
        .order_by_asc(bot_team_chat::Column::RequestedAt)
        .all(db)
        .await?;
    for team in &offered {
        teams.entry(team.id.clone()).or_insert_with(|| team.clone());
    }
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
        invitations.push(TeamInvitation {
            kind,
            team_id: team.id.clone(),
            team_name: team.name.clone(),
            org_name: org_name_of(db, &mut org_names, team).await?,
            bot_id: Some(row.bot_id.clone()),
            invited_by: row.invited_by.clone(),
            user_id: None,
            note: None,
            app_id: None,
            created_at: rfc3339(created_at),
        });
    }
    for team in &offered {
        let (Some(bot_id), Some(at)) =
            (team.lead_offer_bot_id.as_ref(), team.lead_offer_at.as_ref())
        else {
            continue;
        };
        invitations.push(TeamInvitation {
            kind: TeamInvitationKind::Lead,
            team_id: team.id.clone(),
            team_name: team.name.clone(),
            org_name: org_name_of(db, &mut org_names, team).await?,
            bot_id: Some(bot_id.clone()),
            invited_by: team.lead_offer_by.clone(),
            user_id: None,
            note: None,
            app_id: None,
            created_at: rfc3339(at),
        });
    }
    for chat in &chats {
        let Some(row) = bot_rows
            .iter()
            .find(|row| row.bot_id == chat.bot_id && row.status == active())
        else {
            continue;
        };
        let Some(team) = teams.get(&row.team_id) else {
            continue;
        };
        invitations.push(TeamInvitation {
            kind: TeamInvitationKind::ChatRequest,
            team_id: team.id.clone(),
            team_name: team.name.clone(),
            org_name: org_name_of(db, &mut org_names, team).await?,
            bot_id: Some(chat.bot_id.clone()),
            invited_by: None,
            user_id: Some(chat.user_id.clone()),
            note: chat.note.clone(),
            app_id: None,
            created_at: rfc3339(&chat.requested_at),
        });
    }
    Ok(invitations)
}

//! A person's thread with a team: the conversation their `#Team` requests land in. Its members
//! are always the person, the team's current lead bot and the lead's owner; this module keeps
//! them that way. Callers hold the thread's lock ([`crate::routes::inbox::team_room::lock_room`]).

use super::{
    access::{active, is_active_in, org_of, visible_team},
    effects::{Effects, settle},
    levels::person_level,
    room::{bot, person, room_limits},
};
use crate::{
    bot_notify::display_name,
    entity::{bot as bot_entity, bot_team, bot_team_bot, bot_team_thread},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::inbox::{
        active_members,
        members::Principal,
        now_millis,
        person_push::mark_watching,
        team_request::{ensure_thread, lock_request},
        team_room::{RoomJoin, join_room, leave_room},
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
    bot_team::{TeamThread, person_may_talk},
    data_classification::DataClassification,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

pub(crate) const TEAM_NO_LEAD: &str = "TEAM_NO_LEAD";
pub(crate) const TEAM_CLASSIFIED: &str = "TEAM_CLASSIFIED";

pub(crate) fn classified(bot_name: &str, bot_level: Option<DataClassification>) -> ApiError {
    let level = bot_level.map_or("internal", |level| level.as_str());
    ApiError::coded(
        StatusCode::FORBIDDEN,
        TEAM_CLASSIFIED,
        format!(
            "'{bot_name}' is classified {level}, above your classification in this organization"
        ),
    )
}

/// The lead bot of a team as a thread seats it.
pub(crate) struct LeadSeat {
    pub bot: bot_entity::Model,
    pub limits: Option<flow_like_types::Value>,
}

/// The team's lead when it is an active bot of the team and alive.
pub(crate) async fn lead_seat<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
) -> Result<Option<LeadSeat>, ApiError> {
    let Some(lead_id) = team.lead_bot_id.as_deref() else {
        return Ok(None);
    };
    let row = bot_team_bot::Entity::find_by_id((team.id.clone(), lead_id.to_string()))
        .filter(bot_team_bot::Column::Status.eq(active()))
        .one(db)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let lead = bot_entity::Entity::find_by_id(lead_id)
        .filter(bot_entity::Column::DeletedAt.is_null())
        .one(db)
        .await?;
    Ok(lead.map(|bot| LeadSeat {
        limits: room_limits(row.limits.as_ref(), true),
        bot,
    }))
}

pub(crate) async fn thread_of<C: ConnectionTrait>(
    db: &C,
    team_id: &str,
    user_id: &str,
) -> Result<Option<bot_team_thread::Model>, sea_orm::DbErr> {
    bot_team_thread::Entity::find_by_id((team_id.to_string(), user_id.to_string()))
        .one(db)
        .await
}

/// Makes `seats` the members of the conversation; `watchers` among them only watch.
pub(crate) async fn seat_members<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    seats: Vec<RoomJoin>,
    watchers: &[String],
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    effects.wake(join_room(db, conversation_id, by, &seats, now).await?);
    for watcher in watchers {
        if mark_watching(db, conversation_id, watcher).await? {
            effects.wake(vec![watcher.clone()]);
        }
    }
    let current = active_members(db, &[conversation_id.to_string()]).await?;
    let leaving: Vec<Principal> = current
        .iter()
        .map(Principal::from)
        .filter(|member| !seats.iter().any(|seat| seat.principal.id == member.id))
        .collect();
    effects.wake(leave_room(db, conversation_id, by, &leaving, now).await?);
    Ok(())
}

/// Makes the thread's members the person, the team's lead and the lead's owner, who only
/// watches. A person who is
/// no longer an active person of the team's org keeps the thread alone, with its history.
pub(crate) async fn sync_thread<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    thread: &bot_team_thread::Model,
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    let mut seats = vec![RoomJoin {
        principal: person(&thread.user_id),
        limits: None,
    }];
    let mut watchers = Vec::new();
    let org = org_of(db, team).await?;
    if is_active_in(db, &org.ids(), &thread.user_id).await?
        && let Some(lead) = lead_seat(db, team).await?
    {
        if lead.bot.owner_id != thread.user_id {
            seats.push(RoomJoin {
                principal: person(&lead.bot.owner_id),
                limits: None,
            });
            watchers.push(lead.bot.owner_id.clone());
        }
        seats.push(RoomJoin {
            principal: bot(&lead.bot.id),
            limits: lead.limits,
        });
    }
    seat_members(
        db,
        &thread.conversation_id,
        seats,
        &watchers,
        by,
        now,
        effects,
    )
    .await
}

/// [`sync_thread`] for every thread with the team.
pub(crate) async fn sync_team_threads<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    let threads = bot_team_thread::Entity::find()
        .filter(bot_team_thread::Column::TeamId.eq(&team.id))
        .all(db)
        .await?;
    for thread in &threads {
        sync_thread(db, team, thread, by, now, effects).await?;
    }
    Ok(())
}

/// [`sync_thread`] for every thread of `teams`.
pub(crate) async fn sync_org_threads<C: ConnectionTrait>(
    db: &C,
    teams: &[bot_team::Model],
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    for team in teams {
        sync_team_threads(db, team, by, now, effects).await?;
    }
    Ok(())
}

/// Takes the lead and its owner out of every thread with the team, which is about to go; the
/// people keep the history.
pub(crate) async fn release_team_threads<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    let threads = bot_team_thread::Entity::find()
        .filter(bot_team_thread::Column::TeamId.eq(&team.id))
        .all(db)
        .await?;
    for thread in &threads {
        let seats = vec![RoomJoin {
            principal: person(&thread.user_id),
            limits: None,
        }];
        seat_members(db, &thread.conversation_id, seats, &[], by, now, effects).await?;
    }
    bot_team_thread::Entity::delete_many()
        .filter(bot_team_thread::Column::TeamId.eq(&team.id))
        .exec(db)
        .await?;
    Ok(())
}

/// Opens the person's thread with the team's lead, or returns the one they have.
pub(crate) async fn open_thread_in<C: ConnectionTrait>(
    db: &C,
    sub: &str,
    team_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<(TeamThread, Effects), ApiError> {
    let (team, org) = visible_team(db, sub, team_id).await?;
    let Some(lead) = lead_seat(db, &team).await? else {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            TEAM_NO_LEAD,
            format!("Team '{}' has no lead to talk to", team.name),
        ));
    };
    if lead.bot.owner_id != sub {
        let level = person_level(db, &org.root.id, sub).await?;
        let lead_level = lead
            .bot
            .classification
            .as_deref()
            .and_then(DataClassification::parse);
        if !person_may_talk(Some(level), lead_level) {
            return Err(classified(&display_name(&lead.bot), lead_level));
        }
    }
    let mut effects = Effects::default();
    let thread = ensure_thread(db, &team, sub, now, &mut effects).await?;
    sync_thread(db, &team, &thread, sub, now, &mut effects).await?;
    Ok((
        TeamThread {
            team_id: thread.team_id,
            conversation_id: thread.conversation_id,
        },
        effects,
    ))
}

#[utoipa::path(
    post,
    path = "/bot-teams/{team_id}/thread",
    tag = "Bot teams",
    params(("team_id" = String, Path, description = "The team to talk to")),
    responses(
        (status = 200, description = "Your thread with the team's lead, created on first use. The lead answers there, and the lead's owner watches without being pinged.", body = Object),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "TEAM_CLASSIFIED: the lead is classified above you in this organization"),
        (status = 404, description = "TEAM_NOT_FOUND: the team does not exist or you can't see it"),
        (status = 409, description = "TEAM_NO_LEAD: the team has no lead to talk to")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bot-teams/{team_id}/thread", skip(state, user))]
pub async fn open_thread(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(team_id): Path<String>,
) -> Result<Json<TeamThread>, ApiError> {
    let sub = user.sub()?;
    let (thread, effects) = state
        .transaction(|txn| {
            let (sub, team_id) = (sub.clone(), team_id.clone());
            Box::pin(async move {
                lock_request(txn, &sub, &team_id).await?;
                open_thread_in(txn, &sub, &team_id, now_millis()).await
            })
        })
        .await?;
    settle(&state, effects).await;
    Ok(Json(thread))
}

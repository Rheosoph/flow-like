//! A person's thread with a team: the conversation their `#Team` requests land in. Its members
//! are always the person, the team's current lead bot and the lead's owner; this module keeps
//! them that way. Callers hold the thread's lock ([`crate::routes::inbox::team_room::lock_room`]).

use super::{
    access::{active, is_active_in, org_of},
    effects::Effects,
    room::{bot, person, room_limits},
};
use crate::{
    entity::{bot as bot_entity, bot_team, bot_team_bot, bot_team_thread},
    error::ApiError,
    routes::inbox::{
        active_members,
        members::Principal,
        team_room::{RoomJoin, join_room, leave_room},
    },
};
use chrono::{DateTime, FixedOffset};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

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

async fn seat_members<C: ConnectionTrait>(
    db: &C,
    thread: &bot_team_thread::Model,
    seats: Vec<RoomJoin>,
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    effects.wake(join_room(db, &thread.conversation_id, by, &seats, now).await?);
    let current = active_members(db, std::slice::from_ref(&thread.conversation_id)).await?;
    let leaving: Vec<Principal> = current
        .iter()
        .map(Principal::from)
        .filter(|member| !seats.iter().any(|seat| seat.principal.id == member.id))
        .collect();
    effects.wake(leave_room(db, &thread.conversation_id, by, &leaving, now).await?);
    Ok(())
}

/// Makes the thread's members the person, the team's lead and the lead's owner. A person who is
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
    let org = org_of(db, team).await?;
    if is_active_in(db, &org.ids(), &thread.user_id).await?
        && let Some(lead) = lead_seat(db, team).await?
    {
        if lead.bot.owner_id != thread.user_id {
            seats.push(RoomJoin {
                principal: person(&lead.bot.owner_id),
                limits: None,
            });
        }
        seats.push(RoomJoin {
            principal: bot(&lead.bot.id),
            limits: lead.limits,
        });
    }
    seat_members(db, thread, seats, by, now, effects).await
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
        seat_members(db, thread, seats, by, now, effects).await?;
    }
    bot_team_thread::Entity::delete_many()
        .filter(bot_team_thread::Column::TeamId.eq(&team.id))
        .exec(db)
        .await?;
    Ok(())
}

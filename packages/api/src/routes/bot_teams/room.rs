//! Keeps a team's room in step with its tables: active people and active bots are members, the
//! lead reads along and every other bot answers when named.

use super::{access::active, effects::Effects};
use crate::{
    entity::{bot_team, bot_team_bot, bot_team_person},
    error::ApiError,
    routes::inbox::{
        members::Principal,
        team_room::{RoomJoin, join_room, leave_room},
    },
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{bot_effective::BotRoomLimits, inbox::PrincipalKind};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

pub(crate) fn person(id: &str) -> Principal {
    Principal {
        id: id.to_string(),
        kind: PrincipalKind::User,
    }
}

pub(crate) fn bot(id: &str) -> Principal {
    Principal {
        id: id.to_string(),
        kind: PrincipalKind::Bot,
    }
}

/// The room limits of a bot: what its owner stored plus whether it reads along.
pub(crate) fn room_limits(
    stored: Option<&flow_like_types::Value>,
    lead: bool,
) -> Option<flow_like_types::Value> {
    let mut limits: BotRoomLimits = stored
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default();
    limits.mentioned_only = !lead;
    serde_json::to_value(limits).ok()
}

/// Adds every active person and bot of the team to its room and refreshes the bots' limits.
pub(crate) async fn sync_room<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    by: &str,
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    let Some(conversation_id) = team.conversation_id.as_deref() else {
        return Ok(());
    };
    let people = bot_team_person::Entity::find()
        .filter(bot_team_person::Column::TeamId.eq(&team.id))
        .filter(bot_team_person::Column::Status.eq(active()))
        .all(db)
        .await?;
    let bots = bot_team_bot::Entity::find()
        .filter(bot_team_bot::Column::TeamId.eq(&team.id))
        .filter(bot_team_bot::Column::Status.eq(active()))
        .all(db)
        .await?;
    let mut joining: Vec<RoomJoin> = people
        .iter()
        .map(|row| RoomJoin {
            principal: person(&row.user_id),
            limits: None,
        })
        .collect();
    joining.extend(bots.iter().map(|row| RoomJoin {
        principal: bot(&row.bot_id),
        limits: room_limits(
            row.limits.as_ref(),
            team.lead_bot_id.as_deref() == Some(row.bot_id.as_str()),
        ),
    }));
    effects.wake(join_room(db, conversation_id, by, &joining, now).await?);
    Ok(())
}

/// Removes `leaving` from the team's room.
pub(crate) async fn leave<C: ConnectionTrait>(
    db: &C,
    team: &bot_team::Model,
    by: &str,
    leaving: &[Principal],
    now: DateTime<FixedOffset>,
    effects: &mut Effects,
) -> Result<(), ApiError> {
    if let Some(conversation_id) = team.conversation_id.as_deref() {
        effects.wake(leave_room(db, conversation_id, by, leaving, now).await?);
    }
    Ok(())
}

//! Place tasks: the owner's phones watch a circle and report crossings, and the hub fires the task
//! into the owner's chat with the bot. A crossing claims the run with a compare-and-set on
//! `lastRunAt` that encodes [`TaskPlace::claimable`], so two phones that report the same arrival
//! fire once. Time sweeps never see a place task: its `nextRunAt` is NULL.

use super::tasks::trigger_for;
use crate::{
    entity::{bot, bot_task},
    error::ApiError,
    routes::inbox::{
        delivery::BotRecipient,
        triggers::{lock_trigger_conversation, trigger_conversation, write_trigger_in},
    },
};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use flow_like_types::{
    bot::BotCapability,
    bot_task::{
        MAX_WATCHED_PLACES, MIN_PLACE_REFIRE_SECS, PlaceCrossing, STORED_PLACE, TaskPlace,
        WatchedPlace, WatchedPlaceList, fire_message_id,
    },
    inbox::TRIGGER_TEXT_MAX_CHARS,
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DatabaseTransaction, EntityTrait, QueryFilter,
    QueryTrait,
    sea_query::{Expr, ExprTrait},
};

pub(crate) fn holds_location(bot: &bot::Model) -> bool {
    bot.capabilities
        .iter()
        .any(|id| id == BotCapability::Location.as_str())
}

/// The place a place task row watches; `None` for a time task or a row without a full place.
pub(crate) fn place_of(task: &bot_task::Model) -> Option<TaskPlace> {
    Some(TaskPlace {
        latitude: task.place_latitude?,
        longitude: task.place_longitude?,
        radius_m: task.place_radius_m?,
        on: PlaceCrossing::parse(task.place_on.as_deref()?)?,
        label: task.place_label.clone()?,
        repeat: task.place_repeat.unwrap_or(false),
        until: task.place_until.map(|until| until.to_rfc3339()),
    })
}

fn location_bots(owner_id: &str) -> sea_orm::sea_query::SelectStatement {
    use sea_orm::QuerySelect;
    bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::DeletedAt.is_null())
        .filter(
            Expr::col(bot::Column::Capabilities)
                .cast_as("text")
                .like(format!("%\"{}\"%", BotCapability::Location.as_str())),
        )
        .into_query()
}

/// The owner's watched place tasks across their live bots holding `location`, with the places:
/// soonest `until` first (none last), then newest. One indexed query.
pub(crate) async fn watched_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    now: DateTime<Utc>,
) -> Result<Vec<(bot_task::Model, TaskPlace)>, ApiError> {
    let rows = bot_task::Entity::find()
        .filter(bot_task::Column::OwnerId.eq(owner_id))
        .filter(bot_task::Column::Kind.eq(STORED_PLACE))
        .filter(bot_task::Column::Enabled.eq(true))
        .filter(bot_task::Column::BotId.in_subquery(location_bots(owner_id)))
        .all(db)
        .await?;
    let mut watched: Vec<(bot_task::Model, TaskPlace)> = rows
        .into_iter()
        .filter_map(|row| {
            let place = place_of(&row)?;
            place
                .watched(row.enabled, row.last_run_at.map(|at| at.to_utc()), now)
                .then_some((row, place))
        })
        .collect();
    watched.sort_by(|(a, pa), (b, pb)| {
        let until = |place: &TaskPlace| {
            place
                .until
                .as_deref()
                .and_then(|until| DateTime::parse_from_rfc3339(until).ok())
        };
        match (until(pa), until(pb)) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then(b.created_at.cmp(&a.created_at))
        .then(a.id.cmp(&b.id))
    });
    Ok(watched)
}

/// The owner's live bot and its place task; a missing bot, a task of another owner or bot, and a
/// time task all read as missing.
pub(crate) async fn place_task_of<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    task_id: &str,
) -> Result<(bot::Model, bot_task::Model), ApiError> {
    let bot = crate::routes::bots::owned_bot(db, owner_id, bot_id).await?;
    let task = bot_task::Entity::find_by_id(task_id)
        .filter(bot_task::Column::BotId.eq(bot_id))
        .filter(bot_task::Column::OwnerId.eq(owner_id))
        .filter(bot_task::Column::Kind.eq(STORED_PLACE))
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Place task '{task_id}' not found")))?;
    Ok((bot, task))
}

/// What the owner's phones read: at most [`MAX_WATCHED_PLACES`] watched places.
pub(crate) async fn watched_list_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    now: DateTime<Utc>,
) -> Result<WatchedPlaceList, ApiError> {
    let places = watched_in(db, owner_id, now)
        .await?
        .into_iter()
        .take(MAX_WATCHED_PLACES)
        .map(|(row, place)| WatchedPlace {
            task_id: row.id,
            bot_id: row.bot_id,
            latitude: place.latitude,
            longitude: place.longitude,
            radius_m: place.radius_m,
            on: place.on,
            until: place.until,
        })
        .collect();
    Ok(WatchedPlaceList { places })
}

/// Why a crossing did not fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dropped {
    Stale,
    Direction,
    NotWatched,
    Duplicate,
}

impl Dropped {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::Stale => "stale",
            Self::Direction => "direction",
            Self::NotWatched => "not_watched",
            Self::Duplicate => "duplicate",
        }
    }
}

pub(crate) enum Crossed {
    Sent(Vec<String>, Vec<BotRecipient>),
    Dropped(Dropped),
}

fn claimable(task: &bot_task::Model, occurred_at: DateTime<Utc>) -> Condition {
    let unfired = bot_task::Column::LastRunAt.is_null();
    if task.place_repeat.unwrap_or(false) {
        let gap = occurred_at - Duration::seconds(MIN_PLACE_REFIRE_SECS);
        Condition::any()
            .add(unfired)
            .add(bot_task::Column::LastRunAt.lte(gap.fixed_offset()))
    } else {
        Condition::all().add(unfired)
    }
}

async fn claim_crossing_in<C: ConnectionTrait>(
    db: &C,
    task: &bot_task::Model,
    occurred_at: DateTime<Utc>,
    now: DateTime<FixedOffset>,
) -> Result<bool, ApiError> {
    let claimed = bot_task::Entity::update_many()
        .col_expr(
            bot_task::Column::LastRunAt,
            Expr::value(occurred_at.fixed_offset()),
        )
        .col_expr(bot_task::Column::UpdatedAt, Expr::value(now))
        .filter(bot_task::Column::Id.eq(task.id.as_str()))
        .filter(bot_task::Column::Kind.eq(STORED_PLACE))
        .filter(bot_task::Column::Enabled.eq(true))
        .filter(claimable(task, occurred_at))
        .exec(db)
        .await?;
    Ok(claimed.rows_affected == 1)
}

/// [`fire_crossing_locked_in`] in the owner's chat with the bot, which an end-to-end encrypted
/// chat the server cannot write into makes a drop. Call inside a transaction.
pub(crate) async fn fire_crossing_in(
    txn: &DatabaseTransaction,
    task: &bot_task::Model,
    bot: &bot::Model,
    crossing: PlaceCrossing,
    occurred_at: DateTime<Utc>,
    now: DateTime<FixedOffset>,
) -> Result<Crossed, ApiError> {
    let Some(conversation_id) =
        trigger_conversation(txn, &task.owner_id, &task.bot_id, now).await?
    else {
        return Ok(Crossed::Dropped(Dropped::NotWatched));
    };
    lock_trigger_conversation(txn, &conversation_id).await?;
    fire_crossing_locked_in(txn, &conversation_id, task, bot, crossing, occurred_at, now).await
}

/// Fires `task`, a place task of `bot`'s owner, for `crossing` seen at `occurred_at`: drops it
/// when it does not match the direction, is not watched or lost the claim, else writes the
/// trigger into `conversation_id`, whose lock the caller holds.
pub(crate) async fn fire_crossing_locked_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    task: &bot_task::Model,
    bot: &bot::Model,
    crossing: PlaceCrossing,
    occurred_at: DateTime<Utc>,
    now: DateTime<FixedOffset>,
) -> Result<Crossed, ApiError> {
    let Some(place) = place_of(task) else {
        return Ok(Crossed::Dropped(Dropped::NotWatched));
    };
    if !place.fires_on(crossing) {
        return Ok(Crossed::Dropped(Dropped::Direction));
    }
    let watched = bot.deleted_at.is_none()
        && holds_location(bot)
        && place.watched(
            task.enabled,
            task.last_run_at.map(|at| at.to_utc()),
            now.with_timezone(&Utc),
        );
    if !watched {
        return Ok(Crossed::Dropped(Dropped::NotWatched));
    }
    if !claim_crossing_in(db, task, occurred_at, now).await? {
        return Ok(Crossed::Dropped(Dropped::Duplicate));
    }
    let mut body = trigger_for(task);
    body.text = format!("{}\n\n{}", place.fire_note(crossing), task.instruction)
        .chars()
        .take(TRIGGER_TEXT_MAX_CHARS)
        .collect();
    let sent = write_trigger_in(
        db,
        conversation_id,
        &task.owner_id,
        &fire_message_id(&task.id, occurred_at),
        &body,
        now,
    )
    .await?;
    Ok(Crossed::Sent(sent.wake, sent.bots))
}

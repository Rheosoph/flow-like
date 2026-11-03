//! The daily cap on turns bots start for each other. A message from a person (hops 0) always
//! queues its turns; a message from a bot (hops above 0) queues them while the owner's bots have
//! started fewer than their plan allows in the last 24 hours. Turns over the cap aren't queued
//! and the owner gets one notification a day. The count is read inside the sending transaction
//! without a lock, so concurrent sends can overshoot the cap by a few turns.

use super::ledger::Waking;
use crate::{
    capacity,
    entity::{bot_turn, notification, sea_orm_active_enums::NotificationType},
    error::ApiError,
    routes::inbox::{approvals::post_notice_in, delivery::BotRecipient},
};
use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::bot_turn::rate_limit_message_id;
use sea_orm::{
    ActiveEnum, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, sea_query::OnConflict,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const WINDOW_HOURS: i64 = 24;
/// Turns one person or bot who isn't the owner may start on one bot in a day.
pub(crate) const REQUESTER_DAILY_TURNS: u64 = 20;

struct Allowance {
    left: u64,
    limit: u64,
    plan: String,
    exhausted: bool,
}

async fn allowance<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Option<Allowance>, ApiError> {
    let plan = capacity::plan_of(db, owner_id).await?;
    let Ok(limit) = u64::try_from(capacity::max_bot_started_turns_per_day(&plan)) else {
        return Ok(None);
    };
    let started = bot_turn::Entity::find()
        .filter(bot_turn::Column::OwnerId.eq(owner_id))
        .filter(bot_turn::Column::Hops.gt(0))
        .filter(bot_turn::Column::CreatedAt.gt(now - Duration::hours(WINDOW_HOURS)))
        .count(db)
        .await?;
    Ok(Some(Allowance {
        left: limit.saturating_sub(started),
        limit,
        plan: plan.to_value(),
        exhausted: false,
    }))
}

/// The recipients whose turns may queue; the owners that hit the cap are notified.
pub(crate) async fn admit<'a, C: ConnectionTrait>(
    db: &C,
    recipients: Vec<&'a BotRecipient>,
    now: DateTime<FixedOffset>,
) -> Result<Vec<&'a BotRecipient>, ApiError> {
    if !crate::quota::enforcing() {
        return Ok(recipients);
    }
    let owners: BTreeSet<&str> = recipients
        .iter()
        .filter(|recipient| recipient.hops > 0)
        .map(|recipient| recipient.owner_id.as_str())
        .collect();
    if owners.is_empty() {
        return Ok(recipients);
    }
    let mut allowances = BTreeMap::new();
    for owner in owners {
        if let Some(found) = allowance(db, owner, now).await? {
            allowances.insert(owner, found);
        }
    }
    let admitted = recipients
        .into_iter()
        .filter(|recipient| {
            if recipient.hops == 0 {
                return true;
            }
            let Some(room) = allowances.get_mut(recipient.owner_id.as_str()) else {
                return true;
            };
            if room.left > 0 {
                room.left -= 1;
                true
            } else {
                room.exhausted = true;
                false
            }
        })
        .collect();
    for (owner, room) in allowances.iter().filter(|(_, room)| room.exhausted) {
        notify(db, owner, room, now).await?;
    }
    Ok(admitted)
}

async fn notify<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    room: &Allowance,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    tracing::info!(
        owner_id,
        limit = room.limit,
        "Bot-started turns hit the daily cap"
    );
    notification::Entity::insert(notification::ActiveModel {
        id: Set(format!("botcap_{owner_id}_{}", now.format("%Y%m%d"))),
        user_id: Set(owner_id.to_string()),
        app_id: Set(None),
        title: Set("Your bots paused their conversation".to_string()),
        description: Set(Some(format!(
            "Your bots started {} turns for each other in the last day, the limit of the {} plan. Their further messages wait until earlier turns are a day old. Messages from you still get answers.",
            room.limit, room.plan
        ))),
        icon: Set(None),
        link: Set(None),
        r#type: Set(NotificationType::System),
        read: Set(false),
        source_run_id: Set(None),
        source_node_id: Set(None),
        created_at: Set(now),
        read_at: Set(None),
    })
    .on_conflict(
        OnConflict::column(notification::Column::Id)
            .do_nothing()
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(())
}

async fn requester_turns<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    requester_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<u64, ApiError> {
    Ok(bot_turn::Entity::find()
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .filter(bot_turn::Column::RequesterId.eq(requester_id))
        .filter(bot_turn::Column::CreatedAt.gt(now - Duration::hours(WINDOW_HOURS)))
        .count(db)
        .await?)
}

/// The turns whose requester, when not the owner, has started fewer than
/// [`REQUESTER_DAILY_TURNS`] on the bot in the last 24 hours. A refused turn isn't queued and
/// the bot says so in the room. The owner's own messages never count.
pub(crate) async fn admit_requesters<'a, C: ConnectionTrait>(
    db: &C,
    waking: Vec<Waking<'a>>,
    now: DateTime<FixedOffset>,
) -> Result<Vec<Waking<'a>>, ApiError> {
    let mut admitted = Vec::with_capacity(waking.len());
    for item in waking {
        let recipient = item.recipient;
        if item.requester_id == recipient.owner_id
            || requester_turns(db, &recipient.bot_id, &item.requester_id, now).await?
                < REQUESTER_DAILY_TURNS
        {
            admitted.push(item);
            continue;
        }
        tracing::info!(
            bot_id = recipient.bot_id,
            requester_id = item.requester_id,
            "A requester hit the daily turn limit of a bot"
        );
        Box::pin(post_notice_in(
            db,
            &recipient.conversation_id,
            &recipient.bot_id,
            &rate_limit_message_id(&recipient.bot_id, &recipient.trigger_message_id),
            "I can only answer so many requests a day from one person, and that limit is used up. Please try again tomorrow, or ask my owner.",
            now,
        ))
        .await?;
    }
    Ok(admitted)
}

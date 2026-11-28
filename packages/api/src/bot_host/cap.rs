//! The daily cap on turns bots start for each other. A message from a person (hops 0) always
//! queues its turns; a message from a bot (hops above 0) queues them while the owner's bots have
//! started fewer than their plan allows in the last 24 hours. Turns over the cap aren't queued
//! and the owner gets one notification a day. The count is read inside the sending transaction
//! without a lock, so concurrent sends can overshoot the cap by a few turns.

use super::ledger::Waking;
use crate::{
    bot_notify::display_name,
    capacity,
    entity::{bot, bot_turn, notification, sea_orm_active_enums::NotificationType},
    error::ApiError,
    routes::inbox::{approvals::post_notice_in, delivery::BotRecipient},
};
use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::bot_turn::rate_limit_message_id;
use sea_orm::{
    ActiveEnum, ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait, EntityTrait,
    PaginatorTrait, QueryFilter, sea_query::OnConflict,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(crate) const WINDOW_HOURS: i64 = 24;
/// Turns one person who isn't the owner may start on one bot in a day, directly or through the
/// bots their message set talking.
pub(crate) const REQUESTER_DAILY_TURNS: u64 = 20;
/// Turns all people other than its owner may start on one bot in a day together, unless the
/// owner chose another number (`Bot.othersDailyTurns`).
pub(crate) const DEFAULT_OTHERS_DAILY_TURNS: u32 = 100;
/// The most an owner may allow other people in a day.
pub(crate) const MAX_OTHERS_DAILY_TURNS: u32 = 5_000;

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

/// The turns of `bot_id` that `origin_id` started in the last day: their own messages and the
/// bot chains those set off; a turn queued before origins were recorded counts for its requester.
/// Prompted turns only: a direct message, mention or reply. Read-along and reaction turns don't count.
async fn origin_turns<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    origin_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<u64, ApiError> {
    Ok(bot_turn::Entity::find()
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .filter(
            Condition::any()
                .add(bot_turn::Column::OriginId.eq(origin_id))
                .add(
                    Condition::all()
                        .add(bot_turn::Column::OriginId.is_null())
                        .add(bot_turn::Column::RequesterId.eq(origin_id)),
                ),
        )
        .filter(
            Condition::any()
                .add(bot_turn::Column::Unprompted.is_null())
                .add(bot_turn::Column::Unprompted.eq(false)),
        )
        .filter(bot_turn::Column::CreatedAt.gt(now - Duration::hours(WINDOW_HOURS)))
        .count(db)
        .await?)
}

/// The prompted turns of `bot_id` that people other than `owner_id` started in the last day,
/// counted like [`origin_turns`].
async fn others_turns<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    owner_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<u64, ApiError> {
    Ok(bot_turn::Entity::find()
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .filter(
            Condition::any()
                .add(
                    Condition::all()
                        .add(bot_turn::Column::OriginId.is_not_null())
                        .add(bot_turn::Column::OriginId.ne(owner_id)),
                )
                .add(
                    Condition::all()
                        .add(bot_turn::Column::OriginId.is_null())
                        .add(bot_turn::Column::RequesterId.ne(owner_id)),
                ),
        )
        .filter(
            Condition::any()
                .add(bot_turn::Column::Unprompted.is_null())
                .add(bot_turn::Column::Unprompted.eq(false)),
        )
        .filter(bot_turn::Column::CreatedAt.gt(now - Duration::hours(WINDOW_HOURS)))
        .count(db)
        .await?)
}

/// The bots of `waking` that people other than their owner woke: how many turns other people
/// may start on each in a day, and the name its owner knows it by.
async fn others_allowances<C: ConnectionTrait>(
    db: &C,
    waking: &[Waking<'_>],
) -> Result<HashMap<String, (u64, String)>, ApiError> {
    let ids: BTreeSet<String> = waking
        .iter()
        .filter(|item| item.origin_id != item.recipient.owner_id)
        .map(|item| item.recipient.bot_id.clone())
        .collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(bot::Entity::find()
        .filter(bot::Column::Id.is_in(ids))
        .all(db)
        .await?
        .into_iter()
        .map(|row| {
            let limit = row
                .others_daily_turns
                .and_then(|turns| u64::try_from(turns).ok())
                .unwrap_or(u64::from(DEFAULT_OTHERS_DAILY_TURNS));
            (row.id.clone(), (limit, display_name(&row)))
        })
        .collect())
}

/// Why a turn for someone other than the bot's owner doesn't queue.
enum Refusal {
    /// This person used up their [`REQUESTER_DAILY_TURNS`] on the bot.
    Person,
    /// Everyone but the owner together used up the bot's allowance.
    Everyone { limit: u64, bot_name: String },
}

impl Refusal {
    fn notice(&self) -> &'static str {
        match self {
            Self::Person => {
                "I can only answer so many requests a day from one person, and that limit is used up. Please try again tomorrow, or ask my owner."
            }
            Self::Everyone { .. } => {
                "I've answered as many requests from other people today as my owner allows. Please try again tomorrow, or ask my owner."
            }
        }
    }
}

/// The turns whose origin, the person whose message started their chain, may still start turns
/// on the bot when not its owner: fewer than [`REQUESTER_DAILY_TURNS`] of their own in the last
/// 24 hours, and fewer than the bot's allowance for everyone but its owner together. A refused
/// turn isn't queued and the bot says so in the room, except for a message it only reads along
/// with or a reaction; a used-up allowance also tells the owner once a day. Chains the owner
/// started never count, whichever bots carry them on.
pub(crate) async fn admit_requesters<'a, C: ConnectionTrait>(
    db: &C,
    waking: Vec<Waking<'a>>,
    now: DateTime<FixedOffset>,
) -> Result<Vec<Waking<'a>>, ApiError> {
    let allowances = others_allowances(db, &waking).await?;
    let mut admitted = Vec::with_capacity(waking.len());
    for item in waking {
        let recipient = item.recipient;
        if item.origin_id == recipient.owner_id {
            admitted.push(item);
            continue;
        }
        let refusal = if origin_turns(db, &recipient.bot_id, &item.origin_id, now).await?
            >= REQUESTER_DAILY_TURNS
        {
            Some(Refusal::Person)
        } else {
            match allowances.get(&recipient.bot_id) {
                Some((limit, bot_name))
                    if others_turns(db, &recipient.bot_id, &recipient.owner_id, now).await?
                        >= *limit =>
                {
                    Some(Refusal::Everyone {
                        limit: *limit,
                        bot_name: bot_name.clone(),
                    })
                }
                _ => None,
            }
        };
        let Some(refusal) = refusal else {
            admitted.push(item);
            continue;
        };
        tracing::info!(
            bot_id = recipient.bot_id,
            origin_id = item.origin_id,
            requester_id = item.requester_id,
            everyone = matches!(refusal, Refusal::Everyone { .. }),
            "A requester hit a daily turn limit of a bot"
        );
        if let Refusal::Everyone { limit, bot_name } = &refusal {
            notify_others_limit(db, recipient, bot_name, *limit, now).await?;
        }
        if recipient.unprompted() {
            continue;
        }
        Box::pin(post_notice_in(
            db,
            &recipient.conversation_id,
            &recipient.bot_id,
            &rate_limit_message_id(&recipient.bot_id, &recipient.trigger_message_id),
            refusal.notice(),
            now,
        ))
        .await?;
    }
    Ok(admitted)
}

/// Tells the owner, once a day per bot, that other people used up the bot's allowance.
async fn notify_others_limit<C: ConnectionTrait>(
    db: &C,
    recipient: &BotRecipient,
    bot_name: &str,
    limit: u64,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    notification::Entity::insert(notification::ActiveModel {
        id: Set(format!(
            "botothers_{}_{}",
            recipient.bot_id,
            now.format("%Y%m%d")
        )),
        user_id: Set(recipient.owner_id.clone()),
        app_id: Set(None),
        title: Set(format!("{bot_name} reached its daily limit for other people")),
        description: Set(Some(format!(
            "{bot_name} answered {limit} requests from people other than you in the last day, the limit you set. Further requests wait until earlier ones are a day old. You can raise the limit in the bot's safety settings."
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

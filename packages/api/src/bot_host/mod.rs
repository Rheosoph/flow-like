//! Headless bot turns on the server. Every message that wakes a bot queues a `BotTurn` row in
//! the transaction that wrote it ([`ledger`]); the bot's host claims it with a lease and answers
//! as the bot. This module is the server's own host: a resident worker on the long-running
//! backends, an enqueue-only `HostedWork::BotTurn` on AWS, and [`recover`] for what no host is
//! looking after. Desktop and device hosts claim through `routes/bots/turns.rs`.

pub(crate) mod approvals;
pub(crate) mod authorization;
mod cap;
mod decision;
pub(crate) mod decision_wait;
mod event_decision;
pub(crate) mod history;
pub(crate) mod interactive;
pub(crate) mod ledger;
pub(crate) mod memory;
mod monitor_baseline;
pub(crate) mod notes;
pub(crate) mod recovery;
mod reply_stream;
mod room_updates;
pub(crate) mod taint;
pub(crate) mod talk;
pub(crate) mod task_authority;
pub(crate) mod task_schedules;
pub(crate) mod tasks;
pub(crate) mod worker;

#[cfg(test)]
mod approvals_tests;
#[cfg(test)]
mod cancel_tests;
#[cfg(test)]
mod cap_tests;
#[cfg(all(test, feature = "db-chaos"))]
mod chaos_tests;
#[cfg(test)]
mod device_e2e_tests;
#[cfg(test)]
mod device_tests;
#[cfg(test)]
mod fixture;
#[cfg(test)]
mod interactive_tests;
#[cfg(test)]
mod ledger_tests;
#[cfg(test)]
mod memory_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod talk_tests;
#[cfg(test)]
mod task_authority_tests;
#[cfg(test)]
mod tasks_tests;
#[cfg(test)]
mod worker_tests;

pub use ledger::Recovered;
pub(crate) use ledger::enqueue_in;
pub use worker::{WorkerConfig, spawn_worker};

use crate::{
    entity::bot_turn,
    error::ApiError,
    routes::{
        chat::hosted_worker,
        inbox::{delivery::BotRecipient, now_millis},
    },
    state::AppState,
};
use chrono::Duration;
use flow_like_types::{
    bot::BotLocation,
    bot_turn::{STORED_QUEUED, STORED_SERVER, TurnHostKind, turn_id},
    tokio::sync::Notify,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

static KICK: Notify = Notify::const_new();

/// Wakes the resident worker; a kick before it waits is not lost.
pub(crate) fn kick() {
    KICK.notify_one();
}

/// Run by a hosted worker invocation: drives the turn and what queued behind it.
pub(crate) async fn run_hosted(state: &AppState, turn_id: &str) {
    worker::run_hosted(state, turn_id).await;
}

/// Starts the server's host on the turns a committed send queued for `recipients`: wakes the
/// resident worker, and on AWS enqueues a hosted worker invocation per queued turn. Never fails
/// the send; what it misses, [`recover`] picks up.
pub(crate) async fn dispatch(state: &AppState, recipients: &[BotRecipient]) {
    let ids = recipients
        .iter()
        .filter(|recipient| recipient.location == BotLocation::Server)
        .map(|recipient| turn_id(&recipient.bot_id, &recipient.trigger_message_id))
        .collect();
    dispatch_turns(state, ids).await;
}

/// [`dispatch`] for turns named by id; turns that aren't queued server turns are left alone.
pub(crate) async fn dispatch_turns(state: &AppState, ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    kick();
    if !hosted_worker::enabled() {
        return;
    }
    let queued = bot_turn::Entity::find()
        .filter(bot_turn::Column::Id.is_in(ids))
        .filter(bot_turn::Column::HostKind.eq(STORED_SERVER))
        .filter(bot_turn::Column::Status.eq(STORED_QUEUED))
        .all(&state.db)
        .await;
    match queued {
        Ok(turns) => enqueue_hosted(state, turns.iter().map(|turn| turn.id.as_str())).await,
        Err(error) => tracing::warn!(%error, "Bot turns not dispatched"),
    }
}

async fn enqueue_hosted<'a>(state: &AppState, ids: impl Iterator<Item = &'a str>) {
    for id in ids {
        if let Err(error) = hosted_worker::enqueue_bot_turn(state, id).await {
            tracing::warn!(%error, turn_id = %id, "Bot turn not enqueued for the hosted worker");
        }
    }
}

/// Turns the hosted worker was asked for and never claimed: queued server turns idle for this
/// long are enqueued again.
const REDISPATCH_AFTER_SECS: i64 = 30;
const REDISPATCH_BATCH: u64 = 20;

/// Ends or re-queues what no host looks after (see [`ledger::recover_in`]) and, on AWS, asks the
/// hosted worker again for server turns that sat queued.
pub async fn recover(state: &AppState) -> Result<Recovered, ApiError> {
    let now = now_millis();
    if let Err(error) = tasks::fire_due(state).await {
        tracing::warn!(%error, "Bot task sweep incomplete");
    }
    let recovered = state
        .transaction(|txn| Box::pin(async move { ledger::recover_in(txn, now).await }))
        .await?;
    if let Err(error) = decision_wait::cleanup(&state.db, now).await {
        tracing::warn!(%error, "Bot participant wait cleanup failed");
    }
    match ledger::hand_back_in(&state.db, now).await {
        Ok(handed_back) if !handed_back.is_empty() => {
            tracing::info!(
                turns = handed_back.len(),
                "Bot turns no desktop claimed went back to the server"
            );
            dispatch_turns(state, handed_back).await;
        }
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(%error, "Bot turns handed off to a desktop were not taken back")
        }
    }
    match crate::devices::bots_removed::fail_orphaned(state).await {
        Ok(0) => {}
        Ok(ended) => tracing::info!(turns = ended, "Bot turns of removed devices ended"),
        Err(error) => tracing::warn!(%error, "Bot turns of removed devices were not ended"),
    }
    if let Err(error) = talk::expire_asks(state, now).await {
        tracing::warn!(%error, "Bot talk: lapsed questions were not ended");
    }
    match approvals::expire(state, now).await {
        Ok(expired) if expired != approvals::Expired::default() => tracing::info!(
            turns = expired.turns,
            tokens = expired.tokens,
            grants = expired.grants,
            "Bot turn approvals expired"
        ),
        Ok(_) => {}
        Err(error) => tracing::warn!(%error, "Bot turn approval expiry failed"),
    }
    if recovered != Recovered::default() {
        tracing::info!(
            cancelled = recovered.cancelled,
            expired = recovered.expired,
            requeued = recovered.requeued,
            lost = recovered.lost,
            "Bot turn recovery changed turns"
        );
    }
    if hosted_worker::enabled() {
        let idle =
            ledger::queued_heads(&state.db, None, TurnHostKind::Server, now, REDISPATCH_BATCH)
                .await?;
        let cutoff = now - Duration::seconds(REDISPATCH_AFTER_SECS);
        enqueue_hosted(
            state,
            idle.iter()
                .filter(|(turn, _)| turn.updated_at <= cutoff)
                .map(|(turn, _)| turn.id.as_str()),
        )
        .await;
    } else {
        kick();
    }
    crate::routes::bots::live_activities::sweep(state).await;
    Ok(recovered)
}

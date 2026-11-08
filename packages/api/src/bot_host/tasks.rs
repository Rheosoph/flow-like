//! Firing bot tasks. A due task is claimed with a compare-and-set on its `nextRunAt` inside the
//! transaction that writes its `trigger` entry, so any number of sweeps fire a run exactly once
//! and a failed write leaves the run due. A run missed while nothing was sweeping fires once, and
//! the task's next run is the first one after now. Tasks of desktop bots are fired by their desktop
//! host (`POST /bots/{bot_id}/tasks/{task_id}/fire`); the sweep fires the rest.

use crate::{
    entity::{bot, bot_task},
    error::ApiError,
    routes::inbox::{
        delivery::BotRecipient,
        now_millis,
        triggers::{lock_trigger_conversation, trigger_conversation, write_trigger_in},
        wake,
    },
    state::AppState,
};
use chrono::{DateTime, FixedOffset, Utc};
use flow_like_types::{
    bot::BotLocation,
    bot_task::{TaskKind, TaskSchedule, fire_message_id},
    inbox::{TRIGGER_TEXT_MAX_CHARS, TRIGGER_TITLE_MAX_CHARS, TriggerBody, TriggerSource},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, QueryTrait,
    sea_query::Expr,
};

pub(crate) const SCHEDULING_CAPABILITY: &str = "scheduling";
const FIRE_BATCH: u64 = 100;

/// What firing one task came to.
#[derive(Debug)]
pub(crate) enum Fired {
    /// Another sweep, an edit or a pause got there first.
    Lost,
    /// The run was claimed and the bot's conversation can't be written to (end-to-end encrypted).
    Skipped,
    Sent(Vec<String>, Vec<BotRecipient>),
}

pub(crate) fn schedule_of(task: &bot_task::Model) -> Option<TaskSchedule> {
    match TaskKind::parse(&task.kind)? {
        TaskKind::Once => Some(TaskSchedule::Once {
            at: task.run_at?.with_timezone(&Utc),
        }),
        TaskKind::Cron => Some(TaskSchedule::Cron {
            expr: task.cron.clone()?,
            tz: task.timezone.clone()?,
        }),
    }
}

fn truncated(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

pub(crate) fn trigger_for(task: &bot_task::Model) -> TriggerBody {
    TriggerBody {
        source: TriggerSource::Schedule,
        monitoring: task.monitoring.unwrap_or(false),
        ref_id: task.id.clone(),
        title: truncated(&task.title, TRIGGER_TITLE_MAX_CHARS),
        text: truncated(&task.instruction, TRIGGER_TEXT_MAX_CHARS),
        payload: None,
        payload_handle: None,
        bot_id: None,
    }
}

/// Moves the task past the run `task.next_run_at` names, once: `false` when its stored next run
/// is no longer that one.
async fn claim_in<C: ConnectionTrait>(
    db: &C,
    task: &bot_task::Model,
    now: DateTime<FixedOffset>,
) -> Result<bool, ApiError> {
    let Some(due) = task.next_run_at else {
        return Ok(false);
    };
    let next = schedule_of(task)
        .and_then(|schedule| schedule.after_fire(now.with_timezone(&Utc)))
        .map(|next| next.fixed_offset());
    let claimed = bot_task::Entity::update_many()
        .col_expr(bot_task::Column::NextRunAt, Expr::value(next))
        .col_expr(bot_task::Column::LastRunAt, Expr::value(now))
        .col_expr(bot_task::Column::Enabled, Expr::value(next.is_some()))
        .col_expr(bot_task::Column::UpdatedAt, Expr::value(now))
        .filter(bot_task::Column::Id.eq(task.id.as_str()))
        .filter(bot_task::Column::Enabled.eq(true))
        .filter(bot_task::Column::NextRunAt.eq(due))
        .filter(bot_task::Column::UpdatedAt.eq(task.updated_at))
        .filter(match task.monitoring {
            Some(monitoring) => bot_task::Column::Monitoring.eq(monitoring),
            None => bot_task::Column::Monitoring.is_null(),
        })
        .exec(db)
        .await?;
    Ok(claimed.rows_affected == 1)
}

/// Claims the run `task` is due for and writes its trigger into `conversation_id`, whose lock
/// the caller holds.
pub(crate) async fn fire_locked_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    task: &bot_task::Model,
    now: DateTime<FixedOffset>,
) -> Result<Fired, ApiError> {
    let Some(due) = task.next_run_at else {
        return Ok(Fired::Lost);
    };
    if !claim_in(db, task, now).await? {
        return Ok(Fired::Lost);
    }
    let message_id = fire_message_id(&task.id, due.with_timezone(&Utc));
    let sent = write_trigger_in(
        db,
        conversation_id,
        &task.owner_id,
        &message_id,
        &trigger_for(task),
        now,
    )
    .await?;
    Ok(Fired::Sent(sent.wake, sent.bots))
}

/// [`fire_locked_in`] under the conversation's lock. A task whose conversation the server cannot
/// write to (end-to-end encrypted) is moved on without a run.
async fn fire_in(
    txn: &sea_orm::DatabaseTransaction,
    task: &bot_task::Model,
    now: DateTime<FixedOffset>,
) -> Result<Fired, ApiError> {
    match trigger_conversation(txn, &task.owner_id, &task.bot_id, now).await? {
        Some(conversation_id) => {
            lock_trigger_conversation(txn, &conversation_id).await?;
            fire_locked_in(txn, &conversation_id, task, now).await
        }
        None => {
            claim_in(txn, task, now).await?;
            Ok(Fired::Skipped)
        }
    }
}

/// Fires `task` if it is due and nobody else did; `true` when this call fired it. A one-time
/// task's schedule goes with its run, so no scheduler calls for it again.
pub(crate) async fn fire(state: &AppState, task: &bot_task::Model) -> Result<bool, ApiError> {
    let now = now_millis();
    let fired = state
        .transaction(|txn| {
            let task = task.clone();
            Box::pin(async move { fire_in(txn, &task, now).await })
        })
        .await?;
    if !matches!(fired, Fired::Lost) && TaskKind::parse(&task.kind) == Some(TaskKind::Once) {
        super::task_schedules::remove(state, &task.id).await;
    }
    match fired {
        Fired::Lost => Ok(false),
        Fired::Skipped => {
            tracing::warn!(task_id = %task.id, bot_id = %task.bot_id, "Bot task skipped: the conversation is end-to-end encrypted");
            Ok(false)
        }
        Fired::Sent(people, bots) => {
            wake(state, people).await;
            crate::bot_host::dispatch(state, &bots).await;
            Ok(true)
        }
    }
}

fn firing_bots() -> sea_orm::sea_query::SelectStatement {
    bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .filter(bot::Column::DeletedAt.is_null())
        .filter(bot::Column::Location.ne(BotLocation::Desktop.as_str().to_ascii_uppercase()))
        .into_query()
}

/// Enabled tasks of the bots the server fires for whose next run has come.
pub(crate) async fn due_in<C: ConnectionTrait>(
    db: &C,
    now: DateTime<FixedOffset>,
) -> Result<Vec<bot_task::Model>, ApiError> {
    Ok(bot_task::Entity::find()
        .filter(bot_task::Column::Enabled.eq(true))
        .filter(bot_task::Column::NextRunAt.lte(now))
        .filter(bot_task::Column::BotId.in_subquery(firing_bots()))
        .order_by_asc(bot_task::Column::NextRunAt)
        .order_by_asc(bot_task::Column::Id)
        .limit(FIRE_BATCH)
        .all(db)
        .await?)
}

pub(crate) fn holds_capability(bot: &bot::Model) -> bool {
    bot.capabilities
        .iter()
        .any(|id| id == SCHEDULING_CAPABILITY)
}

/// Fires every due task of the server's bots; returns how many this pass fired. A task that
/// fails to fire stays due and is tried again by the next pass.
pub async fn fire_due(state: &AppState) -> Result<u64, ApiError> {
    let due = due_in(&state.db, now_millis()).await?;
    if due.is_empty() {
        return Ok(0);
    }
    let ids: Vec<&str> = due.iter().map(|task| task.bot_id.as_str()).collect();
    let capable: std::collections::HashSet<String> = bot::Entity::find()
        .filter(bot::Column::Id.is_in(ids))
        .all(&state.db)
        .await?
        .into_iter()
        .filter(holds_capability)
        .map(|bot| bot.id)
        .collect();
    let mut fired = 0;
    for task in due.iter().filter(|task| capable.contains(&task.bot_id)) {
        match fire(state, task).await {
            Ok(true) => fired += 1,
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, task_id = %task.id, "Bot task not fired; it stays due")
            }
        }
    }
    Ok(fired)
}

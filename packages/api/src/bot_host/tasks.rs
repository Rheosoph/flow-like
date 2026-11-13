//! Firing bot tasks. A due task is claimed with a compare-and-set on its `nextRunAt` inside the
//! transaction that writes its `trigger` entry, so any number of sweeps fire a run exactly once
//! and a failed write leaves the run due. A run missed while nothing was sweeping fires once, and
//! the task's next run is the first one after now. Tasks of desktop bots are fired by their desktop
//! host (`POST /bots/{bot_id}/tasks/{task_id}/fire`); the sweep fires the rest.
//!
//! A task scheduled in a group or in someone else's chat fires there, sent as the person who
//! asked and naming the bot, so only that bot wakes and its run has that person's rights. Once
//! the bot or that person has left, the task is paused instead.

use crate::{
    entity::{bot, bot_task, conversation, conversation_member},
    error::ApiError,
    routes::inbox::{
        active_member,
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
    inbox::{
        ConversationEncryption, TRIGGER_TEXT_MAX_CHARS, TRIGGER_TITLE_MAX_CHARS, TriggerBody,
        TriggerSource,
    },
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, QueryTrait,
    sea_query::{Expr, ExprTrait},
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
    /// The bot or the person who asked left the group or chat the task fires in; it is paused.
    Paused,
    Sent(Vec<String>, Vec<BotRecipient>),
}

/// Whether a task's run can go into the group or chat it was scheduled in.
enum Place {
    Open,
    Sealed,
    Left,
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
        bot_id: task
            .conversation_id
            .as_ref()
            .map(|_| task.bot_id.clone()),
    }
}

/// Who a run of `task` is sent as: the person who asked for it in their group or chat, else the
/// owner.
pub(crate) fn sender_of(task: &bot_task::Model) -> &str {
    match (&task.conversation_id, &task.requested_by) {
        (Some(_), Some(requested_by)) => requested_by,
        _ => &task.owner_id,
    }
}

/// The task as it was read, still due for `due`: no other sweep, edit or pause moved it since.
fn unchanged(task: &bot_task::Model, due: DateTime<FixedOffset>) -> Condition {
    Condition::all()
        .add(bot_task::Column::Id.eq(task.id.as_str()))
        .add(bot_task::Column::Enabled.eq(true))
        .add(bot_task::Column::NextRunAt.eq(due))
        .add(bot_task::Column::UpdatedAt.eq(task.updated_at))
        .add(match task.monitoring {
            Some(monitoring) => bot_task::Column::Monitoring.eq(monitoring),
            None => bot_task::Column::Monitoring.is_null(),
        })
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
        .filter(unchanged(task, due))
        .exec(db)
        .await?;
    Ok(claimed.rows_affected == 1)
}

/// Pauses the task instead of running it, once: `false` when another sweep or an edit moved it.
async fn pause_in<C: ConnectionTrait>(
    db: &C,
    task: &bot_task::Model,
    now: DateTime<FixedOffset>,
) -> Result<bool, ApiError> {
    let Some(due) = task.next_run_at else {
        return Ok(false);
    };
    let paused = bot_task::Entity::update_many()
        .col_expr(bot_task::Column::Enabled, Expr::value(false))
        .col_expr(bot_task::Column::UpdatedAt, Expr::value(now))
        .filter(unchanged(task, due))
        .exec(db)
        .await?;
    Ok(paused.rows_affected == 1)
}

/// Whether the bot and the person who asked are both still in `conversation_id` and the server
/// can write into it. The caller holds the conversation's lock.
async fn place_of<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    task: &bot_task::Model,
) -> Result<Place, ApiError> {
    let Some(conversation) = conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await?
    else {
        return Ok(Place::Left);
    };
    let present = conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::PrincipalId.is_in([task.bot_id.as_str(), sender_of(task)]))
        .filter(active_member())
        .count(db)
        .await?;
    if present < 2 {
        return Ok(Place::Left);
    }
    Ok(
        match ConversationEncryption::parse(&conversation.encryption) {
            Some(ConversationEncryption::Plain) => Place::Open,
            _ => Place::Sealed,
        },
    )
}

/// Claims the run `task` is due for and writes its trigger into `conversation_id`, whose lock
/// the caller holds. A task of a group or chat the server can't write into moves on without a
/// run; one the bot or the person who asked left is paused.
pub(crate) async fn fire_locked_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    task: &bot_task::Model,
    now: DateTime<FixedOffset>,
) -> Result<Fired, ApiError> {
    let Some(due) = task.next_run_at else {
        return Ok(Fired::Lost);
    };
    if task.conversation_id.is_some() {
        match place_of(db, conversation_id, task).await? {
            Place::Open => {}
            Place::Sealed => {
                claim_in(db, task, now).await?;
                return Ok(Fired::Skipped);
            }
            Place::Left if pause_in(db, task, now).await? => return Ok(Fired::Paused),
            Place::Left => return Ok(Fired::Lost),
        }
    }
    if !claim_in(db, task, now).await? {
        return Ok(Fired::Lost);
    }
    let message_id = fire_message_id(&task.id, due.with_timezone(&Utc));
    let sent = write_trigger_in(
        db,
        conversation_id,
        sender_of(task),
        &message_id,
        &trigger_for(task),
        now,
    )
    .await?;
    Ok(Fired::Sent(sent.wake, sent.bots))
}

/// [`fire_locked_in`] under the conversation's lock, in the group or chat the task was scheduled
/// in or else the owner's chat with the bot. An owner's chat the server cannot write to
/// (end-to-end encrypted) moves the task on without a run.
async fn fire_in(
    txn: &sea_orm::DatabaseTransaction,
    task: &bot_task::Model,
    now: DateTime<FixedOffset>,
) -> Result<Fired, ApiError> {
    let conversation_id = match &task.conversation_id {
        Some(conversation_id) => Some(conversation_id.clone()),
        None => trigger_conversation(txn, &task.owner_id, &task.bot_id, now).await?,
    };
    match conversation_id {
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
    let ends = TaskKind::parse(&task.kind) == Some(TaskKind::Once) || matches!(fired, Fired::Paused);
    if !matches!(fired, Fired::Lost) && ends {
        super::task_schedules::remove(state, &task.id).await;
    }
    match fired {
        Fired::Lost => Ok(false),
        Fired::Skipped => {
            tracing::warn!(task_id = %task.id, bot_id = %task.bot_id, "Bot task skipped: the conversation is end-to-end encrypted");
            Ok(false)
        }
        Fired::Paused => {
            tracing::info!(task_id = %task.id, bot_id = %task.bot_id, conversation_id = ?task.conversation_id, "Bot task paused: the bot or the person who asked for it left its conversation");
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
        .filter(
            Expr::col(bot::Column::Capabilities)
                .cast_as("text")
                .like(format!("%\"{SCHEDULING_CAPABILITY}\"%")),
        )
        .into_query()
}

/// Enabled tasks of the bots the server fires for, holding the scheduling capability, whose next
/// run has come. The capability is filtered in SQL so tasks of bots without it can't fill the
/// batch.
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

//! Event tasks: the events of a bot's own calendar, and the wake-up rows of its connected
//! calendars. An event task's `nextRunAt` is its next wake-up. The wake-up offsets, the
//! monitoring flag, the next wake-up and whether the row is enabled are all worked out here from
//! the bot's [`CalendarWake`] settings and the event, never taken from a client.

use crate::{
    entity::{bot, bot_task},
    error::ApiError,
};
use chrono::{DateTime, FixedOffset, Utc};
use flow_like_types::{
    bot::BotCapability,
    bot_calendar::{CalendarWake, EventSchedule, TaskEvent, UpdateCalendarWake, wake_text},
    bot_task::STORED_EVENT,
    inbox::TRIGGER_TEXT_MAX_CHARS,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter,
};

const ANY_DAY_CRON: &str = "0 0 * * *";

pub(crate) fn holds_calendar(bot: &bot::Model) -> bool {
    bot.capabilities
        .iter()
        .any(|id| id == BotCapability::Calendar.as_str())
}

/// The event an event task row keeps; `None` for any other row or an unreadable event.
pub(crate) fn event_of(task: &bot_task::Model) -> Option<TaskEvent> {
    serde_json::from_value(task.event.clone()?).ok()
}

/// The bot's wake-up settings; the defaults while none were saved or the saved ones are unreadable.
pub(crate) fn wake_of(bot: &bot::Model) -> CalendarWake {
    bot.calendar_wake
        .clone()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

pub(crate) fn wake_json(wake: &CalendarWake) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(wake).map_err(|error| {
        ApiError::internal(format!(
            "Calendar wake-up settings could not be saved: {error}"
        ))
    })
}

pub(crate) fn event_json(event: &TaskEvent) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(event)
        .map_err(|error| ApiError::internal(format!("An event could not be saved: {error}")))
}

/// Refuses a time zone name the schedule code does not know.
pub(crate) fn check_tz(tz: &str, now: DateTime<Utc>) -> Result<(), String> {
    EventSchedule::check_recurrence(ANY_DAY_CRON, Some(tz), now)
}

/// An event with its wake-ups worked out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Armed {
    pub event: TaskEvent,
    pub next: Option<DateTime<Utc>>,
    pub monitoring: bool,
}

impl Armed {
    pub(crate) fn enabled(&self) -> bool {
        self.next.is_some()
    }
}

/// Works out `event`'s wake-ups under `wake`: offsets, the next wake-up after `last_run`, and
/// whether its run may stay quiet.
pub(crate) fn arm(
    wake: &CalendarWake,
    title: &str,
    mut event: TaskEvent,
    cron: Option<&str>,
    tz: Option<&str>,
    last_run: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<Armed, String> {
    event.wake_offsets = wake.resolve(title, &event);
    let next = EventSchedule::of(&event, cron, tz)?.next_wake(now, last_run);
    Ok(Armed {
        event,
        next,
        monitoring: wake.quiet,
    })
}

/// The schedule of an event task row.
pub(crate) fn schedule_of(task: &bot_task::Model) -> Option<EventSchedule> {
    EventSchedule::of(
        &event_of(task)?,
        task.cron.as_deref(),
        task.timezone.as_deref(),
    )
    .ok()
}

/// The trigger text of the wake-up of `task` that was due at `due`.
pub(crate) fn trigger_text(
    task: &bot_task::Model,
    due: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<String> {
    let event = event_of(task)?;
    let schedule =
        EventSchedule::of(&event, task.cron.as_deref(), task.timezone.as_deref()).ok()?;
    let entry_id = event
        .external
        .as_ref()
        .map_or_else(|| task.id.clone(), |external| external.encode());
    let text = wake_text(
        &entry_id,
        &task.title,
        &task.instruction,
        &event,
        schedule.occurrence_for(due).unwrap_or(due),
        now,
        task.timezone.as_deref(),
    );
    Some(text.chars().take(TRIGGER_TEXT_MAX_CHARS).collect())
}

/// Every event task of the bot.
pub(crate) async fn rows_of<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
) -> Result<Vec<bot_task::Model>, ApiError> {
    Ok(bot_task::Entity::find()
        .filter(bot_task::Column::BotId.eq(bot_id))
        .filter(bot_task::Column::Kind.eq(STORED_EVENT))
        .all(db)
        .await?)
}

/// The event task of the bot that keeps wake-ups for the connected-calendar event `external`.
pub(crate) fn row_of_external(
    rows: Vec<bot_task::Model>,
    external: &flow_like_types::bot_calendar::ExternalEventRef,
) -> Option<bot_task::Model> {
    rows.into_iter()
        .find(|row| event_of(row).and_then(|event| event.external).as_ref() == Some(external))
}

/// Saves `update` to the bot's settings and works out every event task of the bot again, in the
/// caller's transaction. Returns the settings now in effect and the rows that changed.
pub(crate) async fn apply_in<C: ConnectionTrait>(
    db: &C,
    bot: &bot::Model,
    update: UpdateCalendarWake,
    now: DateTime<FixedOffset>,
) -> Result<(CalendarWake, Vec<bot_task::Model>), ApiError> {
    let wake = update.apply(wake_of(bot)).map_err(ApiError::bad_request)?;
    let mut active: bot::ActiveModel = bot.clone().into();
    active.calendar_wake = Set(Some(wake_json(&wake)?));
    active.updated_at = Set(now);
    active.update(db).await?;
    let changed = rearm_all_in(db, &bot.id, &wake, now).await?;
    Ok((wake, changed))
}

/// Works out every event task of `bot_id` again under `wake`; only rows that differ are written.
pub(crate) async fn rearm_all_in<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    wake: &CalendarWake,
    now: DateTime<FixedOffset>,
) -> Result<Vec<bot_task::Model>, ApiError> {
    let mut changed = Vec::new();
    for row in rows_of(db, bot_id).await? {
        let Some(event) = event_of(&row) else {
            continue;
        };
        let armed = arm(
            wake,
            &row.title,
            event,
            row.cron.as_deref(),
            row.timezone.as_deref(),
            row.last_run_at.map(|at| at.with_timezone(&Utc)),
            now.with_timezone(&Utc),
        )
        .map_err(ApiError::bad_request)?;
        let next = armed.next.map(|at| at.fixed_offset());
        let event = event_json(&armed.event)?;
        if row.event.as_ref() == Some(&event)
            && row.next_run_at == next
            && row.enabled == armed.enabled()
            && row.monitoring == Some(armed.monitoring)
        {
            continue;
        }
        let mut active: bot_task::ActiveModel = row.into();
        active.event = Set(Some(event));
        active.next_run_at = Set(next);
        active.enabled = Set(armed.enabled());
        active.monitoring = Set(Some(armed.monitoring));
        active.updated_at = Set(now);
        changed.push(active.update(db).await?);
    }
    Ok(changed)
}

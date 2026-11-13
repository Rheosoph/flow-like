//! Bot tasks on the deployment's scheduler (EventBridge on AWS): every enabled task of a bot the
//! server fires holds one schedule, so it fires on its minute instead of on the next maintenance
//! sweep. The schedule calls `POST /sink/trigger/async` the way a cron event's does, with
//! [`schedule_key`] in the place of the event id. It and the sweep both fire through
//! [`tasks::fire`], whose claim lets each run through once, so the sweep stays the fallback for a
//! schedule that could not be saved or did not call.

use super::tasks::{self, holds_capability};
use crate::{
    entity::{bot, bot_task},
    error::ApiError,
    state::AppState,
};
use chrono::{DateTime, Duration, DurationRound, Utc};
use flow_like_sinks::{CronSinkConfig, ScheduledLocal, SchedulerError};
use flow_like_types::{bot::BotLocation, bot_task::TaskKind, tokio};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

const KEY_PREFIX: &str = "bottask_";
/// A call this much before the run is due waits for it; an earlier one is left to the sweep.
const MAX_EARLY_SECS: i64 = 10;

/// What a scheduler call for a task came to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ScheduledFire {
    /// The task or its bot is gone: the scheduler should drop the schedule.
    Missing,
    Fired,
    /// Not due, paused, already fired by the sweep, or not the server's to fire.
    Skipped,
}

/// The scheduler key of `task_id`'s schedule.
pub(crate) fn schedule_key(task_id: &str) -> String {
    format!("{KEY_PREFIX}{task_id}")
}

/// The task a scheduler key names; `None` for a cron event's id.
pub(crate) fn task_of(key: &str) -> Option<&str> {
    key.strip_prefix(KEY_PREFIX).filter(|id| !id.is_empty())
}

/// `at` rounded up to its minute: the scheduler fires on minutes, and never before the run.
fn minute_after(at: DateTime<Utc>) -> DateTime<Utc> {
    let floor = at.duration_trunc(Duration::minutes(1)).unwrap_or(at);
    if floor == at {
        at
    } else {
        floor + Duration::minutes(1)
    }
}

/// The schedule `task` should hold at `now`; `None` when nothing should fire.
fn schedule_of(task: &bot_task::Model, now: DateTime<Utc>) -> Option<CronSinkConfig> {
    if !task.enabled {
        return None;
    }
    match TaskKind::parse(&task.kind)? {
        TaskKind::Once => {
            let at = minute_after(task.next_run_at?.with_timezone(&Utc));
            (at > now).then(|| CronSinkConfig {
                expression: String::new(),
                timezone: "UTC".to_string(),
                scheduled_for: Some(ScheduledLocal {
                    date: at.format("%Y-%m-%d").to_string(),
                    time: at.format("%H:%M").to_string(),
                }),
                active: true,
            })
        }
        TaskKind::Cron => Some(CronSinkConfig {
            expression: task.cron.clone()?,
            timezone: task.timezone.clone().unwrap_or_else(|| "UTC".to_string()),
            scheduled_for: None,
            active: true,
        }),
    }
}

/// The live bot `bot_id` names when the server fires its tasks (desktop hosts fire their own).
async fn server_bot(state: &AppState, bot_id: &str) -> Result<Option<bot::Model>, ApiError> {
    Ok(bot::Entity::find_by_id(bot_id)
        .one(&state.db)
        .await?
        .filter(|bot| {
            bot.deleted_at.is_none()
                && !bot
                    .location
                    .eq_ignore_ascii_case(BotLocation::Desktop.as_str())
        }))
}

/// Brings `task`'s schedule in line with its row: saved while it should fire, removed otherwise.
/// Never fails the caller; a schedule that could not be saved leaves the task to the sweep.
pub(crate) async fn sync(state: &AppState, task: &bot_task::Model) {
    let Some(scheduler) = state.sink_scheduler.as_ref() else {
        return;
    };
    let config = match server_bot(state, &task.bot_id).await {
        Ok(Some(_)) => schedule_of(task, Utc::now()),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(%error, task_id = %task.id, "Bot task schedule not saved; the sweep fires it");
            return;
        }
    };
    let Some(config) = config else {
        return remove(state, &task.id).await;
    };
    let key = schedule_key(&task.id);
    let saved = match scheduler
        .update_schedule(&key, &config.expression, &config)
        .await
    {
        Err(SchedulerError::NotFound(_)) => {
            scheduler
                .create_schedule(&key, &config.expression, &config)
                .await
        }
        other => other,
    };
    if let Err(error) = saved {
        tracing::warn!(%error, task_id = %task.id, "Bot task schedule not saved; the sweep fires it");
    }
}

/// Saves the schedules of every task of `bot_id`, after a change that moves who fires them.
pub(crate) async fn sync_bot(state: &AppState, bot_id: &str) {
    if state.sink_scheduler.is_none() {
        return;
    }
    let rows = bot_task::Entity::find()
        .filter(bot_task::Column::BotId.eq(bot_id))
        .all(&state.db)
        .await;
    match rows {
        Ok(rows) => {
            for task in &rows {
                sync(state, task).await;
            }
        }
        Err(error) => {
            tracing::warn!(%error, bot_id, "Bot task schedules not saved; the sweep fires them")
        }
    }
}

/// Drops `task_id`'s schedule; one that is already gone is fine.
pub(crate) async fn remove(state: &AppState, task_id: &str) {
    let Some(scheduler) = state.sink_scheduler.as_ref() else {
        return;
    };
    match scheduler.delete_schedule(&schedule_key(task_id)).await {
        Ok(()) | Err(SchedulerError::NotFound(_)) => {}
        Err(error) => {
            tracing::warn!(%error, task_id, "Bot task schedule not removed; its calls are skipped")
        }
    }
}

/// What a schedule's call for `task` of `bot` should do at `now`.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Missing,
    Skip,
    /// Fire after this long (zero once the run is due).
    Fire(std::time::Duration),
}

fn verdict(task: &bot_task::Model, bot: Option<&bot::Model>, now: DateTime<Utc>) -> Verdict {
    let Some(bot) = bot.filter(|bot| bot.deleted_at.is_none()) else {
        return Verdict::Missing;
    };
    let fires_here = !bot
        .location
        .eq_ignore_ascii_case(BotLocation::Desktop.as_str())
        && holds_capability(bot);
    let Some(due) = task.next_run_at.filter(|_| task.enabled && fires_here) else {
        return Verdict::Skip;
    };
    let wait = due.with_timezone(&Utc) - now;
    if wait > Duration::seconds(MAX_EARLY_SECS) {
        return Verdict::Skip;
    }
    Verdict::Fire(wait.to_std().unwrap_or_default())
}

/// Fires the task a schedule called for, once it is due; a call up to [`MAX_EARLY_SECS`] early
/// waits for the run.
pub(crate) async fn fire_scheduled(
    state: &AppState,
    task_id: &str,
) -> Result<ScheduledFire, ApiError> {
    let Some(task) = bot_task::Entity::find_by_id(task_id).one(&state.db).await? else {
        return Ok(ScheduledFire::Missing);
    };
    let bot = bot::Entity::find_by_id(&task.bot_id).one(&state.db).await?;
    match verdict(&task, bot.as_ref(), Utc::now()) {
        Verdict::Missing => Ok(ScheduledFire::Missing),
        Verdict::Skip => Ok(ScheduledFire::Skipped),
        Verdict::Fire(wait) => {
            tokio::time::sleep(wait).await;
            Ok(if tasks::fire(state, &task).await? {
                ScheduledFire::Fired
            } else {
                ScheduledFire::Skipped
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn task(kind: &str, next: Option<DateTime<Utc>>, cron: Option<&str>) -> bot_task::Model {
        let now = Utc::now().fixed_offset();
        bot_task::Model {
            id: "task-1".into(),
            bot_id: "bot_a".into(),
            owner_id: "owner".into(),
            kind: kind.into(),
            run_at: next.map(|at| at.fixed_offset()),
            cron: cron.map(str::to_string),
            timezone: cron.map(|_| "Europe/Berlin".to_string()),
            instruction: "Remind me".into(),
            title: "Reminder".into(),
            next_run_at: next.map(|at| at.fixed_offset()),
            last_run_at: None,
            enabled: true,
            monitoring: Some(false),
            created_by: "BOT".into(),
            created_at: now,
            updated_at: now,
            conversation_id: None,
            requested_by: None,
            owner_written: None,
            request_conversation_id: None,
            request_message_id: None,
        }
    }

    #[test]
    fn keys_name_their_task_and_never_an_event() {
        assert_eq!(task_of(&schedule_key("abc")), Some("abc"));
        assert_eq!(task_of("bottask_"), None);
        assert_eq!(task_of("cm1event"), None);
    }

    #[test]
    fn a_one_time_task_fires_on_the_minute_after_its_run() {
        let now = Utc.with_ymd_and_hms(2026, 11, 4, 10, 0, 0).unwrap();
        let next = Utc.with_ymd_and_hms(2026, 11, 4, 10, 32, 41).unwrap();
        let config = schedule_of(&task("ONCE", Some(next), None), now).unwrap();
        let at = config.scheduled_for.unwrap();
        assert_eq!(
            (at.date.as_str(), at.time.as_str()),
            ("2026-11-04", "10:33")
        );
        assert_eq!(config.timezone, "UTC");

        let exact = Utc.with_ymd_and_hms(2026, 11, 4, 10, 32, 0).unwrap();
        let config = schedule_of(&task("ONCE", Some(exact), None), now).unwrap();
        assert_eq!(config.scheduled_for.unwrap().time, "10:32");
    }

    #[test]
    fn nothing_is_scheduled_that_should_not_fire() {
        let now = Utc.with_ymd_and_hms(2026, 11, 4, 10, 0, 0).unwrap();
        let past = Utc.with_ymd_and_hms(2026, 11, 4, 9, 59, 0).unwrap();
        assert!(schedule_of(&task("ONCE", Some(past), None), now).is_none());
        assert!(schedule_of(&task("ONCE", None, None), now).is_none());
        let mut paused = task("CRON", Some(now), Some("*/5 * * * *"));
        paused.enabled = false;
        assert!(schedule_of(&paused, now).is_none());
    }

    async fn stored_bot(location: &str, capabilities: &[&str]) -> bot::Model {
        use sea_orm::{ActiveModelTrait, ActiveValue::Set};
        let db = super::super::fixture::database().await;
        super::super::fixture::insert_bot(&db, "bot_a", "owner", location).await;
        bot::ActiveModel {
            id: Set("bot_a".into()),
            capabilities: Set(capabilities
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .into()),
            ..Default::default()
        }
        .update(&db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_call_fires_a_due_task_of_a_server_bot_that_may_schedule() {
        let now = Utc.with_ymd_and_hms(2026, 11, 4, 10, 0, 0).unwrap();
        let server = stored_bot("SERVER", &["scheduling"]).await;
        let due = task("ONCE", Some(now), None);
        assert_eq!(
            verdict(&due, Some(&server), now),
            Verdict::Fire(std::time::Duration::ZERO)
        );
        let soon = task("ONCE", Some(now + Duration::seconds(4)), None);
        assert_eq!(
            verdict(&soon, Some(&server), now),
            Verdict::Fire(std::time::Duration::from_secs(4))
        );
        let later = task("ONCE", Some(now + Duration::minutes(3)), None);
        assert_eq!(verdict(&later, Some(&server), now), Verdict::Skip);
        let mut paused = due.clone();
        paused.enabled = false;
        assert_eq!(verdict(&paused, Some(&server), now), Verdict::Skip);
    }

    #[tokio::test]
    async fn a_call_skips_desktop_bots_and_bots_without_scheduling_and_drops_deleted_ones() {
        let now = Utc.with_ymd_and_hms(2026, 11, 4, 10, 0, 0).unwrap();
        let due = task("CRON", Some(now), Some("*/5 * * * *"));
        let desktop = stored_bot("DESKTOP", &["scheduling"]).await;
        assert_eq!(verdict(&due, Some(&desktop), now), Verdict::Skip);
        let unable = stored_bot("SERVER", &[]).await;
        assert_eq!(verdict(&due, Some(&unable), now), Verdict::Skip);
        let mut deleted = stored_bot("SERVER", &["scheduling"]).await;
        deleted.deleted_at = Some(now.fixed_offset());
        assert_eq!(verdict(&due, Some(&deleted), now), Verdict::Missing);
        assert_eq!(verdict(&due, None, now), Verdict::Missing);
    }

    #[test]
    fn a_cron_task_keeps_its_expression_and_time_zone() {
        let now = Utc.with_ymd_and_hms(2026, 11, 4, 10, 0, 0).unwrap();
        let config = schedule_of(&task("CRON", Some(now), Some("30 8 * * 1-5")), now).unwrap();
        assert_eq!(config.expression, "30 8 * * 1-5");
        assert_eq!(config.timezone, "Europe/Berlin");
        assert!(config.scheduled_for.is_none() && config.active);
    }
}

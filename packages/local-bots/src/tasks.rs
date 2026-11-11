//! Scheduled tasks of local bots. A due task is claimed with a compare-and-set on `next_run_at`
//! in the transaction that writes its `trigger` entry, so a run is written once however many
//! ticks race for it; a run missed while the app was closed fires once at the next tick and the
//! task's next run is the first one after that. A task the bot scheduled in a group fires there,
//! naming the bot so only it wakes, and is paused once the bot left the group.

use crate::{
    LOCAL_OWNER_ID, LocalBotStore,
    bots::{bot_exists, bot_not_found},
    error::{Error, Result},
    inbox::{in_group, write_group_trigger, write_trigger},
    store::{now_micros, timestamp},
};
use chrono::{DateTime, TimeZone, Utc};
use flow_like_types::{
    bot_task::{
        self as contract, BotTask, CreateBotTask, MAX_TASKS_PER_BOT, TaskConversation,
        TaskCreator, TaskKind, TaskSchedule, UpdateBotTask, fire_message_id,
    },
    create_id,
    inbox::{
        ConversationKind, TRIGGER_TEXT_MAX_CHARS, TRIGGER_TITLE_MAX_CHARS, TriggerBody,
        TriggerSource,
    },
};
use rusqlite::{Connection, OptionalExtension, Row, params};

const SCHEDULING_CAPABILITY: &str = "scheduling";
const COLUMNS: &str = "t.id, t.bot_id, t.kind, t.run_at, t.cron, t.timezone, t.instruction,
    t.title, t.next_run_at, t.last_run_at, t.enabled, t.created_by, t.created_at, t.monitoring,
    t.conversation_id, c.kind, c.title";
const TASKS: &str = "tasks t LEFT JOIN conversations c ON c.id = t.conversation_id";

struct Stored {
    task: BotTask,
}

fn instant(micros: i64) -> DateTime<Utc> {
    Utc.timestamp_micros(micros).single().unwrap_or_default()
}

/// The group a task fires in. A group that is gone still shows, without a name, so the task
/// never reads as one of the person's chat with the bot.
fn conversation_of(row: &Row<'_>) -> rusqlite::Result<Option<TaskConversation>> {
    let Some(id) = row.get::<_, Option<String>>(14)? else {
        return Ok(None);
    };
    let kind: Option<String> = row.get(15)?;
    let title: Option<String> = row.get(16)?;
    Ok(Some(TaskConversation {
        id,
        kind: kind
            .as_deref()
            .and_then(ConversationKind::parse)
            .unwrap_or(ConversationKind::Group),
        title: title.filter(|title| !title.trim().is_empty()),
    }))
}

fn read_task(row: &Row<'_>) -> rusqlite::Result<Stored> {
    let rfc = |micros: Option<i64>| micros.map(|micros| timestamp(micros).to_rfc3339());
    let kind: String = row.get(2)?;
    let created_by: String = row.get(11)?;
    Ok(Stored {
        task: BotTask {
            conversation: conversation_of(row)?,
            id: row.get(0)?,
            bot_id: row.get(1)?,
            owner_id: LOCAL_OWNER_ID.to_string(),
            kind: TaskKind::parse(&kind).unwrap_or(TaskKind::Once),
            at: rfc(row.get(3)?),
            cron: row.get(4)?,
            tz: row.get(5)?,
            instruction: row.get(6)?,
            title: row.get(7)?,
            next_run_at: rfc(row.get(8)?),
            last_run_at: rfc(row.get(9)?),
            enabled: row.get::<_, i64>(10)? != 0,
            monitoring: row.get::<_, i64>(13)? != 0,
            created_by: TaskCreator::parse(&created_by).unwrap_or(TaskCreator::Owner),
            created_at: timestamp(row.get(12)?).to_rfc3339(),
        },
    })
}

fn schedule_of(task: &BotTask) -> Option<TaskSchedule> {
    match task.kind {
        TaskKind::Once => Some(TaskSchedule::Once {
            at: DateTime::parse_from_rfc3339(task.at.as_deref()?)
                .ok()?
                .with_timezone(&Utc),
        }),
        TaskKind::Cron => Some(TaskSchedule::Cron {
            expr: task.cron.clone()?,
            tz: task.tz.clone()?,
        }),
    }
}

fn task_not_found(task_id: &str) -> Error {
    Error::not_found(format!("Task '{task_id}' not found"))
}

fn holds_capability(db: &Connection, bot_id: &str) -> Result<bool> {
    let stored: Option<String> = db
        .prepare_cached("SELECT capabilities FROM bots WHERE id = ?1")?
        .query_row([bot_id], |row| row.get(0))
        .optional()?;
    let ids: Vec<String> = stored
        .and_then(|stored| serde_json::from_str(&stored).ok())
        .unwrap_or_default();
    Ok(ids.iter().any(|id| id == SCHEDULING_CAPABILITY))
}

fn require_scheduling(db: &Connection, bot_id: &str) -> Result<()> {
    if !bot_exists(db, bot_id)? {
        return Err(bot_not_found(bot_id));
    }
    if holds_capability(db, bot_id)? {
        return Ok(());
    }
    Err(Error::invalid(format!(
        "Bot '{bot_id}' doesn't have the scheduling capability switched on"
    )))
}

fn find(db: &Connection, bot_id: &str, task_id: &str) -> Result<BotTask> {
    db.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM {TASKS} WHERE t.id = ?1 AND t.bot_id = ?2"
    ))?
    .query_row(params![task_id, bot_id], read_task)
    .optional()?
    .map(|stored| stored.task)
    .ok_or_else(|| task_not_found(task_id))
}

fn micros_of(text: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.timestamp_micros())
}

fn trigger_for(task: &BotTask) -> TriggerBody {
    TriggerBody {
        source: TriggerSource::Schedule,
        monitoring: task.monitoring,
        ref_id: task.id.clone(),
        title: task.title.chars().take(TRIGGER_TITLE_MAX_CHARS).collect(),
        text: task
            .instruction
            .chars()
            .take(TRIGGER_TEXT_MAX_CHARS)
            .collect(),
        payload: None,
        payload_handle: None,
        bot_id: task.conversation.as_ref().map(|_| task.bot_id.clone()),
    }
}

fn parts_of(schedule: &TaskSchedule) -> (Option<i64>, Option<String>, Option<String>) {
    match schedule {
        TaskSchedule::Once { at } => (Some(at.timestamp_micros()), None, None),
        TaskSchedule::Cron { expr, tz } => (None, Some(expr.clone()), Some(tz.clone())),
    }
}

/// Moves the task past the run it is due for, once. `false` when its next run is no longer
/// `due`.
fn claim(db: &Connection, task: &BotTask, due: i64, now: i64) -> Result<bool> {
    let next = schedule_of(task)
        .and_then(|schedule| schedule.after_fire(instant(now)))
        .map(|next| next.timestamp_micros());
    let claimed = db
        .prepare_cached(
            "UPDATE tasks SET next_run_at = ?1, last_run_at = ?2, enabled = ?3, updated_at = ?2
             WHERE id = ?4 AND enabled = 1 AND next_run_at = ?5",
        )?
        .execute(params![next, now, i64::from(next.is_some()), task.id, due])?;
    Ok(claimed == 1)
}

/// Pauses the task instead of running it, once. `false` when its next run is no longer `due`.
fn pause(db: &Connection, task: &BotTask, due: i64, now: i64) -> Result<bool> {
    let paused = db
        .prepare_cached(
            "UPDATE tasks SET enabled = 0, updated_at = ?1
             WHERE id = ?2 AND enabled = 1 AND next_run_at = ?3",
        )?
        .execute(params![now, task.id, due])?;
    Ok(paused == 1)
}

impl LocalBotStore {
    /// The bot's tasks, soonest first.
    pub fn tasks(&self, bot_id: &str) -> Result<Vec<BotTask>> {
        self.read(|db| {
            if !bot_exists(db, bot_id)? {
                return Err(bot_not_found(bot_id));
            }
            let mut statement = db.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM {TASKS} WHERE t.bot_id = ?1
                 ORDER BY t.next_run_at IS NULL, t.next_run_at, t.created_at, t.id"
            ))?;
            let rows = statement
                .query_map([bot_id], read_task)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().map(|stored| stored.task).collect())
        })
    }

    /// Saves a task that fires in the person's chat with the bot.
    pub fn create_task(
        &self,
        bot_id: &str,
        request: CreateBotTask,
        created_by: TaskCreator,
    ) -> Result<BotTask> {
        self.create_task_in(bot_id, request, created_by, None)
    }

    /// Saves a task. With `conversation_id`, the conversation of the turn that asked for it, a
    /// group the bot is in, the task fires there; otherwise in the person's chat with the bot.
    pub fn create_task_in(
        &self,
        bot_id: &str,
        request: CreateBotTask,
        created_by: TaskCreator,
        conversation_id: Option<&str>,
    ) -> Result<BotTask> {
        self.write(|tx, _changes| {
            require_scheduling(tx, bot_id)?;
            let group = match conversation_id {
                Some(id) if in_group(tx, id, bot_id)? => Some(id),
                _ => None,
            };
            let now = now_micros();
            let kind = request
                .kind
                .ok_or_else(|| Error::invalid("A task needs a kind, once or cron"))?;
            let schedule = TaskSchedule::parse(
                kind,
                request.at.as_deref(),
                request.cron.as_deref(),
                request.tz.as_deref(),
                instant(now),
            )
            .map_err(Error::invalid)?;
            let (title, instruction) =
                contract::check_text(request.title.as_deref(), &request.instruction)
                    .map_err(Error::invalid)?;
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM tasks WHERE bot_id = ?1",
                [bot_id],
                |row| row.get(0),
            )?;
            if count >= MAX_TASKS_PER_BOT as i64 {
                return Err(Error::invalid(format!(
                    "Bot '{bot_id}' already has {MAX_TASKS_PER_BOT} tasks; delete one first"
                )));
            }
            let id = format!("task_{}", create_id());
            let (run_at, cron, timezone) = parts_of(&schedule);
            tx.execute(
                "INSERT INTO tasks (id, bot_id, kind, run_at, cron, timezone, instruction, title,
                     next_run_at, last_run_at, enabled, created_by, created_at, updated_at, monitoring,
                     conversation_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?11, ?12, ?12, ?13, ?14)",
                params![
                    id,
                    bot_id,
                    schedule.kind().stored(),
                    run_at,
                    cron,
                    timezone,
                    instruction,
                    title,
                    schedule
                        .first_run(instant(now))
                        .map(|at| at.timestamp_micros()),
                    i64::from(request.enabled.unwrap_or(true)),
                    created_by.stored(),
                    now,
                    i64::from(request.monitoring),
                    group
                ],
            )?;
            find(tx, bot_id, &id)
        })
    }

    pub fn update_task(
        &self,
        bot_id: &str,
        task_id: &str,
        request: UpdateBotTask,
    ) -> Result<BotTask> {
        self.write(|tx, _changes| {
            let task = find(tx, bot_id, task_id)?;
            let now = now_micros();
            let moves = request.at.is_some() || request.cron.is_some() || request.tz.is_some();
            let resumes = request.enabled == Some(true) && !task.enabled;
            if moves || resumes {
                let rearmed = task.kind == TaskKind::Once && task.last_run_at.is_some();
                let schedule = TaskSchedule::parse(
                    task.kind,
                    request.at.as_deref().or(task.at.as_deref()),
                    request.cron.as_deref().or(task.cron.as_deref()),
                    request.tz.as_deref().or(task.tz.as_deref()),
                    instant(now),
                )
                .map_err(Error::invalid)?;
                let next = schedule
                    .first_run(instant(now))
                    .map(|at| at.timestamp_micros());
                let enabled = request.enabled.unwrap_or(task.enabled || rearmed) && next.is_some();
                let (run_at, cron, timezone) = parts_of(&schedule);
                tx.execute(
                    "UPDATE tasks SET run_at = ?1, cron = ?2, timezone = ?3, next_run_at = ?4,
                         enabled = ?5 WHERE id = ?6",
                    params![run_at, cron, timezone, next, i64::from(enabled), task_id],
                )?;
            } else if let Some(enabled) = request.enabled {
                tx.execute(
                    "UPDATE tasks SET enabled = ?1 WHERE id = ?2",
                    params![i64::from(enabled), task_id],
                )?;
            }
            if let Some(monitoring) = request.monitoring {
                tx.execute(
                    "UPDATE tasks SET monitoring = ?1 WHERE id = ?2",
                    params![i64::from(monitoring), task_id],
                )?;
            }
            if request.instruction.is_some() || request.title.is_some() {
                let (title, instruction) = contract::check_text(
                    request.title.as_deref().or(Some(task.title.as_str())),
                    request.instruction.as_deref().unwrap_or(&task.instruction),
                )
                .map_err(Error::invalid)?;
                tx.execute(
                    "UPDATE tasks SET title = ?1, instruction = ?2 WHERE id = ?3",
                    params![title, instruction, task_id],
                )?;
            }
            tx.execute(
                "UPDATE tasks SET updated_at = ?1 WHERE id = ?2",
                params![now, task_id],
            )?;
            find(tx, bot_id, task_id)
        })
    }

    pub fn delete_task(&self, bot_id: &str, task_id: &str) -> Result<()> {
        self.write(|tx, _changes| {
            find(tx, bot_id, task_id)?;
            tx.execute("DELETE FROM tasks WHERE id = ?1", [task_id])?;
            Ok(())
        })
    }

    /// Fires every enabled task whose next run has come at `now` (epoch microseconds), writing
    /// one trigger entry each in the person's conversation with its bot or in the group the task
    /// was scheduled in. A task whose bot left that group is paused instead. Returns the tasks
    /// this call fired. Safe to call from any number of ticks at once.
    pub fn fire_due_tasks(&self, now: i64) -> Result<Vec<BotTask>> {
        self.write(|tx, changes| {
            let due: Vec<(BotTask, i64)> = {
                let mut statement = tx.prepare_cached(&format!(
                    "SELECT {COLUMNS} FROM {TASKS} WHERE t.enabled = 1 AND t.next_run_at <= ?1
                     ORDER BY t.next_run_at, t.id"
                ))?;
                statement
                    .query_map([now], read_task)?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                    .into_iter()
                    .filter_map(|stored| {
                        let due = stored.task.next_run_at.as_deref().and_then(micros_of)?;
                        Some((stored.task, due))
                    })
                    .collect()
            };
            let mut fired = Vec::new();
            for (task, due) in due {
                if !holds_capability(tx, &task.bot_id)? {
                    continue;
                }
                if let Some(group) = &task.conversation
                    && !in_group(tx, &group.id, &task.bot_id)?
                {
                    pause(tx, &task, due, now)?;
                    continue;
                }
                if !claim(tx, &task, due, now)? {
                    continue;
                }
                let message_id = fire_message_id(&task.id, instant(due));
                let body = trigger_for(&task);
                match &task.conversation {
                    Some(group) => {
                        write_group_trigger(
                            tx,
                            changes,
                            &group.id,
                            &task.bot_id,
                            &message_id,
                            &body,
                        )?;
                    }
                    None => {
                        write_trigger(tx, changes, &task.bot_id, &message_id, &body)?;
                    }
                }
                fired.push(find(tx, &task.bot_id, &task.id)?);
            }
            Ok(fired)
        })
    }
}

//! Scheduled tasks of local bots. A due task is claimed with a compare-and-set on `next_run_at`
//! in the transaction that writes its `trigger` entry, so a run is written once however many
//! ticks race for it; a run missed while the app was closed fires once at the next tick and the
//! task's next run is the first one after that. A task the bot scheduled in a group fires there,
//! naming the bot so only it wakes, and is paused once the bot left the group.

use crate::{
    InboxEntry, LOCAL_OWNER_ID, LocalBotStore,
    bots::{bot_exists, bot_not_found},
    error::{Error, Result},
    inbox::{existing_direct, in_group, message_with_changes, write_group_trigger, write_trigger},
    store::{now_micros, timestamp},
};
use chrono::{DateTime, TimeZone, Utc};
use flow_like_types::{
    bot_task::{
        self as contract, BotTask, CreateBotTask, MAX_TASKS_PER_BOT, TaskConversation,
        TaskCreator, TaskKind, TaskSchedule, UpdateBotTask, fire_message_id,
    },
    bot_turn::turn_id,
    create_id,
    inbox::{
        ConversationKind, InboxEntryKind, TRIGGER_TEXT_MAX_CHARS, TRIGGER_TITLE_MAX_CHARS,
        TriggerBody, TriggerSource,
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

fn lookup(db: &Connection, bot_id: &str, task_id: &str) -> Result<Option<BotTask>> {
    Ok(db
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM {TASKS} WHERE t.id = ?1 AND t.bot_id = ?2"
        ))?
        .query_row(params![task_id, bot_id], read_task)
        .optional()?
        .map(|stored| stored.task))
}

fn find(db: &Connection, bot_id: &str, task_id: &str) -> Result<BotTask> {
    lookup(db, bot_id, task_id)?.ok_or_else(|| task_not_found(task_id))
}

/// The group a turn in `conversation_id` limits a bot's task tools to. Any group counts, also
/// one the bot has left; only a turn in the person's own chat with the bot reaches every task.
fn scope_group(db: &Connection, conversation_id: Option<&str>) -> Result<Option<String>> {
    let Some(id) = conversation_id else {
        return Ok(None);
    };
    let kind: Option<String> = db
        .prepare_cached("SELECT kind FROM conversations WHERE id = ?1")?
        .query_row([id], |row| row.get(0))
        .optional()?;
    let is_group = kind.as_deref().and_then(ConversationKind::parse) == Some(ConversationKind::Group);
    Ok(is_group.then(|| id.to_string()))
}

/// A task outside the group answers like a task that does not exist.
fn find_in_scope(
    db: &Connection,
    bot_id: &str,
    task_id: &str,
    group: Option<&str>,
) -> Result<BotTask> {
    let task = find(db, bot_id, task_id)?;
    match group {
        Some(group) if task.conversation.as_ref().map(|c| c.id.as_str()) != Some(group) => {
            Err(task_not_found(task_id))
        }
        _ => Ok(task),
    }
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

/// The turn a bot's task tool runs in: its conversation, and its id, which the host derives from
/// the bot and the message that started the turn (`bot_turn::turn_id`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ToolTurn<'a> {
    pub conversation_id: Option<&'a str>,
    pub turn_id: Option<&'a str>,
}

/// The person's message that asked the bot for a task, in their own chat with the bot.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskRequest {
    pub conversation_id: String,
    pub message_id: String,
    /// The message and the person's edits and deletion of it, oldest first, as they are now.
    pub entries: Vec<InboxEntry>,
}

/// What a task's runs may act on: an instruction the person wrote in the app, or the message in
/// which they asked the bot for the task. Tasks from before this was recorded have neither.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskAuthority {
    pub task: BotTask,
    pub owner_written: bool,
    pub request: Option<TaskRequest>,
}

/// Who writes a task's instruction: the person in the app, or the bot in a turn.
#[derive(Debug, Clone, Copy)]
enum Writer<'a> {
    Owner,
    Bot(ToolTurn<'a>),
}

/// A writer with what it proves: the bot carries the person's message that started its turn,
/// when the person prompted it in their own chat with the bot.
enum Author {
    Owner,
    Bot(Option<(String, String)>),
}

impl Writer<'_> {
    fn author(self, db: &Connection, bot_id: &str) -> Result<Author> {
        Ok(match self {
            Self::Owner => Author::Owner,
            Self::Bot(turn) => Author::Bot(owner_request(db, bot_id, turn)?),
        })
    }
}

impl Author {
    /// Whether a change gives the task this author's authority: the person sending an
    /// instruction, or the bot changing it.
    fn rewrites(&self, sent: Option<&str>, before: &str, after: &str) -> bool {
        match self {
            Self::Owner => sent.is_some(),
            Self::Bot(_) => before != after,
        }
    }

    fn mark(&self, db: &Connection, task_id: &str) -> Result<()> {
        let (written, request) = match self {
            Self::Owner => (true, None),
            Self::Bot(request) => (false, request.as_ref()),
        };
        db.execute(
            "UPDATE tasks SET owner_written = ?1, request_conversation_id = ?2,
                 request_message_id = ?3 WHERE id = ?4",
            params![
                i64::from(written),
                request.map(|(conversation, _)| conversation),
                request.map(|(_, message)| message),
                task_id
            ],
        )?;
        Ok(())
    }
}

/// The person's message that started `turn`, when it is a turn of `bot_id` in the person's own
/// chat with it that a message or call line of theirs started. Triggers of task runs, mail and
/// webhooks are never the person's message.
fn owner_request(
    db: &Connection,
    bot_id: &str,
    turn: ToolTurn,
) -> Result<Option<(String, String)>> {
    let (Some(conversation_id), Some(turn)) = (turn.conversation_id, turn.turn_id) else {
        return Ok(None);
    };
    if existing_direct(db, bot_id)?.as_deref() != Some(conversation_id) {
        return Ok(None);
    }
    let mut statement = db.prepare_cached(
        "SELECT message_id FROM entries
         WHERE conversation_id = ?1 AND sender_id = ?2 AND kind IN (?3, ?4)
         ORDER BY seq DESC, created_at DESC",
    )?;
    let messages = statement.query_map(
        params![
            conversation_id,
            LOCAL_OWNER_ID,
            InboxEntryKind::Message.as_str(),
            InboxEntryKind::Transcript.as_str()
        ],
        |row| row.get::<_, String>(0),
    )?;
    for message in messages {
        let message = message?;
        if turn_id(bot_id, &message) == turn {
            return Ok(Some((conversation_id.to_string(), message)));
        }
    }
    Ok(None)
}

impl LocalBotStore {
    /// The bot's tasks, soonest first.
    pub fn tasks(&self, bot_id: &str) -> Result<Vec<BotTask>> {
        self.tasks_in_turn(bot_id, None)
    }

    /// The tasks a bot's tool sees in a turn in `conversation_id`: those of that group when it is
    /// a group, all of the bot's tasks in the person's own chat.
    pub fn tasks_in_turn(
        &self,
        bot_id: &str,
        conversation_id: Option<&str>,
    ) -> Result<Vec<BotTask>> {
        self.read(|db| {
            if !bot_exists(db, bot_id)? {
                return Err(bot_not_found(bot_id));
            }
            let group = scope_group(db, conversation_id)?;
            let mut statement = db.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM {TASKS} WHERE t.bot_id = ?1
                 AND (?2 IS NULL OR t.conversation_id = ?2)
                 ORDER BY t.next_run_at IS NULL, t.next_run_at, t.created_at, t.id"
            ))?;
            let rows = statement
                .query_map(params![bot_id, group], read_task)?
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
    /// A task the person creates carries their authority; one a bot creates here carries none.
    pub fn create_task_in(
        &self,
        bot_id: &str,
        request: CreateBotTask,
        created_by: TaskCreator,
        conversation_id: Option<&str>,
    ) -> Result<BotTask> {
        let writer = match created_by {
            TaskCreator::Owner => Writer::Owner,
            TaskCreator::Bot => Writer::Bot(ToolTurn {
                conversation_id,
                turn_id: None,
            }),
        };
        self.create(bot_id, request, writer, conversation_id)
    }

    /// Saves a task the bot schedules in `turn`. It fires in the turn's group, if it runs in one,
    /// and keeps the person's message that started the turn when they prompted it in their own
    /// chat with the bot; the task's runs act on that message, read again then.
    pub fn create_bot_task(
        &self,
        bot_id: &str,
        request: CreateBotTask,
        turn: ToolTurn,
    ) -> Result<BotTask> {
        self.create(bot_id, request, Writer::Bot(turn), turn.conversation_id)
    }

    fn create(
        &self,
        bot_id: &str,
        request: CreateBotTask,
        writer: Writer,
        conversation_id: Option<&str>,
    ) -> Result<BotTask> {
        let created_by = match writer {
            Writer::Owner => TaskCreator::Owner,
            Writer::Bot(_) => TaskCreator::Bot,
        };
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
            writer.author(tx, bot_id)?.mark(tx, &id)?;
            find(tx, bot_id, &id)
        })
    }

    /// The person's change to a task, from the app.
    pub fn update_task(
        &self,
        bot_id: &str,
        task_id: &str,
        request: UpdateBotTask,
    ) -> Result<BotTask> {
        self.update(bot_id, task_id, request, Writer::Owner, None)
    }

    /// Like [`Self::update_task`] for a bot's tool in a turn in `conversation_id`: a task outside
    /// that group is not found, and a rewritten instruction loses its authority.
    pub fn update_task_in_turn(
        &self,
        bot_id: &str,
        task_id: &str,
        request: UpdateBotTask,
        conversation_id: Option<&str>,
    ) -> Result<BotTask> {
        let turn = ToolTurn {
            conversation_id,
            turn_id: None,
        };
        self.update_bot_task(bot_id, task_id, request, turn)
    }

    /// A bot's change to a task in `turn`. Rewriting the instruction moves the task's authority to
    /// the person's message that started the turn, when they prompted it in their own chat with
    /// the bot, and drops it otherwise.
    pub fn update_bot_task(
        &self,
        bot_id: &str,
        task_id: &str,
        request: UpdateBotTask,
        turn: ToolTurn,
    ) -> Result<BotTask> {
        self.update(
            bot_id,
            task_id,
            request,
            Writer::Bot(turn),
            turn.conversation_id,
        )
    }

    fn update(
        &self,
        bot_id: &str,
        task_id: &str,
        request: UpdateBotTask,
        writer: Writer,
        conversation_id: Option<&str>,
    ) -> Result<BotTask> {
        self.write(|tx, _changes| {
            let group = scope_group(tx, conversation_id)?;
            let task = find_in_scope(tx, bot_id, task_id, group.as_deref())?;
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
                let author = writer.author(tx, bot_id)?;
                if author.rewrites(
                    request.instruction.as_deref(),
                    &task.instruction,
                    &instruction,
                ) {
                    author.mark(tx, task_id)?;
                }
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

    /// What task `task_id`'s runs may act on, read now; `None` once the task is gone.
    pub fn task_authority(&self, bot_id: &str, task_id: &str) -> Result<Option<TaskAuthority>> {
        self.read(|db| {
            let Some(task) = lookup(db, bot_id, task_id)? else {
                return Ok(None);
            };
            let (owner_written, conversation_id, message_id): (i64, Option<String>, Option<String>) =
                db.query_row(
                    "SELECT owner_written, request_conversation_id, request_message_id
                     FROM tasks WHERE id = ?1",
                    [task_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
            let request = match (conversation_id, message_id) {
                (Some(conversation_id), Some(message_id)) => Some(TaskRequest {
                    entries: message_with_changes(db, &conversation_id, &message_id)?,
                    conversation_id,
                    message_id,
                }),
                _ => None,
            };
            Ok(Some(TaskAuthority {
                task,
                owner_written: owner_written != 0,
                request,
            }))
        })
    }

    pub fn delete_task(&self, bot_id: &str, task_id: &str) -> Result<()> {
        self.delete_task_in_turn(bot_id, task_id, None)
    }

    /// Like [`Self::delete_task`] for a bot's tool in a turn in `conversation_id`.
    pub fn delete_task_in_turn(
        &self,
        bot_id: &str,
        task_id: &str,
        conversation_id: Option<&str>,
    ) -> Result<()> {
        self.write(|tx, _changes| {
            let group = scope_group(tx, conversation_id)?;
            find_in_scope(tx, bot_id, task_id, group.as_deref())?;
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

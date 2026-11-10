//! The `schedule` tools of the `scheduling` capability: a bot schedules work for itself, once
//! ("remind me tomorrow at 9", "check the report in 2 hours") or on a cron schedule in a time
//! zone. A task that fires wakes the bot with its instruction and the answer lands in the chat
//! with its owner. The host keeps the tasks behind [`Scheduler`].

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use flow_like_types::Result;
use flow_like_types::bot_task::{
    BotTask, CreateBotTask, MIN_CRON_INTERVAL_SECS, TaskKind, TaskSchedule, UpdateBotTask,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::outcome::ToolOutput;
use crate::spec::{Approval, ToolSpec, schema_of};

pub const SCHEDULE_TASK: &str = "schedule_task";
pub const LIST_TASKS: &str = "list_tasks";
pub const CANCEL_TASK: &str = "cancel_task";
pub const UPDATE_TASK: &str = "update_task";

const CAPABILITY: &str = "scheduling";
const MAX_RELATIVE_MINUTES: i64 = 60 * 24 * 365;

/// The bot's own tasks on this host.
#[async_trait]
pub trait Scheduler: Send + Sync {
    /// Saves a task made by the bot; the host enforces the per-bot cap.
    async fn create(&self, request: CreateBotTask) -> Result<BotTask>;
    async fn list(&self) -> Result<Vec<BotTask>>;
    /// `false` when the bot has no such task.
    async fn cancel(&self, task_id: &str) -> Result<bool>;
    /// Changes what is given and keeps the rest; `None` when the bot has no such task.
    async fn update(&self, _task_id: &str, _request: UpdateBotTask) -> Result<Option<BotTask>> {
        Err(flow_like_types::anyhow!(
            "Changing a scheduled task is not available on this host"
        ))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleTaskArgs {
    /// Check for meaningful changes and stay quiet when none are found. Use only for monitoring, never for a reminder or a requested report.
    #[serde(default)]
    pub monitoring: bool,
    /// What to do when the task fires, written for yourself: the instruction is the only thing you
    /// get then, so include the topic, relevant context and the next action. Name the file, site
    /// or person it concerns, and any memory pointers needed to resume. At most 4,000 characters.
    pub instruction: String,
    /// A short name shown to the owner. Defaults to the start of the instruction.
    #[serde(default)]
    pub title: Option<String>,
    /// One-time task at this moment: an RFC 3339 time with the offset, e.g.
    /// `2026-11-04T09:00:00+01:00`. Must be in the future.
    #[serde(default)]
    pub at: Option<String>,
    /// One-time task this many minutes from now (up to a year), e.g. 120 for "in 2 hours".
    #[serde(default)]
    pub in_minutes: Option<i64>,
    /// Recurring task: five fields `minute hour day-of-month month day-of-week`, e.g.
    /// `30 8 * * 1-5` for weekdays at 08:30. Day-of-week is 0-6 (0 = Sunday) or SUN-SAT. At most
    /// every 5 minutes.
    #[serde(default)]
    pub cron: Option<String>,
    /// IANA time zone of `cron`, e.g. `Europe/Berlin`. Defaults to UTC.
    #[serde(default)]
    pub tz: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListTasksArgs {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CancelTaskArgs {
    /// The `id` that `list_tasks` or `schedule_task` returned.
    pub task_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateTaskArgs {
    /// The `id` that `list_tasks` or `schedule_task` returned.
    pub task_id: String,
    /// Move a one-time task to this moment: an RFC 3339 time with the offset. Must be in the future.
    #[serde(default)]
    pub at: Option<String>,
    /// Move a one-time task to this many minutes from now (up to a year).
    #[serde(default)]
    pub in_minutes: Option<i64>,
    /// New cron expression for a recurring task: five fields, at most every 5 minutes.
    #[serde(default)]
    pub cron: Option<String>,
    /// New IANA time zone of the task's `cron`.
    #[serde(default)]
    pub tz: Option<String>,
    /// New instruction, written for yourself, at most 4,000 characters.
    #[serde(default)]
    pub instruction: Option<String>,
    /// New short name shown to the owner.
    #[serde(default)]
    pub title: Option<String>,
    /// `false` pauses the task, `true` resumes it.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Whether the task stays quiet when it finds nothing new.
    #[serde(default)]
    pub monitoring: Option<bool>,
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: SCHEDULE_TASK,
            description: "Schedule work for yourself. Proactively set a one-time follow-up when an ongoing topic has a useful next action at a known future time, such as checking an expected reply or revisiting a deadline. Check `list_tasks` first to avoid duplicates and respect the owner's reminder preferences. Give exactly one of `at` (an RFC 3339 time with offset, resolved in the owner's time zone), `in_minutes` (relative, for \"in 2 hours\") or `cron` (recurring, with `tz`). Prefer `at` or `in_minutes` for follow-ups; use `cron` only for an established recurring need. When it fires you get the instruction as a message from the owner and answer in their chat. Include enough context to resume and confirm a reminder only after it is saved. A recurring task runs at most every 5 minutes; a bot keeps at most 50 tasks.",
            capability: Some(CAPABILITY),
            module: "schedule",
            approval: Approval::None,
            approval_args: &[],
            mutating: true,
            input_schema: schema_of::<ScheduleTaskArgs>(),
        },
        ToolSpec {
            name: LIST_TASKS,
            description: "List your scheduled tasks with their next run time, soonest first.",
            capability: Some(CAPABILITY),
            module: "schedule",
            approval: Approval::None,
            approval_args: &[],
            mutating: false,
            input_schema: schema_of::<ListTasksArgs>(),
        },
        ToolSpec {
            name: CANCEL_TASK,
            description: "Cancel one of your scheduled tasks by its id; it never fires again.",
            capability: Some(CAPABILITY),
            module: "schedule",
            approval: Approval::None,
            approval_args: &[],
            mutating: true,
            input_schema: schema_of::<CancelTaskArgs>(),
        },
        ToolSpec {
            name: UPDATE_TASK,
            description: "Change one of your scheduled tasks by its id without cancelling it: move it (`at`, `in_minutes` or `cron` with `tz`), rewrite its `instruction` or `title`, or pause and resume it with `enabled`. Give at least one change; what you leave out stays. The new instruction is what you get when it fires, so write it with enough context to resume. Check `list_tasks` for the id and the current values.",
            capability: Some(CAPABILITY),
            module: "schedule",
            approval: Approval::None,
            approval_args: &[],
            mutating: true,
            input_schema: schema_of::<UpdateTaskArgs>(),
        },
    ]
}

fn scheduler(env: &dyn ToolEnv) -> ToolResult<Arc<dyn Scheduler>> {
    env.scheduler().ok_or_else(|| {
        ToolError::new(
            code::FAILED,
            "This host cannot keep scheduled tasks for the bot",
        )
    })
}

fn blank(value: &Option<String>) -> bool {
    value.as_deref().is_none_or(|value| value.trim().is_empty())
}

/// The request a `schedule_task` call stands for, checked the way the host checks it so the model
/// reads the reason at once.
pub fn request_from(args: ScheduleTaskArgs) -> ToolResult<CreateBotTask> {
    let given = [
        !blank(&args.at),
        args.in_minutes.is_some(),
        !blank(&args.cron),
    ];
    if given.iter().filter(|given| **given).count() != 1 {
        return Err(ToolError::invalid(
            "Give exactly one of 'at', 'in_minutes' or 'cron'",
        ));
    }
    let now = Utc::now();
    let (kind, at) = match (&args.in_minutes, blank(&args.cron)) {
        (Some(minutes), _) => {
            if !(1..=MAX_RELATIVE_MINUTES).contains(minutes) {
                return Err(ToolError::invalid(format!(
                    "'in_minutes' must be between 1 and {MAX_RELATIVE_MINUTES}; got {minutes}"
                )));
            }
            (
                TaskKind::Once,
                Some((now + Duration::minutes(*minutes)).to_rfc3339()),
            )
        }
        (None, true) => (TaskKind::Once, args.at.clone()),
        (None, false) => (TaskKind::Cron, None),
    };
    TaskSchedule::parse(
        kind,
        at.as_deref(),
        args.cron.as_deref(),
        args.tz.as_deref(),
        now,
    )
    .map_err(ToolError::invalid)?;
    flow_like_types::bot_task::check_text(args.title.as_deref(), &args.instruction)
        .map_err(ToolError::invalid)?;
    Ok(CreateBotTask {
        kind: Some(kind),
        at,
        cron: args.cron,
        tz: args.tz,
        instruction: args.instruction,
        title: args.title,
        enabled: None,
        monitoring: args.monitoring,
    })
}

/// The task id and the change an `update_task` call stands for, checked the way the host checks
/// it. A time zone alone is checked by the host, which knows the task's cron.
pub fn update_from(args: UpdateTaskArgs) -> ToolResult<(String, UpdateBotTask)> {
    let task_id = args.task_id.trim().to_string();
    if task_id.is_empty() {
        return Err(ToolError::invalid("'task_id' is empty"));
    }
    let moves = [
        !blank(&args.at),
        args.in_minutes.is_some(),
        !blank(&args.cron),
    ];
    if moves.iter().filter(|given| **given).count() > 1 {
        return Err(ToolError::invalid(
            "Give at most one of 'at', 'in_minutes' or 'cron'",
        ));
    }
    let now = Utc::now();
    let mut at = args.at.filter(|at| !at.trim().is_empty());
    if let Some(minutes) = args.in_minutes {
        if !(1..=MAX_RELATIVE_MINUTES).contains(&minutes) {
            return Err(ToolError::invalid(format!(
                "'in_minutes' must be between 1 and {MAX_RELATIVE_MINUTES}; got {minutes}"
            )));
        }
        at = Some((now + Duration::minutes(minutes)).to_rfc3339());
    }
    let cron = args.cron.filter(|cron| !cron.trim().is_empty());
    if at.is_some() {
        TaskSchedule::parse(TaskKind::Once, at.as_deref(), None, None, now)
            .map_err(ToolError::invalid)?;
    } else if cron.is_some() {
        TaskSchedule::parse(
            TaskKind::Cron,
            None,
            cron.as_deref(),
            args.tz.as_deref(),
            now,
        )
        .map_err(ToolError::invalid)?;
    }
    if let Some(instruction) = &args.instruction {
        flow_like_types::bot_task::check_text(args.title.as_deref(), instruction)
            .map_err(ToolError::invalid)?;
    }
    let request = UpdateBotTask {
        at,
        cron,
        tz: args.tz.filter(|tz| !tz.trim().is_empty()),
        instruction: args.instruction,
        title: args.title,
        enabled: args.enabled,
        monitoring: args.monitoring,
    };
    if request == UpdateBotTask::default() {
        return Err(ToolError::invalid(
            "Give at least one change: 'at', 'in_minutes', 'cron', 'tz', 'instruction', 'title', 'enabled' or 'monitoring'",
        ));
    }
    Ok((task_id, request))
}

fn task_json(task: &BotTask) -> serde_json::Value {
    json!({
        "id": task.id,
        "title": task.title,
        "monitoring": task.monitoring,
        "kind": task.kind,
        "at": task.at,
        "cron": task.cron,
        "tz": task.tz,
        "instruction": task.instruction,
        "next_run_at": task.next_run_at,
        "last_run_at": task.last_run_at,
        "enabled": task.enabled,
        "created_by": task.created_by,
    })
}

pub async fn schedule_task(env: &dyn ToolEnv, args: ScheduleTaskArgs) -> ToolResult<ToolOutput> {
    let scheduler = scheduler(env)?;
    let request = request_from(args)?;
    let task = scheduler
        .create(request)
        .await
        .map_err(|error| ToolError::new(code::FAILED, error.to_string()))?;
    Ok(ToolOutput::new(json!({
        "scheduled": true,
        "task": task_json(&task),
        "min_cron_interval_minutes": MIN_CRON_INTERVAL_SECS / 60,
    })))
}

pub async fn list_tasks(env: &dyn ToolEnv, _args: ListTasksArgs) -> ToolResult<ToolOutput> {
    let tasks = scheduler(env)?
        .list()
        .await
        .map_err(|error| ToolError::new(code::FAILED, error.to_string()))?;
    Ok(ToolOutput::new(json!({
        "count": tasks.len(),
        "tasks": tasks.iter().map(task_json).collect::<Vec<_>>(),
    })))
}

pub async fn cancel_task(env: &dyn ToolEnv, args: CancelTaskArgs) -> ToolResult<ToolOutput> {
    let task_id = args.task_id.trim();
    if task_id.is_empty() {
        return Err(ToolError::invalid("'task_id' is empty"));
    }
    let cancelled = scheduler(env)?
        .cancel(task_id)
        .await
        .map_err(|error| ToolError::new(code::FAILED, error.to_string()))?;
    if !cancelled {
        return Err(ToolError::not_found(format!(
            "No task '{task_id}'; call list_tasks for the ids"
        )));
    }
    Ok(ToolOutput::new(
        json!({ "cancelled": true, "task_id": task_id }),
    ))
}

pub async fn update_task(env: &dyn ToolEnv, args: UpdateTaskArgs) -> ToolResult<ToolOutput> {
    let (task_id, request) = update_from(args)?;
    let task = scheduler(env)?
        .update(&task_id, request)
        .await
        .map_err(|error| ToolError::new(code::FAILED, error.to_string()))?
        .ok_or_else(|| {
            ToolError::not_found(format!("No task '{task_id}'; call list_tasks for the ids"))
        })?;
    Ok(ToolOutput::new(json!({
        "updated": true,
        "task": task_json(&task),
    })))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::testing::FakeEnv;
    use flow_like_types::bot_task::TaskCreator;
    use serde_json::{Value, json};

    #[derive(Default)]
    struct FakeScheduler {
        tasks: Mutex<Vec<BotTask>>,
        created: Mutex<Vec<CreateBotTask>>,
        updated: Mutex<Vec<(String, UpdateBotTask)>>,
    }

    #[async_trait]
    impl Scheduler for FakeScheduler {
        async fn create(&self, request: CreateBotTask) -> Result<BotTask> {
            self.created.lock().unwrap().push(request.clone());
            let task = BotTask {
                id: format!("t{}", self.tasks.lock().unwrap().len() + 1),
                bot_id: "bot".into(),
                owner_id: "owner".into(),
                kind: request.kind.unwrap(),
                at: request.at.clone(),
                cron: request.cron.clone(),
                tz: request.tz.clone(),
                instruction: request.instruction.clone(),
                title: request.title.clone().unwrap_or_default(),
                next_run_at: request.at,
                last_run_at: None,
                enabled: true,
                monitoring: request.monitoring,
                created_by: TaskCreator::Bot,
                created_at: Utc::now().to_rfc3339(),
            };
            self.tasks.lock().unwrap().push(task.clone());
            Ok(task)
        }

        async fn list(&self) -> Result<Vec<BotTask>> {
            Ok(self.tasks.lock().unwrap().clone())
        }

        async fn update(&self, task_id: &str, request: UpdateBotTask) -> Result<Option<BotTask>> {
            self.updated
                .lock()
                .unwrap()
                .push((task_id.to_string(), request.clone()));
            let mut tasks = self.tasks.lock().unwrap();
            let Some(task) = tasks.iter_mut().find(|task| task.id == task_id) else {
                return Ok(None);
            };
            if let Some(instruction) = request.instruction {
                task.instruction = instruction;
            }
            if let Some(enabled) = request.enabled {
                task.enabled = enabled;
            }
            Ok(Some(task.clone()))
        }

        async fn cancel(&self, task_id: &str) -> Result<bool> {
            let mut tasks = self.tasks.lock().unwrap();
            let before = tasks.len();
            tasks.retain(|task| task.id != task_id);
            Ok(tasks.len() < before)
        }
    }

    async fn env_with(scheduler: Arc<FakeScheduler>) -> FakeEnv {
        let env = FakeEnv::new().await;
        *env.scheduler.lock().unwrap() = Some(scheduler);
        env
    }

    async fn call(env: &FakeEnv, tool: &str, args: Value) -> crate::ToolOutcome {
        crate::run(tool, args, env).await
    }

    #[test]
    fn the_specs_need_the_scheduling_capability() {
        for name in [SCHEDULE_TASK, LIST_TASKS, CANCEL_TASK, UPDATE_TASK] {
            let spec = crate::spec(name).expect("registered");
            assert_eq!(spec.capability, Some("scheduling"), "{name}");
        }
        assert!(!crate::spec(LIST_TASKS).unwrap().mutating);
    }

    #[tokio::test]
    async fn a_relative_time_becomes_a_future_once_task() {
        let scheduler = Arc::new(FakeScheduler::default());
        let env = env_with(scheduler.clone()).await;
        let outcome = call(
            &env,
            SCHEDULE_TASK,
            json!({ "instruction": "Check the report", "in_minutes": 120 }),
        )
        .await;
        assert_eq!(outcome.status, crate::Status::Ok, "{:?}", outcome.error);
        let created = scheduler.created.lock().unwrap();
        assert_eq!(created[0].kind, Some(TaskKind::Once));
        let at = chrono::DateTime::parse_from_rfc3339(created[0].at.as_deref().unwrap()).unwrap();
        let wait = at.signed_duration_since(Utc::now()).num_minutes();
        assert!((118..=120).contains(&wait), "{wait}");
        assert_eq!(outcome.result["scheduled"], true);
    }

    #[tokio::test]
    async fn a_cron_task_keeps_its_time_zone() {
        let scheduler = Arc::new(FakeScheduler::default());
        let env = env_with(scheduler.clone()).await;
        let outcome = call(
            &env,
            SCHEDULE_TASK,
            json!({ "instruction": "Morning brief", "cron": "30 8 * * 1-5", "tz": "Europe/Berlin" }),
        )
        .await;
        assert_eq!(outcome.status, crate::Status::Ok, "{:?}", outcome.error);
        let created = scheduler.created.lock().unwrap();
        assert_eq!(created[0].kind, Some(TaskKind::Cron));
        assert_eq!(created[0].tz.as_deref(), Some("Europe/Berlin"));
    }

    #[tokio::test]
    async fn bad_schedules_never_reach_the_host() {
        let scheduler = Arc::new(FakeScheduler::default());
        let env = env_with(scheduler.clone()).await;
        for args in [
            json!({ "instruction": "x" }),
            json!({ "instruction": "x", "at": "2026-11-04T09:00:00Z", "in_minutes": 5 }),
            json!({ "instruction": "x", "at": "2001-01-01T00:00:00Z" }),
            json!({ "instruction": "x", "at": "tomorrow at nine" }),
            json!({ "instruction": "x", "in_minutes": 0 }),
            json!({ "instruction": "x", "cron": "* * * * *" }),
            json!({ "instruction": "x", "cron": "0 9 * * *", "tz": "Mars/Base" }),
            json!({ "instruction": "  ", "in_minutes": 5 }),
        ] {
            let outcome = call(&env, SCHEDULE_TASK, args.clone()).await;
            assert_eq!(
                outcome.error.expect("refused").code,
                code::INVALID_ARGS,
                "{args}"
            );
        }
        assert!(scheduler.created.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn tasks_list_and_cancel_by_id() {
        let scheduler = Arc::new(FakeScheduler::default());
        let env = env_with(scheduler.clone()).await;
        call(
            &env,
            SCHEDULE_TASK,
            json!({ "instruction": "a", "in_minutes": 5 }),
        )
        .await;
        let listed = call(&env, LIST_TASKS, json!({})).await;
        assert_eq!(listed.result["count"], 1);
        let id = listed.result["tasks"][0]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let missing = call(&env, CANCEL_TASK, json!({ "task_id": "nope" })).await;
        assert_eq!(missing.error.expect("refused").code, code::NOT_FOUND);
        let cancelled = call(&env, CANCEL_TASK, json!({ "task_id": id })).await;
        assert_eq!(cancelled.result["cancelled"], true);
        assert_eq!(call(&env, LIST_TASKS, json!({})).await.result["count"], 0);
    }

    #[tokio::test]
    async fn a_task_is_changed_by_id_and_bad_changes_never_reach_the_host() {
        let scheduler = Arc::new(FakeScheduler::default());
        let env = env_with(scheduler.clone()).await;
        call(
            &env,
            SCHEDULE_TASK,
            json!({ "instruction": "a", "in_minutes": 5 }),
        )
        .await;
        for args in [
            json!({ "task_id": "t1" }),
            json!({ "task_id": " " , "enabled": false }),
            json!({ "task_id": "t1", "at": "2026-11-04T09:00:00Z", "in_minutes": 5 }),
            json!({ "task_id": "t1", "at": "2001-01-01T00:00:00Z" }),
            json!({ "task_id": "t1", "cron": "* * * * *" }),
            json!({ "task_id": "t1", "instruction": "  " }),
            json!({ "task_id": "t1", "enabled": false, "surprise": 1 }),
        ] {
            let outcome = call(&env, UPDATE_TASK, args.clone()).await;
            assert_eq!(
                outcome.error.expect("refused").code,
                code::INVALID_ARGS,
                "{args}"
            );
        }
        assert!(scheduler.updated.lock().unwrap().is_empty());
        let missing = call(
            &env,
            UPDATE_TASK,
            json!({ "task_id": "nope", "enabled": false }),
        )
        .await;
        assert_eq!(missing.error.expect("refused").code, code::NOT_FOUND);
        let changed = call(
            &env,
            UPDATE_TASK,
            json!({ "task_id": "t1", "instruction": "b", "enabled": false }),
        )
        .await;
        assert_eq!(changed.status, crate::Status::Ok, "{:?}", changed.error);
        assert_eq!(changed.result["task"]["instruction"], "b");
        assert_eq!(changed.result["task"]["enabled"], false);
    }

    #[tokio::test]
    async fn a_host_that_cannot_change_tasks_says_so() {
        struct ReadOnly;
        #[async_trait]
        impl Scheduler for ReadOnly {
            async fn create(&self, _request: CreateBotTask) -> Result<BotTask> {
                unreachable!()
            }
            async fn list(&self) -> Result<Vec<BotTask>> {
                Ok(Vec::new())
            }
            async fn cancel(&self, _task_id: &str) -> Result<bool> {
                Ok(false)
            }
        }
        let env = FakeEnv::new().await;
        *env.scheduler.lock().unwrap() = Some(Arc::new(ReadOnly));
        let outcome = call(
            &env,
            UPDATE_TASK,
            json!({ "task_id": "t1", "enabled": true }),
        )
        .await;
        assert_eq!(outcome.error.expect("failed").code, code::FAILED);
    }

    #[tokio::test]
    async fn a_host_without_a_scheduler_fails_plainly() {
        let env = FakeEnv::new().await;
        let outcome = call(&env, LIST_TASKS, json!({})).await;
        assert_eq!(outcome.error.expect("failed").code, code::FAILED);
    }
}

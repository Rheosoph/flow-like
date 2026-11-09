//! The `schedule` tools of the `scheduling` capability: a bot schedules work for itself, once
//! ("remind me tomorrow at 9", "check the report in 2 hours") or on a cron schedule in a time
//! zone. A task that fires wakes the bot with its instruction and the answer lands in the chat
//! with its owner. The host keeps the tasks behind [`Scheduler`].

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use flow_like_types::Result;
use flow_like_types::bot_task::{
    BotTask, CreateBotTask, MIN_CRON_INTERVAL_SECS, TaskKind, TaskSchedule,
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
        for name in [SCHEDULE_TASK, LIST_TASKS, CANCEL_TASK] {
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
    async fn a_host_without_a_scheduler_fails_plainly() {
        let env = FakeEnv::new().await;
        let outcome = call(&env, LIST_TASKS, json!({})).await;
        assert_eq!(outcome.error.expect("failed").code, code::FAILED);
    }
}

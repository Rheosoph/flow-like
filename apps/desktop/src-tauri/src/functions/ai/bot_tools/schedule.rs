//! The desktop's `Scheduler`: a local bot's tasks live in the local store, an account bot's in the
//! hub, which fires them (a desktop bot's through its desktop host). A task fires in the group or
//! chat of the turn that scheduled it: the local store takes the turn's group, the hub reads the
//! conversation and the person who asked from the turn it keeps.

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_bot_tools::Scheduler;
use flow_like_local_bots::ToolTurn;
use flow_like_types::{
    Result, anyhow,
    bot_task::{BotTask, CreateBotTask, UpdateBotTask},
};
use reqwest::Method;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::functions::local_bots::{LocalBots, on_store};

/// How the account's tasks are reached: the signed-in session's calls to the hub.
#[async_trait]
pub(crate) trait TaskApi: Send + Sync {
    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> std::result::Result<Value, String>;
}

/// The turn a scheduler serves: the hub's id of an account bot's turn and the conversation it
/// runs in.
#[derive(Debug, Clone, Default)]
pub(crate) struct TaskTurn {
    pub turn_id: Option<String>,
    pub conversation_id: Option<String>,
}

struct LocalScheduler {
    app: AppHandle,
    bot_id: String,
    conversation_id: Option<String>,
    turn_id: Option<String>,
}

impl LocalScheduler {
    async fn on_store<T: Send + 'static>(
        &self,
        work: impl FnOnce(&flow_like_local_bots::LocalBotStore) -> flow_like_local_bots::Result<T>
        + Send
        + 'static,
    ) -> Result<T> {
        let bots = self
            .app
            .try_state::<LocalBots>()
            .ok_or_else(|| anyhow!("Bots on this device are unavailable"))?;
        on_store(&bots, work)
            .await
            .map_err(|error| anyhow!(error.to_string()))
    }
}

#[async_trait]
impl Scheduler for LocalScheduler {
    async fn create(&self, request: CreateBotTask) -> Result<BotTask> {
        let (bot_id, conversation_id, turn_id) = (
            self.bot_id.clone(),
            self.conversation_id.clone(),
            self.turn_id.clone(),
        );
        self.on_store(move |store| {
            let turn = ToolTurn {
                conversation_id: conversation_id.as_deref(),
                turn_id: turn_id.as_deref(),
            };
            store.create_bot_task(&bot_id, request, turn)
        })
        .await
    }

    async fn list(&self) -> Result<Vec<BotTask>> {
        let (bot_id, conversation_id) = (self.bot_id.clone(), self.conversation_id.clone());
        self.on_store(move |store| store.tasks_in_turn(&bot_id, conversation_id.as_deref()))
            .await
    }

    async fn update(&self, task_id: &str, request: UpdateBotTask) -> Result<Option<BotTask>> {
        let (bot_id, task_id, conversation_id, turn_id) = (
            self.bot_id.clone(),
            task_id.to_string(),
            self.conversation_id.clone(),
            self.turn_id.clone(),
        );
        self.on_store(move |store| {
            let scope = conversation_id.as_deref();
            if !store
                .tasks_in_turn(&bot_id, scope)?
                .iter()
                .any(|task| task.id == task_id)
            {
                return Ok(None);
            }
            let turn = ToolTurn {
                conversation_id: scope,
                turn_id: turn_id.as_deref(),
            };
            store
                .update_bot_task(&bot_id, &task_id, request, turn)
                .map(Some)
        })
        .await
    }

    async fn cancel(&self, task_id: &str) -> Result<bool> {
        let (bot_id, task_id, conversation_id) = (
            self.bot_id.clone(),
            task_id.to_string(),
            self.conversation_id.clone(),
        );
        self.on_store(move |store| {
            let scope = conversation_id.as_deref();
            if !store
                .tasks_in_turn(&bot_id, scope)?
                .iter()
                .any(|task| task.id == task_id)
            {
                return Ok(false);
            }
            store
                .delete_task_in_turn(&bot_id, &task_id, scope)
                .map(|()| true)
        })
        .await
    }
}

struct AccountScheduler {
    api: Arc<dyn TaskApi>,
    bot_id: String,
    turn_id: Option<String>,
}

impl AccountScheduler {
    fn base(&self) -> String {
        format!(
            "bots/{}/tasks",
            super::super::native_platform_tools::segment(&self.bot_id)
        )
    }
}

impl AccountScheduler {
    /// The turn a write names. A write always names one, empty when the hub doesn't know the
    /// turn: a write without it is the owner's own, and the owner's instruction is authority.
    fn written_in(&self) -> &str {
        self.turn_id.as_deref().unwrap_or_default()
    }

    fn turn_path(&self, path: String) -> String {
        match &self.turn_id {
            Some(turn_id) => format!(
                "{path}?turn_id={}",
                super::super::native_platform_tools::segment(turn_id)
            ),
            None => path,
        }
    }
}

#[async_trait]
impl Scheduler for AccountScheduler {
    async fn create(&self, request: CreateBotTask) -> Result<BotTask> {
        let body = json!({
            "kind": request.kind,
            "at": request.at,
            "cron": request.cron,
            "tz": request.tz,
            "instruction": request.instruction,
            "title": request.title,
            "monitoring": request.monitoring,
            "turn_id": self.written_in(),
        });
        let created = self
            .api
            .request(Method::POST, &self.base(), Some(&body))
            .await
            .map_err(|error| anyhow!(error))?;
        serde_json::from_value(created)
            .map_err(|error| anyhow!("The hub sent an unusable task: {error}"))
    }

    async fn list(&self) -> Result<Vec<BotTask>> {
        let listed = self
            .api
            .request(Method::GET, &self.turn_path(self.base()), None)
            .await
            .map_err(|error| anyhow!(error))?;
        serde_json::from_value(
            listed
                .get("tasks")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        )
        .map_err(|error| anyhow!("The hub sent unusable tasks: {error}"))
    }

    async fn update(&self, task_id: &str, request: UpdateBotTask) -> Result<Option<BotTask>> {
        if !self.list().await?.iter().any(|task| task.id == task_id) {
            return Ok(None);
        }
        let path = format!(
            "{}/{}",
            self.base(),
            super::super::native_platform_tools::segment(task_id)
        );
        let mut body = serde_json::to_value(&request)
            .map_err(|error| anyhow!("The task change could not be sent: {error}"))?;
        body["turn_id"] = json!(self.written_in());
        let updated = self
            .api
            .request(Method::PATCH, &path, Some(&body))
            .await
            .map_err(|error| anyhow!(error))?;
        serde_json::from_value(updated)
            .map(Some)
            .map_err(|error| anyhow!("The hub sent an unusable task: {error}"))
    }

    async fn cancel(&self, task_id: &str) -> Result<bool> {
        if !self.list().await?.iter().any(|task| task.id == task_id) {
            return Ok(false);
        }
        let path = format!(
            "{}/{}",
            self.base(),
            super::super::native_platform_tools::segment(task_id)
        );
        self.api
            .request(Method::DELETE, &self.turn_path(path), None)
            .await
            .map(|_| true)
            .map_err(|error| anyhow!(error))
    }
}

/// The scheduler for `bot_id` in `turn`: the local store's when this device holds the bot, else
/// the hub's.
pub(crate) async fn scheduler_for(
    app: &AppHandle,
    bot_id: &str,
    api: Arc<dyn TaskApi>,
    turn: TaskTurn,
) -> Arc<dyn Scheduler> {
    let local = match app.try_state::<LocalBots>() {
        Some(bots) => {
            let lookup = bot_id.to_string();
            on_store(&bots, move |store| {
                store.get_bot(&lookup).map(|bot| bot.is_some())
            })
            .await
            .unwrap_or(false)
        }
        None => false,
    };
    if local {
        Arc::new(LocalScheduler {
            app: app.clone(),
            bot_id: bot_id.to_string(),
            conversation_id: turn.conversation_id,
            turn_id: turn.turn_id,
        })
    } else {
        Arc::new(AccountScheduler {
            api,
            bot_id: bot_id.to_string(),
            turn_id: turn.turn_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Recorder(Mutex<Vec<(Method, String, Value)>>);

    #[async_trait]
    impl TaskApi for Recorder {
        async fn request(
            &self,
            method: Method,
            path: &str,
            body: Option<&Value>,
        ) -> std::result::Result<Value, String> {
            self.0
                .lock()
                .unwrap()
                .push((method, path.to_string(), body.cloned().unwrap_or(Value::Null)));
            let task = json!({
                "id": "task1", "bot_id": "bot1", "kind": "once", "instruction": "x",
                "title": "t", "enabled": true, "created_by": "bot",
                "created_at": "2026-11-08T12:00:00Z"
            });
            Ok(json!({ "tasks": [task.clone()], "id": "task1", "bot_id": "bot1", "kind": "once",
                "instruction": "x", "title": "t", "enabled": true, "created_by": "bot",
                "created_at": "2026-11-08T12:00:00Z" }))
        }
    }

    #[tokio::test]
    async fn an_account_scheduler_names_its_turn_on_every_task_call() {
        let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
        let scheduler = AccountScheduler {
            api: recorder.clone(),
            bot_id: "bot1".into(),
            turn_id: Some("turn 1".into()),
        };
        scheduler.list().await.unwrap();
        scheduler
            .update("task1", UpdateBotTask::default())
            .await
            .unwrap();
        scheduler.cancel("task1").await.unwrap();
        let calls = recorder.0.lock().unwrap();
        let sent: Vec<(&str, &str)> = calls
            .iter()
            .map(|(method, path, _)| (method.as_str(), path.as_str()))
            .collect();
        assert_eq!(
            sent,
            [
                ("GET", "bots/bot1/tasks?turn_id=turn%201"),
                ("GET", "bots/bot1/tasks?turn_id=turn%201"),
                ("PATCH", "bots/bot1/tasks/task1"),
                ("GET", "bots/bot1/tasks?turn_id=turn%201"),
                ("DELETE", "bots/bot1/tasks/task1?turn_id=turn%201"),
            ]
        );
        assert_eq!(calls[2].2["turn_id"], "turn 1");
    }

    #[tokio::test]
    async fn a_bots_write_without_a_known_turn_never_passes_as_the_owners() {
        let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
        let scheduler = AccountScheduler {
            api: recorder.clone(),
            bot_id: "bot1".into(),
            turn_id: None,
        };
        scheduler
            .create(CreateBotTask {
                kind: Some(flow_like_types::bot_task::TaskKind::Once),
                instruction: "Mail the summary".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        scheduler
            .update("task1", UpdateBotTask::default())
            .await
            .unwrap();
        let calls = recorder.0.lock().unwrap();
        let writes: Vec<&Value> = calls
            .iter()
            .filter(|(method, _, _)| *method != Method::GET)
            .map(|(_, _, body)| body)
            .collect();
        assert_eq!(writes.len(), 2);
        assert!(writes.iter().all(|body| body["turn_id"] == ""));
    }
}

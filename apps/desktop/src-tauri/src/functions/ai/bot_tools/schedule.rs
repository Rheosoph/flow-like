//! The desktop's `Scheduler`: a local bot's tasks live in the local store, an account bot's in the
//! hub, which fires them (a desktop bot's through its desktop host).

use std::sync::Arc;

use async_trait::async_trait;
use flow_like_bot_tools::Scheduler;
use flow_like_types::{
    Result, anyhow,
    bot_task::{BotTask, CreateBotTask, TaskCreator},
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

struct LocalScheduler {
    app: AppHandle,
    bot_id: String,
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
        let bot_id = self.bot_id.clone();
        self.on_store(move |store| store.create_task(&bot_id, request, TaskCreator::Bot))
            .await
    }

    async fn list(&self) -> Result<Vec<BotTask>> {
        let bot_id = self.bot_id.clone();
        self.on_store(move |store| store.tasks(&bot_id)).await
    }

    async fn cancel(&self, task_id: &str) -> Result<bool> {
        let (bot_id, task_id) = (self.bot_id.clone(), task_id.to_string());
        self.on_store(move |store| {
            if !store.tasks(&bot_id)?.iter().any(|task| task.id == task_id) {
                return Ok(false);
            }
            store.delete_task(&bot_id, &task_id).map(|()| true)
        })
        .await
    }
}

struct AccountScheduler {
    api: Arc<dyn TaskApi>,
    bot_id: String,
}

impl AccountScheduler {
    fn base(&self) -> String {
        format!(
            "bots/{}/tasks",
            super::super::native_platform_tools::segment(&self.bot_id)
        )
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
            .request(Method::GET, &self.base(), None)
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
            .request(Method::DELETE, &path, None)
            .await
            .map(|_| true)
            .map_err(|error| anyhow!(error))
    }
}

/// The scheduler for `bot_id`: the local store's when this device holds the bot, else the hub's.
pub(crate) async fn scheduler_for(
    app: &AppHandle,
    bot_id: &str,
    api: Arc<dyn TaskApi>,
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
        })
    } else {
        Arc::new(AccountScheduler {
            api,
            bot_id: bot_id.to_string(),
        })
    }
}

//! The server's `Scheduler`: a bot's tasks in the `BotTask` table, kept through the same
//! functions the owner's endpoints use, so caps and schedule rules are the API's.

use crate::{
    routes::{
        bots::{lock_bot, tasks},
        inbox::now_millis,
    },
    state::AppState,
};
use async_trait::async_trait;
use flow_like_bot_tools::Scheduler;
use flow_like_types::{
    Result, anyhow,
    bot_task::{BotTask, CreateBotTask, TaskCreator, TaskKind},
};

pub(crate) struct ApiScheduler {
    state: AppState,
    owner_id: String,
    bot_id: String,
}

impl ApiScheduler {
    pub(crate) fn new(state: &AppState, owner_id: &str, bot_id: &str) -> Self {
        Self {
            state: state.clone(),
            owner_id: owner_id.to_string(),
            bot_id: bot_id.to_string(),
        }
    }
}

pub(crate) fn to_task(row: crate::entity::bot_task::Model) -> BotTask {
    BotTask {
        kind: TaskKind::parse(&row.kind).unwrap_or(TaskKind::Once),
        created_by: TaskCreator::parse(&row.created_by).unwrap_or(TaskCreator::Owner),
        id: row.id,
        bot_id: row.bot_id,
        owner_id: row.owner_id,
        at: row.run_at.map(|at| at.to_rfc3339()),
        cron: row.cron,
        tz: row.timezone,
        instruction: row.instruction,
        title: row.title,
        next_run_at: row.next_run_at.map(|at| at.to_rfc3339()),
        last_run_at: row.last_run_at.map(|at| at.to_rfc3339()),
        enabled: row.enabled,
        created_at: row.created_at.to_rfc3339(),
    }
}

#[async_trait]
impl Scheduler for ApiScheduler {
    async fn create(&self, request: CreateBotTask) -> Result<BotTask> {
        let row = self
            .state
            .transaction(|txn| {
                let (owner, bot, request) =
                    (self.owner_id.clone(), self.bot_id.clone(), request.clone());
                Box::pin(async move {
                    lock_bot(txn, &bot).await?;
                    tasks::create_in(txn, &owner, &bot, request, TaskCreator::Bot, now_millis())
                        .await
                })
            })
            .await
            .map_err(|error| anyhow!("{error}"))?;
        Ok(to_task(row))
    }

    async fn list(&self) -> Result<Vec<BotTask>> {
        let rows = tasks::list_in(&self.state.db, &self.owner_id, &self.bot_id)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        Ok(rows.into_iter().map(to_task).collect())
    }

    async fn cancel(&self, task_id: &str) -> Result<bool> {
        match tasks::delete_in(&self.state.db, &self.owner_id, &self.bot_id, task_id).await {
            Ok(()) => Ok(true),
            Err(error) if error.status() == axum::http::StatusCode::NOT_FOUND => Ok(false),
            Err(error) => Err(anyhow!("{error}")),
        }
    }
}

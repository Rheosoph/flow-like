//! The server's `Scheduler`: a bot's tasks in the `BotTask` table, kept through the same
//! functions the owner's endpoints use, so caps and schedule rules are the API's. A task the bot
//! schedules fires in the group or chat of the turn that asked for it, read from the turn the
//! server keeps, never from the tool's arguments; the owner's message that asked for it, which
//! its runs act on, comes from that turn the same way.

use crate::{
    bot_host::{
        task_authority::{TaskEdit, author_of_turn},
        task_schedules,
    },
    entity::bot_task,
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
    bot_task::{BotTask, CreateBotTask, TaskConversation, TaskCreator, TaskKind, UpdateBotTask},
};
use sea_orm::ConnectionTrait;

pub(crate) struct ApiScheduler {
    state: AppState,
    owner_id: String,
    bot_id: String,
    turn_id: String,
}

impl ApiScheduler {
    pub(crate) fn new(state: &AppState, owner_id: &str, bot_id: &str, turn_id: &str) -> Self {
        Self {
            state: state.clone(),
            owner_id: owner_id.to_string(),
            bot_id: bot_id.to_string(),
            turn_id: turn_id.to_string(),
        }
    }
}

impl ApiScheduler {
    async fn scope(&self) -> Result<tasks::TaskScope> {
        tasks::scope_of_turn(&self.state.db, &self.owner_id, &self.bot_id, &self.turn_id)
            .await
            .map_err(|error| anyhow!("{error}"))
    }
}

pub(crate) fn to_task(row: bot_task::Model, conversation: Option<TaskConversation>) -> BotTask {
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
        monitoring: row.monitoring.unwrap_or(false),
        created_at: row.created_at.to_rfc3339(),
        conversation,
    }
}

async fn tasks_of<C: ConnectionTrait>(db: &C, rows: Vec<bot_task::Model>) -> Result<Vec<BotTask>> {
    let known = tasks::conversations_of(db, &rows)
        .await
        .map_err(|error| anyhow!("{error}"))?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let conversation = tasks::conversation_of(&row, &known);
            to_task(row, conversation)
        })
        .collect())
}

async fn task_of<C: ConnectionTrait>(db: &C, row: bot_task::Model) -> Result<BotTask> {
    tasks_of(db, vec![row])
        .await?
        .pop()
        .ok_or_else(|| anyhow!("A saved task could not be read back"))
}

#[async_trait]
impl Scheduler for ApiScheduler {
    async fn create(&self, request: CreateBotTask) -> Result<BotTask> {
        let row = self
            .state
            .transaction(|txn| {
                let (owner, bot, turn, request) = (
                    self.owner_id.clone(),
                    self.bot_id.clone(),
                    self.turn_id.clone(),
                    request.clone(),
                );
                Box::pin(async move {
                    lock_bot(txn, &bot).await?;
                    let origin = tasks::origin_of_turn(txn, &owner, &bot, &turn).await?;
                    let author = author_of_turn(txn, &owner, &bot, Some(turn.as_str())).await?;
                    tasks::create_in(
                        txn,
                        &owner,
                        &bot,
                        request,
                        author,
                        origin.as_ref(),
                        now_millis(),
                    )
                    .await
                })
            })
            .await
            .map_err(|error| anyhow!("{error}"))?;
        task_schedules::sync(&self.state, &row).await;
        task_of(&self.state.db, row).await
    }

    async fn list(&self) -> Result<Vec<BotTask>> {
        let scope = self.scope().await?;
        let rows = tasks::list_scoped_in(&self.state.db, &self.owner_id, &self.bot_id, &scope)
            .await
            .map_err(|error| anyhow!("{error}"))?;
        tasks_of(&self.state.db, rows).await
    }

    async fn update(&self, task_id: &str, request: UpdateBotTask) -> Result<Option<BotTask>> {
        let scope = self.scope().await?;
        let result = self
            .state
            .transaction(|txn| {
                let (owner, bot, task, request, scope, turn) = (
                    self.owner_id.clone(),
                    self.bot_id.clone(),
                    task_id.to_string(),
                    request.clone(),
                    scope.clone(),
                    self.turn_id.clone(),
                );
                Box::pin(async move {
                    lock_bot(txn, &bot).await?;
                    let edit = TaskEdit {
                        change: request,
                        author: author_of_turn(txn, &owner, &bot, Some(turn.as_str())).await?,
                    };
                    tasks::update_scoped_in(
                        txn,
                        &owner,
                        &bot,
                        &task,
                        edit,
                        chrono::Utc::now().fixed_offset(),
                        &scope,
                    )
                    .await
                })
            })
            .await;
        match result {
            Ok(row) => {
                task_schedules::sync(&self.state, &row).await;
                task_of(&self.state.db, row).await.map(Some)
            }
            Err(error) if error.status() == axum::http::StatusCode::NOT_FOUND => Ok(None),
            Err(error) => Err(anyhow!("{error}")),
        }
    }

    async fn cancel(&self, task_id: &str) -> Result<bool> {
        let scope = self.scope().await?;
        let deleted = tasks::delete_scoped_in(
            &self.state.db,
            &self.owner_id,
            &self.bot_id,
            task_id,
            &scope,
        )
        .await;
        match deleted {
            Ok(()) => {
                task_schedules::remove(&self.state, task_id).await;
                Ok(true)
            }
            Err(error) if error.status() == axum::http::StatusCode::NOT_FOUND => Ok(false),
            Err(error) => Err(anyhow!("{error}")),
        }
    }
}

//! Bot tasks: work a bot with the scheduling capability runs for its owner at a time, once or on a
//! cron schedule in a time zone. A task fires by writing a `trigger` entry into the bot's
//! conversation with its owner (see `bot_host::tasks`); the owner lists, pauses, edits and
//! deletes tasks here, and the bot creates its own through the `schedule` tools.

use super::{lock_bot, owned_bot};
use crate::{
    bot_host::{
        task_schedules,
        tasks::{self as firing, SCHEDULING_CAPABILITY},
    },
    entity::{bot, bot_task},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset, Utc};
use flow_like_types::bot_task::{
    self as contract, CreateBotTask, MAX_TASKS_PER_BOT, TaskCreator, TaskKind, TaskSchedule,
    UpdateBotTask,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub(crate) const TASK_CAPABILITY_OFF: &str = "BOT_TASK_CAPABILITY_OFF";
pub(crate) const TASK_LIMIT_REACHED: &str = "BOT_TASK_LIMIT_REACHED";

/// A task as its owner sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct BotTaskInfo {
    pub id: String,
    pub bot_id: String,
    /// `once` or `cron`.
    pub kind: String,
    /// RFC 3339 time of a once task.
    pub at: Option<String>,
    /// Five-field cron expression of a cron task.
    pub cron: Option<String>,
    /// IANA time zone of the cron expression.
    pub tz: Option<String>,
    pub instruction: String,
    pub title: String,
    pub next_run_at: Option<DateTime<FixedOffset>>,
    pub last_run_at: Option<DateTime<FixedOffset>>,
    pub enabled: bool,
    /// `bot` or `owner`.
    pub created_by: String,
    pub created_at: DateTime<FixedOffset>,
}

impl From<bot_task::Model> for BotTaskInfo {
    fn from(row: bot_task::Model) -> Self {
        Self {
            id: row.id,
            bot_id: row.bot_id,
            kind: row.kind.to_ascii_lowercase(),
            at: row.run_at.map(|at| at.to_rfc3339()),
            cron: row.cron,
            tz: row.timezone,
            instruction: row.instruction,
            title: row.title,
            next_run_at: row.next_run_at,
            last_run_at: row.last_run_at,
            enabled: row.enabled,
            created_by: row.created_by.to_ascii_lowercase(),
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotTaskList {
    pub tasks: Vec<BotTaskInfo>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBotTaskRequest {
    /// `once` or `cron`.
    pub kind: String,
    /// RFC 3339 time of a once task, in the future.
    pub at: Option<String>,
    /// Five-field cron expression (minute hour day-of-month month day-of-week), at most every 5
    /// minutes.
    pub cron: Option<String>,
    /// IANA time zone for the cron expression; UTC when missing.
    pub tz: Option<String>,
    /// What the bot does when the task fires, at most 4,000 characters.
    pub instruction: String,
    /// A short name for the task; the start of the instruction when missing.
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateBotTaskRequest {
    pub at: Option<String>,
    pub cron: Option<String>,
    pub tz: Option<String>,
    pub instruction: Option<String>,
    pub title: Option<String>,
    /// Pausing keeps the task; resuming computes its next run from now.
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FireBotTaskResponse {
    /// Whether this call fired the task; false when it was not due or another host fired it.
    pub fired: bool,
}

fn utc(at: DateTime<FixedOffset>) -> DateTime<Utc> {
    at.with_timezone(&Utc)
}

fn bad_request(message: String) -> ApiError {
    ApiError::bad_request(message)
}

fn refuse_without_capability(bot: &bot::Model) -> Result<(), ApiError> {
    if bot
        .capabilities
        .iter()
        .any(|id| id == SCHEDULING_CAPABILITY)
    {
        return Ok(());
    }
    Err(ApiError::coded(
        StatusCode::FORBIDDEN,
        TASK_CAPABILITY_OFF,
        format!(
            "Bot '{}' doesn't have the scheduling capability switched on",
            bot.id
        ),
    ))
}

pub(crate) async fn list_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<Vec<bot_task::Model>, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    Ok(bot_task::Entity::find()
        .filter(bot_task::Column::BotId.eq(bot_id))
        .order_by_asc(bot_task::Column::NextRunAt)
        .order_by_asc(bot_task::Column::CreatedAt)
        .order_by_asc(bot_task::Column::Id)
        .all(db)
        .await?)
}

async fn owned_task<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    task_id: &str,
) -> Result<bot_task::Model, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    bot_task::Entity::find_by_id(task_id)
        .filter(bot_task::Column::BotId.eq(bot_id))
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Task '{task_id}' not found")))
}

/// Inserts the task once the bot is the caller's, holds the capability and has room, and the
/// schedule is valid.
pub(crate) async fn create_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    request: CreateBotTask,
    created_by: TaskCreator,
    now: DateTime<FixedOffset>,
) -> Result<bot_task::Model, ApiError> {
    let bot = owned_bot(db, owner_id, bot_id).await?;
    refuse_without_capability(&bot)?;
    let kind = request
        .kind
        .ok_or_else(|| bad_request("A task needs a kind, once or cron".into()))?;
    let schedule = TaskSchedule::parse(
        kind,
        request.at.as_deref(),
        request.cron.as_deref(),
        request.tz.as_deref(),
        utc(now),
    )
    .map_err(bad_request)?;
    let (title, instruction) = contract::check_text(request.title.as_deref(), &request.instruction)
        .map_err(bad_request)?;
    let count = bot_task::Entity::find()
        .filter(bot_task::Column::BotId.eq(bot_id))
        .count(db)
        .await?;
    if count >= MAX_TASKS_PER_BOT as u64 {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            TASK_LIMIT_REACHED,
            format!("Bot '{bot_id}' already has {MAX_TASKS_PER_BOT} tasks; delete one first"),
        ));
    }
    let (run_at, cron, timezone) = parts_of(&schedule);
    Ok(bot_task::ActiveModel {
        id: Set(format!("task_{}", uuid::Uuid::new_v4().simple())),
        bot_id: Set(bot_id.to_string()),
        owner_id: Set(owner_id.to_string()),
        kind: Set(schedule.kind().stored()),
        run_at: Set(run_at),
        cron: Set(cron),
        timezone: Set(timezone),
        instruction: Set(instruction),
        title: Set(title),
        next_run_at: Set(schedule.first_run(utc(now)).map(|at| at.fixed_offset())),
        last_run_at: Set(None),
        enabled: Set(request.enabled.unwrap_or(true)),
        created_by: Set(created_by.stored()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await?)
}

fn parts_of(
    schedule: &TaskSchedule,
) -> (
    Option<DateTime<FixedOffset>>,
    Option<String>,
    Option<String>,
) {
    match schedule {
        TaskSchedule::Once { at } => (Some(at.fixed_offset()), None, None),
        TaskSchedule::Cron { expr, tz } => (None, Some(expr.clone()), Some(tz.clone())),
    }
}

pub(crate) async fn update_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    task_id: &str,
    request: UpdateBotTask,
    now: DateTime<FixedOffset>,
) -> Result<bot_task::Model, ApiError> {
    let row = owned_task(db, owner_id, bot_id, task_id).await?;
    let kind = TaskKind::parse(&row.kind).unwrap_or(TaskKind::Once);
    let moves = request.at.is_some() || request.cron.is_some() || request.tz.is_some();
    let resumes = request.enabled == Some(true) && !row.enabled;
    let mut active: bot_task::ActiveModel = row.clone().into();
    if moves || resumes {
        let at = request
            .at
            .clone()
            .or_else(|| row.run_at.map(|at| at.to_rfc3339()));
        let cron = request.cron.clone().or_else(|| row.cron.clone());
        let tz = request.tz.clone().or_else(|| row.timezone.clone());
        let schedule = TaskSchedule::parse(
            kind,
            at.as_deref(),
            cron.as_deref(),
            tz.as_deref(),
            utc(now),
        )
        .map_err(bad_request)?;
        let (run_at, cron, timezone) = parts_of(&schedule);
        active.run_at = Set(run_at);
        active.cron = Set(cron);
        active.timezone = Set(timezone);
        let next = schedule.first_run(utc(now)).map(|at| at.fixed_offset());
        active.next_run_at = Set(next);
        let rearmed = kind == TaskKind::Once && row.last_run_at.is_some();
        active.enabled = Set(request.enabled.unwrap_or(row.enabled || rearmed) && next.is_some());
    } else if let Some(enabled) = request.enabled {
        active.enabled = Set(enabled);
    }
    if request.instruction.is_some() || request.title.is_some() {
        let instruction = request.instruction.as_deref().unwrap_or(&row.instruction);
        let title = request.title.as_deref().or(Some(row.title.as_str()));
        let (title, instruction) = contract::check_text(title, instruction).map_err(bad_request)?;
        active.title = Set(title);
        active.instruction = Set(instruction);
    }
    active.updated_at = Set(now);
    Ok(active.update(db).await?)
}

pub(crate) async fn delete_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    task_id: &str,
) -> Result<(), ApiError> {
    owned_task(db, owner_id, bot_id, task_id).await?;
    bot_task::Entity::delete_many()
        .filter(bot_task::Column::Id.eq(task_id))
        .filter(bot_task::Column::BotId.eq(bot_id))
        .exec(db)
        .await?;
    Ok(())
}

fn kind_of(request: &CreateBotTaskRequest) -> Result<TaskKind, ApiError> {
    TaskKind::parse(&request.kind)
        .ok_or_else(|| bad_request(format!("kind '{}' is not once or cron", request.kind)))
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/tasks",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The bot's scheduled tasks, soonest first.", body = BotTaskList),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/tasks", skip(state, user))]
pub async fn list_bot_tasks(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<BotTaskList>, ApiError> {
    let sub = user.sub()?;
    let tasks = list_in(&state.db, &sub, &bot_id)
        .await?
        .into_iter()
        .map(BotTaskInfo::from)
        .collect();
    Ok(Json(BotTaskList { tasks }))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/tasks",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = CreateBotTaskRequest,
    responses(
        (status = 201, description = "A new task: the bot runs its instruction at the scheduled time and answers in your chat.", body = BotTaskInfo),
        (status = 400, description = "The schedule or text is invalid, or a cron runs more often than every 5 minutes"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "BOT_TASK_CAPABILITY_OFF: switch the bot's scheduling capability on first"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_TASK_LIMIT_REACHED: a bot has at most 50 tasks")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/tasks", skip(state, user, body))]
pub async fn create_bot_task(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<CreateBotTaskRequest>,
) -> Result<(StatusCode, Json<BotTaskInfo>), ApiError> {
    let sub = user.sub()?;
    let request = CreateBotTask {
        kind: Some(kind_of(&body)?),
        at: body.at,
        cron: body.cron,
        tz: body.tz,
        instruction: body.instruction,
        title: body.title,
        enabled: None,
    };
    let row = state
        .transaction(|txn| {
            let (sub, bot_id, request) = (sub.clone(), bot_id.clone(), request.clone());
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                let now = Utc::now().fixed_offset();
                create_in(txn, &sub, &bot_id, request, TaskCreator::Owner, now).await
            })
        })
        .await?;
    task_schedules::sync(&state, &row).await;
    Ok((StatusCode::CREATED, Json(row.into())))
}

#[utoipa::path(
    patch,
    path = "/bots/{bot_id}/tasks/{task_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("task_id" = String, Path, description = "The task's id")
    ),
    request_body = UpdateBotTaskRequest,
    responses(
        (status = 200, description = "The updated task. Moving the schedule or resuming a paused task computes its next run from now.", body = BotTaskInfo),
        (status = 400, description = "The schedule or text is invalid"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or task")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PATCH /bots/{bot_id}/tasks/{task_id}", skip(state, user, body))]
pub async fn update_bot_task(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, task_id)): Path<(String, String)>,
    Json(body): Json<UpdateBotTaskRequest>,
) -> Result<Json<BotTaskInfo>, ApiError> {
    let sub = user.sub()?;
    let request = UpdateBotTask {
        at: body.at,
        cron: body.cron,
        tz: body.tz,
        instruction: body.instruction,
        title: body.title,
        enabled: body.enabled,
    };
    let row = update_in(
        &state.db,
        &sub,
        &bot_id,
        &task_id,
        request,
        Utc::now().fixed_offset(),
    )
    .await?;
    task_schedules::sync(&state, &row).await;
    Ok(Json(row.into()))
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/tasks/{task_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("task_id" = String, Path, description = "The task's id")
    ),
    responses(
        (status = 204, description = "The task is gone and never fires"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or task")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bots/{bot_id}/tasks/{task_id}", skip(state, user))]
pub async fn delete_bot_task(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, task_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    delete_in(&state.db, &sub, &bot_id, &task_id).await?;
    task_schedules::remove(&state, &task_id).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/tasks/{task_id}/fire",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("task_id" = String, Path, description = "The task's id")
    ),
    responses(
        (status = 200, description = "A desktop host fires a due task of its bot: the run is claimed once and written into your chat. A task that is not due, paused or already fired answers `fired: false`.", body = FireBotTaskResponse),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or task")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/tasks/{task_id}/fire", skip(state, user))]
pub async fn fire_bot_task(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, task_id)): Path<(String, String)>,
) -> Result<Json<FireBotTaskResponse>, ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    let task = owned_task(&state.db, &sub, &bot_id, &task_id).await?;
    let due = firing::holds_capability(&bot)
        && task.enabled
        && task.next_run_at.is_some_and(|at| at <= Utc::now());
    let fired = due && firing::fire(&state, &task).await?;
    Ok(Json(FireBotTaskResponse { fired }))
}

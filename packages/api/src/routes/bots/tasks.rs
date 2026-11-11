//! Bot tasks: work a bot with the scheduling capability runs for its owner at a time, once or on a
//! cron schedule in a time zone. A task fires by writing a `trigger` entry into the bot's
//! conversation with its owner, or into the group or chat a bot scheduled it in (see
//! `bot_host::tasks`); the owner lists, pauses, edits and deletes tasks here, and the bot creates
//! its own through the `schedule` tools.

use super::{lock_bot, owned_bot};
use crate::{
    bot_host::{
        task_schedules,
        tasks::{self as firing, SCHEDULING_CAPABILITY},
    },
    entity::{bot, bot_task, bot_turn, conversation},
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
use flow_like_types::{
    bot_task::{
        self as contract, CreateBotTask, MAX_TASKS_PER_BOT, TaskConversation, TaskCreator,
        TaskKind, TaskSchedule, UpdateBotTask,
    },
    inbox::{ConversationEncryption, ConversationKind},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

pub(crate) const TASK_CAPABILITY_OFF: &str = "BOT_TASK_CAPABILITY_OFF";
pub(crate) const TASK_LIMIT_REACHED: &str = "BOT_TASK_LIMIT_REACHED";
pub(crate) const TASK_CONVERSATION_ENCRYPTED: &str = "BOT_TASK_CONVERSATION_ENCRYPTED";

/// Where a task a bot schedules in a turn fires: that turn's group or chat, sent as the person
/// who asked, so the run has that person's rights and never the owner's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskOrigin {
    pub conversation_id: String,
    pub requested_by: String,
}

/// The conversation a task fires in, when that is not your chat with the bot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct BotTaskConversation {
    pub id: String,
    /// `group` or `direct`.
    pub kind: String,
    /// The group's name, when it has one.
    pub title: Option<String>,
}

impl From<TaskConversation> for BotTaskConversation {
    fn from(conversation: TaskConversation) -> Self {
        Self {
            id: conversation.id,
            kind: conversation.kind.as_str().to_string(),
            title: conversation.title,
        }
    }
}

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
    pub monitoring: bool,
    /// `bot` or `owner`.
    pub created_by: String,
    pub created_at: DateTime<FixedOffset>,
    /// The group or chat the task fires in; none fires in your chat with the bot.
    pub conversation: Option<BotTaskConversation>,
}

impl BotTaskInfo {
    pub(crate) fn of(row: bot_task::Model, conversation: Option<TaskConversation>) -> Self {
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
            monitoring: row.monitoring.unwrap_or(false),
            created_by: row.created_by.to_ascii_lowercase(),
            created_at: row.created_at,
            conversation: conversation.map(BotTaskConversation::from),
        }
    }
}

/// The conversations `rows` fire in, by id, read in one query.
pub(crate) async fn conversations_of<C: ConnectionTrait>(
    db: &C,
    rows: &[bot_task::Model],
) -> Result<HashMap<String, TaskConversation>, ApiError> {
    let ids: Vec<&str> = rows
        .iter()
        .filter_map(|row| row.conversation_id.as_deref())
        .collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(conversation::Entity::find()
        .filter(conversation::Column::Id.is_in(ids))
        .all(db)
        .await?
        .into_iter()
        .map(|row| {
            let conversation = TaskConversation {
                kind: ConversationKind::parse(&row.kind).unwrap_or(ConversationKind::Group),
                title: row.title.filter(|title| !title.trim().is_empty()),
                id: row.id,
            };
            (conversation.id.clone(), conversation)
        })
        .collect())
}

/// Where `row` fires, from [`conversations_of`]. A conversation that is gone still shows, as a
/// group without a name, so the task never reads as one of the owner's chat.
pub(crate) fn conversation_of(
    row: &bot_task::Model,
    known: &HashMap<String, TaskConversation>,
) -> Option<TaskConversation> {
    let id = row.conversation_id.as_ref()?;
    Some(known.get(id).cloned().unwrap_or_else(|| TaskConversation {
        id: id.clone(),
        kind: ConversationKind::Group,
        title: None,
    }))
}

pub(crate) async fn infos_of<C: ConnectionTrait>(
    db: &C,
    rows: Vec<bot_task::Model>,
) -> Result<Vec<BotTaskInfo>, ApiError> {
    let known = conversations_of(db, &rows).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let conversation = conversation_of(&row, &known);
            BotTaskInfo::of(row, conversation)
        })
        .collect())
}

async fn info_of<C: ConnectionTrait>(
    db: &C,
    row: bot_task::Model,
) -> Result<BotTaskInfo, ApiError> {
    let known = conversations_of(db, std::slice::from_ref(&row)).await?;
    let conversation = conversation_of(&row, &known);
    Ok(BotTaskInfo::of(row, conversation))
}

/// Where a task scheduled in `turn_id`, a turn of the owner's bot, fires. `None` keeps it in the
/// owner's chat with the bot: for a turn in that chat, a turn another bot asked for, or a turn
/// the server doesn't know. A group or chat the server can't write into is refused, because the
/// task would never fire.
pub(crate) async fn origin_of_turn<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    turn_id: &str,
) -> Result<Option<TaskOrigin>, ApiError> {
    let Some(turn) = bot_turn::Entity::find_by_id(turn_id)
        .filter(bot_turn::Column::BotId.eq(bot_id))
        .filter(bot_turn::Column::OwnerId.eq(owner_id))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    let requester = turn.requester_id.unwrap_or_else(|| owner_id.to_string());
    let Some(conversation) = conversation::Entity::find_by_id(&turn.conversation_id)
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    let direct = ConversationKind::parse(&conversation.kind) == Some(ConversationKind::Direct);
    if direct && requester == owner_id {
        return Ok(None);
    }
    if bot::Entity::find_by_id(&requester).one(db).await?.is_some() {
        return Ok(None);
    }
    if ConversationEncryption::parse(&conversation.encryption) != Some(ConversationEncryption::Plain)
    {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            TASK_CONVERSATION_ENCRYPTED,
            format!(
                "Conversation '{}' is end-to-end encrypted, so a task scheduled there could never fire: the server can't post into it",
                conversation.id
            ),
        ));
    }
    Ok(Some(TaskOrigin {
        conversation_id: conversation.id,
        requested_by: requester,
    }))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotTaskList {
    pub tasks: Vec<BotTaskInfo>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBotTaskRequest {
    /// Check for useful changes; routine successful runs may stay quiet.
    #[serde(default)]
    pub monitoring: bool,
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
    /// The bot's turn that asked for the task, sent by the device running that turn. The task
    /// then fires in that turn's group or chat, as the person who asked. A bot's own token always
    /// uses its turn.
    #[serde(default)]
    pub turn_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateBotTaskRequest {
    pub monitoring: Option<bool>,
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
/// schedule is valid. With an `origin` it fires there instead of the owner's chat.
pub(crate) async fn create_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    request: CreateBotTask,
    created_by: TaskCreator,
    origin: Option<&TaskOrigin>,
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
        monitoring: Set(Some(request.monitoring)),
        created_by: Set(created_by.stored()),
        created_at: Set(now),
        updated_at: Set(now),
        conversation_id: Set(origin.map(|origin| origin.conversation_id.clone())),
        requested_by: Set(origin.map(|origin| origin.requested_by.clone())),
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
    if let Some(monitoring) = request.monitoring {
        active.monitoring = Set(Some(monitoring));
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
    let sub = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
    let rows = list_in(&state.db, &sub, &bot_id).await?;
    let tasks = infos_of(&state.db, rows).await?;
    Ok(Json(BotTaskList { tasks }))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/tasks",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = CreateBotTaskRequest,
    responses(
        (status = 201, description = "A new task: the bot runs its instruction at the scheduled time and answers in your chat, or in the group or chat of the turn that asked for it.", body = BotTaskInfo),
        (status = 400, description = "The schedule or text is invalid, or a cron runs more often than every 5 minutes"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "BOT_TASK_CAPABILITY_OFF: switch the bot's scheduling capability on first"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_TASK_LIMIT_REACHED: a bot has at most 50 tasks. BOT_TASK_CONVERSATION_ENCRYPTED: the turn's conversation is end-to-end encrypted, so the task could never fire there")
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
    let sub = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
    let (creator, turn_id) = match &user {
        AppUser::Bot(bot) => (TaskCreator::Bot, Some(bot.turn_id.clone())),
        _ => (TaskCreator::Owner, body.turn_id.clone()),
    };
    let request = CreateBotTask {
        kind: Some(kind_of(&body)?),
        at: body.at,
        cron: body.cron,
        tz: body.tz,
        instruction: body.instruction,
        title: body.title,
        enabled: None,
        monitoring: body.monitoring,
    };
    let row = state
        .transaction(|txn| {
            let (sub, bot_id, request, turn_id) = (
                sub.clone(),
                bot_id.clone(),
                request.clone(),
                turn_id.clone(),
            );
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                let origin = match turn_id.as_deref() {
                    Some(turn_id) => origin_of_turn(txn, &sub, &bot_id, turn_id).await?,
                    None => None,
                };
                let now = Utc::now().fixed_offset();
                create_in(txn, &sub, &bot_id, request, creator, origin.as_ref(), now).await
            })
        })
        .await?;
    task_schedules::sync(&state, &row).await;
    Ok((StatusCode::CREATED, Json(info_of(&state.db, row).await?)))
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
    let sub = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
    let request = UpdateBotTask {
        at: body.at,
        cron: body.cron,
        tz: body.tz,
        instruction: body.instruction,
        title: body.title,
        enabled: body.enabled,
        monitoring: body.monitoring,
    };
    let row = state
        .transaction(|txn| {
            let (sub, bot_id, task_id, request) = (
                sub.clone(),
                bot_id.clone(),
                task_id.clone(),
                request.clone(),
            );
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                update_in(
                    txn,
                    &sub,
                    &bot_id,
                    &task_id,
                    request,
                    Utc::now().fixed_offset(),
                )
                .await
            })
        })
        .await?;
    task_schedules::sync(&state, &row).await;
    Ok(Json(info_of(&state.db, row).await?))
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
    let sub = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
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
        (status = 200, description = "A desktop host fires a due task of its bot: the run is claimed once and written into your chat, or into the group or chat the task was scheduled in. A task that is not due, paused or already fired answers `fired: false`.", body = FireBotTaskResponse),
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
    let sub = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    let task = owned_task(&state.db, &sub, &bot_id, &task_id).await?;
    let due = firing::holds_capability(&bot)
        && task.enabled
        && task.next_run_at.is_some_and(|at| at <= Utc::now());
    let fired = due && firing::fire(&state, &task).await?;
    Ok(Json(FireBotTaskResponse { fired }))
}

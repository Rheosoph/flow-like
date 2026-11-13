//! Bot skills: guidance the owner installs on a bot. Only the owner manages them (someone
//! else's bot reads as missing, and every query is scoped to the bot in SQL). A skill the bot
//! proposed, a zip the owner imported or a package the owner pinned starts `pending`; it reaches
//! the bot's prompt and tools only after the owner approves it. A zip is untrusted: it is read
//! by the executor (`skill_import` job), never in this process.

use super::{lock_bot, owned_bot, workspace};
use crate::{
    bot_tools::skills as skill_jobs,
    entity::{app_package, bot, bot_skill, membership, wasm_package, wasm_package_version},
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
use flow_like_types::bot_skill::{
    self as contract, BotSkill, CreateBotSkill, MAX_SKILLS_PER_BOT, MAX_ZIP_BYTES, SkillSource,
    SkillStatus, UpdateBotSkill,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub(crate) const SKILL_PACKAGE_UNCOVERED: &str = "BOT_SKILL_PACKAGE_UNCOVERED";
pub(crate) const SKILL_PACKAGE_EMPTY: &str = "BOT_SKILL_PACKAGE_HAS_NO_SKILLS";
pub(crate) const SKILL_PACKAGE_UNAVAILABLE: &str = "BOT_SKILL_PACKAGE_UNAVAILABLE";

pub(crate) const SKILLS_CAPABILITY: &str = "skills";
pub(crate) const SKILL_CAPABILITY_OFF: &str = "BOT_SKILL_CAPABILITY_OFF";
pub(crate) const SKILL_LIMIT_REACHED: &str = "BOT_SKILL_LIMIT_REACHED";
pub(crate) const SKILL_EXISTS: &str = "BOT_SKILL_EXISTS";
pub(crate) const SKILL_FROM_PACKAGE: &str = "BOT_SKILL_FROM_PACKAGE";
pub(crate) const SKILL_IMPORT_NEEDS_EXECUTOR: &str = "BOT_SKILL_IMPORT_NEEDS_EXECUTOR";
pub(crate) const SKILL_IMPORT_REFUSED: &str = "BOT_SKILL_IMPORT_REFUSED";
/// Skills the bot proposed that wait for the owner; more would bury the real ones.
pub(crate) const MAX_PENDING_PROPOSALS: u64 = 20;

/// A skill as its owner sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct BotSkillInfo {
    pub id: String,
    pub bot_id: String,
    /// Lowercase letters, digits and single hyphens.
    pub name: String,
    pub description: String,
    /// Markdown instructions.
    pub body: String,
    /// `owner`, `import`, `bot_proposed` or `package:<id>@<version>`.
    pub source: String,
    /// `active`, `pending` or `disabled`. Only an active skill reaches the bot.
    pub status: String,
    /// Package node names the skill's instructions call.
    pub tools: Vec<String>,
    pub created_at: DateTime<FixedOffset>,
    pub updated_at: DateTime<FixedOffset>,
}

pub(crate) fn tools_of(row: &bot_skill::Model) -> Vec<String> {
    serde_json::from_value(row.tools.clone()).unwrap_or_default()
}

impl From<bot_skill::Model> for BotSkillInfo {
    fn from(row: bot_skill::Model) -> Self {
        Self {
            tools: tools_of(&row),
            status: row.status.to_ascii_lowercase(),
            id: row.id,
            bot_id: row.bot_id,
            name: row.name,
            description: row.description,
            body: row.body,
            source: row.source,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// The contract view of a row, as the tools see it.
pub(crate) fn to_skill(row: bot_skill::Model) -> BotSkill {
    BotSkill {
        tools: tools_of(&row),
        status: SkillStatus::parse(&row.status).unwrap_or(SkillStatus::Disabled),
        source: SkillSource::parse(&row.source).unwrap_or(SkillSource::Import),
        updated_at: row.updated_at.to_rfc3339(),
        id: row.id,
        bot_id: row.bot_id,
        name: row.name,
        description: row.description,
        body: row.body,
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotSkillList {
    pub skills: Vec<BotSkillInfo>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBotSkillRequest {
    /// Lowercase letters, digits and single hyphens, at most 64 characters.
    pub name: String,
    /// One line, at most 1,024 characters: what the skill does and when to use it.
    pub description: String,
    /// Markdown instructions, at most 32 KiB.
    pub body: String,
    /// Package node names the instructions call.
    #[serde(default)]
    pub tools: Vec<String>,
    /// `active`, `pending` or `disabled`; active when missing. A skill moved from a device keeps
    /// the status it had.
    #[serde(default)]
    pub status: Option<String>,
    /// `owner`, `import` or `bot_proposed`; owner when missing. A package skill comes only from
    /// a package install.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ImportBotSkillRequest {
    /// The zipped skill folder, base64 encoded, at most 8 MiB unpacked from base64.
    pub zip_base64: String,
}

/// Largest JSON body of an import: the zip's base64 plus the envelope.
pub(crate) const IMPORT_BODY_LIMIT: usize = (MAX_ZIP_BYTES as usize / 3 + 1) * 4 + 1024;

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateBotSkillRequest {
    pub description: Option<String>,
    pub body: Option<String>,
    pub tools: Option<Vec<String>>,
}

fn bad_request(message: String) -> ApiError {
    ApiError::bad_request(message)
}

fn holds_capability(bot: &bot::Model) -> bool {
    bot.capabilities.iter().any(|id| id == SKILLS_CAPABILITY)
}

fn refuse_without_capability(bot: &bot::Model) -> Result<(), ApiError> {
    if holds_capability(bot) {
        return Ok(());
    }
    Err(ApiError::coded(
        StatusCode::FORBIDDEN,
        SKILL_CAPABILITY_OFF,
        format!(
            "Bot '{}' doesn't have the skills capability switched on",
            bot.id
        ),
    ))
}

pub(crate) async fn list_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
) -> Result<Vec<bot_skill::Model>, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    Ok(bot_skill::Entity::find()
        .filter(bot_skill::Column::BotId.eq(bot_id))
        .order_by_asc(bot_skill::Column::Name)
        .all(db)
        .await?)
}

async fn owned_skill<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    skill_id: &str,
) -> Result<bot_skill::Model, ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    bot_skill::Entity::find_by_id(skill_id)
        .filter(bot_skill::Column::BotId.eq(bot_id))
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Skill '{skill_id}' not found")))
}

pub(crate) async fn by_name<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    name: &str,
) -> Result<Option<bot_skill::Model>, ApiError> {
    Ok(bot_skill::Entity::find()
        .filter(bot_skill::Column::BotId.eq(bot_id))
        .filter(bot_skill::Column::Name.eq(name))
        .one(db)
        .await?)
}

/// What the bot's prompt names: its active skills and nothing else.
pub(crate) async fn active_index<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
) -> Result<Vec<contract::SkillIndexEntry>, ApiError> {
    Ok(bot_skill::Entity::find()
        .filter(bot_skill::Column::BotId.eq(bot_id))
        .filter(bot_skill::Column::Status.eq(SkillStatus::Active.stored()))
        .order_by_asc(bot_skill::Column::Name)
        .all(db)
        .await?
        .into_iter()
        .map(|row| contract::SkillIndexEntry {
            name: row.name,
            description: row.description,
        })
        .collect())
}

/// How a new skill gets in: who made it and whether the owner already approved it.
#[derive(Clone)]
pub(crate) struct Origin {
    pub source: SkillSource,
    pub status: SkillStatus,
}

/// The origin a create request names: an owner skill, or a skill moved from a device with the
/// status it had. A package source cannot be claimed here.
pub(crate) fn origin_of(source: Option<&str>, status: Option<&str>) -> Result<Origin, ApiError> {
    let source = match source.map(str::trim).filter(|source| !source.is_empty()) {
        None => SkillSource::Owner,
        Some(text) => match SkillSource::parse(text) {
            Some(parsed) if parsed.package_id().is_none() => parsed,
            _ => {
                return Err(bad_request(format!(
                    "A skill source is owner, import or bot_proposed; got '{text}'"
                )));
            }
        },
    };
    let status = match status.map(str::trim).filter(|status| !status.is_empty()) {
        None => SkillStatus::Active,
        Some(text) => SkillStatus::parse(text).ok_or_else(|| {
            bad_request(format!(
                "A skill status is active, pending or disabled; got '{text}'"
            ))
        })?,
    };
    Ok(Origin { source, status })
}

impl Origin {
    pub(crate) fn owner() -> Self {
        Self {
            source: SkillSource::Owner,
            status: SkillStatus::Active,
        }
    }

    pub(crate) fn proposed_by_bot() -> Self {
        Self {
            source: SkillSource::BotProposed,
            status: SkillStatus::Pending,
        }
    }
}

/// Inserts a skill once the bot is the caller's, holds the capability and has room, and every
/// text passes the skill rules. A name the bot already has is refused.
pub(crate) async fn create_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    request: CreateBotSkill,
    origin: Origin,
    now: DateTime<FixedOffset>,
) -> Result<bot_skill::Model, ApiError> {
    let bot = owned_bot(db, owner_id, bot_id).await?;
    refuse_without_capability(&bot)?;
    let (name, description, body) =
        contract::check_fields(&request.name, &request.description, &request.body)
            .map_err(bad_request)?;
    let tools = contract::check_tools(&request.tools).map_err(bad_request)?;
    let count = bot_skill::Entity::find()
        .filter(bot_skill::Column::BotId.eq(bot_id))
        .count(db)
        .await?;
    if count >= MAX_SKILLS_PER_BOT as u64 {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            SKILL_LIMIT_REACHED,
            format!("Bot '{bot_id}' already has {MAX_SKILLS_PER_BOT} skills; delete one first"),
        ));
    }
    if origin.source == SkillSource::BotProposed {
        let pending = bot_skill::Entity::find()
            .filter(bot_skill::Column::BotId.eq(bot_id))
            .filter(bot_skill::Column::Status.eq(SkillStatus::Pending.stored()))
            .count(db)
            .await?;
        if pending >= MAX_PENDING_PROPOSALS {
            return Err(ApiError::coded(
                StatusCode::CONFLICT,
                SKILL_LIMIT_REACHED,
                format!(
                    "Bot '{bot_id}' already has {MAX_PENDING_PROPOSALS} skills waiting for approval"
                ),
            ));
        }
    }
    if by_name(db, bot_id, &name).await?.is_some() {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            SKILL_EXISTS,
            format!("Bot '{bot_id}' already has a skill named '{name}'"),
        ));
    }
    Ok(bot_skill::ActiveModel {
        id: Set(format!("skill_{}", uuid::Uuid::new_v4().simple())),
        bot_id: Set(bot_id.to_string()),
        name: Set(name),
        description: Set(description),
        body: Set(body),
        source: Set(origin.source.stored()),
        status: Set(origin.status.stored().to_string()),
        tools: Set(serde_json::json!(tools)),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await?)
}

fn refuse_package_edit(row: &bot_skill::Model) -> Result<(), ApiError> {
    if SkillSource::parse(&row.source).is_some_and(|source| source.package_id().is_some()) {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            SKILL_FROM_PACKAGE,
            format!(
                "Skill '{}' comes from a package; update the package instead of editing it",
                row.name
            ),
        ));
    }
    Ok(())
}

pub(crate) async fn update_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    skill_id: &str,
    request: UpdateBotSkill,
    now: DateTime<FixedOffset>,
) -> Result<bot_skill::Model, ApiError> {
    let row = owned_skill(db, owner_id, bot_id, skill_id).await?;
    refuse_package_edit(&row)?;
    let (_, description, body) = contract::check_fields(
        &row.name,
        request.description.as_deref().unwrap_or(&row.description),
        request.body.as_deref().unwrap_or(&row.body),
    )
    .map_err(bad_request)?;
    let tools = match request.tools {
        Some(tools) => contract::check_tools(&tools).map_err(bad_request)?,
        None => tools_of(&row),
    };
    let mut active: bot_skill::ActiveModel = row.into();
    active.description = Set(description);
    active.body = Set(body);
    active.tools = Set(serde_json::json!(tools));
    active.updated_at = Set(now);
    Ok(active.update(db).await?)
}

/// Moves the skill to `status`. Approving re-reads the stored text against the skill rules, so
/// a row that was written around them never goes live.
pub(crate) async fn set_status_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    skill_id: &str,
    status: SkillStatus,
    now: DateTime<FixedOffset>,
) -> Result<bot_skill::Model, ApiError> {
    let row = owned_skill(db, owner_id, bot_id, skill_id).await?;
    if status == SkillStatus::Active {
        contract::check_fields(&row.name, &row.description, &row.body).map_err(bad_request)?;
    }
    let mut active: bot_skill::ActiveModel = row.into();
    active.status = Set(status.stored().to_string());
    active.updated_at = Set(now);
    Ok(active.update(db).await?)
}

pub(crate) async fn delete_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    skill_id: &str,
) -> Result<bot_skill::Model, ApiError> {
    let row = owned_skill(db, owner_id, bot_id, skill_id).await?;
    bot_skill::Entity::delete_by_id(row.id.clone())
        .exec(db)
        .await?;
    Ok(row)
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/skills",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The bot's skills with their status, by name. The bot's own runtime token may read them.", body = BotSkillList),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/skills", skip(state, user))]
pub async fn list_bot_skills(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<BotSkillList>, ApiError> {
    let sub = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
    let skills = list_in(&state.db, &sub, &bot_id)
        .await?
        .into_iter()
        .map(BotSkillInfo::from)
        .collect();
    Ok(Json(BotSkillList { skills }))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/skills",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = CreateBotSkillRequest,
    responses(
        (status = 201, description = "A new skill, active at once: you wrote it.", body = BotSkillInfo),
        (status = 400, description = "The name, description or body breaks the skill rules, or hides characters"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "BOT_SKILL_CAPABILITY_OFF: switch the bot's skills capability on first"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_SKILL_EXISTS or BOT_SKILL_LIMIT_REACHED")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/skills", skip(state, user, body))]
pub async fn create_bot_skill(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<CreateBotSkillRequest>,
) -> Result<(StatusCode, Json<BotSkillInfo>), ApiError> {
    let sub = user.sub()?;
    let origin = origin_of(body.source.as_deref(), body.status.as_deref())?;
    let request = CreateBotSkill {
        name: body.name,
        description: body.description,
        body: body.body,
        tools: body.tools,
    };
    let row = state
        .transaction(|txn| {
            let (sub, bot_id, request, origin) =
                (sub.clone(), bot_id.clone(), request.clone(), origin.clone());
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                let now = Utc::now().fixed_offset();
                create_in(txn, &sub, &bot_id, request, origin.clone(), now).await
            })
        })
        .await?;
    Ok((StatusCode::CREATED, Json(row.into())))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/skills/propose",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = CreateBotSkillRequest,
    responses(
        (status = 201, description = "A skill the bot proposed, saved as pending until you approve it.", body = BotSkillInfo),
        (status = 400, description = "The name, description or body breaks the skill rules, or hides characters"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "BOT_SKILL_CAPABILITY_OFF: switch the bot's skills capability on first"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_SKILL_EXISTS or BOT_SKILL_LIMIT_REACHED")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/skills/propose", skip(state, user, body))]
pub async fn propose_bot_skill(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<CreateBotSkillRequest>,
) -> Result<(StatusCode, Json<BotSkillInfo>), ApiError> {
    let sub = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
    let request = CreateBotSkill {
        name: body.name,
        description: body.description,
        body: body.body,
        tools: Vec::new(),
    };
    let row = state
        .transaction(|txn| {
            let (sub, bot_id, request) = (sub.clone(), bot_id.clone(), request.clone());
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                let now = Utc::now().fixed_offset();
                create_in(txn, &sub, &bot_id, request, Origin::proposed_by_bot(), now).await
            })
        })
        .await?;
    Ok((StatusCode::CREATED, Json(row.into())))
}

#[utoipa::path(
    patch,
    path = "/bots/{bot_id}/skills/{skill_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("skill_id" = String, Path, description = "The skill's id")
    ),
    request_body = UpdateBotSkillRequest,
    responses(
        (status = 200, description = "The updated skill. Its status stays as it was.", body = BotSkillInfo),
        (status = 400, description = "The text breaks the skill rules or hides characters"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or skill"),
        (status = 409, description = "BOT_SKILL_FROM_PACKAGE: a package skill can't be edited")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PATCH /bots/{bot_id}/skills/{skill_id}",
    skip(state, user, body)
)]
pub async fn update_bot_skill(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, skill_id)): Path<(String, String)>,
    Json(body): Json<UpdateBotSkillRequest>,
) -> Result<Json<BotSkillInfo>, ApiError> {
    let sub = user.sub()?;
    let request = UpdateBotSkill {
        description: body.description,
        body: body.body,
        tools: body.tools,
    };
    let row = update_in(
        &state.db,
        &sub,
        &bot_id,
        &skill_id,
        request,
        Utc::now().fixed_offset(),
    )
    .await?;
    Ok(Json(row.into()))
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/skills/{skill_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("skill_id" = String, Path, description = "The skill's id")
    ),
    responses(
        (status = 204, description = "The skill and the files it brought are gone"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or skill")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bots/{bot_id}/skills/{skill_id}", skip(state, user))]
pub async fn delete_bot_skill(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, skill_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let row = delete_in(&state.db, &sub, &bot_id, &skill_id).await?;
    skill_jobs::remove_references(&state, &sub, &bot_id, &row.name).await;
    if let Some(package_id) =
        SkillSource::parse(&row.source).and_then(|source| source.package_id().map(str::to_string))
    {
        let workspace_id = workspace::ensure(&state, &sub, &bot_id).await?.app_id;
        release_package_pin_in(&state.db, &bot_id, &workspace_id, &package_id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn move_to(
    state: &AppState,
    user: &AppUser,
    bot_id: &str,
    skill_id: &str,
    status: SkillStatus,
) -> Result<Json<BotSkillInfo>, ApiError> {
    let sub = user.sub()?;
    let row = set_status_in(
        &state.db,
        &sub,
        bot_id,
        skill_id,
        status,
        Utc::now().fixed_offset(),
    )
    .await?;
    Ok(Json(row.into()))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/skills/{skill_id}/approve",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("skill_id" = String, Path, description = "The skill's id")
    ),
    responses(
        (status = 200, description = "The skill is active: the bot sees it in its prompt and can load it.", body = BotSkillInfo),
        (status = 400, description = "The stored text breaks the skill rules, so it can't go live"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or skill")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bots/{bot_id}/skills/{skill_id}/approve",
    skip(state, user)
)]
pub async fn approve_bot_skill(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, skill_id)): Path<(String, String)>,
) -> Result<Json<BotSkillInfo>, ApiError> {
    move_to(&state, &user, &bot_id, &skill_id, SkillStatus::Active).await
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/skills/{skill_id}/disable",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("skill_id" = String, Path, description = "The skill's id")
    ),
    responses(
        (status = 200, description = "The skill is switched off: the bot no longer sees it.", body = BotSkillInfo),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or skill")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bots/{bot_id}/skills/{skill_id}/disable",
    skip(state, user)
)]
pub async fn disable_bot_skill(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, skill_id)): Path<(String, String)>,
) -> Result<Json<BotSkillInfo>, ApiError> {
    move_to(&state, &user, &bot_id, &skill_id, SkillStatus::Disabled).await
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/skills/import",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = ImportBotSkillRequest,
    responses(
        (status = 201, description = "The skill was read and saved as pending, with its files in the bot's workspace under bot/skills. Approve it to use it.", body = BotSkillInfo),
        (status = 400, description = "BOT_SKILL_IMPORT_REFUSED: the zip is invalid, too large, unsafe or hides characters"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "BOT_SKILL_CAPABILITY_OFF: switch the bot's skills capability on first"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_SKILL_FROM_PACKAGE or BOT_SKILL_LIMIT_REACHED"),
        (status = 503, description = "BOT_SKILL_IMPORT_NEEDS_EXECUTOR: this server has no executor to read the zip safely")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/skills/import", skip(state, user, body))]
pub async fn import_bot_skill(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<ImportBotSkillRequest>,
) -> Result<(StatusCode, Json<BotSkillInfo>), ApiError> {
    let sub = user.sub()?;
    let bot = owned_bot(&state.db, &sub, &bot_id).await?;
    refuse_without_capability(&bot)?;
    let zip = {
        use base64::{Engine, engine::general_purpose::STANDARD};
        STANDARD
            .decode(body.zip_base64.trim())
            .map_err(|_| bad_request("The skill zip is not valid base64".to_string()))?
    };
    if zip.len() as u64 > MAX_ZIP_BYTES {
        return Err(ApiError::coded(
            StatusCode::PAYLOAD_TOO_LARGE,
            SKILL_IMPORT_REFUSED,
            format!(
                "A skill zip has at most {MAX_ZIP_BYTES} bytes; this one has {}",
                zip.len()
            ),
        ));
    }
    let workspace = workspace::ensure(&state, &sub, &bot_id).await?.app_id;
    let imported = skill_jobs::import_zip(&state, &sub, &bot_id, &workspace, zip.into()).await?;
    let request = CreateBotSkill {
        name: imported.name,
        description: imported.description,
        body: imported.body,
        tools: Vec::new(),
    };
    let row = state
        .transaction(|txn| {
            let (sub, bot_id, request) = (sub.clone(), bot_id.clone(), request.clone());
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                let now = Utc::now().fixed_offset();
                reimport_in(txn, &sub, &bot_id, request, now).await
            })
        })
        .await?;
    Ok((StatusCode::CREATED, Json(row.into())))
}

/// A skill read from a zip: new, or replacing an earlier import or owner skill of the same
/// name, which goes back to `pending` because its text changed.
pub(crate) async fn reimport_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    request: CreateBotSkill,
    now: DateTime<FixedOffset>,
) -> Result<bot_skill::Model, ApiError> {
    let origin = Origin {
        source: SkillSource::Import,
        status: SkillStatus::Pending,
    };
    let Some(existing) = by_name(db, bot_id, request.name.trim()).await? else {
        return create_in(db, owner_id, bot_id, request, origin, now).await;
    };
    owned_bot(db, owner_id, bot_id).await?;
    refuse_package_edit(&existing)?;
    let (_, description, body) =
        contract::check_fields(&request.name, &request.description, &request.body)
            .map_err(bad_request)?;
    let mut active: bot_skill::ActiveModel = existing.into();
    active.description = Set(description);
    active.body = Set(body);
    active.source = Set(origin.source.stored());
    active.status = Set(origin.status.stored().to_string());
    active.updated_at = Set(now);
    Ok(active.update(db).await?)
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct InstallBotSkillPackageRequest {
    /// The registry package that ships the skills.
    pub package_id: String,
    /// The version to pin; the package's current version when missing.
    pub version: Option<String>,
}

/// The package capability tags the bot's capabilities do not cover, or an empty list.
pub(crate) fn uncovered_for(bot: &bot::Model, tags: &[String]) -> Vec<String> {
    let held: Vec<String> = bot.capabilities.iter().map(|id| id.to_string()).collect();
    contract::uncovered_tags(tags, &held)
}

/// Pins `package_id` to the bot's workspace and saves its skills as pending, one per skill
/// the pinned version ships. The package must be held by the owner, cover only capabilities
/// the bot has, and ship skills. A skill a package already brought is replaced and goes back
/// to pending; a skill of another origin under the same name is never overwritten.
pub(crate) async fn install_package_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    workspace_id: &str,
    request: &InstallBotSkillPackageRequest,
    now: DateTime<FixedOffset>,
) -> Result<Vec<bot_skill::Model>, ApiError> {
    let bot = owned_bot(db, owner_id, bot_id).await?;
    refuse_without_capability(&bot)?;
    let package = wasm_package::Entity::find_by_id(&request.package_id)
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found("Package not found"))?;
    crate::package_license::ensure_holds(db, owner_id, &package).await?;
    let version = request
        .version
        .clone()
        .unwrap_or_else(|| package.version.clone());
    let row = wasm_package_version::Entity::find()
        .filter(wasm_package_version::Column::PackageId.eq(&package.id))
        .filter(wasm_package_version::Column::Version.eq(&version))
        .one(db)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "Version '{version}' of the package is not published"
            ))
        })?;
    if row.status != crate::entity::sea_orm_active_enums::WasmPackageStatus::Active {
        return Err(ApiError::bad_request(format!(
            "Version '{version}' of '{}' has not been approved by the registry yet",
            package.id
        )));
    }
    let entries: Vec<flow_like_wasm_schema::manifest::PackageSkillEntry> = row
        .skills
        .clone()
        .and_then(|stored| serde_json::from_value(stored).ok())
        .unwrap_or_default();
    if entries.is_empty() {
        return Err(ApiError::coded(
            StatusCode::BAD_REQUEST,
            SKILL_PACKAGE_EMPTY,
            format!("Version '{version}' of '{}' ships no skills", package.id),
        ));
    }
    let tags = serde_json::from_value::<flow_like_wasm_schema::manifest::PackagePermissions>(
        package.permissions.clone(),
    )
    .map(|permissions| permissions.capability_tags())
    .unwrap_or_default();
    let missing = uncovered_for(&bot, &tags);
    if !missing.is_empty() {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            SKILL_PACKAGE_UNCOVERED,
            format!(
                "'{}' needs what bot '{bot_id}' does not have switched on: {}",
                package.id,
                missing.join(", ")
            ),
        ));
    }
    for entry in &entries {
        entry.validate().map_err(bad_request)?;
        if let Some(existing) = by_name(db, bot_id, &entry.name).await?
            && SkillSource::parse(&existing.source).and_then(|s| s.package_id().map(str::to_string))
                != Some(package.id.clone())
        {
            return Err(ApiError::coded(
                StatusCode::CONFLICT,
                SKILL_EXISTS,
                format!("Bot '{bot_id}' already has a skill named '{}'", entry.name),
            ));
        }
    }
    pin_to_workspace(db, owner_id, workspace_id, &package.id, &version, now).await?;
    let source = SkillSource::Package {
        id: package.id.clone(),
        version: version.clone(),
    }
    .stored();
    let mut saved = Vec::with_capacity(entries.len());
    for entry in entries {
        let tools = contract::check_tools(&entry.tools).map_err(bad_request)?;
        let description = contract::check_description(&entry.description).map_err(bad_request)?;
        let body = entry.instructions.clone().unwrap_or_default();
        let model = match by_name(db, bot_id, &entry.name).await? {
            Some(existing) => {
                let mut active: bot_skill::ActiveModel = existing.into();
                active.description = Set(description);
                active.body = Set(body);
                active.source = Set(source.clone());
                active.status = Set(SkillStatus::Pending.stored().to_string());
                active.tools = Set(serde_json::json!(tools));
                active.updated_at = Set(now);
                active.update(db).await?
            }
            None => {
                bot_skill::ActiveModel {
                    id: Set(format!("skill_{}", uuid::Uuid::new_v4().simple())),
                    bot_id: Set(bot_id.to_string()),
                    name: Set(entry.name.clone()),
                    description: Set(description),
                    body: Set(body),
                    source: Set(source.clone()),
                    status: Set(SkillStatus::Pending.stored().to_string()),
                    tools: Set(serde_json::json!(tools)),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(db)
                .await?
            }
        };
        saved.push(model);
    }
    Ok(saved)
}

/// Removes the package's pin from the workspace once no skill of the bot comes from it; while
/// another one does, the pin stays. Returns whether the pin went.
pub(crate) async fn release_package_pin_in<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    workspace_id: &str,
    package_id: &str,
) -> Result<bool, ApiError> {
    let prefix = format!("package:{package_id}@");
    let remaining = bot_skill::Entity::find()
        .filter(bot_skill::Column::BotId.eq(bot_id))
        .filter(bot_skill::Column::Source.starts_with(&prefix))
        .count(db)
        .await?;
    if remaining > 0 {
        return Ok(false);
    }
    let removed = app_package::Entity::delete_many()
        .filter(app_package::Column::AppId.eq(workspace_id))
        .filter(app_package::Column::PackageId.eq(package_id))
        .exec(db)
        .await?;
    Ok(removed.rows_affected > 0)
}

async fn pin_to_workspace<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    workspace_id: &str,
    package_id: &str,
    version: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    let existing = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(workspace_id))
        .filter(app_package::Column::PackageId.eq(package_id))
        .one(db)
        .await?;
    if let Some(existing) = existing {
        let mut active: app_package::ActiveModel = existing.into();
        active.version = Set(version.to_string());
        active.update(db).await?;
        return Ok(());
    }
    let member = membership::Entity::find()
        .filter(membership::Column::AppId.eq(workspace_id))
        .filter(membership::Column::UserId.eq(owner_id))
        .one(db)
        .await?
        .ok_or_else(|| ApiError::bad_request("The bot's workspace has no owner membership"))?;
    app_package::ActiveModel {
        id: Set(flow_like_types::create_id()),
        app_id: Set(workspace_id.to_string()),
        membership_id: Set(Some(member.id)),
        package_id: Set(package_id.to_string()),
        version: Set(version.to_string()),
        added_at: Set(now),
        auto_update: Set(false),
        stale: Set(false),
        stale_since: Set(None),
    }
    .insert(db)
    .await?;
    Ok(())
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/skills/package",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = InstallBotSkillPackageRequest,
    responses(
        (status = 201, description = "The package is pinned to the bot's workspace and its skills are saved as pending. Approve them to use them.", body = BotSkillList),
        (status = 400, description = "The package ships no skills (BOT_SKILL_PACKAGE_HAS_NO_SKILLS) or a skill breaks the rules"),
        (status = 401, description = "Not signed in"),
        (status = 402, description = "A paid package you have not bought"),
        (status = 403, description = "BOT_SKILL_CAPABILITY_OFF, or no access to the package"),
        (status = 404, description = "You have no bot with this id, or the package or version does not exist"),
        (status = 409, description = "BOT_SKILL_PACKAGE_UNCOVERED: the package needs capabilities the bot does not have (for example internet or connectors), or BOT_SKILL_EXISTS")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/skills/package", skip(state, user, body))]
pub async fn install_bot_skill_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<InstallBotSkillPackageRequest>,
) -> Result<(StatusCode, Json<BotSkillList>), ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    let workspace_id = workspace::ensure(&state, &sub, &bot_id).await?.app_id;
    let rows = state
        .transaction(|txn| {
            let (sub, bot_id, workspace_id, body) = (
                sub.clone(),
                bot_id.clone(),
                workspace_id.clone(),
                body.clone_request(),
            );
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                install_package_in(
                    txn,
                    &sub,
                    &bot_id,
                    &workspace_id,
                    &body,
                    Utc::now().fixed_offset(),
                )
                .await
            })
        })
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(BotSkillList {
            skills: rows.into_iter().map(BotSkillInfo::from).collect(),
        }),
    ))
}

impl InstallBotSkillPackageRequest {
    fn clone_request(&self) -> Self {
        Self {
            package_id: self.package_id.clone(),
            version: self.version.clone(),
        }
    }
}

/// The package one skill's tools run in, for a host that runs them itself.
#[derive(Debug, Serialize, ToSchema)]
pub struct BotSkillPackage {
    pub package_id: String,
    /// The version the bot's workspace pins.
    pub version: String,
    /// A short-lived download link of the package's portable WASM.
    pub wasm_url: String,
    /// blake3 of the WASM bytes, hex.
    pub wasm_hash: String,
    /// The manifest's permissions: memory and time tiers, capabilities, allowed hosts.
    #[schema(value_type = Option<Object>)]
    pub permissions: Option<serde_json::Value>,
}

/// The bot's active package skill `skill_id` and the package it names. A skill that is not
/// active reads as missing; an instruction skill has no package.
pub(crate) async fn package_skill_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    skill_id: &str,
) -> Result<(BotSkill, String), ApiError> {
    refuse_without_capability(&owned_bot(db, owner_id, bot_id).await?)?;
    let skill = to_skill(owned_skill(db, owner_id, bot_id, skill_id).await?);
    if skill.status != SkillStatus::Active {
        return Err(ApiError::not_found(format!(
            "Skill '{skill_id}' is not active"
        )));
    }
    let package_id = skill
        .source
        .package_id()
        .map(str::to_string)
        .ok_or_else(|| {
            ApiError::bad_request(format!("Skill '{}' ships no package tools", skill.name))
        })?;
    Ok((skill, package_id))
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/skills/{skill_id}/package",
    tag = "Bots",
    description = "The package an active skill's tools run in, for a device that runs them in its own sandbox: the version the bot's workspace pins and a short-lived download of its WASM. Needs the bot's own runtime token and only works while its turn runs.",
    params(
        ("bot_id" = String, Path, description = "The bot's id"),
        ("skill_id" = String, Path, description = "The skill's id")
    ),
    responses(
        (status = 200, description = "The package version, its download link and permissions", body = BotSkillPackage),
        (status = 400, description = "The skill ships no package tools"),
        (status = 401, description = "Not a runtime token of this bot, or its turn ended"),
        (status = 403, description = "The caller is not the bot's runtime token, or BOT_SKILL_CAPABILITY_OFF"),
        (status = 404, description = "The bot has no such active skill, or no workspace"),
        (status = 409, description = "BOT_SKILL_PACKAGE_UNAVAILABLE: the workspace does not pin a usable version of the package")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "GET /bots/{bot_id}/skills/{skill_id}/package",
    skip(state, user)
)]
pub async fn bot_skill_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, skill_id)): Path<(String, String)>,
) -> Result<Json<BotSkillPackage>, ApiError> {
    if !matches!(user, AppUser::Bot(_)) {
        return Err(ApiError::forbidden(
            "Only the bot's own runtime token downloads its skill packages",
        ));
    }
    let owner = crate::bot_caller::acting_owner_for(&state.db, &user, &bot_id).await?;
    let (skill, package_id) = package_skill_in(&state.db, &owner, &bot_id, &skill_id).await?;
    let workspace = workspace::live_workspace(&state.db, &owner, &bot_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Bot '{bot_id}' has no workspace")))?;
    let package = skill_jobs::pinned_package(&state, &workspace, &package_id)
        .await
        .map_err(|message| {
            ApiError::coded(StatusCode::CONFLICT, SKILL_PACKAGE_UNAVAILABLE, message)
        })?;
    tracing::info!(
        bot_id = %bot_id,
        skill = %skill.name,
        package_id = %package_id,
        version = %package.version,
        "[skills] signed a skill package download for the bot's device turn"
    );
    Ok(Json(BotSkillPackage {
        package_id,
        version: package.version,
        wasm_url: package.wasm_url,
        wasm_hash: package.wasm_hash,
        permissions: package.permissions,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_create_request_names_an_owner_import_or_proposed_origin_never_a_package() {
        let owner = origin_of(None, None).unwrap();
        assert_eq!(
            (owner.source, owner.status),
            (SkillSource::Owner, SkillStatus::Active)
        );
        let moved = origin_of(Some("bot_proposed"), Some("PENDING")).unwrap();
        assert_eq!(
            (moved.source, moved.status),
            (SkillSource::BotProposed, SkillStatus::Pending)
        );
        assert!(origin_of(Some("package:evil@1.0.0"), None).is_err());
        assert!(origin_of(Some("nonsense"), None).is_err());
        assert!(origin_of(None, Some("live")).is_err());
    }
}

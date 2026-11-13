//! Moving a bot that lived on a desktop without an account into the signed-in person's account.
//! The bot gets an id minted here; the id it had on the device becomes its import key, so a move
//! interrupted at any step can be repeated and finds the bot the first attempt created. The
//! check reports every part of the bot this account can't take; the import re-runs the same
//! review under the owner's lock and stores the bot only when nothing blocks it.

use super::{
    apps::{BOT_APP_LIMIT_REACHED, MAX_APPS_PER_BOT, unrestricted_row},
    crud::{custom_bot_count, server_bot_refusal},
    model::{Bot, BotSettings, CreateBotRequest},
    provision::new_bot_row,
    validation::{
        self, BOT_CAPABILITY_LOCATION, BOT_LIMIT_REACHED, BOT_SERVER_LIMIT_REACHED, MAX_CUSTOM_BOTS,
    },
    workspace::{BOT_WORKSPACE_LOCKED, workspaces_among},
};
use crate::{
    audit, audit_branch,
    db::coordination::coordinate,
    entity::{bot, bot_app, membership, profile},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::user::ensure_user_exists,
    state::AppState,
};
use axum::{Extension, Json, extract::State, http::StatusCode};
use chrono::{DateTime, FixedOffset};
use flow_like_types::bot::{BotCapability, BotExecutionMode, BotLocation};
use flow_like_types::bot_restrictions::{APP_REFUSES_BOTS, app_refuses_bots_message};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait,
    QueryFilter, QuerySelect, Statement, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc};
use utoipa::ToSchema;

pub(super) const BOT_INVALID: &str = "BOT_INVALID";
pub(super) const BOT_PROFILE_UNAVAILABLE: &str = "BOT_PROFILE_UNAVAILABLE";
pub(super) const BOT_DEVICE_UNAVAILABLE: &str = "BOT_DEVICE_UNAVAILABLE";
pub(super) const BOT_APP_UNAVAILABLE: &str = "BOT_APP_UNAVAILABLE";

/// The longest `local_id` a device may send.
pub(crate) const LOCAL_ID_MAX_CHARS: usize = 128;

/// A bot from a device. The server mints its id; `local_id` only finds it again when the move
/// is repeated.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ImportBotRequest {
    /// The bot's id on the device: the key a repeated move finds this bot by, never its id in
    /// your account. 1 to 128 characters.
    #[schema(example = "bot_tz4a98xxat96iws9zmbrgj3a")]
    pub local_id: String,
    #[serde(flatten)]
    pub bot: CreateBotRequest,
    /// Apps the bot uses; you must be a member of each. At most 200.
    #[serde(default)]
    pub app_ids: Vec<String>,
    /// When the bot was created on the device; a time in the future counts as now.
    pub created_at: Option<DateTime<FixedOffset>>,
}

/// The part of a bot an issue is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportIssuePart {
    Bot,
    Profile,
    Device,
    App,
    Capability,
}

/// Something about a bot this account can't take as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ImportIssue {
    pub part: ImportIssuePart,
    /// `BOT_INVALID`, `BOT_LIMIT_REACHED`, `BOT_SERVER_LIMIT_REACHED`, `BOT_CAPABILITY_LOCATION`,
    /// `BOT_PROFILE_UNAVAILABLE`, `BOT_DEVICE_UNAVAILABLE`, `BOT_DOCKER_UNAVAILABLE`, `BOT_APP_UNAVAILABLE`,
    /// `BOT_APP_LIMIT_REACHED`, `BOT_WORKSPACE_LOCKED` or `APP_REFUSES_BOTS`.
    #[schema(example = "BOT_APP_UNAVAILABLE")]
    pub code: String,
    pub message: String,
    /// The app, profile, device or capability the issue is about.
    pub ref_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ImportBotCheck {
    /// Empty when the bot can move.
    pub issues: Vec<ImportIssue>,
}

impl ImportIssue {
    fn new(
        part: ImportIssuePart,
        code: &str,
        message: impl Into<String>,
        ref_id: Option<&str>,
    ) -> Self {
        Self {
            part,
            code: code.to_string(),
            message: message.into(),
            ref_id: ref_id.map(str::to_string),
        }
    }

    /// What an import this issue blocks answers.
    fn into_error(self) -> ApiError {
        let status = match self.code.as_str() {
            BOT_LIMIT_REACHED | BOT_SERVER_LIMIT_REACHED | BOT_WORKSPACE_LOCKED => {
                StatusCode::CONFLICT
            }
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        };
        ApiError::coded(status, self.code, self.message)
    }
}

/// A `local_id` is an opaque key of 1 to 128 characters; anything else is a broken client.
fn check_local_id(local_id: &str) -> Result<(), ApiError> {
    let chars = local_id.chars().count();
    if (1..=LOCAL_ID_MAX_CHARS).contains(&chars) {
        return Ok(());
    }
    Err(ApiError::bad_request(format!(
        "local_id has {chars} characters; send the bot's id on your device, 1 to {LOCAL_ID_MAX_CHARS} characters"
    )))
}

/// The owner's live bot an earlier move of this local id created.
async fn moved_before<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    local_id: &str,
) -> Result<Option<bot::Model>, DbErr> {
    bot::Entity::find()
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::ImportKey.eq(local_id))
        .filter(bot::Column::SystemKey.is_null())
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await
}

/// A deleted bot of the owner keeps its import key until a new move of that local id needs
/// it, so the unique key (owner, import key) admits the new bot.
async fn release_import_key<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    local_id: &str,
) -> Result<(), DbErr> {
    bot::Entity::update_many()
        .col_expr(bot::Column::ImportKey, Expr::value(Option::<String>::None))
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::ImportKey.eq(local_id))
        .filter(bot::Column::DeletedAt.is_not_null())
        .exec(db)
        .await
        .map(|_| ())
}

/// A bot that passed every check, ready to be stored under a minted id.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct NewBot {
    pub import_key: String,
    pub preset_id: Option<String>,
    pub settings: BotSettings,
    pub app_ids: Vec<String>,
    pub created_at: Option<DateTime<FixedOffset>>,
}

/// What importing a bot would do.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Review {
    /// The caller moved this bot before; importing it again returns it.
    Imported(bot::Model),
    Ready(NewBot),
    Blocked(Vec<ImportIssue>),
}

impl Review {
    pub fn into_issues(self) -> Vec<ImportIssue> {
        match self {
            Self::Blocked(issues) => issues,
            Self::Imported(_) | Self::Ready(_) => Vec::new(),
        }
    }
}

/// Every issue that keeps `request` from moving into the owner's account, in the order an
/// import reports them, or the bot an earlier move of this local id created. A malformed
/// `local_id` is a bad request, not an issue.
pub(super) async fn review<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    request: &ImportBotRequest,
) -> Result<Review, ApiError> {
    check_local_id(&request.local_id)?;
    if let Some(existing) = moved_before(db, owner_id, &request.local_id).await? {
        return Ok(Review::Imported(existing));
    }
    let mut issues = Vec::new();
    if custom_bot_count(db, owner_id).await? >= MAX_CUSTOM_BOTS {
        issues.push(ImportIssue::new(
            ImportIssuePart::Bot,
            BOT_LIMIT_REACHED,
            format!(
                "You already have {MAX_CUSTOM_BOTS} bots of your own; delete one before moving another"
            ),
            None,
        ));
    }
    if request.bot.location.unwrap_or_default() == BotLocation::Server
        && let Some(message) = server_bot_refusal(db, owner_id).await?
    {
        issues.push(ImportIssue::new(
            ImportIssuePart::Bot,
            BOT_SERVER_LIMIT_REACHED,
            message,
            None,
        ));
    }
    let settings = settings_or_issues(&request.bot, &mut issues);
    if let Some((_, settings)) = &settings
        && let Err(error) = super::crud::verify_decision_model(db, settings).await
    {
        if error.status() != StatusCode::BAD_REQUEST {
            return Err(error);
        }
        issues.push(ImportIssue::new(
            ImportIssuePart::Bot,
            BOT_INVALID,
            "Server and managed-device bots require a hosted System One decision model",
            settings.model.decision.as_deref(),
        ));
    }
    issues.extend(reference_issues(db, owner_id, &request.bot).await?);
    if let Some((_, settings)) = &settings
        && settings.execution_mode == BotExecutionMode::Docker
        && !issues
            .iter()
            .any(|issue| issue.part == ImportIssuePart::Device)
        && let Some(device_id) = settings.device_id.as_deref()
        && let Err(error) = super::crud::verify_docker_ready(db, owner_id, device_id).await
    {
        if error.public_code() != "BOT_DOCKER_UNAVAILABLE" {
            return Err(error);
        }
        issues.push(ImportIssue::new(
            ImportIssuePart::Device,
            error.public_code(),
            error
                .public_message()
                .unwrap_or("Docker is unavailable on the selected device"),
            Some(device_id),
        ));
    }
    let app_ids = requested_apps(&request.app_ids);
    issues.extend(app_issues(db, owner_id, &app_ids).await?);
    Ok(match settings {
        Some((preset_id, settings)) if issues.is_empty() => Review::Ready(NewBot {
            import_key: request.local_id.clone(),
            preset_id,
            settings,
            app_ids,
            created_at: request.created_at,
        }),
        _ => Review::Blocked(issues),
    })
}

/// The preset and settings the fields make, validated like a new bot's (a duplicate of Flux
/// keeps `flowpilot`); otherwise the fields' issues are pushed and there are none.
fn settings_or_issues(
    fields: &CreateBotRequest,
    issues: &mut Vec<ImportIssue>,
) -> Option<(Option<String>, BotSettings)> {
    let misplaced = location_issues(fields);
    let settings = match validation::moved_settings(fields.clone()) {
        Ok(valid) => Some(valid),
        Err(error) if error.public_code() == BOT_CAPABILITY_LOCATION && !misplaced.is_empty() => {
            None
        }
        Err(error) => {
            issues.push(ImportIssue::new(
                ImportIssuePart::Bot,
                BOT_INVALID,
                error
                    .public_message()
                    .unwrap_or("The bot's settings are invalid"),
                None,
            ));
            None
        }
    };
    issues.extend(misplaced);
    settings
}

/// One issue per capability the bot's location can't hold.
fn location_issues(fields: &CreateBotRequest) -> Vec<ImportIssue> {
    let location = fields.location.unwrap_or_default();
    let asked: HashSet<BotCapability> = fields
        .capabilities
        .iter()
        .flatten()
        .filter_map(|id| BotCapability::parse(id))
        .collect();
    BotCapability::ALL
        .into_iter()
        .filter(|capability| asked.contains(capability) && !location.allows(*capability))
        .map(|capability| {
            ImportIssue::new(
                ImportIssuePart::Capability,
                BOT_CAPABILITY_LOCATION,
                validation::location_refusal(location, capability),
                Some(capability.as_str()),
            )
        })
        .collect()
}

fn reference(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|id| !id.is_empty())
}

/// The profile and device the bot points at must be the owner's.
async fn reference_issues<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    fields: &CreateBotRequest,
) -> Result<Vec<ImportIssue>, DbErr> {
    let mut issues = Vec::new();
    if let Some(profile_id) = reference(&fields.profile_id)
        && !owns_profile(db, owner_id, profile_id).await?
    {
        issues.push(ImportIssue::new(
            ImportIssuePart::Profile,
            BOT_PROFILE_UNAVAILABLE,
            format!("Profile '{profile_id}' is not one of your profiles"),
            Some(profile_id),
        ));
    }
    if let Some(device_id) = reference(&fields.device_id)
        && !owns_device(db, owner_id, device_id).await?
    {
        issues.push(ImportIssue::new(
            ImportIssuePart::Device,
            BOT_DEVICE_UNAVAILABLE,
            format!("Device '{device_id}' is not an active device of yours"),
            Some(device_id),
        ));
    }
    Ok(issues)
}

pub(super) async fn owns_profile<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    profile_id: &str,
) -> Result<bool, DbErr> {
    let found = profile::Entity::find()
        .select_only()
        .column(profile::Column::Id)
        .filter(profile::Column::UserId.eq(owner_id))
        .filter(profile::Column::Id.eq(profile_id))
        .filter(profile::Column::DeletedAt.is_null())
        .into_tuple::<String>()
        .one(db)
        .await?;
    Ok(found.is_some())
}

pub(super) async fn owns_device<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    device_id: &str,
) -> Result<bool, DbErr> {
    let found = db
        .query_one_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            r#"SELECT "id" FROM "ManagedDevice" WHERE "id" = $1 AND "ownerId" = $2 AND "status" = 'active'"#,
            [device_id.into(), owner_id.into()],
        ))
        .await?;
    Ok(found.is_some())
}

/// Trimmed app ids, each once, in the order asked.
fn requested_apps(app_ids: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    app_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty() && seen.insert(*id))
        .map(str::to_string)
        .collect()
}

/// Each app must be one the owner is a member of and no bot's workspace, like attaching it.
async fn app_issues<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    app_ids: &[String],
) -> Result<Vec<ImportIssue>, DbErr> {
    if app_ids.len() as u64 > MAX_APPS_PER_BOT {
        return Ok(vec![ImportIssue::new(
            ImportIssuePart::App,
            BOT_APP_LIMIT_REACHED,
            format!(
                "A bot uses at most {MAX_APPS_PER_BOT} apps; this one uses {}",
                app_ids.len()
            ),
            None,
        )]);
    }
    if app_ids.is_empty() {
        return Ok(Vec::new());
    }
    let member_of: HashSet<String> = membership::Entity::find()
        .select_only()
        .column(membership::Column::AppId)
        .filter(membership::Column::UserId.eq(owner_id))
        .filter(membership::Column::AppId.is_in(app_ids.iter().cloned()))
        .into_tuple::<String>()
        .all(db)
        .await?
        .into_iter()
        .collect();
    let workspaces = workspaces_among(db, app_ids.iter()).await?;
    let refusing = crate::bot_app_gate::refusing_among(db, app_ids.iter()).await?;
    Ok(app_ids
        .iter()
        .filter_map(|app_id| {
            if !member_of.contains(app_id) {
                Some(ImportIssue::new(
                    ImportIssuePart::App,
                    BOT_APP_UNAVAILABLE,
                    format!("You are not a member of app '{app_id}', so your bot can't use it"),
                    Some(app_id),
                ))
            } else if workspaces.contains(app_id) {
                Some(ImportIssue::new(
                    ImportIssuePart::App,
                    BOT_WORKSPACE_LOCKED,
                    format!("App '{app_id}' is a bot's workspace, so your bot can't use it"),
                    Some(app_id),
                ))
            } else if let Some(name) = refusing.get(app_id) {
                Some(ImportIssue::new(
                    ImportIssuePart::App,
                    APP_REFUSES_BOTS,
                    app_refuses_bots_message(name),
                    Some(app_id),
                ))
            } else {
                None
            }
        })
        .collect())
}

/// What an import did.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Imported {
    Created(bot::Model),
    /// The caller moved this bot before; nothing changed, listed apps included.
    Existing(bot::Model),
}

impl Imported {
    /// 201 for a bot this call stored, 200 for one the caller moved before.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Created(_) => StatusCode::CREATED,
            Self::Existing(_) => StatusCode::OK,
        }
    }
}

/// Stores the bot under a minted id with its apps unless the caller moved it before. The
/// caller holds the owner's lock, so the review re-run here is exact, the bot count included.
pub(super) async fn import_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    request: &ImportBotRequest,
    now: DateTime<FixedOffset>,
) -> Result<Imported, ApiError> {
    match review(db, owner_id, request).await? {
        Review::Imported(existing) => Ok(Imported::Existing(existing)),
        Review::Ready(ready) => insert(db, owner_id, ready, now)
            .await
            .map(Imported::Created),
        Review::Blocked(issues) => Err(issues.into_iter().next().map_or_else(
            || ApiError::internal("A bot import was blocked without an issue"),
            ImportIssue::into_error,
        )),
    }
}

async fn insert<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    ready: NewBot,
    now: DateTime<FixedOffset>,
) -> Result<bot::Model, ApiError> {
    release_import_key(db, owner_id, &ready.import_key).await?;
    let created_at = ready.created_at.filter(|at| *at <= now).unwrap_or(now);
    let mut row = new_bot_row(owner_id, None, ready.preset_id, &ready.settings, created_at)?;
    row.import_key = Set(Some(ready.import_key));
    row.updated_at = Set(now);
    let stored = row.insert(db).await?;
    if !ready.app_ids.is_empty() {
        bot_app::Entity::insert_many(
            ready
                .app_ids
                .iter()
                .map(|app_id| unrestricted_row(&stored.id, app_id, now)),
        )
        .exec_without_returning(db)
        .await?;
    }
    Ok(stored)
}

#[utoipa::path(
    post,
    path = "/bots/import/check",
    tag = "Bots",
    request_body = ImportBotRequest,
    responses(
        (status = 200, description = "Everything that keeps this bot from moving into your account, part by part; empty when nothing does, also when you moved it before. Checking changes nothing.", body = ImportBotCheck),
        (status = 400, description = "local_id is empty or longer than 128 characters"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/import/check", skip(state, user, body))]
pub async fn check_bot_import(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<ImportBotRequest>,
) -> Result<Json<ImportBotCheck>, ApiError> {
    let sub = user.sub()?;
    let issues = review(&state.db, &sub, &body).await?.into_issues();
    Ok(Json(ImportBotCheck { issues }))
}

#[utoipa::path(
    post,
    path = "/bots/import",
    tag = "Bots",
    request_body = ImportBotRequest,
    responses(
        (status = 201, description = "The bot is in your account under a new id, with the apps it uses; its id on your device finds it again when you repeat the move", body = Bot),
        (status = 200, description = "You moved this bot (this local_id) before; it comes back unchanged, so an interrupted move can be repeated", body = Bot),
        (status = 400, description = "local_id is empty or longer than 128 characters"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 409, description = "BOT_LIMIT_REACHED: you already have 50 bots of your own; BOT_SERVER_LIMIT_REACHED: the bot would run on the server and your plan's bots on the server are used up (bots on a desktop or device don't count); BOT_WORKSPACE_LOCKED: one of its apps is a bot's workspace"),
        (status = 422, description = "The first issue the check reports: BOT_INVALID, BOT_CAPABILITY_LOCATION, BOT_PROFILE_UNAVAILABLE, BOT_DEVICE_UNAVAILABLE, BOT_APP_LIMIT_REACHED or BOT_APP_UNAVAILABLE")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/import", skip(state, user, body))]
pub async fn import_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<ImportBotRequest>,
) -> Result<(StatusCode, Json<Bot>), ApiError> {
    let sub = user.sub()?;
    ensure_user_exists(&state, &sub).await?;
    let request = Arc::new(body);
    let imported = state
        .transaction(|txn| {
            let (sub, request) = (sub.clone(), request.clone());
            Box::pin(async move {
                coordinate(txn, "bot-owner", &[&sub]).await?;
                import_in(txn, &sub, &request, chrono::Utc::now().fixed_offset()).await
            })
        })
        .await?;
    let status = imported.status();
    let (Imported::Created(row) | Imported::Existing(row)) = imported;
    let bot = Bot::from(row);
    if status == StatusCode::CREATED {
        record_import(&state, &user, &bot, &requested_apps(&request.app_ids)).await;
    }
    Ok((status, Json(bot)))
}

/// `bot.import` on the person's chain and, like attaching, `bot.app.attach` on each app's.
async fn record_import(state: &AppState, user: &AppUser, bot: &Bot, app_ids: &[String]) {
    audit!(
        state,
        user,
        "bot.import",
        "Bot",
        bot.id,
        serde_json::json!({
            "apps": app_ids,
            "capabilities": bot.capabilities,
            "location": bot.location,
            "preset_id": bot.preset_id,
        })
    );
    for app_id in app_ids {
        audit_branch!(state, user, app_id, "bot.app.attach", "Bot", bot.id);
    }
}

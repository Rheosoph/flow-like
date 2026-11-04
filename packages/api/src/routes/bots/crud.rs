use super::{
    lock_bot,
    model::{Bot, BotList, BotSettings, CreateBotRequest, UpdateBotRequest, location_column},
    owned_bot, provision,
    validation::{
        self, BOT_LIMIT_REACHED, BOT_SERVER_LIMIT_REACHED, BOT_SYSTEM_UNDELETABLE, MAX_CUSTOM_BOTS,
    },
    workspace,
};
use crate::{
    audit, capacity,
    db::coordination::coordinate,
    entity::{bot, bot_app, bot_secret, profile},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::{inbox, user::ensure_user_exists},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use flow_like_types::bot::{BotCapability, BotLocation, BotSystemKind};
use sea_orm::{
    ActiveEnum, ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait,
    DatabaseTransaction, DbErr, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect, Statement,
    sea_query::Expr,
};

/// The owner's bots that count toward the limit: created by them and not deleted.
pub(super) async fn custom_bot_count<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
) -> Result<u64, DbErr> {
    bot::Entity::find()
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::SystemKey.is_null())
        .filter(bot::Column::DeletedAt.is_null())
        .count(db)
        .await
}

/// The owner's bots that count toward their plan's server-bot limit: created by them, on the
/// server and not deleted.
pub(super) async fn server_bot_count<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
) -> Result<u64, DbErr> {
    bot::Entity::find()
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::SystemKey.is_null())
        .filter(bot::Column::DeletedAt.is_null())
        .filter(bot::Column::Location.eq(location_column(BotLocation::Server)))
        .count(db)
        .await
}

/// Why the owner can't keep one more bot on the server, once their plan's limit is reached.
pub(super) async fn server_bot_refusal<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
) -> Result<Option<String>, ApiError> {
    if !crate::quota::enforcing() {
        return Ok(None);
    }
    let plan = capacity::plan_of(db, owner_id).await?;
    let Ok(limit) = u64::try_from(capacity::max_server_bots(&plan)) else {
        return Ok(None);
    };
    let count = server_bot_count(db, owner_id).await?;
    Ok((count >= limit).then(|| {
        format!(
            "Your {} plan allows {limit} bots on the server and you have {count}; move one to a desktop or device, delete one, or upgrade your plan",
            plan.to_value()
        )
    }))
}

/// Refuses one more bot on the server past the owner's plan. Assistant and Flux never count.
pub(super) async fn admit_server_bot<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
) -> Result<(), ApiError> {
    match server_bot_refusal(db, owner_id).await? {
        Some(message) => Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_SERVER_LIMIT_REACHED,
            message,
        )),
        None => Ok(()),
    }
}

/// Refuses a new bot of the owner past the all-bots safety cap ([`MAX_CUSTOM_BOTS`]) and, on
/// the server, past their plan. The caller holds the owner's `bot-owner` lock, so both counts
/// are exact under concurrent creates.
pub(super) async fn admit_new_bot<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    location: BotLocation,
) -> Result<(), ApiError> {
    if custom_bot_count(db, owner_id).await? >= MAX_CUSTOM_BOTS {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_LIMIT_REACHED,
            format!(
                "You already have {MAX_CUSTOM_BOTS} bots of your own; delete one before creating another"
            ),
        ));
    }
    if location == BotLocation::Server {
        admit_server_bot(db, owner_id).await?;
    }
    Ok(())
}

fn not_found(bot_id: &str) -> ApiError {
    ApiError::not_found(format!("Bot '{bot_id}' not found"))
}

/// On an update only what changed is checked, so a profile deleted later doesn't block
/// renaming the bot.
fn profile_changed(before: Option<&BotSettings>, after: &BotSettings) -> bool {
    before.is_none_or(|before| {
        before.profile_id != after.profile_id
            || (!before.has(BotCapability::ProfileApps) && after.has(BotCapability::ProfileApps))
    })
}

fn device_changed(before: Option<&BotSettings>, after: &BotSettings) -> bool {
    before.is_none_or(|before| {
        before.device_id != after.device_id
            || (before.location != BotLocation::Device && after.location == BotLocation::Device)
    })
}

async fn verify_profile<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    profile_id: &str,
) -> Result<(), ApiError> {
    let found = profile::Entity::find()
        .select_only()
        .column(profile::Column::Id)
        .filter(profile::Column::UserId.eq(owner_id))
        .filter(profile::Column::Id.eq(profile_id))
        .filter(profile::Column::DeletedAt.is_null())
        .into_tuple::<String>()
        .one(db)
        .await?;
    found.map(|_| ()).ok_or_else(|| {
        ApiError::bad_request(format!(
            "Profile '{profile_id}' is not one of your profiles"
        ))
    })
}

async fn verify_device<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    device_id: &str,
) -> Result<(), ApiError> {
    let found = db
        .query_one_raw(Statement::from_sql_and_values(
            db.get_database_backend(),
            r#"SELECT "id" FROM "ManagedDevice" WHERE "id" = $1 AND "ownerId" = $2 AND "status" = 'active'"#,
            [device_id.into(), owner_id.into()],
        ))
        .await?;
    found.map(|_| ()).ok_or_else(|| {
        ApiError::bad_request(format!(
            "Device '{device_id}' is not an active device of yours"
        ))
    })
}

/// The profile and device a bot points at must be the owner's.
async fn verify_references<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    before: Option<&BotSettings>,
    after: &BotSettings,
) -> Result<(), ApiError> {
    if let Some(profile_id) = after.profile_id.as_deref()
        && profile_changed(before, after)
    {
        verify_profile(db, owner_id, profile_id).await?;
    }
    if let Some(device_id) = after.device_id.as_deref()
        && device_changed(before, after)
    {
        verify_device(db, owner_id, device_id).await?;
    }
    Ok(())
}

/// Inserts a bot the owner created unless [`admit_new_bot`] refuses it, under the owner's lock.
async fn insert_custom_bot(
    state: &AppState,
    owner_id: &str,
    location: BotLocation,
    row: bot::ActiveModel,
) -> Result<bot::Model, ApiError> {
    state
        .transaction(|txn| {
            let (row, owner_id) = (row.clone(), owner_id.to_string());
            Box::pin(async move {
                coordinate(txn, "bot-owner", &[&owner_id]).await?;
                admit_new_bot(txn, &owner_id, location).await?;
                Ok(row.insert(txn).await?)
            })
        })
        .await
}

/// Writes `settings` over the stored bot unless it was deleted or changed hands meanwhile.
async fn save_settings<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    current: bot::Model,
    settings: &BotSettings,
) -> Result<bot::Model, ApiError> {
    let bot_id = current.id.clone();
    let mut row: bot::ActiveModel = current.into();
    settings.write(&mut row)?;
    row.updated_at = Set(chrono::Utc::now().fixed_offset());
    bot::Entity::update(row)
        .validate()?
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::DeletedAt.is_null())
        .exec(db)
        .await
        .map_err(|error| match error {
            DbErr::RecordNotUpdated => not_found(&bot_id),
            error => error.into(),
        })
}

/// A bot after a patch and the names of the fields it changed.
#[derive(Debug)]
pub(super) struct Patched {
    pub bot: bot::Model,
    pub changed: Vec<&'static str>,
}

/// Re-reads the bot, merges `body` into what is stored and writes the result if anything
/// changed. The caller holds the bot's lock, so concurrent patches apply one after the other
/// instead of the later one restoring what the earlier one changed; when `body` sets the
/// location to `server` it holds the owner's lock first, so the server-bot count is exact.
pub(super) async fn patch_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    body: UpdateBotRequest,
) -> Result<Patched, ApiError> {
    let current = owned_bot(db, owner_id, bot_id).await?;
    let system = current.system_key.as_deref().and_then(BotSystemKind::parse);
    let before = BotSettings::from_row(&current);
    let after = validation::apply_patch(&before, system, body)?;
    if after == before {
        return Ok(Patched {
            bot: current,
            changed: Vec::new(),
        });
    }
    verify_references(db, owner_id, Some(&before), &after).await?;
    if system.is_none()
        && before.location != BotLocation::Server
        && after.location == BotLocation::Server
    {
        admit_server_bot(db, owner_id).await?;
    }
    let changed = before.changed_fields(&after);
    Ok(Patched {
        bot: save_settings(db, owner_id, current, &after).await?,
        changed,
    })
}

/// Marks the bot deleted and drops its app attachments and secrets with it.
async fn soft_delete(
    txn: &DatabaseTransaction,
    owner_id: &str,
    bot_id: &str,
) -> Result<(), ApiError> {
    lock_bot(txn, bot_id).await?;
    let now = chrono::Utc::now().fixed_offset();
    let deleted = bot::Entity::update_many()
        .col_expr(bot::Column::DeletedAt, Expr::value(now))
        .col_expr(bot::Column::UpdatedAt, Expr::value(now))
        .filter(bot::Column::Id.eq(bot_id))
        .filter(bot::Column::OwnerId.eq(owner_id))
        .filter(bot::Column::SystemKey.is_null())
        .filter(bot::Column::DeletedAt.is_null())
        .exec(txn)
        .await?;
    if deleted.rows_affected == 0 {
        return Err(not_found(bot_id));
    }
    bot_app::Entity::delete_many()
        .filter(bot_app::Column::BotId.eq(bot_id))
        .exec(txn)
        .await?;
    bot_secret::Entity::delete_many()
        .filter(bot_secret::Column::BotId.eq(bot_id))
        .exec(txn)
        .await?;
    crate::mail_ingress::bot_address::release_in(txn, bot_id).await?;
    Ok(())
}

#[utoipa::path(
    get,
    path = "/bots",
    tag = "Bots",
    responses(
        (status = 200, description = "Your bots: Assistant and Flux first, then the bots you created, oldest first. Assistant and Flux are set up on the first call.", body = BotList),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots", skip(state, user))]
pub async fn list_bots(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<BotList>, ApiError> {
    let sub = user.sub()?;
    let bots = provision::bots_with_system(&state, &sub).await?;
    Ok(Json(BotList {
        bots: bots.into_iter().map(Bot::from).collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/bots",
    tag = "Bots",
    request_body = CreateBotRequest,
    responses(
        (status = 200, description = "The new bot", body = Bot),
        (status = 400, description = "A field is empty, too long or malformed, a capability is unknown, or the profile or device is not yours"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 409, description = "BOT_LIMIT_REACHED: you already have 50 bots of your own; BOT_SERVER_LIMIT_REACHED: the bot would run on the server and your plan's bots on the server are used up (bots on a desktop or device don't count)"),
        (status = 422, description = "BOT_CAPABILITY_LOCATION: a bot on the server can't use a computer, a browser or the command line; BOT_CAPABILITIES_FIXED: only Flux and its duplicates hold 'flowpilot'")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots", skip(state, user, body))]
pub async fn create_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<CreateBotRequest>,
) -> Result<Json<Bot>, ApiError> {
    let sub = user.sub()?;
    let (preset_id, settings) = validation::create_settings(body)?;
    verify_references(&state.db, &sub, None, &settings).await?;
    ensure_user_exists(&state, &sub).await?;
    let now = chrono::Utc::now().fixed_offset();
    let row = provision::new_bot_row(&sub, None, preset_id, &settings, now)?;
    let bot = Bot::from(insert_custom_bot(&state, &sub, settings.location, row).await?);
    audit!(
        state,
        user,
        "bot.create",
        "Bot",
        bot.id,
        serde_json::json!({
            "capabilities": bot.capabilities,
            "location": bot.location,
            "preset_id": bot.preset_id,
        })
    );
    Ok(Json(bot))
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The bot", body = Bot),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}", skip(state, user))]
pub async fn get_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<Bot>, ApiError> {
    let sub = user.sub()?;
    Ok(Json(Bot::from(owned_bot(&state.db, &sub, &bot_id).await?)))
}

#[utoipa::path(
    patch,
    path = "/bots/{bot_id}",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = UpdateBotRequest,
    responses(
        (status = 200, description = "The bot after the change", body = Bot),
        (status = 400, description = "A field is empty, too long or malformed, a capability is unknown, or the profile or device is not yours"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_SERVER_LIMIT_REACHED: the bot would move to the server and your plan's bots on the server are used up (Assistant, Flux and bots on a desktop or device don't count)"),
        (status = 422, description = "BOT_CAPABILITY_LOCATION: the bot would run on the server with a computer, browser or command-line capability; BOT_CAPABILITIES_FIXED: the capabilities of Flux and of bots with 'flowpilot' can't be changed, and no other bot gets 'flowpilot'")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PATCH /bots/{bot_id}", skip(state, user, body))]
pub async fn update_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<UpdateBotRequest>,
) -> Result<Json<Bot>, ApiError> {
    let sub = user.sub()?;
    let patched = state
        .transaction(|txn| {
            let (sub, bot_id, body) = (sub.clone(), bot_id.clone(), body.clone());
            Box::pin(async move {
                if body.location == Some(BotLocation::Server) {
                    coordinate(txn, "bot-owner", &[&sub]).await?;
                }
                lock_bot(txn, &bot_id).await?;
                patch_in(txn, &sub, &bot_id, body).await
            })
        })
        .await?;
    if patched.changed.contains(&"location") {
        crate::bot_host::task_schedules::sync_bot(&state, &bot_id).await;
    }
    if !patched.changed.is_empty() {
        audit!(
            state,
            user,
            "bot.update",
            "Bot",
            bot_id,
            serde_json::json!({ "fields": patched.changed })
        );
    }
    Ok(Json(Bot::from(patched.bot)))
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 204, description = "The bot is deleted with its app attachments, secrets, permission grants, host heartbeats and conversation summaries, and leaves its conversations. Its workspace goes too, files and databases included; a large one finishes in the background. What it did keeps showing it."),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people manage bots, not API keys or running flows"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_SYSTEM_UNDELETABLE: Assistant and Flux can't be deleted")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bots/{bot_id}", skip(state, user))]
pub async fn delete_bot(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let current = owned_bot(&state.db, &sub, &bot_id).await?;
    if let Some(key) = current.system_key.as_deref() {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            BOT_SYSTEM_UNDELETABLE,
            format!("The built-in bot '{key}' can't be deleted"),
        ));
    }
    let (first, workspace_deletion) = state
        .transaction(|txn| {
            let (sub, bot_id) = (sub.clone(), bot_id.clone());
            Box::pin(async move {
                soft_delete(txn, &sub, &bot_id).await?;
                let workspace_deletion = workspace::queue_deletion(txn, &sub, &bot_id).await?;
                let first = inbox::retirement::retire_next(txn, &sub, &bot_id).await?;
                Ok::<_, ApiError>((first, workspace_deletion))
            })
        })
        .await?;
    inbox::retirement::finish_retirement(&state, &sub, &bot_id, first).await;
    if let Some(queued) = workspace_deletion {
        workspace::delete_queued(&state, &queued).await;
    }
    audit!(state, user, "bot.delete", "Bot", bot_id);
    Ok(StatusCode::NO_CONTENT)
}

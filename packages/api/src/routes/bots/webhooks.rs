//! Bot webhooks: public URLs that wake a bot. The owner creates one per integration (the secret is
//! shown once, only its hash is stored); a call with the secret in a header or query parameter,
//! or an optional HMAC signature keyed by the secret's hash, writes a `trigger` entry into the
//! bot's conversation with its owner and answers with the turn it queued. Servers only: a bot
//! that lives in a local store has no public URL.

use super::{
    fire::{FiredTurn, fire_trigger, turn_state, wait_for_turn},
    lock_bot, owned_bot, workspace,
};
use crate::{
    credentials::{CredentialsAccess, RuntimeCredentials},
    entity::{bot, bot_webhook},
    error::ApiError,
    middleware::jwt::{AppUser, viewer_authorization},
    state::AppState,
};
use axum::{
    Extension, Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{DateTime, FixedOffset, Utc};
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_types::{
    bot_message::WorkspaceHandle,
    inbox::{
        TRIGGER_PAYLOAD_MAX_BYTES, TRIGGER_TEXT_MAX_CHARS, TRIGGER_TITLE_MAX_CHARS, TriggerBody,
        TriggerSource,
    },
};
use hmac::{Hmac, Mac};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};
use utoipa::{IntoParams, ToSchema};

pub const MAX_PAYLOAD_BYTES: usize = 256 * 1024;
pub const MAX_WAIT_SECS: u64 = 30;
const MAX_WEBHOOKS_PER_BOT: u64 = 10;
const NAME_MAX_CHARS: usize = 64;
const INSTRUCTION_MAX_CHARS: usize = 4_000;
const CALLS_PER_MINUTE: u32 = 30;
const REFUSALS_PER_MINUTE: u32 = 10;
const SECRET_HEADER: &str = "x-flow-like-webhook-secret";
const SIGNATURE_HEADER: &str = "x-flow-like-signature";
const WEBHOOK_CAPABILITY: &str = "webhook";
const WEBHOOK_LIMIT_REACHED: &str = "BOT_WEBHOOK_LIMIT_REACHED";
const WEBHOOK_CAPABILITY_OFF: &str = "BOT_WEBHOOK_CAPABILITY_OFF";
const WEBHOOK_DISABLED: &str = "BOT_WEBHOOK_DISABLED";
const WEBHOOK_REFUSED: &str = "BOT_WEBHOOK_UNAUTHORIZED";
const PAYLOAD_TOO_LARGE: &str = "BOT_WEBHOOK_PAYLOAD_TOO_LARGE";

/// A webhook as its owner sees it; never the secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct BotWebhookInfo {
    pub id: String,
    pub bot_id: String,
    pub name: String,
    /// What the bot is asked to do with each call.
    pub instruction: String,
    pub enabled: bool,
    /// The path to call under the API base: `POST {api}{path}`.
    pub path: String,
    pub created_at: DateTime<FixedOffset>,
    pub last_called_at: Option<DateTime<FixedOffset>>,
}

impl From<bot_webhook::Model> for BotWebhookInfo {
    fn from(row: bot_webhook::Model) -> Self {
        Self {
            path: hook_path(&row.id),
            id: row.id,
            bot_id: row.bot_id,
            name: row.name,
            instruction: row.instruction,
            enabled: row.enabled,
            created_at: row.created_at,
            last_called_at: row.last_called_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BotWebhookList {
    pub webhooks: Vec<BotWebhookInfo>,
}

/// A new or rotated webhook with the only copy of its credentials.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct IssuedBotWebhook {
    pub webhook: BotWebhookInfo,
    /// Send it in `X-Flow-Like-Webhook-Secret`, as a bearer token, or in the `secret` query
    /// parameter when the sender can only configure a URL. Shown once.
    pub secret: String,
    /// Optional alternative to sending the secret: sign the raw body with HMAC-SHA256 under
    /// this key (the lowercase hex blake3 hash of the secret) and send
    /// `X-Flow-Like-Signature: sha256=<hex>`.
    /// Shown once.
    pub signing_key: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBotWebhookRequest {
    /// At most 64 characters; it titles the entry the bot sees.
    pub name: String,
    /// What the bot should do with a call, at most 4,000 characters.
    #[serde(default)]
    pub instruction: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateBotWebhookRequest {
    pub name: Option<String>,
    pub instruction: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Deserialize, IntoParams)]
pub struct HookQuery {
    /// Webhook secret for senders that can only configure a URL. Keep the complete URL private.
    /// Omit this when using a secret header, bearer token, or HMAC signature.
    pub secret: Option<String>,
    /// Seconds to wait for the bot's reply, at most 30. Without it the call answers 202 at once.
    pub wait: Option<u64>,
}

pub(crate) fn hook_path(id: &str) -> String {
    format!("/bots/hooks/{id}")
}

pub(crate) fn hash_secret(secret: &str) -> String {
    blake3::hash(secret.as_bytes()).to_hex().to_string()
}

fn new_secret() -> String {
    format!(
        "whsec_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn text_within(label: &str, value: &str, max: usize) -> Result<String, ApiError> {
    let value = value.trim();
    if value.chars().count() > max {
        return Err(ApiError::bad_request(format!(
            "{label} has at most {max} characters; this one has {}",
            value.chars().count()
        )));
    }
    Ok(value.to_string())
}

fn name_of(value: &str) -> Result<String, ApiError> {
    let name = text_within("A webhook name", value, NAME_MAX_CHARS)?;
    if name.is_empty() {
        return Err(ApiError::bad_request("A webhook needs a name"));
    }
    Ok(name)
}

fn has_capability(bot: &bot::Model) -> bool {
    bot.capabilities.iter().any(|id| id == WEBHOOK_CAPABILITY)
}

fn refuse_without_capability(bot: &bot::Model) -> Result<(), ApiError> {
    if has_capability(bot) {
        Ok(())
    } else {
        Err(ApiError::coded(
            StatusCode::FORBIDDEN,
            WEBHOOK_CAPABILITY_OFF,
            format!(
                "Bot '{}' doesn't have the webhook capability switched on",
                bot.id
            ),
        ))
    }
}

pub(crate) struct WindowLimiter {
    limit: u32,
    window: Duration,
    hits: Mutex<HashMap<String, (Instant, u32)>>,
}

impl WindowLimiter {
    pub(crate) fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window,
            hits: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn exhausted(&self, key: &str, now: Instant) -> bool {
        let hits = self
            .hits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        hits.get(key).is_some_and(|(start, count)| {
            now.duration_since(*start) < self.window && *count >= self.limit
        })
    }

    pub(crate) fn allow(&self, key: &str, now: Instant) -> bool {
        let mut hits = self
            .hits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if hits.len() > 10_000 {
            hits.retain(|_, (start, _)| now.duration_since(*start) < self.window);
        }
        let entry = hits.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 0);
        }
        if entry.1 >= self.limit {
            return false;
        }
        entry.1 += 1;
        true
    }
}

static CALLS: LazyLock<WindowLimiter> =
    LazyLock::new(|| WindowLimiter::new(CALLS_PER_MINUTE, Duration::from_secs(60)));
static REFUSALS: LazyLock<WindowLimiter> =
    LazyLock::new(|| WindowLimiter::new(REFUSALS_PER_MINUTE, Duration::from_secs(60)));

fn secret_from(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(SECRET_HEADER)
        .and_then(|value| value.to_str().ok())
        .or_else(|| viewer_authorization(headers).and_then(|value| value.strip_prefix("Bearer ")))
        .map(str::trim)
        .filter(|secret| !secret.is_empty())
}

fn signature_matches(secret_hash: &str, signature: &str, body: &[u8]) -> bool {
    if secret_hash.is_empty() {
        return false;
    }
    let Some(hex_signature) = signature.trim().strip_prefix("sha256=") else {
        return false;
    };
    let Ok(expected) = hex::decode(hex_signature) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret_hash.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

fn secret_matches(secret_hash: &str, presented: &str) -> bool {
    match blake3::Hash::from_hex(secret_hash) {
        Ok(stored) => blake3::hash(presented.as_bytes()) == stored,
        Err(_) => false,
    }
}

/// Whether the call carries the webhook's secret or a valid signature of its body.
pub(crate) fn authorized(
    secret_hash: &str,
    headers: &HeaderMap,
    query_secret: Option<&str>,
    body: &[u8],
) -> bool {
    let by_secret = secret_from(headers).is_some_and(|secret| secret_matches(secret_hash, secret));
    let by_query = query_secret
        .filter(|secret| !secret.is_empty())
        .is_some_and(|secret| secret_matches(secret_hash, secret));
    let by_signature = headers
        .get(SIGNATURE_HEADER)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|signature| signature_matches(secret_hash, signature, body));
    by_secret || by_query || by_signature
}

pub(crate) fn check_payload_size(len: usize) -> Result<(), ApiError> {
    if len > MAX_PAYLOAD_BYTES {
        return Err(ApiError::coded(
            StatusCode::PAYLOAD_TOO_LARGE,
            PAYLOAD_TOO_LARGE,
            format!("A webhook call carries at most {MAX_PAYLOAD_BYTES} bytes; this one has {len}"),
        ));
    }
    Ok(())
}

/// The call as the bot sees it: its content type and the body, parsed when it is JSON.
pub(crate) fn payload_of(content_type: Option<&str>, body: &[u8]) -> Value {
    let parsed = serde_json::from_slice::<Value>(body).ok();
    let body = match (parsed, std::str::from_utf8(body)) {
        (Some(json), _) => json,
        (None, Ok(text)) => Value::String(text.to_string()),
        (None, Err(_)) => json!({ "base64": base64_of(body) }),
    };
    json!({ "content_type": content_type.unwrap_or(""), "body": body })
}

fn base64_of(bytes: &[u8]) -> String {
    use base64::{Engine, engine::general_purpose::STANDARD};
    STANDARD.encode(bytes)
}

fn truncated(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

pub(crate) fn trigger_for(
    hook: &bot_webhook::Model,
    payload: Option<Value>,
    payload_handle: Option<WorkspaceHandle>,
) -> TriggerBody {
    let text = if hook.instruction.trim().is_empty() {
        "A webhook was called. Look at its payload and decide what to do.".to_string()
    } else {
        hook.instruction.clone()
    };
    TriggerBody {
        monitoring: false,
        source: TriggerSource::Webhook,
        ref_id: hook.id.clone(),
        title: truncated(&hook.name, TRIGGER_TITLE_MAX_CHARS),
        text: truncated(&text, TRIGGER_TEXT_MAX_CHARS),
        payload,
        payload_handle,
        bot_id: None,
    }
}

async fn store_payload(
    state: &AppState,
    owner_id: &str,
    bot_id: &str,
    hook_id: &str,
    message_id: &str,
    payload: &Value,
) -> Result<WorkspaceHandle, ApiError> {
    let bytes = serde_json::to_vec(payload).map_err(|error| {
        ApiError::internal(format!("Serializing a webhook payload failed: {error}"))
    })?;
    let app_id = workspace::ensure(state, owner_id, bot_id).await?.app_id;
    let credentials =
        RuntimeCredentials::scoped(owner_id, &app_id, state, CredentialsAccess::EditAppContent)
            .await
            .map_err(|error| {
                ApiError::internal(format!(
                    "Scoping the bot workspace credentials failed: {error}"
                ))
            })?;
    let store = credentials.to_store(false).await.map_err(|error| {
        ApiError::internal(format!("Opening the bot workspace failed: {error}"))
    })?;
    let path = format!("inbox/webhooks/{hook_id}/{message_id}.json");
    let key = store
        .construct_upload(&app_id, &path)
        .await
        .map_err(|error| ApiError::internal(format!("Bot workspace path '{path}': {error}")))?;
    let digest = hex::encode(Sha256::digest(&bytes));
    let size = bytes.len() as u64;
    store
        .as_generic()
        .put(&key, bytes::Bytes::from(bytes).into())
        .await
        .map_err(|error| ApiError::internal(format!("Saving a webhook payload failed: {error}")))?;
    Ok(WorkspaceHandle {
        app_id,
        path,
        bytes: size,
        sha256: digest,
        mime: "application/json".into(),
    })
}

async fn webhook_rows(state: &AppState, bot_id: &str) -> Result<Vec<bot_webhook::Model>, ApiError> {
    Ok(bot_webhook::Entity::find()
        .filter(bot_webhook::Column::BotId.eq(bot_id))
        .order_by_asc(bot_webhook::Column::CreatedAt)
        .order_by_asc(bot_webhook::Column::Id)
        .all(&state.db)
        .await?)
}

async fn owned_webhook(
    state: &AppState,
    owner_id: &str,
    bot_id: &str,
    hook_id: &str,
) -> Result<bot_webhook::Model, ApiError> {
    owned_bot(&state.db, owner_id, bot_id).await?;
    bot_webhook::Entity::find_by_id(hook_id)
        .filter(bot_webhook::Column::BotId.eq(bot_id))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Webhook '{hook_id}' not found")))
}

/// Inserts the webhook once the bot is the caller's, has the capability and has room.
pub(crate) async fn create_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    name: String,
    instruction: String,
    secret_hash: String,
) -> Result<bot_webhook::Model, ApiError> {
    let bot = owned_bot(db, owner_id, bot_id).await?;
    refuse_without_capability(&bot)?;
    let count = bot_webhook::Entity::find()
        .filter(bot_webhook::Column::BotId.eq(bot_id))
        .count(db)
        .await?;
    if count >= MAX_WEBHOOKS_PER_BOT {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            WEBHOOK_LIMIT_REACHED,
            format!("Bot '{bot_id}' already has {MAX_WEBHOOKS_PER_BOT} webhooks; delete one first"),
        ));
    }
    Ok(bot_webhook::ActiveModel {
        id: Set(format!("bh_{}", uuid::Uuid::new_v4().simple())),
        bot_id: Set(bot_id.to_string()),
        name: Set(name),
        secret_hash: Set(secret_hash),
        instruction: Set(instruction),
        enabled: Set(true),
        created_at: Set(Utc::now().fixed_offset()),
        last_called_at: Set(None),
    }
    .insert(db)
    .await?)
}

/// Replaces the secret; the old secret and signing key stop working with this write.
pub(crate) async fn rotate_in<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    bot_id: &str,
    hook_id: &str,
) -> Result<(bot_webhook::Model, String), ApiError> {
    owned_bot(db, owner_id, bot_id).await?;
    let row = bot_webhook::Entity::find_by_id(hook_id)
        .filter(bot_webhook::Column::BotId.eq(bot_id))
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Webhook '{hook_id}' not found")))?;
    let secret = new_secret();
    let mut active: bot_webhook::ActiveModel = row.into();
    active.secret_hash = Set(hash_secret(&secret));
    Ok((active.update(db).await?, secret))
}

fn issued(row: bot_webhook::Model, secret: String) -> IssuedBotWebhook {
    let signing_key = row.secret_hash.clone();
    IssuedBotWebhook {
        webhook: row.into(),
        secret,
        signing_key,
    }
}

#[utoipa::path(
    get,
    path = "/bots/{bot_id}/webhooks",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    responses(
        (status = 200, description = "The bot's webhooks, oldest first, without their secrets.", body = BotWebhookList),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no bot with this id")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /bots/{bot_id}/webhooks", skip(state, user))]
pub async fn list_bot_webhooks(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
) -> Result<Json<BotWebhookList>, ApiError> {
    let sub = user.sub()?;
    owned_bot(&state.db, &sub, &bot_id).await?;
    let webhooks = webhook_rows(&state, &bot_id)
        .await?
        .into_iter()
        .map(BotWebhookInfo::from)
        .collect();
    Ok(Json(BotWebhookList { webhooks }))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/webhooks",
    tag = "Bots",
    params(("bot_id" = String, Path, description = "Your bot's id")),
    request_body = CreateBotWebhookRequest,
    responses(
        (status = 201, description = "A new webhook URL for your bot. The response holds the only copy of its secret.", body = IssuedBotWebhook),
        (status = 400, description = "The name is empty or too long, or the instruction is too long"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "BOT_WEBHOOK_CAPABILITY_OFF: switch the bot's webhook capability on first"),
        (status = 404, description = "You have no bot with this id"),
        (status = 409, description = "BOT_WEBHOOK_LIMIT_REACHED: a bot has at most 10 webhooks")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /bots/{bot_id}/webhooks", skip(state, user, body))]
pub async fn create_bot_webhook(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(bot_id): Path<String>,
    Json(body): Json<CreateBotWebhookRequest>,
) -> Result<(StatusCode, Json<IssuedBotWebhook>), ApiError> {
    let sub = user.sub()?;
    let name = name_of(&body.name)?;
    let instruction = text_within("An instruction", &body.instruction, INSTRUCTION_MAX_CHARS)?;
    let secret = new_secret();
    let row = state
        .transaction(|txn| {
            let (sub, bot_id, name, instruction, secret_hash) = (
                sub.clone(),
                bot_id.clone(),
                name.clone(),
                instruction.clone(),
                hash_secret(&secret),
            );
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                create_in(txn, &sub, &bot_id, name, instruction, secret_hash).await
            })
        })
        .await?;
    Ok((StatusCode::CREATED, Json(issued(row, secret))))
}

#[utoipa::path(
    patch,
    path = "/bots/{bot_id}/webhooks/{hook_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("hook_id" = String, Path, description = "The webhook's id")
    ),
    request_body = UpdateBotWebhookRequest,
    responses(
        (status = 200, description = "The updated webhook", body = BotWebhookInfo),
        (status = 400, description = "The name or instruction is invalid"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or webhook")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PATCH /bots/{bot_id}/webhooks/{hook_id}",
    skip(state, user, body)
)]
pub async fn update_bot_webhook(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, hook_id)): Path<(String, String)>,
    Json(body): Json<UpdateBotWebhookRequest>,
) -> Result<Json<BotWebhookInfo>, ApiError> {
    let sub = user.sub()?;
    let row = owned_webhook(&state, &sub, &bot_id, &hook_id).await?;
    let mut active: bot_webhook::ActiveModel = row.into();
    if let Some(name) = &body.name {
        active.name = Set(name_of(name)?);
    }
    if let Some(instruction) = &body.instruction {
        active.instruction = Set(text_within(
            "An instruction",
            instruction,
            INSTRUCTION_MAX_CHARS,
        )?);
    }
    if let Some(enabled) = body.enabled {
        active.enabled = Set(enabled);
    }
    Ok(Json(active.update(&state.db).await?.into()))
}

#[utoipa::path(
    post,
    path = "/bots/{bot_id}/webhooks/{hook_id}/rotate",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("hook_id" = String, Path, description = "The webhook's id")
    ),
    responses(
        (status = 200, description = "The webhook with a new secret; the old secret and signing key stop working at once. The response holds the only copy of the new secret.", body = IssuedBotWebhook),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or webhook")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /bots/{bot_id}/webhooks/{hook_id}/rotate",
    skip(state, user)
)]
pub async fn rotate_bot_webhook(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, hook_id)): Path<(String, String)>,
) -> Result<Json<IssuedBotWebhook>, ApiError> {
    let sub = user.sub()?;
    let (row, secret) = rotate_in(&state.db, &sub, &bot_id, &hook_id).await?;
    Ok(Json(issued(row, secret)))
}

#[utoipa::path(
    delete,
    path = "/bots/{bot_id}/webhooks/{hook_id}",
    tag = "Bots",
    params(
        ("bot_id" = String, Path, description = "Your bot's id"),
        ("hook_id" = String, Path, description = "The webhook's id")
    ),
    responses(
        (status = 204, description = "The webhook's URL stops working"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You have no such bot or webhook")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /bots/{bot_id}/webhooks/{hook_id}", skip(state, user))]
pub async fn delete_bot_webhook(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((bot_id, hook_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    owned_webhook(&state, &sub, &bot_id, &hook_id).await?;
    bot_webhook::Entity::delete_many()
        .filter(bot_webhook::Column::Id.eq(&hook_id))
        .filter(bot_webhook::Column::BotId.eq(&bot_id))
        .exec(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn unauthorized() -> ApiError {
    ApiError::coded(
        StatusCode::UNAUTHORIZED,
        WEBHOOK_REFUSED,
        "This webhook doesn't exist or the secret is wrong",
    )
}

/// The webhook and its live bot once the call proved the secret; every refusal before that is the
/// same 401, so a caller can't tell a missing webhook from a wrong secret.
pub(crate) async fn admit(
    db: &DatabaseConnection,
    hook_id: &str,
    headers: &HeaderMap,
    query_secret: Option<&str>,
    body: &[u8],
    now: Instant,
) -> Result<(bot_webhook::Model, bot::Model), ApiError> {
    let probe = format!("probe:{hook_id}");
    if REFUSALS.exhausted(&probe, now) {
        return Err(ApiError::too_many_requests(
            "Too many refused webhook calls; wait a minute",
        ));
    }
    let hook = bot_webhook::Entity::find_by_id(hook_id).one(db).await?;
    let stored = hook
        .as_ref()
        .map(|hook| hook.secret_hash.clone())
        .unwrap_or_default();
    let Some(hook) = hook.filter(|_| authorized(&stored, headers, query_secret, body)) else {
        REFUSALS.allow(&probe, now);
        return Err(unauthorized());
    };
    if !CALLS.allow(hook_id, now) {
        return Err(ApiError::too_many_requests(format!(
            "A webhook takes at most {CALLS_PER_MINUTE} calls a minute"
        )));
    }
    check_payload_size(body.len())?;
    if !hook.enabled {
        return Err(ApiError::coded(
            StatusCode::FORBIDDEN,
            WEBHOOK_DISABLED,
            "This webhook is switched off",
        ));
    }
    let bot = bot::Entity::find_by_id(&hook.bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found("The bot of this webhook is gone"))?;
    refuse_without_capability(&bot)?;
    Ok((hook, bot))
}

/// Wake a bot with a payload. Supply the webhook secret through `X-Flow-Like-Webhook-Secret`,
/// `Authorization: Bearer <secret>`, or `?secret=<secret>`. HMAC signing is optional and can
/// replace the secret with `X-Flow-Like-Signature: sha256=<hex>` over the exact request body.
#[utoipa::path(
    post,
    path = "/bots/hooks/{hook_id}",
    tag = "Bots",
    params(("hook_id" = String, Path, description = "The webhook's id"), HookQuery),
    request_body(content = String, description = "Any body up to 256 KiB; JSON is passed on as JSON", content_type = "application/json"),
    responses(
        (status = 202, description = "The bot was woken: the turn it queued. Its reply lands in the bot's chat with its owner.", body = FiredTurn),
        (status = 200, description = "With `wait`: the bot answered in time; the reply is in the body", body = FiredTurn),
        (status = 401, description = "BOT_WEBHOOK_UNAUTHORIZED: no such webhook, or the secret or signature is wrong"),
        (status = 403, description = "BOT_WEBHOOK_DISABLED or BOT_WEBHOOK_CAPABILITY_OFF"),
        (status = 409, description = "BOT_TURN_NOT_QUEUED: the bot took no turn"),
        (status = 413, description = "BOT_WEBHOOK_PAYLOAD_TOO_LARGE: more than 256 KiB"),
        (status = 429, description = "More than 30 calls a minute, or too many refused calls")
    )
)]
#[tracing::instrument(name = "POST /bots/hooks/{hook_id}", skip(state, query, headers, body))]
pub async fn call_bot_webhook(
    State(state): State<AppState>,
    Path(hook_id): Path<String>,
    Query(query): Query<HookQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let (hook, bot) = admit(
        &state.db,
        &hook_id,
        &headers,
        query.secret.as_deref(),
        &body,
        Instant::now(),
    )
    .await?;
    let message_id = format!("wh-{}", uuid::Uuid::new_v4().simple());
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let payload = payload_of(content_type, &body);
    let (inline, handle) = if serde_json::to_vec(&payload).map_or(0, |bytes| bytes.len())
        <= TRIGGER_PAYLOAD_MAX_BYTES
    {
        (Some(payload), None)
    } else {
        let handle = store_payload(
            &state,
            &bot.owner_id,
            &bot.id,
            &hook.id,
            &message_id,
            &payload,
        )
        .await?;
        (None, Some(handle))
    };
    let trigger = trigger_for(&hook, inline, handle);
    let turn = fire_trigger(&state, &bot.owner_id, &bot.id, &message_id, &trigger).await?;
    bot_webhook::Entity::update_many()
        .col_expr(
            bot_webhook::Column::LastCalledAt,
            Expr::value(Utc::now().fixed_offset()),
        )
        .filter(bot_webhook::Column::Id.eq(&hook.id))
        .exec(&state.db)
        .await?;
    let wait = query.wait.unwrap_or(0).min(MAX_WAIT_SECS);
    if wait == 0 {
        let state = turn_state(&state.db, &bot.owner_id, &turn).await?;
        return Ok((StatusCode::ACCEPTED, Json(state)).into_response());
    }
    let state = wait_for_turn(&state.db, &bot.owner_id, &turn, Duration::from_secs(wait)).await?;
    let status = if state.reply.is_some() {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    Ok((status, Json(state)).into_response())
}

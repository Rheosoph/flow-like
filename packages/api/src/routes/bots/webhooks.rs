//! Bot webhooks: public URLs that wake a bot. The owner creates one per integration (the secret is
//! shown once; its hash is stored for comparison and the secret itself encrypted like a bot
//! secret, to key signatures). A call with the secret in a header or query parameter, or a signed
//! call, writes a `trigger` entry into the bot's conversation with its owner and answers with the
//! turn it queued. Servers only: a bot that lives in a local store has no public URL.
//!
//! Signed call: send `X-Flow-Like-Timestamp` (Unix seconds, within five minutes of the server's
//! clock), `X-Flow-Like-Delivery` (an id unique per delivery) and
//! `X-Flow-Like-Signature: sha256=<hex>`, the HMAC-SHA256 under the webhook secret of
//! `<timestamp>.<delivery id>.<raw body>`. A delivery id is accepted once per window. Hooks
//! created before the secret was kept encrypted keep accepting the old signature (HMAC of the
//! body under the secret's hash) until the owner rotates the secret.

use super::{
    fire::{FiredTurn, fire_trigger, turn_state, wait_for_turn},
    lock_bot, owned_bot, secret_cipher, workspace,
};
use crate::{
    cache::{PlatformCache, Reservation},
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
const TIMESTAMP_HEADER: &str = "x-flow-like-timestamp";
const DELIVERY_HEADER: &str = "x-flow-like-delivery";
const SIGNATURE_TOLERANCE_SECS: i64 = 300;
const DELIVERY_ID_MAX_CHARS: usize = 128;
const DELIVERY_TTL: Duration = Duration::from_secs(2 * SIGNATURE_TOLERANCE_SECS as u64);
const DELIVERY_NAMESPACE: &str = "bot_webhook_delivery";
const LIMIT_NAMESPACE: &str = "bot_webhook_limit";
const LIMIT_WINDOW_SECS: i64 = 60;
const DUPLICATE_DELIVERY: &str = "BOT_WEBHOOK_DUPLICATE_DELIVERY";
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
    /// Optional alternative to sending the secret, for senders that sign: the same value as
    /// `secret`. Send `X-Flow-Like-Timestamp` (Unix seconds, within five minutes of now),
    /// `X-Flow-Like-Delivery` (an id unique per delivery) and `X-Flow-Like-Signature:
    /// sha256=<hex>`, the HMAC-SHA256 under this key of `<timestamp>.<delivery id>.<raw body>`.
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

fn secret_name(hook_id: &str) -> String {
    format!("webhook:{hook_id}")
}

fn seal_secret(
    master: &[u8; 32],
    bot_id: &str,
    hook_id: &str,
    secret: &str,
) -> Result<String, ApiError> {
    secret_cipher::encrypt(master, bot_id, &secret_name(hook_id), secret).map_err(|error| {
        ApiError::internal(format!(
            "Encrypting the secret of webhook '{hook_id}' failed: {error}"
        ))
    })
}

fn open_secret(master: &[u8; 32], hook: &bot_webhook::Model) -> Option<String> {
    let stored = hook.secret_ciphertext.as_deref()?;
    match secret_cipher::decrypt(master, &hook.bot_id, &secret_name(&hook.id), stored) {
        Ok(secret) => Some(secret),
        Err(error) => {
            tracing::warn!(hook_id = %hook.id, %error, "Webhook secret could not be decrypted; only its plain secret is accepted");
            None
        }
    }
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

fn window_slot(key: &str, unix_now: i64) -> String {
    format!("{key}:{}", unix_now.div_euclid(LIMIT_WINDOW_SECS))
}

fn cache_unavailable(operation: &str, error: impl std::fmt::Display) {
    tracing::warn!(%error, "Webhook {operation} fell back to this replica; the shared cache failed");
}

impl WindowLimiter {
    pub(crate) async fn shared_exhausted(
        &self,
        cache: Option<&PlatformCache>,
        key: &str,
        env: &AdmitEnv<'_>,
    ) -> bool {
        if self.exhausted(key, env.now) {
            return true;
        }
        let Some(cache) = cache else { return false };
        match cache
            .get::<u32>(LIMIT_NAMESPACE, &window_slot(key, env.unix_now))
            .await
        {
            Ok(count) => count.unwrap_or(0) >= self.limit,
            Err(error) => {
                cache_unavailable("rate check", error);
                false
            }
        }
    }

    pub(crate) async fn shared_allow(
        &self,
        cache: Option<&PlatformCache>,
        key: &str,
        env: &AdmitEnv<'_>,
    ) -> bool {
        if !self.allow(key, env.now) {
            return false;
        }
        let Some(cache) = cache else { return true };
        let slot = window_slot(key, env.unix_now);
        let count = match cache.get::<u32>(LIMIT_NAMESPACE, &slot).await {
            Ok(count) => count.unwrap_or(0),
            Err(error) => {
                cache_unavailable("rate check", error);
                return true;
            }
        };
        if count >= self.limit {
            return false;
        }
        let ttl = Duration::from_secs(2 * LIMIT_WINDOW_SECS as u64);
        if let Err(error) = cache.set(LIMIT_NAMESPACE, &slot, &(count + 1), ttl).await {
            cache_unavailable("rate count", error);
        }
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

fn header_text<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn decoded_signature(signature: &str) -> Option<Vec<u8>> {
    hex::decode(signature.trim().strip_prefix("sha256=")?).ok()
}

fn legacy_signature_matches(secret_hash: &str, signature: &str, body: &[u8]) -> bool {
    if secret_hash.is_empty() {
        return false;
    }
    let Some(expected) = decoded_signature(signature) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret_hash.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

fn valid_delivery_id(delivery: &str) -> bool {
    !delivery.is_empty()
        && delivery.chars().count() <= DELIVERY_ID_MAX_CHARS
        && delivery
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || matches!(char, '-' | '_' | '.' | ':'))
}

/// The delivery id of a call signed under `secret` over `<timestamp>.<delivery>.<body>` whose
/// timestamp is within the tolerance of `unix_now`.
pub(crate) fn signed_delivery(
    secret: &str,
    headers: &HeaderMap,
    body: &[u8],
    unix_now: i64,
) -> Option<String> {
    let timestamp: i64 = header_text(headers, TIMESTAMP_HEADER)?
        .trim()
        .parse()
        .ok()?;
    if timestamp.abs_diff(unix_now) > SIGNATURE_TOLERANCE_SECS as u64 {
        return None;
    }
    let delivery = header_text(headers, DELIVERY_HEADER)?.trim();
    if !valid_delivery_id(delivery) {
        return None;
    }
    let expected = decoded_signature(header_text(headers, SIGNATURE_HEADER)?)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(format!("{timestamp}.{delivery}.").as_bytes());
    mac.update(body);
    mac.verify_slice(&expected).ok()?;
    Some(delivery.to_string())
}

fn secret_matches(secret_hash: &str, presented: &str) -> bool {
    match blake3::Hash::from_hex(secret_hash) {
        Ok(stored) => blake3::hash(presented.as_bytes()) == stored,
        Err(_) => false,
    }
}

fn secret_presented(secret_hash: &str, headers: &HeaderMap, query_secret: Option<&str>) -> bool {
    let by_secret = secret_from(headers).is_some_and(|secret| secret_matches(secret_hash, secret));
    let by_query = query_secret
        .filter(|secret| !secret.is_empty())
        .is_some_and(|secret| secret_matches(secret_hash, secret));
    by_secret || by_query
}

fn legacy_signature_presented(secret_hash: &str, headers: &HeaderMap, body: &[u8]) -> bool {
    header_text(headers, SIGNATURE_HEADER)
        .is_some_and(|signature| legacy_signature_matches(secret_hash, signature, body))
}

/// Whether the call carries the secret or the body's signature keyed by the secret's hash, the
/// way hooks without a stored secret verify.
pub(crate) fn authorized(
    secret_hash: &str,
    headers: &HeaderMap,
    query_secret: Option<&str>,
    body: &[u8],
) -> bool {
    secret_presented(secret_hash, headers, query_secret)
        || legacy_signature_presented(secret_hash, headers, body)
}

#[derive(Debug, PartialEq, Eq)]
enum Proof {
    Secret,
    Signed { delivery: String },
}

fn proof_of(
    hook: &bot_webhook::Model,
    plain_secret: Option<&str>,
    headers: &HeaderMap,
    query_secret: Option<&str>,
    body: &[u8],
    unix_now: i64,
) -> Option<Proof> {
    if secret_presented(&hook.secret_hash, headers, query_secret) {
        return Some(Proof::Secret);
    }
    match (plain_secret, &hook.secret_ciphertext) {
        (Some(secret), _) => signed_delivery(secret, headers, body, unix_now)
            .map(|delivery| Proof::Signed { delivery }),
        (None, None) => {
            legacy_signature_presented(&hook.secret_hash, headers, body).then_some(Proof::Secret)
        }
        (None, Some(_)) => None,
    }
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
    secret: &str,
    master: &[u8; 32],
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
    let id = format!("bh_{}", uuid::Uuid::new_v4().simple());
    Ok(bot_webhook::ActiveModel {
        secret_ciphertext: Set(Some(seal_secret(master, bot_id, &id, secret)?)),
        id: Set(id),
        bot_id: Set(bot_id.to_string()),
        name: Set(name),
        secret_hash: Set(hash_secret(secret)),
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
    master: &[u8; 32],
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
    active.secret_ciphertext = Set(Some(seal_secret(master, bot_id, hook_id, &secret)?));
    Ok((active.update(db).await?, secret))
}

fn issued(row: bot_webhook::Model, secret: String) -> IssuedBotWebhook {
    IssuedBotWebhook {
        webhook: row.into(),
        signing_key: secret.clone(),
        secret,
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
            let (sub, bot_id, name, instruction, secret, master) = (
                sub.clone(),
                bot_id.clone(),
                name.clone(),
                instruction.clone(),
                secret.clone(),
                state.encryption_key,
            );
            Box::pin(async move {
                lock_bot(txn, &bot_id).await?;
                create_in(txn, &sub, &bot_id, name, instruction, &secret, &master).await
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
    let (row, secret) =
        rotate_in(&state.db, &sub, &bot_id, &hook_id, &state.encryption_key).await?;
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

pub(crate) struct AdmitEnv<'a> {
    pub master: &'a [u8; 32],
    pub cache: Option<&'a PlatformCache>,
    pub now: Instant,
    pub unix_now: i64,
}

async fn claim_delivery(
    cache: Option<&PlatformCache>,
    hook_id: &str,
    delivery: &str,
) -> Result<(), ApiError> {
    let Some(cache) = cache else { return Ok(()) };
    let key = format!("{hook_id}:{delivery}");
    match cache
        .try_insert(DELIVERY_NAMESPACE, &key, &true, DELIVERY_TTL)
        .await
    {
        Ok(Reservation::Acquired) => Ok(()),
        Ok(Reservation::Held(_)) => Err(ApiError::coded(
            StatusCode::CONFLICT,
            DUPLICATE_DELIVERY,
            format!("Delivery '{delivery}' of this webhook was already accepted"),
        )),
        Err(error) => {
            cache_unavailable("delivery check", error);
            Ok(())
        }
    }
}

/// The webhook and its live bot once the call proved the secret; every refusal before that is the
/// same 401, so a caller can't tell a missing webhook from a wrong secret. A signed call also
/// claims its delivery id, so the same signed request is accepted once.
pub(crate) async fn admit(
    db: &DatabaseConnection,
    hook_id: &str,
    headers: &HeaderMap,
    query_secret: Option<&str>,
    body: &[u8],
    env: &AdmitEnv<'_>,
) -> Result<(bot_webhook::Model, bot::Model), ApiError> {
    let probe = format!("probe:{hook_id}");
    if REFUSALS.shared_exhausted(env.cache, &probe, env).await {
        return Err(ApiError::too_many_requests(
            "Too many refused webhook calls; wait a minute",
        ));
    }
    let hook = bot_webhook::Entity::find_by_id(hook_id).one(db).await?;
    let admitted = hook.and_then(|hook| {
        let plain = open_secret(env.master, &hook);
        proof_of(
            &hook,
            plain.as_deref(),
            headers,
            query_secret,
            body,
            env.unix_now,
        )
        .map(|proof| (hook, proof))
    });
    let Some((hook, proof)) = admitted else {
        REFUSALS.shared_allow(env.cache, &probe, env).await;
        return Err(unauthorized());
    };
    if !CALLS.shared_allow(env.cache, hook_id, env).await {
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
    if let Proof::Signed { delivery } = proof {
        claim_delivery(env.cache, hook_id, &delivery).await?;
    }
    Ok((hook, bot))
}

/// Wake a bot with a payload. Supply the webhook secret through `X-Flow-Like-Webhook-Secret`,
/// `Authorization: Bearer <secret>`, or `?secret=<secret>`. Senders that sign can replace the
/// secret with three headers: `X-Flow-Like-Timestamp` (Unix seconds, at most five minutes off the
/// server clock), `X-Flow-Like-Delivery` (an id of letters, digits, `-_.:`, at most 128
/// characters, unique per delivery) and `X-Flow-Like-Signature: sha256=<hex>`, the lowercase hex
/// HMAC-SHA256 under the webhook secret of the string `<timestamp>.<delivery id>.<raw body>`. A
/// delivery id is accepted once per ten minutes (a repeat answers 409). Webhooks created before
/// this scheme keep verifying `sha256=<hex>` of the body alone, keyed by the blake3 hex hash of
/// the secret, until the owner rotates the secret.
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
        (status = 409, description = "BOT_TURN_NOT_QUEUED: the bot took no turn; BOT_WEBHOOK_DUPLICATE_DELIVERY: a signed call repeated a delivery id"),
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
    let cache = match state.cache.platform().await {
        Ok(cache) => Some(cache),
        Err(error) => {
            cache_unavailable("cache lookup", error);
            None
        }
    };
    let env = AdmitEnv {
        master: &state.encryption_key,
        cache: cache.as_ref(),
        now: Instant::now(),
        unix_now: Utc::now().timestamp(),
    };
    let (hook, bot) = admit(
        &state.db,
        &hook_id,
        &headers,
        query.secret.as_deref(),
        &body,
        &env,
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

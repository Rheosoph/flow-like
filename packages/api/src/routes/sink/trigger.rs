//! Sink trigger utilities and HTTP endpoint
//!
//! Provides:
//! - `trigger_event` - Utility function for programmatic event triggering (Lambda, SQS, etc.)
//! - `http_trigger` - HTTP endpoint for HTTP sinks
//! - `telegram_trigger` - Telegram webhook endpoint with secret token & IP verification
//! - `service_trigger` - Service-to-service trigger for internal services (cron, discord bot, etc.)

use crate::{
    entity::{
        event, event_sink, execution_run, membership, pat,
        sea_orm_active_enums::{RunMode, RunStatus},
    },
    error::ApiError,
    execution::{
        DispatchRequest, DispatchTrigger, ExecutionBackend, ExecutionJwtParams, TokenType,
        collect_generic_result, collect_generic_result_bytes, is_jwt_configured, rejection,
        resolve_wasm_packages, sign_execution_jwt,
    },
    routes::app::events::db::get_event_from_db,
    state::AppState,
};
use axum::{
    Json,
    body::Body,
    extract::{ConnectInfo, FromRequest, Multipart, Path, Query, State},
    http::{HeaderMap, Request, StatusCode, header},
    response::{IntoResponse, Response},
};
use flow_like_storage::{Path as StorePath, files::store::FlowLikeStore, object_store::PutPayload};
use flow_like_types::dispatch::REQUEST_FILES_STORE_REF;
use flow_like_types::{Bytes, Result as FlResult, anyhow, create_id, tokio};
use ipnetwork::IpNetwork;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, sync::Arc};
use utoipa::ToSchema;

/// Telegram server IP ranges (CIDR notation)
/// Webhooks are sent from these ranges only
const TELEGRAM_IP_RANGES: &[&str] = &["149.154.160.0/20", "91.108.4.0/22"];

/// Check if an IP address is within Telegram's allowed ranges
fn is_telegram_ip(ip: &std::net::IpAddr) -> bool {
    // Only IPv4 is supported by Telegram webhooks
    let ipv4 = match ip {
        std::net::IpAddr::V4(v4) => v4,
        std::net::IpAddr::V6(_) => return false,
    };

    for range in TELEGRAM_IP_RANGES {
        if let Ok(network) = range.parse::<IpNetwork>()
            && network.contains(std::net::IpAddr::V4(*ipv4))
        {
            return true;
        }
    }
    false
}

/// Merge two JSON payloads: base payload from event config + override payload from request.
/// Request payload values take precedence over event config values.
/// If both are objects, they are deep-merged. Otherwise, request payload wins entirely.
fn merge_payloads(
    base: Option<serde_json::Value>,
    override_payload: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    match (base, override_payload) {
        (None, None) => None,
        (Some(base), None) => Some(base),
        (None, Some(over)) => Some(over),
        (
            Some(serde_json::Value::Object(mut base_map)),
            Some(serde_json::Value::Object(over_map)),
        ) => {
            // Deep merge objects: override values take precedence
            for (key, value) in over_map {
                base_map.insert(key, value);
            }
            Some(serde_json::Value::Object(base_map))
        }
        // If either is not an object, override wins entirely
        (_, Some(over)) => Some(over),
    }
}

const HTTP_SINK_BODY_LIMIT_BYTES: usize = 10 * 1024 * 1024;

#[derive(Default)]
struct ParsedHttpRequestPayload {
    payload: Option<serde_json::Value>,
}

fn authorization_token_from_headers(headers: &HeaderMap) -> Option<String> {
    crate::middleware::jwt::viewer_authorization(headers)
        .map(normalize_authorization_token)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_authorization_token(value: &str) -> &str {
    let trimmed = value.trim();
    if let Some((scheme, token)) = trimmed.split_once(' ')
        && scheme.eq_ignore_ascii_case("Bearer")
    {
        return token.trim();
    }
    trimmed
}

fn parse_pat_token(value: &str) -> Option<(&str, &str)> {
    let pat_parts = value.trim().strip_prefix("pat_")?;
    let (pat_id, secret) = pat_parts.split_once('.')?;
    if pat_id.is_empty() || secret.is_empty() || secret.contains('.') {
        return None;
    }
    Some((pat_id, secret))
}

pub(crate) async fn resolve_sink_pat_user_id(
    state: &AppState,
    sink: &event_sink::Model,
    stored_pat: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let Some(stored_pat) = stored_pat else {
        return Ok(None);
    };

    let Some((pat_id, pat_secret)) = parse_pat_token(stored_pat) else {
        tracing::warn!(
            sink_id = %sink.id,
            event_id = %sink.event_id,
            "Stored sink PAT has invalid format; refusing sink execution"
        );
        return Err(ApiError::unauthorized("Stored sink PAT has invalid format"));
    };

    let mut hasher = blake3::Hasher::new();
    hasher.update(pat_secret.as_bytes());
    let secret_hash = hasher.finalize().to_hex().to_string().to_lowercase();

    let db_pat = pat::Entity::find()
        .filter(
            pat::Column::Id
                .eq(pat_id)
                .and(pat::Column::Key.eq(secret_hash)),
        )
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to validate sink PAT: {}", e)))?;

    let Some(db_pat) = db_pat else {
        tracing::warn!(
            sink_id = %sink.id,
            event_id = %sink.event_id,
            "Stored sink PAT no longer validates; refusing sink execution"
        );
        return Err(ApiError::unauthorized(
            "Stored sink PAT no longer validates",
        ));
    };

    if let Some(valid_until) = db_pat.valid_until
        && valid_until < chrono::Utc::now().naive_utc()
    {
        tracing::warn!(
            sink_id = %sink.id,
            event_id = %sink.event_id,
            user_id = %db_pat.user_id,
            "Stored sink PAT is expired; refusing sink execution"
        );
        return Err(ApiError::unauthorized("Stored sink PAT is expired"));
    }

    let user_id = db_pat.user_id;
    let member = membership::Entity::find()
        .filter(membership::Column::AppId.eq(sink.app_id.clone()))
        .filter(membership::Column::UserId.eq(user_id.clone()))
        .one(&state.db)
        .await
        .map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to validate sink PAT membership: {}", e))
        })?;

    if member.is_none() {
        tracing::warn!(
            sink_id = %sink.id,
            event_id = %sink.event_id,
            app_id = %sink.app_id,
            user_id = %user_id,
            "Stored sink PAT owner is not a project member; refusing sink execution"
        );
        return Err(ApiError::forbidden(
            "Stored sink PAT owner is not a project member",
        ));
    }

    Ok(Some(user_id))
}

fn is_multipart_content_type(content_type: Option<&str>) -> bool {
    content_type
        .map(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("multipart/form-data"))
        })
        .unwrap_or(false)
}

fn is_urlencoded_content_type(content_type: Option<&str>) -> bool {
    content_type
        .map(|value| {
            value.split(';').next().is_some_and(|mime| {
                mime.trim()
                    .eq_ignore_ascii_case("application/x-www-form-urlencoded")
            })
        })
        .unwrap_or(false)
}

fn decode_form_component(value: &str) -> String {
    let value = value.replace('+', " ");
    urlencoding::decode(&value)
        .unwrap_or(std::borrow::Cow::Borrowed(value.as_str()))
        .into_owned()
}

fn normalize_form_key(raw_key: &str, fallback: &str) -> (String, bool) {
    let decoded = decode_form_component(raw_key);
    let trimmed = decoded.trim();
    let key = if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    };

    if let Some(stripped) = key.strip_suffix("[]") {
        let stripped = stripped.trim();
        return (
            if stripped.is_empty() {
                fallback.to_string()
            } else {
                stripped.to_string()
            },
            true,
        );
    }

    (key, false)
}

fn insert_payload_value(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key: String,
    value: serde_json::Value,
    force_array: bool,
) {
    match map.get_mut(&key) {
        Some(serde_json::Value::Array(values)) => values.push(value),
        Some(existing) => {
            let previous = std::mem::replace(existing, serde_json::Value::Null);
            *existing = serde_json::Value::Array(vec![previous, value]);
        }
        None if force_array => {
            map.insert(key, serde_json::Value::Array(vec![value]));
        }
        None => {
            map.insert(key, value);
        }
    }
}

fn parse_form_encoded_payload(input: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();

    for pair in input.split('&').filter(|pair| !pair.is_empty()) {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let (key, force_array) = normalize_form_key(raw_key, "value");
        let value = serde_json::Value::String(decode_form_component(raw_value));
        insert_payload_value(&mut map, key, value, force_array);
    }

    map
}

fn merge_query_and_body(
    query: serde_json::Map<String, serde_json::Value>,
    body: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    match (query.is_empty(), body) {
        (true, None) => None,
        (false, None) => Some(serde_json::Value::Object(query)),
        (true, Some(body)) => Some(body),
        (false, Some(serde_json::Value::Object(mut body_map))) => {
            let mut merged = query;
            merged.append(&mut body_map);
            Some(serde_json::Value::Object(merged))
        }
        (false, Some(body)) => {
            let mut merged = query;
            merged.insert("_body".to_string(), body);
            Some(serde_json::Value::Object(merged))
        }
    }
}

fn sanitize_request_file_name(filename: Option<&str>, fallback_index: usize) -> String {
    let raw = filename
        .and_then(|name| name.rsplit(['/', '\\']).next())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("file");

    let mut sanitized = String::with_capacity(raw.len().min(120));
    for ch in raw.chars().take(120) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }

    let sanitized = sanitized.trim_matches(|ch| ch == '.' || ch == '_');
    if sanitized.is_empty() {
        format!("file-{fallback_index}")
    } else {
        sanitized.to_string()
    }
}

fn sanitize_store_path_segment(value: &str, fallback: &str) -> String {
    crate::credentials::storage_path_segment(value, fallback)
}

fn flow_path_value(path: &str) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "store_ref": REQUEST_FILES_STORE_REF,
        "cache_store_ref": null
    })
}

/// Loads the sink's persisted profile and re-hydrates decrypted custom-bit
/// secrets (persisted profile JSON never contains them).
async fn hydrated_sink_profile(
    state: &AppState,
    sink: &event_sink::Model,
) -> Option<serde_json::Value> {
    let mut profile = sink.profile_json.clone()?;
    crate::execution::hydrate_profile_custom_bit_secrets(state, &mut profile).await;
    Some(profile)
}

async fn parse_multipart_payload(
    request: Request<Body>,
    query: serde_json::Map<String, serde_json::Value>,
    body_limit: usize,
    file_store: FlowLikeStore,
    file_path_prefix: String,
) -> Result<ParsedHttpRequestPayload, ApiError> {
    let mut multipart = Multipart::from_request(request, &()).await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to parse multipart HTTP sink payload");
        ApiError::bad_request("Invalid multipart/form-data request")
    })?;

    let mut body_map = serde_json::Map::new();
    let mut total_bytes = 0usize;
    let mut file_count = 0usize;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        tracing::warn!(error = %e, "Failed to read multipart field");
        ApiError::bad_request("Invalid multipart/form-data field")
    })? {
        let raw_name = field.name().unwrap_or("file").to_string();
        let file_name = field.file_name().map(ToOwned::to_owned);
        let (key, force_array) = normalize_form_key(&raw_name, "file");
        let bytes = field.bytes().await.map_err(|e| {
            tracing::warn!(error = %e, "Failed to read multipart field bytes");
            ApiError::bad_request("Invalid multipart/form-data field")
        })?;

        total_bytes = total_bytes.saturating_add(bytes.len());
        if total_bytes > body_limit {
            return Err(ApiError::bad_request("Request body exceeds size limit"));
        }

        if file_name.is_some() {
            file_count += 1;
            let file_index = file_count;
            let sanitized_name = sanitize_request_file_name(file_name.as_deref(), file_index);
            let path = format!("{file_path_prefix}/{file_index:04}-{sanitized_name}");
            file_store
                .as_generic()
                .put(
                    &StorePath::from(path.clone()),
                    PutPayload::from_bytes(Bytes::copy_from_slice(&bytes)),
                )
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, path = %path, "Failed to stage multipart file");
                    ApiError::internal_error(anyhow!("Failed to stage multipart file"))
                })?;
            insert_payload_value(&mut body_map, key, flow_path_value(&path), force_array);
        } else {
            let value = serde_json::Value::String(String::from_utf8_lossy(&bytes).to_string());
            insert_payload_value(&mut body_map, key, value, force_array);
        }
    }

    Ok(ParsedHttpRequestPayload {
        payload: merge_query_and_body(query, Some(serde_json::Value::Object(body_map))),
    })
}

async fn parse_http_request_payload(
    request: Request<Body>,
    body_limit: usize,
    file_store: Option<FlowLikeStore>,
    file_path_prefix: Option<String>,
) -> Result<ParsedHttpRequestPayload, ApiError> {
    let (parts, body) = request.into_parts();
    let query = parts
        .uri
        .query()
        .map(parse_form_encoded_payload)
        .unwrap_or_default();
    let content_type = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);

    if is_multipart_content_type(content_type.as_deref()) {
        let file_store = file_store.ok_or_else(|| {
            ApiError::internal_error(anyhow!("Temporary file store is not configured"))
        })?;
        let file_path_prefix = file_path_prefix.ok_or_else(|| {
            ApiError::internal_error(anyhow!("Temporary file path is not configured"))
        })?;
        return parse_multipart_payload(
            Request::from_parts(parts, body),
            query,
            body_limit,
            file_store,
            file_path_prefix,
        )
        .await;
    }

    let body_bytes = axum::body::to_bytes(body, body_limit).await.map_err(|e| {
        tracing::error!("Failed to read body: {}", e);
        ApiError::bad_request("Failed to read request body")
    })?;

    let body_payload = if body_bytes.is_empty() {
        None
    } else if is_urlencoded_content_type(content_type.as_deref()) {
        let body_str = std::str::from_utf8(&body_bytes)
            .map_err(|_| ApiError::bad_request("Invalid form body"))?;
        Some(serde_json::Value::Object(parse_form_encoded_payload(
            body_str,
        )))
    } else {
        match serde_json::from_slice(&body_bytes) {
            Ok(value) => Some(value),
            Err(_) => Some(serde_json::Value::String(
                String::from_utf8_lossy(&body_bytes).to_string(),
            )),
        }
    };

    Ok(ParsedHttpRequestPayload {
        payload: merge_query_and_body(query, body_payload),
    })
}

/// Refresh any expired OAuth tokens in the provided map.
///
/// For each provider whose access_token has expired but has a refresh_token,
/// calls the provider's token endpoint. On success, updates the map entry with
/// the new token data and persists the updated encrypted blob back to the
/// EventSink row.
pub(crate) async fn maybe_refresh_oauth_tokens(
    state: &AppState,
    sink_id: &str,
    mut tokens: std::collections::HashMap<String, serde_json::Value>,
) -> std::collections::HashMap<String, serde_json::Value> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut any_refreshed = false;

    let provider_ids: Vec<String> = tokens.keys().cloned().collect();
    for provider_id in provider_ids {
        let value = match tokens.get(&provider_id) {
            Some(v) => v,
            None => continue,
        };

        // Check if the token is expired (same 60-second buffer as OAuthToken::is_expired)
        let expires_at = value
            .get("expires_at")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        if expires_at == 0 || expires_at > now + 60 {
            continue;
        }

        let refresh_token = match value.get("refresh_token").and_then(|v| v.as_str()) {
            Some(rt) if !rt.is_empty() => rt.to_string(),
            _ => {
                tracing::warn!(
                    provider = %provider_id,
                    sink = %sink_id,
                    "OAuth token expired but no refresh_token available"
                );
                continue;
            }
        };

        match crate::routes::oauth::refresh_oauth_token_for_provider(
            &state.secrets,
            &provider_id,
            &refresh_token,
        )
        .await
        {
            Ok(response) => {
                let new_expires_at = response.expires_in.map(|ei| now + ei as u64);

                let new_value = serde_json::json!({
                    "access_token": response.access_token,
                    "refresh_token": response.refresh_token.as_deref()
                        .unwrap_or(&refresh_token),
                    "expires_at": new_expires_at,
                    "token_type": response.token_type.as_deref().unwrap_or("Bearer"),
                });

                tokens.insert(provider_id.clone(), new_value);
                any_refreshed = true;

                tracing::info!(
                    provider = %provider_id,
                    sink = %sink_id,
                    "Refreshed expired OAuth token for scheduled execution"
                );
            }
            Err(err) => {
                tracing::warn!(
                    provider = %provider_id,
                    sink = %sink_id,
                    error = %err,
                    "Failed to refresh OAuth token, proceeding with expired token"
                );
            }
        }
    }

    if any_refreshed && let Ok(json_str) = serde_json::to_string(&tokens) {
        use crate::routes::app::events::db::encrypt_token;
        let encrypted = encrypt_token(&json_str, &state.encryption_key);

        let update = event_sink::ActiveModel {
            id: Set(sink_id.to_string()),
            oauth_tokens_encrypted: Set(Some(encrypted)),
            updated_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };

        if let Err(err) = update.update(&state.db).await {
            tracing::error!(
                sink = %sink_id,
                error = %err,
                "Failed to persist refreshed OAuth tokens to EventSink"
            );
        }
    }

    tokens
}

/// Input for programmatic event triggering
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TriggerEventInput {
    /// The event ID to trigger
    pub event_id: String,
    /// Optional payload to pass to the event
    pub payload: Option<serde_json::Value>,
}

/// Response from trigger operations
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TriggerResponse {
    pub triggered: bool,
    pub run_id: Option<String>,
    pub message: String,
}

/// Record a fire that never became a run.
///
/// Cron schedules and bot services are unattended: the HTTP status they get
/// back is discarded, so without this the only trace of a dead schedule is a
/// log line in a process nobody is watching. Resolving the event row directly
/// is deliberate — the common failures here (orphaned schedule, sink-type
/// mismatch, expired PAT) are exactly the ones where the sink is unusable but
/// the event still knows which board the user meant.
async fn record_trigger_rejection(
    state: &AppState,
    event_id: &str,
    stage: rejection::RejectionStage,
    reason: &str,
    payload: Option<serde_json::Value>,
    run_id: Option<String>,
    actor_user_id: Option<String>,
) -> Option<String> {
    let Some(context) =
        rejection::context_for_event(state, event_id, stage, reason.to_string()).await
    else {
        tracing::warn!(
            event_id = %event_id,
            reason = %reason,
            "Trigger rejected for an event that no longer exists; nothing to record"
        );
        return None;
    };

    let mut context = context
        .with_payload(payload)
        .with_actor(actor_user_id, None);
    if let Some(run_id) = run_id {
        context = context.with_run_id(run_id);
    }

    Some(rejection::record(state, context).await)
}

/// Utility function to trigger an event programmatically.
///
/// Use this in Lambda handlers, SQS processors, cron job workers, etc.
///
/// If the sink has stored PAT and/or OAuth tokens, they will be decrypted and
/// passed to the executor, enabling access to models and personal files.
///
/// # Example
/// ```ignore
/// // In a Lambda handler
/// let result = trigger_event(&state, TriggerEventInput {
///     event_id: "event_123".to_string(),
///     payload: Some(json!({"key": "value"})),
/// }).await?;
/// ```
pub async fn trigger_event(
    state: &AppState,
    input: TriggerEventInput,
) -> FlResult<TriggerResponse> {
    use crate::routes::app::events::db::decrypt_token;
    let encryption_key = &state.encryption_key;
    // Look up sink by event_id
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(&input.event_id))
        .filter(event_sink::Column::Active.eq(true))
        .one(&state.db)
        .await?
        .ok_or_else(|| anyhow!("No active sink found for event {}", input.event_id))?;

    // Get the event from database
    let event = get_event_from_db(&state.db, &sink.event_id, &sink.app_id).await?;

    // Check JWT is configured
    if !is_jwt_configured() {
        return Err(anyhow!("Execution JWT signing not configured"));
    }

    // Create run
    let run_id = create_id();
    let expires_at = chrono::Utc::now().naive_utc() + chrono::Duration::hours(24);
    // Decrypt PAT from sink if available
    let token = sink
        .pat_encrypted
        .as_ref()
        .and_then(|encrypted| decrypt_token(encrypted, encryption_key));

    let pat_actor_user_id = resolve_sink_pat_user_id(state, &sink, token.as_deref()).await?;
    let actor_user_id = pat_actor_user_id;
    let executor_subject = actor_user_id
        .clone()
        .unwrap_or_else(|| format!("sink:{}", sink.id));
    let actor_event_id = Some(sink.event_id.clone());
    debug_assert!(actor_user_id.is_some() || actor_event_id.is_some());

    let input_payload_len = input
        .payload
        .as_ref()
        .map(|p| {
            serde_json::to_string(p)
                .map(|s| s.len() as i64)
                .unwrap_or(0)
        })
        .unwrap_or(0);

    let event_json = serde_json::to_string(&event)?;

    // Get credentials scoped to the PAT owner when the sink was registered with
    // a valid PAT. Without a valid PAT this falls back to the sink actor.
    let credentials = state
        .scoped_credentials(
            &executor_subject,
            &sink.app_id,
            crate::credentials::CredentialsAccess::ServerExecute,
        )
        .await?;
    let shared_credentials = credentials.into_shared_credentials();
    let credentials_json = serde_json::to_string(&shared_credentials)?;

    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    // Sign JWT
    let executor_jwt = sign_execution_jwt(ExecutionJwtParams {
        user_id: executor_subject.clone(),
        technical_user_id: None,
        run_id: run_id.clone(),
        app_id: sink.app_id.clone(),
        board_id: event.board_id.clone(),
        event_id: Some(sink.event_id.clone()),
        app_chain: None,
        correlation: Some(crate::correlation::CorrelationContext::root(&run_id)),
        callback_url: callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(24 * 60 * 60),
    })?;

    // Decrypt OAuth tokens from sink if available
    let oauth_tokens: Option<std::collections::HashMap<String, serde_json::Value>> = sink
        .oauth_tokens_encrypted
        .as_ref()
        .and_then(|encrypted| decrypt_token(encrypted, encryption_key))
        .and_then(|json| serde_json::from_str(&json).ok());

    // Refresh any expired OAuth tokens before dispatch (writes back to DB on success)
    let oauth_tokens = match oauth_tokens {
        Some(tokens) => Some(maybe_refresh_oauth_tokens(state, &sink.id, tokens).await),
        None => None,
    };

    let wasm_packages = resolve_wasm_packages(state, &sink.app_id).await;

    // Build dispatch request
    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: sink.app_id.clone(),
        board_id: event.board_id.clone(),
        board_version: event.board_version,
        node_id: event.node_id.clone(),
        event_json: Some(event_json),
        payload: input.payload,
        user_id: executor_subject,
        credentials_json,
        jwt: executor_jwt,
        callback_url,
        token,        // PAT from sink (if configured)
        oauth_tokens, // OAuth tokens from sink (refreshed if needed)
        stream_state: false,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::from_event(Some(
            &event,
        ))),
        runtime_variables: None,
        user_context: None, // Sink triggers don't have user context
        profile: hydrated_sink_profile(state, &sink).await,
        wasm_packages,
        channel: None,
        // Programmatic trigger: cron workers, Lambda handlers, queue processors.
        trigger: DispatchTrigger::System,
    };

    // Create run record
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(event.board_id.clone()),
        version: Set(None),
        event_id: Set(actor_event_id),
        node_id: Set(Some(event.id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(RunMode::Http),
        log_level: Set(0),
        input_payload_len: Set(input_payload_len),
        input_payload_key: Set(None),
        output_payload_len: Set(0),
        error_message: Set(None),
        progress: Set(0),
        current_step: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        expires_at: Set(Some(expires_at)),
        user_id: Set(actor_user_id),
        technical_user_id: Set(None),
        caller_app_chain: Set(None),
        trace_id: Set(Some(run_id.clone())),
        parent_run_id: Set(None),
        correlation_keys: Set(None),
        app_id: Set(sink.app_id.clone()),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
    };

    // Insert run record
    run.insert(&state.db).await?;

    // Dispatch (fire and forget for programmatic triggers)
    // Use async dispatch which respects ASYNC_EXECUTION_BACKEND config
    let dispatch_result = state.dispatcher.dispatch_async(request).await;

    match dispatch_result {
        Ok(_) => Ok(TriggerResponse {
            triggered: true,
            run_id: Some(run_id),
            message: "Event triggered successfully".to_string(),
        }),
        Err(e) => Ok(TriggerResponse {
            triggered: false,
            run_id: Some(run_id),
            message: format!("Dispatch failed: {}", e),
        }),
    }
}

/// POST/GET/etc /sink/trigger/{app_id}/{path}
/// HTTP endpoint for HTTP sinks
#[utoipa::path(
    post,
    path = "/sink/trigger/http/{app_id}/{path}",
    tag = "sink",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("path" = String, Path, description = "HTTP path for the sink")
    ),
    responses(
        (status = 200, description = "Event triggered successfully"),
        (status = 401, description = "Invalid or missing auth token"),
        (status = 404, description = "Route not found"),
        (status = 500, description = "Internal server error")
    )
)]
#[tracing::instrument(name = "ANY /sink/trigger/{app_id}/{path}", skip(state, path, request))]
pub async fn trigger_http(
    State(state): State<AppState>,
    Path((app_id, path)): Path<(String, String)>,
    request: Request<Body>,
) -> Result<Response, ApiError> {
    use crate::routes::app::events::db::decrypt_token;
    let encryption_key = &state.encryption_key;
    let method = request.method().clone();
    let headers = request.headers().clone();

    // Normalize path
    let normalized_path = if path.starts_with('/') {
        path
    } else {
        format!("/{}", path)
    };

    tracing::info!(
        "HTTP sink trigger: {} {} for app {}",
        method.as_str(),
        normalized_path,
        app_id
    );

    // Look up the sink using the unique (appId, path, method) index.
    // Two targeted queries — exact method match first, then a fallback for
    // legacy rows whose method column is still NULL from before this
    // migration landed. Postgres's unique constraint treats NULLs as
    // distinct, so pre-migration rows can coexist with new ones during
    // rollout; once they are re-saved through `sync_event_with_sink_tokens`
    // they'll pick up the explicit method and the fallback becomes a noop.
    let method_str = method.as_str().to_ascii_uppercase();

    let exact_match = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(&app_id))
        .filter(event_sink::Column::Path.eq(&normalized_path))
        .filter(event_sink::Column::Method.eq(&method_str))
        .filter(event_sink::Column::Active.eq(true))
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            ApiError::internal_error(anyhow!("Database error"))
        })?;

    let sink = match exact_match {
        Some(s) => s,
        None => {
            let legacy = event_sink::Entity::find()
                .filter(event_sink::Column::AppId.eq(&app_id))
                .filter(event_sink::Column::Path.eq(&normalized_path))
                .filter(event_sink::Column::Method.is_null())
                .filter(event_sink::Column::Active.eq(true))
                .one(&state.db)
                .await
                .map_err(|e| {
                    tracing::error!("Database error: {}", e);
                    ApiError::internal_error(anyhow!("Database error"))
                })?;

            match legacy {
                Some(s) => s,
                None => {
                    tracing::warn!(
                        "No active HTTP sink found for {} {} in app {}",
                        method_str,
                        normalized_path,
                        app_id
                    );
                    return Ok((
                        StatusCode::NOT_FOUND,
                        Json(TriggerResponse {
                            triggered: false,
                            run_id: None,
                            message: "Route not found".to_string(),
                        }),
                    )
                        .into_response());
                }
            }
        }
    };

    // Check auth token if set
    if let Some(expected_token) = &sink.auth_token {
        let expected_token = normalize_authorization_token(expected_token);
        let provided_token = authorization_token_from_headers(&headers);

        match provided_token {
            Some(token) if token == expected_token => {}
            _ => {
                return Ok((
                    StatusCode::UNAUTHORIZED,
                    Json(TriggerResponse {
                        triggered: false,
                        run_id: None,
                        message: "Invalid or missing auth token".to_string(),
                    }),
                )
                    .into_response());
            }
        }
    }

    // Create the run id before parsing so multipart files can be staged under
    // a stable, per-run temporary object prefix.
    let run_id = create_id();
    // Decrypt PAT from sink if available
    let token = sink
        .pat_encrypted
        .as_ref()
        .and_then(|encrypted| decrypt_token(encrypted, encryption_key));

    let actor_user_id = resolve_sink_pat_user_id(&state, &sink, token.as_deref()).await?;
    let executor_subject = actor_user_id
        .clone()
        .unwrap_or_else(|| format!("sink:{}", sink.id));
    let credentials = state
        .scoped_credentials(
            &executor_subject,
            &app_id,
            crate::credentials::CredentialsAccess::ServerExecute,
        )
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get credentials: {}", e)))?;
    let request_file_store = credentials
        .to_store_type(flow_like::credentials::StoreType::Tmp)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to create scratch store: {}", e)))?;
    // Offloaded under the executing subject's own scratch directory rather than a
    // shared tmp/global one: the executor downstream reads this file with the same
    // credentials, and an Azure directory SAS signs exactly one directory.
    let request_file_prefix = format!(
        "tmp/user/{}/apps/{}/runs/{}/request",
        sanitize_store_path_segment(&executor_subject, "user"),
        sanitize_store_path_segment(&app_id, "app"),
        sanitize_store_path_segment(&run_id, "run")
    );

    let parsed_payload = parse_http_request_payload(
        request,
        HTTP_SINK_BODY_LIMIT_BYTES,
        Some(request_file_store),
        Some(request_file_prefix),
    )
    .await?;
    let payload = parsed_payload.payload;

    // Get the event from database (config lives in Event)
    let event = get_event_from_db(&state.db, &sink.event_id, &sink.app_id)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get event: {}", e)))?;

    // Check JWT configured
    if !is_jwt_configured() {
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured"
        )));
    }

    let expires_at = chrono::Utc::now().naive_utc() + chrono::Duration::hours(24);

    let input_payload_len = payload
        .as_ref()
        .map(|p| {
            serde_json::to_string(p)
                .map(|s| s.len() as i64)
                .unwrap_or(0)
        })
        .unwrap_or(0);

    let event_json = serde_json::to_string(&event)
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize event: {}", e)))?;

    let shared_credentials = credentials.into_shared_credentials();
    let credentials_json = serde_json::to_string(&shared_credentials)
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize credentials: {}", e)))?;

    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    // Sign JWT
    let executor_jwt = sign_execution_jwt(ExecutionJwtParams {
        user_id: executor_subject.clone(),
        technical_user_id: None,
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: event.board_id.clone(),
        event_id: Some(sink.event_id.clone()),
        app_chain: None,
        correlation: Some(crate::correlation::CorrelationContext::root(&run_id)),
        callback_url: callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(24 * 60 * 60),
    })
    .map_err(|e| ApiError::internal_error(anyhow!("Failed to sign JWT: {}", e)))?;

    // Decrypt OAuth tokens from sink if available
    let oauth_tokens: Option<std::collections::HashMap<String, serde_json::Value>> = sink
        .oauth_tokens_encrypted
        .as_ref()
        .and_then(|encrypted| decrypt_token(encrypted, encryption_key))
        .and_then(|json| serde_json::from_str(&json).ok());

    // Refresh any expired OAuth tokens before dispatch (writes back to DB on success)
    let oauth_tokens = match oauth_tokens {
        Some(tokens) => Some(maybe_refresh_oauth_tokens(&state, &sink.id, tokens).await),
        None => None,
    };

    let wasm_packages = resolve_wasm_packages(&state, &app_id).await;

    // Build dispatch request
    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: app_id.clone(),
        board_id: event.board_id.clone(),
        board_version: event.board_version,
        node_id: event.node_id.clone(),
        event_json: Some(event_json),
        payload,
        user_id: executor_subject,
        credentials_json,
        jwt: executor_jwt,
        callback_url,
        token,        // PAT from sink (if configured)
        oauth_tokens, // OAuth tokens from sink (if configured)
        stream_state: false,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::from_event(Some(
            &event,
        ))),
        runtime_variables: None,
        user_context: None, // HTTP sink triggers don't have user context
        profile: hydrated_sink_profile(&state, &sink).await,
        wasm_packages,
        channel: None,
        // Inbound webhook.
        trigger: DispatchTrigger::System,
    };

    // Create run record
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(event.board_id.clone()),
        version: Set(None),
        event_id: Set(Some(sink.event_id.clone())),
        node_id: Set(Some(event.id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(RunMode::Http),
        log_level: Set(0),
        input_payload_len: Set(input_payload_len),
        input_payload_key: Set(None),
        output_payload_len: Set(0),
        error_message: Set(None),
        progress: Set(0),
        current_step: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        expires_at: Set(Some(expires_at)),
        user_id: Set(actor_user_id),
        technical_user_id: Set(None),
        caller_app_chain: Set(None),
        trace_id: Set(Some(run_id.clone())),
        parent_run_id: Set(None),
        correlation_keys: Set(None),
        app_id: Set(app_id.clone()),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
    };

    tracing::info!(run_id = %run_id, "Dispatching HTTP sink");

    // Persist the run record BEFORE dispatch so infrastructure failures
    // (executor crashes, network drops, timeouts) leave a visible Pending
    // row that can be reconciled, rather than a silently lost workflow.
    if let Err(e) = run.insert(&state.db).await {
        tracing::error!(run_id = %run_id, error = %e, "Failed to create run record");
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(TriggerResponse {
                triggered: false,
                run_id: Some(run_id),
                message: format!("Failed to create run record: {}", e),
            }),
        )
            .into_response());
    }

    // Match the desktop HTTP sink: wait for the first `generic_result`
    // event and return it as a single JSON response. External callers of
    // this webhook want a synchronous request/response, not SSE —
    // invoke_event is the streaming entry point for clients that want
    // live updates. 120s covers typical LLM-bound flows; longer-running
    // workloads should invoke via SSE or async.
    const HTTP_SINK_RESULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

    let backend = state.dispatcher.backend();

    match backend {
        ExecutionBackend::LambdaStream => {
            let dispatch_result = state.dispatcher.dispatch_streaming(request).await;

            match dispatch_result {
                Ok((_dispatch_response, byte_stream)) => {
                    tracing::info!(run_id = %run_id, "Got Lambda response, collecting result");
                    let db_arc = Arc::new(state.db.clone());
                    let run_id_owned = run_id.clone();
                    let generic_result = collect_generic_result_bytes(
                        byte_stream,
                        run_id_owned,
                        Some(db_arc),
                        HTTP_SINK_RESULT_TIMEOUT,
                    )
                    .await;

                    match generic_result {
                        Some(payload) => Ok((StatusCode::OK, Json(payload)).into_response()),
                        None => Ok((
                            StatusCode::OK,
                            Json(TriggerResponse {
                                triggered: true,
                                run_id: Some(run_id),
                                message: "Event triggered".to_string(),
                            }),
                        )
                            .into_response()),
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to dispatch Lambda streaming");
                    Ok((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(TriggerResponse {
                            triggered: false,
                            run_id: Some(run_id),
                            message: format!("Dispatch failed: {}", e),
                        }),
                    )
                        .into_response())
                }
            }
        }
        _ => {
            let dispatch_result = state.dispatcher.dispatch_http_sse(request).await;

            match dispatch_result {
                Ok((_dispatch_response, executor_response)) => {
                    tracing::info!(run_id = %run_id, "Got executor response, collecting result");
                    let db_arc = Arc::new(state.db.clone());
                    let run_id_owned = run_id.clone();
                    let generic_result = collect_generic_result(
                        executor_response,
                        run_id_owned,
                        Some(db_arc),
                        HTTP_SINK_RESULT_TIMEOUT,
                    )
                    .await;

                    match generic_result {
                        Some(payload) => Ok((StatusCode::OK, Json(payload)).into_response()),
                        None => Ok((
                            StatusCode::OK,
                            Json(TriggerResponse {
                                triggered: true,
                                run_id: Some(run_id),
                                message: "Event triggered".to_string(),
                            }),
                        )
                            .into_response()),
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to dispatch");
                    Ok((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(TriggerResponse {
                            triggered: false,
                            run_id: Some(run_id),
                            message: format!("Dispatch failed: {}", e),
                        }),
                    )
                        .into_response())
                }
            }
        }
    }
}

/// Query params for Telegram webhook (optional secret_token as query param)
#[derive(Debug, Deserialize)]
pub struct TelegramQueryParams {
    /// Secret token can be passed as query param as an alternative to header
    pub secret_token: Option<String>,
}

/// POST /sink/trigger/telegram/{event_id}
/// Telegram webhook endpoint - async execution with secret token & IP verification
#[utoipa::path(
    post,
    path = "/sink/trigger/telegram/{event_id}",
    tag = "sink",
    params(
        ("event_id" = String, Path, description = "Event ID"),
        ("secret_token" = Option<String>, Query, description = "Telegram secret token")
    ),
    responses(
        (status = 200, description = "Webhook received and processing", body = TriggerResponse),
        (status = 401, description = "Invalid or missing secret token"),
        (status = 403, description = "Request not from Telegram servers"),
        (status = 404, description = "Webhook not found or inactive")
    )
)]
#[tracing::instrument(
    name = "POST /sink/trigger/telegram/{event_id}",
    skip(state, query, headers, body, connect_info)
)]
pub async fn trigger_telegram(
    State(state): State<AppState>,
    Path(event_id): Path<String>,
    Query(query): Query<TelegramQueryParams>,
    headers: HeaderMap,
    ConnectInfo(connect_info): ConnectInfo<SocketAddr>,
    body: Body,
) -> Result<Response, ApiError> {
    use crate::routes::app::events::db::decrypt_token;
    let encryption_key = &state.encryption_key;

    let client_ip = connect_info.ip();

    tracing::info!(
        "Telegram webhook trigger for event {} from IP {}",
        event_id,
        client_ip
    );

    // Verify IP is from Telegram (in production)
    // Skip in development/local mode
    let api_base_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let is_development = api_base_url.contains("localhost") || api_base_url.contains("127.0.0.1");

    if !is_development && !is_telegram_ip(&client_ip) {
        tracing::warn!(
            "Telegram webhook request from non-Telegram IP: {}",
            client_ip
        );
        return Ok((
            StatusCode::FORBIDDEN,
            Json(TriggerResponse {
                triggered: false,
                run_id: None,
                message: "Request not from Telegram servers".to_string(),
            }),
        )
            .into_response());
    }

    // Look up sink by event_id
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(&event_id))
        .filter(event_sink::Column::Active.eq(true))
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            ApiError::internal_error(anyhow!("Database error"))
        })?;

    let sink = match sink {
        Some(s) => s,
        None => {
            tracing::warn!("No active Telegram sink found for event {}", event_id);
            return Ok((
                StatusCode::NOT_FOUND,
                Json(TriggerResponse {
                    triggered: false,
                    run_id: None,
                    message: "Webhook not found or inactive".to_string(),
                }),
            )
                .into_response());
        }
    };

    // Verify secret token (from header X-Telegram-Bot-Api-Secret-Token or query param)
    if let Some(expected_secret) = &sink.webhook_secret {
        let provided_secret = headers
            .get("X-Telegram-Bot-Api-Secret-Token")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .or(query.secret_token);

        match provided_secret {
            Some(token) if &token == expected_secret => {}
            _ => {
                tracing::warn!(
                    "Invalid or missing Telegram secret token for event {}",
                    event_id
                );
                return Ok((
                    StatusCode::UNAUTHORIZED,
                    Json(TriggerResponse {
                        triggered: false,
                        run_id: None,
                        message: "Invalid or missing secret token".to_string(),
                    }),
                )
                    .into_response());
            }
        }
    }

    // Parse body (Telegram sends JSON)
    let body_bytes = axum::body::to_bytes(body, 10 * 1024 * 1024) // 10MB limit
        .await
        .map_err(|e| {
            tracing::error!("Failed to read body: {}", e);
            ApiError::bad_request("Failed to read request body")
        })?;

    let payload: Option<serde_json::Value> = if !body_bytes.is_empty() {
        match serde_json::from_slice(&body_bytes) {
            Ok(v) => Some(v),
            Err(_) => Some(serde_json::Value::String(
                String::from_utf8_lossy(&body_bytes).to_string(),
            )),
        }
    } else {
        None
    };

    // Get the event from database
    let event = get_event_from_db(&state.db, &sink.event_id, &sink.app_id)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get event: {}", e)))?;

    // Check JWT configured
    if !is_jwt_configured() {
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured"
        )));
    }

    // Create run
    let run_id = create_id();
    let expires_at = chrono::Utc::now().naive_utc() + chrono::Duration::hours(24);

    let input_payload_len = payload
        .as_ref()
        .map(|p| {
            serde_json::to_string(p)
                .map(|s| s.len() as i64)
                .unwrap_or(0)
        })
        .unwrap_or(0);

    let event_json = serde_json::to_string(&event)
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize event: {}", e)))?;

    // Decrypt PAT from sink if available
    let token = sink
        .pat_encrypted
        .as_ref()
        .and_then(|encrypted| decrypt_token(encrypted, encryption_key));

    let actor_user_id = resolve_sink_pat_user_id(&state, &sink, token.as_deref()).await?;
    let executor_subject = actor_user_id
        .clone()
        .unwrap_or_else(|| format!("sink:{}", sink.id));

    let credentials = state
        .scoped_credentials(
            &executor_subject,
            &sink.app_id,
            crate::credentials::CredentialsAccess::ServerExecute,
        )
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get credentials: {}", e)))?;

    let shared_credentials = credentials.into_shared_credentials();
    let credentials_json = serde_json::to_string(&shared_credentials)
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize credentials: {}", e)))?;

    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    // Sign JWT
    let executor_jwt = sign_execution_jwt(ExecutionJwtParams {
        user_id: executor_subject.clone(),
        technical_user_id: None,
        run_id: run_id.clone(),
        app_id: sink.app_id.clone(),
        board_id: event.board_id.clone(),
        event_id: Some(sink.event_id.clone()),
        app_chain: None,
        correlation: Some(crate::correlation::CorrelationContext::root(&run_id)),
        callback_url: callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(24 * 60 * 60),
    })
    .map_err(|e| ApiError::internal_error(anyhow!("Failed to sign JWT: {}", e)))?;

    // Decrypt OAuth tokens from sink if available
    let oauth_tokens: Option<std::collections::HashMap<String, serde_json::Value>> = sink
        .oauth_tokens_encrypted
        .as_ref()
        .and_then(|encrypted| decrypt_token(encrypted, encryption_key))
        .and_then(|json| serde_json::from_str(&json).ok());

    // Refresh any expired OAuth tokens before dispatch (writes back to DB on success)
    let oauth_tokens = match oauth_tokens {
        Some(tokens) => Some(maybe_refresh_oauth_tokens(&state, &sink.id, tokens).await),
        None => None,
    };

    let wasm_packages = resolve_wasm_packages(&state, &sink.app_id).await;

    // Build dispatch request (async - no streaming)
    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: sink.app_id.clone(),
        board_id: event.board_id.clone(),
        board_version: event.board_version,
        node_id: event.node_id.clone(),
        event_json: Some(event_json),
        payload,
        user_id: executor_subject,
        credentials_json,
        jwt: executor_jwt,
        callback_url,
        token,        // PAT from sink (if configured)
        oauth_tokens, // OAuth tokens from sink (if configured)
        stream_state: false,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::from_event(Some(
            &event,
        ))),
        runtime_variables: None,
        user_context: None, // Telegram webhook triggers don't have user context
        profile: hydrated_sink_profile(&state, &sink).await,
        wasm_packages,
        channel: None,
        // Telegram bot service.
        trigger: DispatchTrigger::System,
    };

    // Create run record
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(event.board_id.clone()),
        version: Set(None),
        event_id: Set(Some(sink.event_id.clone())),
        node_id: Set(Some(event.id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(RunMode::Http),
        log_level: Set(0),
        input_payload_len: Set(input_payload_len),
        input_payload_key: Set(None),
        output_payload_len: Set(0),
        error_message: Set(None),
        progress: Set(0),
        current_step: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        expires_at: Set(Some(expires_at)),
        user_id: Set(actor_user_id),
        technical_user_id: Set(None),
        caller_app_chain: Set(None),
        trace_id: Set(Some(run_id.clone())),
        parent_run_id: Set(None),
        correlation_keys: Set(None),
        app_id: Set(sink.app_id.clone()),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
    };

    tracing::info!(run_id = %run_id, "Dispatching Telegram webhook (async)");

    // Insert run record
    run.insert(&state.db).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to create run record");
        ApiError::internal_error(anyhow!("Failed to create run record"))
    })?;

    // Dispatch async (fire and forget) - Telegram expects fast response
    let dispatcher = state.dispatcher.clone();
    let run_id_for_log = run_id.clone();
    tokio::spawn(async move {
        if let Err(e) = dispatcher.dispatch_async(request).await {
            tracing::error!(run_id = %run_id_for_log, error = %e, "Telegram webhook dispatch failed");
        }
    });

    // Return immediately - Telegram expects fast acknowledgement
    Ok((
        StatusCode::OK,
        Json(TriggerResponse {
            triggered: true,
            run_id: Some(run_id),
            message: "Webhook received and processing".to_string(),
        }),
    )
        .into_response())
}

/// Verify Discord Ed25519 signature
/// Discord sends: X-Signature-Ed25519 (signature) and X-Signature-Timestamp (timestamp)
/// The message to verify is: timestamp + body
fn verify_discord_signature(
    public_key_hex: &str,
    signature_hex: &str,
    timestamp: &str,
    body: &[u8],
) -> bool {
    use ed25519_dalek::{Signature, VerifyingKey};

    // Decode the public key from hex
    let public_key_bytes: [u8; 32] = match hex::decode(public_key_hex) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        }
        _ => return false,
    };

    // Decode the signature from hex
    let signature_bytes: [u8; 64] = match hex::decode(signature_hex) {
        Ok(bytes) if bytes.len() == 64 => {
            let mut arr = [0u8; 64];
            arr.copy_from_slice(&bytes);
            arr
        }
        _ => return false,
    };

    // Create verifying key
    let verifying_key = match VerifyingKey::from_bytes(&public_key_bytes) {
        Ok(key) => key,
        Err(_) => return false,
    };

    // Create signature
    let signature = Signature::from_bytes(&signature_bytes);

    // Build the message: timestamp + body
    let mut message = timestamp.as_bytes().to_vec();
    message.extend_from_slice(body);

    // Verify the signature
    use ed25519_dalek::Verifier;
    verifying_key.verify(&message, &signature).is_ok()
}

/// POST /sink/trigger/discord/{event_id}
/// Discord interactions webhook endpoint - async execution with Ed25519 signature verification
/// Discord requires responding to PING interactions with PONG, and must respond within 3 seconds
#[utoipa::path(
    post,
    path = "/sink/trigger/discord/{event_id}",
    tag = "sink",
    params(
        ("event_id" = String, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "Interaction processed"),
        (status = 401, description = "Invalid signature"),
        (status = 404, description = "Webhook not found or inactive")
    )
)]
#[tracing::instrument(
    name = "POST /sink/trigger/discord/{event_id}",
    skip(state, headers, body)
)]
pub async fn trigger_discord(
    State(state): State<AppState>,
    Path(event_id): Path<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    use crate::routes::app::events::db::decrypt_token;
    let encryption_key = &state.encryption_key;

    tracing::info!("Discord webhook trigger for event {}", event_id);

    // Read body first (needed for signature verification)
    let body_bytes = axum::body::to_bytes(body, 10 * 1024 * 1024) // 10MB limit
        .await
        .map_err(|e| {
            tracing::error!("Failed to read body: {}", e);
            ApiError::bad_request("Failed to read request body")
        })?;

    // Get signature headers
    let signature = headers
        .get("X-Signature-Ed25519")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let timestamp = headers
        .get("X-Signature-Timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Look up sink by event_id
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(&event_id))
        .filter(event_sink::Column::Active.eq(true))
        .one(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("Database error: {}", e);
            ApiError::internal_error(anyhow!("Database error"))
        })?;

    let sink = match sink {
        Some(s) => s,
        None => {
            tracing::warn!("No active Discord sink found for event {}", event_id);
            return Ok((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "Webhook not found or inactive"
                })),
            )
                .into_response());
        }
    };

    // Verify Ed25519 signature using the public key from sink config
    // The public key should be stored in webhook_secret field (it's the Discord app's public key)
    if let Some(public_key) = &sink.webhook_secret {
        // Skip verification in development mode
        let api_base_url =
            std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
        let is_development =
            api_base_url.contains("localhost") || api_base_url.contains("127.0.0.1");

        if !is_development
            && !verify_discord_signature(public_key, signature, timestamp, &body_bytes)
        {
            tracing::warn!("Invalid Discord signature for event {}", event_id);
            return Ok((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Invalid signature"
                })),
            )
                .into_response());
        }
    }

    // Parse the interaction payload
    let interaction: serde_json::Value = serde_json::from_slice(&body_bytes).map_err(|e| {
        tracing::error!("Failed to parse Discord interaction: {}", e);
        ApiError::bad_request("Invalid JSON payload")
    })?;

    // Check interaction type
    let interaction_type = interaction
        .get("type")
        .and_then(|t| t.as_u64())
        .unwrap_or(0);

    // Type 1 = PING - must respond with PONG immediately
    if interaction_type == 1 {
        tracing::info!("Discord PING received, responding with PONG");
        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "type": 1  // PONG
            })),
        )
            .into_response());
    }

    // For other interaction types (commands, components, etc.), dispatch async
    // Get the event from database
    let event = get_event_from_db(&state.db, &sink.event_id, &sink.app_id)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get event: {}", e)))?;

    // Check JWT configured
    if !is_jwt_configured() {
        return Err(ApiError::internal_error(anyhow!(
            "Execution JWT signing not configured"
        )));
    }

    // Create run
    let run_id = create_id();
    let expires_at = chrono::Utc::now().naive_utc() + chrono::Duration::hours(24);

    let input_payload_len = serde_json::to_string(&interaction)
        .map(|s| s.len() as i64)
        .unwrap_or(0);

    let event_json = serde_json::to_string(&event)
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize event: {}", e)))?;

    // Decrypt PAT from sink if available
    let token = sink
        .pat_encrypted
        .as_ref()
        .and_then(|encrypted| decrypt_token(encrypted, encryption_key));

    let actor_user_id = resolve_sink_pat_user_id(&state, &sink, token.as_deref()).await?;
    let executor_subject = actor_user_id
        .clone()
        .unwrap_or_else(|| format!("sink:{}", sink.id));

    let credentials = state
        .scoped_credentials(
            &executor_subject,
            &sink.app_id,
            crate::credentials::CredentialsAccess::ServerExecute,
        )
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get credentials: {}", e)))?;

    let shared_credentials = credentials.into_shared_credentials();
    let credentials_json = serde_json::to_string(&shared_credentials)
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to serialize credentials: {}", e)))?;

    let callback_url =
        std::env::var("API_BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());

    // Sign JWT
    let executor_jwt = sign_execution_jwt(ExecutionJwtParams {
        user_id: executor_subject.clone(),
        technical_user_id: None,
        run_id: run_id.clone(),
        app_id: sink.app_id.clone(),
        board_id: event.board_id.clone(),
        event_id: Some(sink.event_id.clone()),
        app_chain: None,
        correlation: Some(crate::correlation::CorrelationContext::root(&run_id)),
        callback_url: callback_url.clone(),
        token_type: TokenType::Executor,
        ttl_seconds: Some(24 * 60 * 60),
    })
    .map_err(|e| ApiError::internal_error(anyhow!("Failed to sign JWT: {}", e)))?;

    // Decrypt OAuth tokens from sink if available
    let oauth_tokens: Option<std::collections::HashMap<String, serde_json::Value>> = sink
        .oauth_tokens_encrypted
        .as_ref()
        .and_then(|encrypted| decrypt_token(encrypted, encryption_key))
        .and_then(|json| serde_json::from_str(&json).ok());

    // Refresh any expired OAuth tokens before dispatch (writes back to DB on success)
    let oauth_tokens = match oauth_tokens {
        Some(tokens) => Some(maybe_refresh_oauth_tokens(&state, &sink.id, tokens).await),
        None => None,
    };

    let wasm_packages = resolve_wasm_packages(&state, &sink.app_id).await;

    // Build dispatch request (async - no streaming)
    let request = DispatchRequest {
        run_id: run_id.clone(),
        app_id: sink.app_id.clone(),
        board_id: event.board_id.clone(),
        board_version: event.board_version,
        node_id: event.node_id.clone(),
        event_json: Some(event_json),
        payload: Some(interaction.clone()),
        user_id: executor_subject,
        credentials_json,
        jwt: executor_jwt,
        callback_url,
        token,        // PAT from sink (if configured)
        oauth_tokens, // OAuth tokens from sink (if configured)
        stream_state: false,
        execution_mode: Some(flow_like::flow::execution::ExecutionMode::from_event(Some(
            &event,
        ))),
        runtime_variables: None,
        user_context: None, // Discord webhook triggers don't have user context
        profile: hydrated_sink_profile(&state, &sink).await,
        wasm_packages,
        channel: None,
        // Discord bot service.
        trigger: DispatchTrigger::System,
    };

    // Create run record
    let run = execution_run::ActiveModel {
        id: Set(run_id.clone()),
        board_id: Set(event.board_id.clone()),
        version: Set(None),
        event_id: Set(Some(sink.event_id.clone())),
        node_id: Set(Some(event.id.clone())),
        status: Set(RunStatus::Pending),
        mode: Set(RunMode::Http),
        log_level: Set(0),
        input_payload_len: Set(input_payload_len),
        input_payload_key: Set(None),
        output_payload_len: Set(0),
        error_message: Set(None),
        progress: Set(0),
        current_step: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        expires_at: Set(Some(expires_at)),
        user_id: Set(actor_user_id),
        technical_user_id: Set(None),
        caller_app_chain: Set(None),
        trace_id: Set(Some(run_id.clone())),
        parent_run_id: Set(None),
        correlation_keys: Set(None),
        app_id: Set(sink.app_id.clone()),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
    };

    tracing::info!(run_id = %run_id, "Dispatching Discord webhook (async)");

    // Insert run record
    run.insert(&state.db).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to create run record");
        ApiError::internal_error(anyhow!("Failed to create run record"))
    })?;

    // Dispatch async (fire and forget) - Discord expects response within 3 seconds
    let dispatcher = state.dispatcher.clone();
    let run_id_for_log = run_id.clone();
    tokio::spawn(async move {
        if let Err(e) = dispatcher.dispatch_async(request).await {
            tracing::error!(run_id = %run_id_for_log, error = %e, "Discord webhook dispatch failed");
        }
    });

    // Discord expects a deferred response for commands (type 5)
    // This tells Discord we're processing and will follow up later
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "type": 5  // DEFERRED_CHANNEL_MESSAGE_WITH_SOURCE
        })),
    )
        .into_response())
}

/// JWT claims for sink trigger service tokens
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinkTriggerClaims {
    /// Subject - always "sink-trigger"
    pub sub: String,
    /// Issuer - always "flow-like"
    pub iss: String,
    /// JWT ID - unique identifier for revocation checking
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
    /// Which sink types this token can trigger
    pub sink_types: Vec<String>,
    /// Optional: restrict to specific app IDs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_ids: Option<Vec<String>>,
    /// Issued at timestamp
    pub iat: usize,
    /// Expiration timestamp (optional - can be very long-lived)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exp: Option<usize>,
}

/// Request body for service-to-service trigger
#[derive(Debug, Deserialize, ToSchema)]
pub struct ServiceTriggerRequest {
    /// The event ID to trigger
    pub event_id: String,
    /// The sink type (must match token's allowed sink_types)
    pub sink_type: String,
    /// Optional payload
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

/// Response from service trigger
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ServiceTriggerResponse {
    pub success: bool,
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Validate a sink trigger JWT and extract claims (without DB check)
fn validate_sink_trigger_jwt(token: &str, secret: &str) -> Result<SinkTriggerClaims, ApiError> {
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.required_spec_claims.remove("exp");
    validation.validate_exp = false;
    let key = jsonwebtoken::DecodingKey::from_secret(secret.as_bytes());

    let token_data = jsonwebtoken::decode::<SinkTriggerClaims>(token, &key, &validation)
        .map_err(|e| ApiError::unauthorized(format!("Invalid sink trigger token: {}", e)))?;

    // Verify subject
    if token_data.claims.sub != "sink-trigger" {
        return Err(ApiError::unauthorized("Invalid token subject"));
    }

    // Verify issuer
    if token_data.claims.iss != "flow-like" {
        return Err(ApiError::unauthorized("Invalid token issuer"));
    }

    if let Some(exp) = token_data.claims.exp {
        let now = chrono::Utc::now().timestamp() as usize;
        if exp <= now {
            return Err(ApiError::unauthorized("Sink trigger token has expired"));
        }
    }

    Ok(token_data.claims)
}

/// Check if a sink token has been revoked
async fn is_token_revoked(db: &sea_orm::DatabaseConnection, jti: &str) -> Result<bool, ApiError> {
    use crate::entity::sink_token;
    use sea_orm::EntityTrait;

    let token = sink_token::Entity::find_by_id(jti)
        .one(db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Database error: {}", e)))?;

    match token {
        Some(t) => Ok(t.revoked),
        // If token not found in DB, it's either an old token (pre-registration system)
        // or an invalid jti. We allow it for backward compatibility but log a warning.
        None => {
            tracing::warn!(jti = %jti, "Token jti not found in database - allowing for backward compatibility");
            Ok(false)
        }
    }
}

/// POST /sink/trigger/async
///
/// Service-to-service trigger endpoint for internal sink services (cron, discord bot, telegram bot, etc.)
///
/// Authentication: Bearer token with scoped sink trigger JWT
///
/// The JWT must include:
/// - `sub`: "sink-trigger"
/// - `iss`: "flow-like"
/// - `jti`: JWT ID for revocation checking (optional for backward compatibility)
/// - `sink_types`: Array of allowed sink types (e.g., ["cron"] or ["discord"])
///
/// Security: Each service gets a JWT scoped to only its sink type:
/// - Cron service gets JWT with `sink_types: ["cron"]`
/// - Discord bot gets JWT with `sink_types: ["discord"]`
/// - Telegram bot gets JWT with `sink_types: ["telegram"]`
///
/// If a service is compromised, it can only trigger events of its own type.
/// Tokens can be individually revoked via /admin/sinks/{jti}.
///
/// Idempotency: callers may include an `Idempotency-Key` header. If the same
/// key is seen within a short TTL (~15 minutes) the cached response is
/// returned instead of re-dispatching. This shields downstream flows from
/// EventBridge Scheduler's and Lambda's automatic retries on transient errors.
#[utoipa::path(
    post,
    path = "/sink/trigger/async",
    tag = "sink",
    request_body = ServiceTriggerRequest,
    responses(
        (status = 200, description = "Service trigger response", body = ServiceTriggerResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Token not authorized for sink type"),
        (status = 404, description = "Sink not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "POST /sink/trigger/async", skip_all)]
pub async fn trigger_service(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ServiceTriggerRequest>,
) -> Result<Json<ServiceTriggerResponse>, ApiError> {
    // Extract and validate Bearer token
    let auth_header = crate::middleware::jwt::viewer_authorization(&headers)
        .ok_or_else(|| ApiError::unauthorized("Missing Authorization header"))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::unauthorized("Invalid Authorization header format"))?;

    let sink_secret = state
        .sink_secret
        .as_deref()
        .ok_or_else(|| ApiError::internal_error(anyhow!("SINK_SECRET not configured")))?;
    let claims = validate_sink_trigger_jwt(token, sink_secret)?;

    // Check if token has been revoked (if jti is present)
    if let Some(ref jti) = claims.jti
        && is_token_revoked(&state.db, jti).await?
    {
        tracing::warn!(jti = %jti, "Attempted use of revoked sink token");
        return Err(ApiError::unauthorized("Token has been revoked"));
    }

    // Look up (or reserve) an idempotency key if provided. The cache stores
    // the previously-produced ServiceTriggerResponse and short-circuits repeats.
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());

    if let Some(ref key) = idempotency_key
        && let Some(cached) = state.trigger_idempotency.get(key)
    {
        tracing::info!(
            idempotency_key = %key,
            event_id = %request.event_id,
            "Returning cached idempotent trigger response"
        );
        return Ok(Json(cached));
    }

    // Check if this JWT is allowed to trigger this sink type
    if !claims.sink_types.contains(&request.sink_type) {
        tracing::warn!(
            requested_type = %request.sink_type,
            allowed_types = ?claims.sink_types,
            "Token not authorized for sink type"
        );
        return Err(ApiError::forbidden(format!(
            "Token not authorized for sink type: {}",
            request.sink_type
        )));
    }

    // Get the event sink from database
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(&request.event_id))
        .filter(event_sink::Column::Active.eq(true))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Database error: {}", e)))?;

    let sink = match sink {
        Some(sink) => sink,
        None => {
            let reason = format!("No active sink found for event {}", request.event_id);
            let _ = record_trigger_rejection(
                &state,
                &request.event_id,
                rejection::RejectionStage::Trigger,
                &reason,
                request.payload.clone(),
                None,
                None,
            )
            .await;
            return Err(ApiError::not_found(reason));
        }
    };

    // Verify sink type matches
    if sink.sink_type != request.sink_type {
        tracing::warn!(
            event_id = %request.event_id,
            expected_type = %sink.sink_type,
            requested_type = %request.sink_type,
            "Sink type mismatch"
        );
        let reason = format!(
            "Sink type mismatch: event {} is of type {}, not {}",
            request.event_id, sink.sink_type, request.sink_type
        );
        let _ = record_trigger_rejection(
            &state,
            &request.event_id,
            rejection::RejectionStage::Trigger,
            &reason,
            request.payload.clone(),
            None,
            None,
        )
        .await;
        return Err(ApiError::bad_request(reason));
    }

    // Check app_id restriction if present in token
    if let Some(ref allowed_apps) = claims.app_ids
        && !allowed_apps.contains(&sink.app_id)
    {
        return Err(ApiError::forbidden(format!(
            "Token not authorized for app: {}",
            sink.app_id
        )));
    }

    // Get the event to access its config for additional payload
    let event = get_event_from_db(&state.db, &request.event_id, &sink.app_id)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get event: {}", e)))?;

    // Merge payloads: event config payload (base) + request payload (override)
    // event.config is stored as JSON bytes (Vec<u8>) in the database
    let event_payload: Option<serde_json::Value> = if event.config.is_empty() {
        None
    } else {
        serde_json::from_slice::<serde_json::Value>(&event.config)
            .ok()
            .and_then(|config| config.get("payload").cloned())
    };
    let merged_payload = merge_payloads(event_payload, request.payload);

    tracing::info!(
        event_id = %request.event_id,
        sink_type = %request.sink_type,
        app_id = %sink.app_id,
        "Service trigger: triggering event"
    );

    // Use the existing trigger_event utility
    let response = match trigger_event(
        &state,
        TriggerEventInput {
            event_id: request.event_id.clone(),
            payload: merged_payload.clone(),
        },
    )
    .await
    {
        Ok(result) if result.triggered => ServiceTriggerResponse {
            success: true,
            run_id: result.run_id,
            error: None,
        },
        // The run row already exists but was never dispatched; finalize it with
        // the reason instead of leaving it Pending until the sweeper times it
        // out with no explanation.
        Ok(result) => {
            tracing::error!(message = %result.message, "Service trigger was not dispatched");
            let _ = record_trigger_rejection(
                &state,
                &request.event_id,
                rejection::RejectionStage::Dispatch,
                &result.message,
                merged_payload,
                result.run_id.clone(),
                None,
            )
            .await;
            ServiceTriggerResponse {
                success: false,
                run_id: result.run_id,
                error: Some(result.message),
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "Service trigger failed");
            let reason = e.to_string();
            let run_id = record_trigger_rejection(
                &state,
                &request.event_id,
                rejection::RejectionStage::Trigger,
                &reason,
                merged_payload,
                None,
                None,
            )
            .await;
            ServiceTriggerResponse {
                success: false,
                run_id,
                error: Some(reason),
            }
        }
    };

    if let Some(key) = idempotency_key {
        state.trigger_idempotency.insert(key, response.clone());
    }

    Ok(Json(response))
}

/// GET /sink/schedules
///
/// List all active cron schedules. Used by docker-compose sink service
/// to sync its in-memory scheduler with the database.
#[utoipa::path(
    get,
    path = "/sink/schedules",
    tag = "sink",
    responses(
        (status = 200, description = "List of cron schedules", body = Vec<CronScheduleInfo>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Token not authorized for cron schedules")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /sink/schedules", skip(state, headers))]
pub async fn get_cron_sinks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<CronScheduleInfo>>, ApiError> {
    // Extract and validate Bearer token
    let auth_header = crate::middleware::jwt::viewer_authorization(&headers)
        .ok_or_else(|| ApiError::unauthorized("Missing Authorization header"))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::unauthorized("Invalid Authorization header format"))?;

    let sink_secret = state
        .sink_secret
        .as_deref()
        .ok_or_else(|| ApiError::internal_error(anyhow!("SINK_SECRET not configured")))?;
    let claims = validate_sink_trigger_jwt(token, sink_secret)?;

    // Only allow tokens with cron access to list schedules
    if !claims.sink_types.contains(&"cron".to_string()) {
        return Err(ApiError::forbidden(
            "Token not authorized to list cron schedules",
        ));
    }

    // Get all active cron sinks
    let sinks = event_sink::Entity::find()
        .filter(event_sink::Column::SinkType.eq("cron"))
        .filter(event_sink::Column::Active.eq(true))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Database error: {}", e)))?;

    let schedules: Vec<CronScheduleInfo> = sinks
        .into_iter()
        .filter_map(|s| {
            s.cron_expression.map(|expr| CronScheduleInfo {
                // The docker-compose cron worker keys its in-memory and Redis
                // state by this id. Keep it equal to event_id so last-triggered
                // updates address the same record that schedule sync stores.
                id: s.event_id.clone(),
                event_id: s.event_id,
                cron_expression: expr,
                app_id: s.app_id,
                enabled: s.active,
                last_triggered: None,
                next_trigger: None,
            })
        })
        .collect();

    Ok(Json(schedules))
}

/// Cron schedule info returned by list_cron_schedules
#[derive(Debug, Serialize, ToSchema)]
pub struct CronScheduleInfo {
    pub id: String,
    pub event_id: String,
    pub cron_expression: String,
    pub app_id: String,
    pub enabled: bool,
    pub last_triggered: Option<chrono::DateTime<chrono::Utc>>,
    pub next_trigger: Option<chrono::DateTime<chrono::Utc>>,
}

/// Sink config info returned by list_sink_configs
#[derive(Debug, Serialize, ToSchema)]
pub struct SinkConfigInfo {
    pub event_id: String,
    pub app_id: String,
    pub sink_type: String,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

/// Query parameters for list_sink_configs
#[derive(Debug, Deserialize, ToSchema)]
pub struct SinkConfigsQuery {
    pub sink_type: String,
}

/// GET /sink/configs?sink_type=discord
///
/// List all active sink configs for a specific sink type.
/// Used by sink services (Discord bot, Telegram bot) to sync their configs.
#[utoipa::path(
    get,
    path = "/sink/configs",
    tag = "sink",
    params(
        ("sink_type" = String, Query, description = "Sink type to filter by")
    ),
    responses(
        (status = 200, description = "List of sink configs", body = Vec<SinkConfigInfo>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Token not authorized for sink type")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /sink/configs", skip_all)]
pub async fn get_sink_configs(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<SinkConfigsQuery>,
) -> Result<Json<Vec<SinkConfigInfo>>, ApiError> {
    // Extract and validate Bearer token
    let auth_header = crate::middleware::jwt::viewer_authorization(&headers)
        .ok_or_else(|| ApiError::unauthorized("Missing Authorization header"))?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::unauthorized("Invalid Authorization header format"))?;

    let sink_secret = state
        .sink_secret
        .as_deref()
        .ok_or_else(|| ApiError::internal_error(anyhow!("SINK_SECRET not configured")))?;
    let claims = validate_sink_trigger_jwt(token, sink_secret)?;

    // Only allow tokens with access to the requested sink type
    if !claims.sink_types.contains(&query.sink_type)
        && !claims.sink_types.contains(&"*".to_string())
    {
        return Err(ApiError::forbidden(format!(
            "Token not authorized for sink type: {}",
            query.sink_type
        )));
    }

    // Get all active sinks of the requested type
    let sinks = event_sink::Entity::find()
        .filter(event_sink::Column::SinkType.eq(&query.sink_type))
        .filter(event_sink::Column::Active.eq(true))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Database error: {}", e)))?;

    // Fetch events to get config data
    let event_ids: Vec<String> = sinks.iter().map(|s| s.event_id.clone()).collect();
    let events = event::Entity::find()
        .filter(event::Column::Id.is_in(event_ids))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Database error: {}", e)))?;

    let event_configs: std::collections::HashMap<String, serde_json::Value> = events
        .into_iter()
        .filter_map(|e| e.config.map(|c| (e.id, c)))
        .collect();

    let configs: Vec<SinkConfigInfo> = sinks
        .into_iter()
        .map(|s| {
            let config = event_configs.get(&s.event_id).cloned();
            SinkConfigInfo {
                event_id: s.event_id,
                app_id: s.app_id,
                sink_type: s.sink_type,
                active: s.active,
                config,
            }
        })
        .collect();

    Ok(Json(configs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm, EncodingKey, Header};

    const TEST_SECRET: &str = "test-sink-secret";

    fn sign_test_sink_claims(exp: Option<usize>) -> String {
        let now = chrono::Utc::now().timestamp() as usize;
        let claims = SinkTriggerClaims {
            sub: "sink-trigger".to_string(),
            iss: "flow-like".to_string(),
            jti: None,
            sink_types: vec!["cron".to_string()],
            app_ids: None,
            iat: now,
            exp,
        };

        jsonwebtoken::encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
        )
        .expect("test sink token should encode")
    }

    #[test]
    fn accepts_long_lived_sink_token_without_exp() {
        let token = sign_test_sink_claims(None);
        let claims = validate_sink_trigger_jwt(&token, TEST_SECRET)
            .expect("sink token without exp should validate");

        assert_eq!(claims.sub, "sink-trigger");
        assert_eq!(claims.iss, "flow-like");
        assert_eq!(claims.sink_types, vec!["cron".to_string()]);
        assert_eq!(claims.exp, None);
    }

    #[test]
    fn rejects_expired_sink_token_when_exp_is_present() {
        let expired_at = (chrono::Utc::now() - chrono::Duration::minutes(1)).timestamp() as usize;
        let token = sign_test_sink_claims(Some(expired_at));

        assert!(validate_sink_trigger_jwt(&token, TEST_SECRET).is_err());
    }
}

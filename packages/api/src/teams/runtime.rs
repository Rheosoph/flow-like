use super::{Connection, auth, cards, client, now, store};
use crate::{
    entity::{app, event_sink, sea_orm_active_enums::Status},
    error::ApiError,
    execution::{ExecutionClaims, state::ExecutionRunRecord},
    middleware::jwt::AppUser,
    routes::{
        app::events::db::get_event_from_db,
        execution::progress::{ExecutionEventInput, get_state_store},
        sink::trigger::{TriggerEventInput, trigger_event_with_run_id},
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use flow_like_types::{
    channel::{ChannelClientDescriptor, ChannelPush, ChannelPushKind},
    interaction::InteractionRequest,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;

const DAY_MS: i64 = 86_400_000;
const HISTORY_MS: i64 = 30 * DAY_MS;
const DISPATCH_LEASE_MS: i64 = 120_000;
const DELIVERY_LEASE_MS: i64 = 60_000;
const MAX_MESSAGE_BYTES: usize = 24_000;
const MAX_HISTORY: usize = 20;
const MAX_COMPLETED_RUNS: usize = 100;
const MAX_RECORDED_REPLIES: usize = 200;

#[derive(Clone, Serialize, Deserialize)]
struct Session {
    connection_id: String,
    app_id: String,
    event_id: String,
    run_id: String,
    tenant_id: String,
    client_id: String,
    conversation_id: String,
    service_url: String,
    activity_id: String,
    user_id: String,
    bot_id: String,
    history_key: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Conversation {
    #[serde(default)]
    messages: Vec<Value>,
    #[serde(default)]
    local: Value,
    #[serde(default)]
    last_run: String,
    /// Runs whose user message is already in `messages`.
    #[serde(default)]
    completed_runs: Vec<String>,
    /// Replies already in `messages`, keyed by run and reply.
    #[serde(default)]
    recorded: Vec<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct UserSession {
    value: Value,
    completed_runs: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Action {
    session: Session,
    request: InteractionRequest,
    allowed_responders: Vec<String>,
    state: String,
    lease_until: i64,
    #[serde(default)]
    answer: Option<Value>,
    #[serde(default)]
    responder: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Delivery {
    status: String,
    message_id: Option<String>,
    hash: String,
    #[serde(default)]
    lease_until: i64,
}

#[derive(Serialize, Deserialize)]
struct Dispatch {
    status: String,
    lease_until: i64,
}

#[derive(Debug, PartialEq, Eq)]
enum Claim {
    Done,
    InFlight,
    Retry,
}

fn hash(parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for part in parts {
        h.update(&(part.len() as u64).to_be_bytes());
        h.update(part.as_bytes());
    }
    h.finalize().to_hex().to_string()
}

/// Teams runs are identified by a [`hash`] of the originating activity.
fn is_teams_run(run_id: &str) -> bool {
    run_id.len() == 64 && run_id.bytes().all(|b| b.is_ascii_hexdigit())
}

async fn live(
    state: &AppState,
    c: &Connection,
) -> Result<(event_sink::Model, flow_like::flow::event::Event), ApiError> {
    super::ensure_enabled(state)?;
    if c.status != "ready" {
        return Err(ApiError::forbidden("This Teams bot is not connected"));
    }
    if app::Entity::find_by_id(&c.app_id)
        .filter(app::Column::Status.eq(Status::Active))
        .one(&state.db)
        .await?
        .is_none()
    {
        return Err(ApiError::FORBIDDEN);
    }
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(&c.app_id))
        .filter(event_sink::Column::EventId.eq(&c.event_id))
        .filter(event_sink::Column::SinkType.eq("teams"))
        .filter(event_sink::Column::Active.eq(true))
        .one(&state.db)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;
    let event = get_event_from_db(&state.db, &c.event_id, &c.app_id).await?;
    if !event.active
        || event.event_type != "teams"
        || event.execution_mode != flow_like::flow::event::EventExecutionMode::Remote
    {
        return Err(ApiError::FORBIDDEN);
    }
    Ok((sink, event))
}

fn string<'a>(value: &'a Value, pointer: &str) -> Result<&'a str, ApiError> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() < 4096)
        .ok_or_else(|| ApiError::bad_request("Teams activity is missing required context"))
}

#[utoipa::path(
    post,
    path = "/sink/trigger/teams/{connection_id}",
    operation_id = "receive_teams_activity",
    tag = "sink",
    description = "Messaging endpoint for Microsoft Teams. Accepts Bot Framework activities signed by Microsoft for this bot and runs its Teams event; card actions answer waiting Interactions.",
    params(("connection_id" = String, Path, description = "Teams bot connection ID from the bot setup")),
    request_body(content = Object, description = "Bot Framework activity"),
    responses(
        (status = 200, description = "Activity accepted; card actions return an invoke response", body = Object),
        (status = 400, description = "The activity is malformed or too large"),
        (status = 401, description = "The request is not signed by Microsoft for this bot"),
        (status = 403, description = "Teams is disabled, the bot, event or app is inactive, or the conversation belongs to another tenant"),
        (status = 404, description = "Unknown bot"),
        (status = 409, description = "The same message is already being dispatched"),
        (status = 500, description = "The workflow could not be dispatched; Microsoft may retry")
    )
)]
pub async fn incoming(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(activity): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    super::ensure_enabled(&state)?;
    let (c, _) = store::connection(&state, &id)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if c.id != id {
        return Err(ApiError::NOT_FOUND);
    }
    auth::verify(&c, &headers, &activity).await?;
    let (sink, event) = live(&state, &c).await?;
    record_activity(&state, &c).await?;
    let action_data = activity
        .pointer("/value/action/data")
        .or_else(|| activity.get("value"));
    if let Some(data) = action_data.filter(|data| data.get("flow_like_action").is_some()) {
        let result = respond(&state, &c, &activity, data).await;
        return match result {
            Ok(message) => Ok(Json(
                json!({"statusCode":200,"type":"application/vnd.microsoft.card.adaptive","value":{"type":"AdaptiveCard","version":"1.4","body":[{"type":"TextBlock","text":message,"wrap":true}]}}),
            )),
            Err(error) if error.status().is_client_error() => Ok(Json(
                json!({"statusCode":error.status().as_u16(),"type":"application/vnd.microsoft.error","value":{"code":"InvalidAction","message":error.public_message().unwrap_or("This action is not available to you")}}),
            )),
            Err(error) => Err(error),
        };
    }
    if activity["type"] != "message" || activity["from"]["id"] == activity["recipient"]["id"] {
        return Ok(Json(json!({})));
    }
    let text = activity["text"].as_str().unwrap_or_default();
    if text.len() > MAX_MESSAGE_BYTES {
        return Err(ApiError::bad_request(
            "Teams messages may contain at most 24000 bytes",
        ));
    }
    if text.trim().is_empty() {
        return Ok(Json(json!({})));
    }
    let conversation = string(&activity, "/conversation/id")?;
    let activity_id = string(&activity, "/id")?;
    let user_id = super::guid(string(&activity, "/from/aadObjectId")?)?;
    let run = hash(&[
        &c.id,
        &c.client_id,
        &c.customer_tenant_id,
        conversation,
        activity_id,
    ]);
    let session = Session {
        connection_id: c.id.clone(),
        app_id: c.app_id.clone(),
        event_id: c.event_id.clone(),
        run_id: run.clone(),
        tenant_id: c.customer_tenant_id.clone(),
        client_id: c.client_id.clone(),
        conversation_id: conversation.into(),
        service_url: string(&activity, "/serviceUrl")?.into(),
        activity_id: activity_id.into(),
        user_id,
        bot_id: string(&activity, "/recipient/id")?.into(),
        history_key: format!(
            "conversation:{}",
            hash(&[&c.id, &c.client_id, &c.customer_tenant_id, conversation])
        ),
    };
    let Some(lease) = claim_dispatch(&state, &c, &run).await? else {
        return Ok(Json(json!({})));
    };
    let result = dispatch(&state, &c, sink, event, &activity, session).await;
    release_dispatch(&state, &run, lease, result.is_ok()).await;
    result.map(|()| Json(json!({})))
}

async fn record_activity(state: &AppState, c: &Connection) -> Result<(), ApiError> {
    let key = format!("health:{}", c.id);
    let health = json!({"client_id":c.client_id,"tenant_id":c.customer_tenant_id,"at":now()});
    let expires = now() + HISTORY_MS;
    if let Some((_, revision)) = store::get::<Value>(state, &key).await? {
        store::update_until(state, &key, &health, revision, expires).await?;
    } else {
        store::insert(state, &key, &c.id, &health, expires).await?;
    }
    Ok(())
}

fn dispatch_claim(existing: &Dispatch, now: i64) -> Claim {
    match existing.status.as_str() {
        "dispatched" => Claim::Done,
        "dispatching" if existing.lease_until > now => Claim::InFlight,
        _ => Claim::Retry,
    }
}

/// Microsoft retries a message whose response was slow. The retry must neither start the
/// run a second time nor lose it when the first dispatch failed. Returns the lease revision,
/// or `None` when the message was already dispatched.
async fn claim_dispatch(
    state: &AppState,
    c: &Connection,
    run: &str,
) -> Result<Option<i32>, ApiError> {
    let key = format!("dispatch:{run}");
    let claim = Dispatch {
        status: "dispatching".into(),
        lease_until: now() + DISPATCH_LEASE_MS,
    };
    if store::insert(state, &key, &c.id, &claim, now() + DAY_MS).await? {
        return Ok(Some(0));
    }
    let in_flight = || ApiError::conflict("This Teams message is already being dispatched");
    let (existing, revision) = store::get::<Dispatch>(state, &key)
        .await?
        .ok_or_else(in_flight)?;
    match dispatch_claim(&existing, now()) {
        Claim::Done => Ok(None),
        Claim::InFlight => Err(in_flight()),
        Claim::Retry => {
            if store::update(state, &key, &claim, revision).await? {
                Ok(Some(revision + 1))
            } else {
                Err(in_flight())
            }
        }
    }
}

async fn release_dispatch(state: &AppState, run: &str, revision: i32, dispatched: bool) {
    let released = Dispatch {
        status: if dispatched { "dispatched" } else { "failed" }.into(),
        lease_until: 0,
    };
    match store::update(state, &format!("dispatch:{run}"), &released, revision).await {
        Ok(true) => {}
        Ok(false) => tracing::warn!(run_id = %run, "Teams dispatch lease changed before release"),
        Err(error) => {
            tracing::warn!(run_id = %run, %error, "Could not release the Teams dispatch lease")
        }
    }
}

async fn dispatch(
    state: &AppState,
    c: &Connection,
    sink: event_sink::Model,
    event: flow_like::flow::event::Event,
    activity: &Value,
    session: Session,
) -> Result<(), ApiError> {
    let run = session.run_id.clone();
    // A Microsoft retry keeps the deployment accepted with the first delivery.
    let event_key = format!("event:{run}");
    store::insert(state, &event_key, &c.id, &event, now() + DAY_MS).await?;
    let event = store::get::<flow_like::flow::event::Event>(state, &event_key)
        .await?
        .ok_or(ApiError::NOT_FOUND)?
        .0;
    store::insert(
        state,
        &format!("run:{run}"),
        &c.id,
        &session,
        now() + DAY_MS,
    )
    .await?;
    // Keep the original payload on retries, even if other messages arrived meanwhile.
    let payload_key = format!("input:{run}");
    let payload = match store::get::<Value>(state, &payload_key).await? {
        Some((payload, _)) => payload,
        None => {
            let payload = input_payload(state, c, activity, &session).await?;
            if store::insert(state, &payload_key, &c.id, &payload, now() + DAY_MS).await? {
                payload
            } else {
                store::get::<Value>(state, &payload_key)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)?
                    .0
            }
        }
    };
    let result = trigger_event_with_run_id(
        state,
        sink,
        event,
        TriggerEventInput {
            event_id: c.event_id.clone(),
            payload: Some(payload),
            idempotency_key: Some(run.clone()),
        },
        run,
    )
    .await
    .map_err(ApiError::internal_error)?;
    if !result.triggered {
        return Err(ApiError::internal(
            "Teams workflow dispatch failed. Microsoft can retry this message.",
        ));
    }
    Ok(())
}

async fn input_payload(
    state: &AppState,
    c: &Connection,
    activity: &Value,
    session: &Session,
) -> Result<Value, ApiError> {
    let mut conversation = store::get::<Conversation>(state, &session.history_key)
        .await?
        .map(|v| v.0)
        .unwrap_or_default();
    let mut text = activity["text"].as_str().unwrap_or_default().to_owned();
    if let Some(entities) = activity["entities"].as_array() {
        for entity in entities {
            if entity["type"] == "mention"
                && entity["mentioned"]["id"] == activity["recipient"]["id"]
                && let Some(mention) = entity["text"].as_str()
            {
                text = text.replace(mention, "");
            }
        }
    }
    conversation
        .messages
        .push(json!({"role":"user","content":text.trim()}));
    keep_last(&mut conversation.messages, MAX_HISTORY);
    let mut local = if conversation.local.is_object() {
        conversation.local
    } else {
        json!({})
    };
    local["teams"] = json!({"session_id":session.run_id});
    let mut global = store::get::<UserSession>(state, &user_key(session))
        .await?
        .map(|v| v.0.value)
        .unwrap_or_else(|| json!({}));
    if !global.is_object() {
        global = json!({});
    }
    global["teams"] = json!({"session_id":session.run_id});
    Ok(
        json!({"chat_id":hash(&[&c.id,&session.conversation_id]),"messages":conversation.messages,"local_session":local,"global_session":global,"user":{"sub":format!("{}:{}",c.customer_tenant_id,session.user_id),"name":activity["from"]["name"].as_str().unwrap_or("Teams user"),"bot":false},"actions":[],"tools":[],"attachments":[]}),
    )
}

fn user_key(s: &Session) -> String {
    format!(
        "user:{}",
        hash(&[&s.connection_id, &s.client_id, &s.tenant_id, &s.user_id])
    )
}

fn keep_last<T>(values: &mut Vec<T>, limit: usize) {
    if values.len() > limit {
        values.drain(..values.len() - limit);
    }
}

fn activity_url(
    session: &Session,
    message: Option<&str>,
    reply: bool,
) -> Result<reqwest::Url, ApiError> {
    let mut url = auth::service_url(&session.service_url)?;
    let mut path = url
        .path_segments_mut()
        .map_err(|_| ApiError::UNAUTHORIZED)?;
    path.pop_if_empty().extend([
        "v3",
        "conversations",
        &session.conversation_id,
        "activities",
    ]);
    if let Some(id) = message {
        path.push(id);
    } else if reply {
        path.push(&session.activity_id);
    }
    drop(path);
    Ok(url)
}

/// What a repeated request gets: the original message ID, permission to send again after a
/// definite rejection, a wait while the first attempt runs, or a refusal when the first
/// attempt may have reached Teams.
fn delivery_retry(existing: &Delivery, digest: &str, now: i64) -> Result<Option<String>, ApiError> {
    if existing.hash != digest {
        return Err(ApiError::unprocessable(
            "This Teams request ID was already used for a different message",
        ));
    }
    match existing.status.as_str() {
        "sent" => existing.message_id.clone().map(Some).ok_or_else(|| {
            ApiError::unprocessable("Teams accepted this message without returning its ID")
        }),
        "rejected" => Ok(None),
        "sending" if existing.lease_until > now => Err(ApiError::conflict(
            "This Teams message is still being delivered. Retry shortly.",
        )),
        _ => Err(ApiError::unprocessable(
            "Teams did not confirm an earlier attempt of this message. It may already be in the conversation, so it is not sent again.",
        )),
    }
}

fn teams_rejection(status: StatusCode, retry_after: Option<u64>) -> ApiError {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return ApiError::too_many_requests(format!(
            "Teams is throttling this bot. Retry after {} seconds.",
            retry_after.unwrap_or(1)
        ));
    }
    if status.is_server_error() || status == StatusCode::REQUEST_TIMEOUT {
        return ApiError::bad_gateway(format!("Teams returned HTTP {status}. Retry shortly."));
    }
    ApiError::unprocessable(format!(
        "Teams rejected the message (HTTP {status}). Check that the bot is installed in this conversation and its credentials are valid."
    ))
}

async fn settle(state: &AppState, id: &str, delivery: &Delivery, revision: i32, status: &str) {
    let settled = Delivery {
        status: status.into(),
        lease_until: 0,
        ..delivery.clone()
    };
    match store::update(state, id, &settled, revision).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(delivery = %id, status, "Teams delivery changed before it was settled")
        }
        Err(error) => {
            tracing::warn!(delivery = %id, status, %error, "Could not record the Teams delivery outcome")
        }
    }
}

/// Sends at most once per `key` and run. Deterministic failures are 4xx, 409 means the same
/// delivery is still running, and 429/5xx are transient.
async fn deliver(
    state: &AppState,
    c: &Connection,
    session: &Session,
    key: &str,
    body: Value,
    update: Option<&str>,
) -> Result<String, ApiError> {
    let encoded = body.to_string();
    if encoded.len() > MAX_MESSAGE_BYTES {
        return Err(ApiError::coded(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE",
            "Teams responses may contain at most 24000 bytes",
        ));
    }
    let url = activity_url(session, update, true)?;
    let id = format!("delivery:{}", hash(&[&session.run_id, key]));
    let sending = Delivery {
        status: "sending".into(),
        message_id: None,
        hash: hash(&[&encoded, update.unwrap_or_default()]),
        lease_until: now() + DELIVERY_LEASE_MS,
    };
    let mut revision = 0;
    if !store::insert(state, &id, &c.id, &sending, now() + DAY_MS).await? {
        let in_flight =
            || ApiError::conflict("This Teams message is still being delivered. Retry shortly.");
        let (existing, previous) = store::get::<Delivery>(state, &id)
            .await?
            .ok_or_else(in_flight)?;
        if let Some(message) = delivery_retry(&existing, &sending.hash, now())? {
            return Ok(message);
        }
        if !store::update(state, &id, &sending, previous).await? {
            return Err(in_flight());
        }
        revision = previous + 1;
    }
    let token = match auth::bot_token(state, c).await {
        Ok(token) => token,
        Err(error) => {
            settle(state, &id, &sending, revision, "rejected").await;
            return Err(error);
        }
    };
    let method = if update.is_some() {
        reqwest::Method::PUT
    } else {
        reqwest::Method::POST
    };
    let response = match client()?
        .request(method, url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) if error.is_connect() => {
            settle(state, &id, &sending, revision, "rejected").await;
            return Err(ApiError::bad_gateway(
                "Teams could not be reached. Retry shortly.",
            ));
        }
        Err(_) => {
            settle(state, &id, &sending, revision, "uncertain").await;
            return Err(ApiError::unprocessable(
                "Teams did not confirm this message in time. It may already be in the conversation, so it is not sent again.",
            ));
        }
    };
    let status = response.status();
    if !status.is_success() {
        settle(state, &id, &sending, revision, "rejected").await;
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            auth::forget_bot_token(c);
        }
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse().ok());
        return Err(teams_rejection(status, retry_after));
    }
    let message = response
        .bytes()
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value["id"].as_str().map(str::to_owned))
        .or_else(|| update.map(str::to_owned));
    let sent = Delivery {
        status: "sent".into(),
        message_id: message.clone(),
        lease_until: 0,
        ..sending
    };
    match store::update(state, &id, &sent, revision).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(delivery = %id, "Teams delivery changed after the message was sent")
        }
        Err(error) => {
            tracing::warn!(delivery = %id, %error, "Could not record a sent Teams message")
        }
    }
    message.ok_or_else(|| {
        ApiError::unprocessable("Teams accepted this message without returning its ID")
    })
}

async fn session_for_run(
    state: &AppState,
    app_id: &str,
    run_id: &str,
) -> Result<Option<(Session, Connection, ExecutionRunRecord)>, ApiError> {
    let Some((session, _)) = store::get::<Session>(state, &format!("run:{}", run_id)).await? else {
        return Ok(None);
    };
    if session.app_id != app_id {
        return Err(ApiError::FORBIDDEN);
    }
    let run = get_state_store(state)
        .await?
        .get_run_for_app(run_id, app_id)
        .await
        .map_err(|_| ApiError::internal("Cannot verify Teams execution"))?
        .ok_or(ApiError::FORBIDDEN)?;
    if run.event_id.as_deref() != Some(&session.event_id)
        || run.expires_at.is_some_and(|expiry| expiry <= now())
    {
        return Err(ApiError::FORBIDDEN);
    }
    if run.shadow_of_run_id.is_some()
        || run.regression_run_id.is_some()
        || matches!(
            run.run_variant,
            crate::execution::state::RunVariant::Shadow
                | crate::execution::state::RunVariant::Regression
        )
    {
        return Err(ApiError::FORBIDDEN);
    }
    let (c, _) = store::connection(state, &session.connection_id)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    live(state, &c).await?;
    if c.customer_tenant_id != session.tenant_id || c.client_id != session.client_id {
        return Err(ApiError::FORBIDDEN);
    }
    Ok(Some((session, c, run)))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TeamsSendRequest {
    /// Run ID from the Teams event's session.
    session_id: String,
    /// Idempotency key of 1–128 characters. Repeating it returns the first message's ID.
    request_id: String,
    /// Markdown text.
    #[serde(default)]
    text: Option<String>,
    /// Adaptive Card JSON.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    card: Option<Value>,
    /// Replace this earlier message instead of sending a new one.
    #[serde(default)]
    message_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct TeamsSendResponse {
    message_id: String,
}

#[utoipa::path(
    post,
    path = "/execution/apps/{app_id}/teams/send",
    operation_id = "send_teams_message",
    tag = "execution",
    description = "Send or update a message in the Teams conversation that started this run. Executor only; retry only 409, 429 and 5xx responses.",
    params(("app_id" = String, Path, description = "Application ID")),
    request_body = TeamsSendRequest,
    responses(
        (status = 200, description = "Delivered. A repeated request ID returns the original message ID.", body = TeamsSendResponse),
        (status = 400, description = "Invalid request, text or card"),
        (status = 403, description = "The caller is not this app's running Teams execution"),
        (status = 409, description = "The same request is still being delivered"),
        (status = 413, description = "The message exceeds 24000 bytes"),
        (status = 422, description = "Teams rejected the message, or an earlier attempt may already have been delivered"),
        (status = 429, description = "Teams is throttling the bot"),
        (status = 502, description = "Teams or Microsoft authentication failed transiently")
    ),
    security(("executor_jwt" = []))
)]
pub async fn send(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app): Path<String>,
    Json(input): Json<TeamsSendRequest>,
) -> Result<Json<TeamsSendResponse>, ApiError> {
    let AppUser::Executor(claims) = user else {
        return Err(ApiError::FORBIDDEN);
    };
    if claims.app_id != app || claims.run_id != input.session_id {
        return Err(ApiError::FORBIDDEN);
    }
    if input.request_id.is_empty() || input.request_id.len() > 128 {
        return Err(ApiError::bad_request(
            "Provide a Teams request ID of 1–128 characters",
        ));
    }
    let (session, c, run) = session_for_run(&state, &claims.app_id, &claims.run_id)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;
    if run.status.is_terminal() {
        return Err(ApiError::forbidden("This Teams execution has ended"));
    }
    let text = input.text.filter(|v| !v.trim().is_empty());
    let mut body = json!({"type":"message","textFormat":"markdown"});
    if let Some(text) = &text {
        body["text"] = json!(text);
    }
    if let Some(card) = input.card {
        if card["type"] != "AdaptiveCard" {
            return Err(ApiError::bad_request("Provide an Adaptive Card"));
        }
        body["attachments"] =
            json!([{"contentType":"application/vnd.microsoft.card.adaptive","content":card}]);
    }
    if body.get("text").is_none() && body.get("attachments").is_none() {
        return Err(ApiError::bad_request(
            "Provide message text or an Adaptive Card",
        ));
    }
    let update = input.message_id.as_deref().filter(|s| !s.is_empty());
    let key = format!("node:{}", input.request_id);
    let message_id = deliver(&state, &c, &session, &key, body, update).await?;
    if update.is_none()
        && let Some(text) = &text
        && let Err(error) = remember(&state, &session, &key, text, None).await
    {
        tracing::warn!(run_id = %session.run_id, %error, "Could not add a Teams reply to the conversation history");
    }
    Ok(Json(TeamsSendResponse { message_id }))
}

pub(crate) async fn output(
    state: &AppState,
    claims: &ExecutionClaims,
    events: &[ExecutionEventInput],
) -> Result<(), ApiError> {
    if claims.event_id.is_none()
        || !is_teams_run(&claims.run_id)
        || !events
            .iter()
            .any(|e| matches!(e.event_type.as_str(), "chat_out" | "interaction_request"))
    {
        return Ok(());
    }
    let Some((session, c, run)) = session_for_run(state, &claims.app_id, &claims.run_id).await?
    else {
        return Ok(());
    };
    for event in events {
        match event.event_type.as_str() {
            "interaction_request" => {
                let request: InteractionRequest = serde_json::from_value(event.payload.clone())
                    .map_err(|_| ApiError::bad_request("Invalid Teams interaction request"))?;
                let channel = request.channel.as_ref().ok_or_else(|| {
                    ApiError::bad_request("Teams interactions require a reply channel")
                })?;
                if channel.channel_id != claims.run_id
                    || channel.request_id.as_deref() != Some(&request.id)
                    || request.expires_at <= (now() / 1000) as u64
                    || request.run_id.as_deref() != Some(&claims.run_id)
                    || request.app_id.as_deref() != Some(&claims.app_id)
                {
                    return Err(ApiError::FORBIDDEN);
                }
                let id = hash(&[&c.id, &claims.run_id, &request.id]);
                let action = Action {
                    session: session.clone(),
                    request: request.clone(),
                    allowed_responders: if c.allowed_responders.is_empty() {
                        vec![session.user_id.clone()]
                    } else {
                        c.allowed_responders.clone()
                    },
                    state: "pending".into(),
                    lease_until: 0,
                    answer: None,
                    responder: None,
                };
                store::insert(
                    state,
                    &format!("action:{id}"),
                    &c.id,
                    &action,
                    action_expiry(request.expires_at, run.expires_at, now()),
                )
                .await?;
                let card = cards::render(&request, &id)?;
                deliver(
                    state,
                    &c,
                    &session,
                    &format!("interaction:{}", request.id),
                    json!({"type":"message","attachments":[{"contentType":"application/vnd.microsoft.card.adaptive","content":card}]}),
                    None,
                )
                .await?;
            }
            "chat_out" => {
                let text = response_text(&event.payload);
                if !text.is_empty() {
                    deliver(
                        state,
                        &c,
                        &session,
                        "chat_out",
                        json!({"type":"message","text":text,"textFormat":"markdown"}),
                        None,
                    )
                    .await?;
                }
                remember(state, &session, "chat_out", &text, Some(&event.payload)).await?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// The executor chooses an Interaction's expiry; approval state never outlives the run.
fn action_expiry(requested_secs: u64, run_expires_at: Option<i64>, now: i64) -> i64 {
    i64::try_from(requested_secs)
        .unwrap_or(i64::MAX)
        .saturating_mul(1000)
        .min(run_expires_at.unwrap_or(i64::MAX))
        .min(now + DAY_MS)
}

fn response_text(payload: &Value) -> String {
    let message = &payload["response"]["choices"][0]["message"];
    if let Some(text) = message["content"].as_str().filter(|s| !s.is_empty()) {
        return text.into();
    }
    message["content_parts"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter(|part| part["type"] == "text")
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Adds one reply to the history. The run's user message comes with its first reply;
/// `entry` makes retried replies no-ops. Returns whether anything changed.
fn record_reply(
    conversation: &mut Conversation,
    run: &str,
    entry: &str,
    user_message: Option<&Value>,
    text: &str,
    local: Option<&Value>,
) -> bool {
    if conversation
        .recorded
        .iter()
        .any(|recorded| recorded == entry)
    {
        return false;
    }
    if !conversation.completed_runs.iter().any(|done| done == run) {
        if let Some(message) = user_message {
            conversation.messages.push(message.clone());
        }
        conversation.completed_runs.push(run.into());
        keep_last(&mut conversation.completed_runs, MAX_COMPLETED_RUNS);
    }
    if !text.is_empty() {
        conversation
            .messages
            .push(json!({"role":"assistant","content":text}));
    }
    keep_last(&mut conversation.messages, MAX_HISTORY);
    if let Some(local) = local {
        conversation.local = local.clone();
    }
    conversation.last_run = run.into();
    conversation.recorded.push(entry.into());
    keep_last(&mut conversation.recorded, MAX_RECORDED_REPLIES);
    true
}

/// Records a reply sent by `chat_out` or the send endpoint. Only `chat_out` carries the
/// sessions the workflow returned, so only it updates them.
async fn remember(
    state: &AppState,
    s: &Session,
    key: &str,
    text: &str,
    sessions: Option<&Value>,
) -> Result<(), ApiError> {
    let input = store::get::<Value>(state, &format!("input:{}", s.run_id))
        .await?
        .map(|v| v.0)
        .unwrap_or_default();
    let local = sessions.map(|payload| {
        let mut local = payload
            .get("local_session")
            .filter(|value| value.is_object())
            .unwrap_or(&input["local_session"])
            .clone();
        if let Some(local) = local.as_object_mut() {
            local.remove("teams");
        }
        local
    });
    let user_message = input["messages"]
        .as_array()
        .and_then(|messages| messages.last());
    let entry = hash(&[&s.run_id, key]);
    let expires = now() + HISTORY_MS;
    for attempt in 0..4 {
        let previous = store::get::<Conversation>(state, &s.history_key).await?;
        let (mut conversation, revision) = previous.map(|(c, r)| (c, Some(r))).unwrap_or_default();
        if !record_reply(
            &mut conversation,
            &s.run_id,
            &entry,
            user_message,
            text,
            local.as_ref(),
        ) {
            break;
        }
        let saved = if let Some(revision) = revision {
            store::update_until(state, &s.history_key, &conversation, revision, expires).await?
        } else {
            store::insert(
                state,
                &s.history_key,
                &s.connection_id,
                &conversation,
                expires,
            )
            .await?
        };
        if saved {
            break;
        }
        if attempt == 3 {
            return Err(ApiError::conflict(
                "Teams conversation changed. Retry saving this response.",
            ));
        }
    }
    if let Some(global) = sessions
        .and_then(|payload| payload.get("global_session"))
        .filter(|v| v.is_object())
    {
        let mut global = global.clone();
        if let Some(map) = global.as_object_mut() {
            map.remove("teams");
        }
        let key = user_key(s);
        for attempt in 0..4 {
            let previous = store::get::<UserSession>(state, &key).await?;
            let (mut session, revision) = previous.map(|(s, r)| (s, Some(r))).unwrap_or_default();
            if session.completed_runs.contains(&s.run_id) {
                break;
            }
            session.value = global.clone();
            session.completed_runs.push(s.run_id.clone());
            keep_last(&mut session.completed_runs, MAX_COMPLETED_RUNS);
            let saved = if let Some(revision) = revision {
                store::update_until(state, &key, &session, revision, expires).await?
            } else {
                store::insert(state, &key, &s.connection_id, &session, expires).await?
            };
            if saved {
                break;
            }
            if attempt == 3 {
                return Err(ApiError::conflict(
                    "Teams user session changed. Retry saving this response.",
                ));
            }
        }
    }
    Ok(())
}

async fn respond(
    state: &AppState,
    c: &Connection,
    activity: &Value,
    data: &Value,
) -> Result<&'static str, ApiError> {
    let id = data["flow_like_action"]
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or(ApiError::FORBIDDEN)?;
    let key = format!("action:{id}");
    let (mut action, revision) = store::get::<Action>(state, &key)
        .await?
        .ok_or_else(|| ApiError::gone("This request has expired"))?;
    let sender = super::guid(string(activity, "/from/aadObjectId")?)?;
    if action.session.connection_id != c.id
        || action.session.tenant_id != c.customer_tenant_id
        || action.session.client_id != c.client_id
        || string(activity, "/conversation/id")? != action.session.conversation_id
        || !action.allowed_responders.contains(&sender)
        || (if c.allowed_responders.is_empty() {
            sender != action.session.user_id
        } else {
            !c.allowed_responders.contains(&sender)
        })
    {
        return Err(ApiError::forbidden(
            "This approval is assigned to another person or conversation",
        ));
    }
    if action.state == "responded" {
        return Ok("This request was already answered.");
    }
    if action.lease_until > now() {
        return Err(ApiError::conflict(
            "This answer is being processed. Try again shortly.",
        ));
    }
    let answer = cards::response(&action.request, data)?;
    if action
        .answer
        .as_ref()
        .is_some_and(|previous| previous != &answer)
        || action
            .responder
            .as_ref()
            .is_some_and(|previous| previous != &sender)
    {
        return Err(ApiError::conflict(
            "Another answer has already been submitted",
        ));
    }
    let run = get_state_store(state)
        .await?
        .get_run_for_app(&action.session.run_id, &c.app_id)
        .await
        .map_err(|_| ApiError::internal("Cannot verify the waiting execution"))?
        .ok_or(ApiError::NOT_FOUND)?;
    if run.status.is_terminal() || run.expires_at.is_some_and(|expiry| expiry <= now()) {
        return Err(ApiError::gone("The workflow is no longer waiting"));
    }
    action.state = "submitting".into();
    action.lease_until = now() + 30000;
    action.answer = Some(answer.clone());
    action.responder = Some(sender);
    if !store::update(state, &key, &action, revision).await? {
        return Err(ApiError::conflict("Another answer is being processed"));
    }
    let handle = action.request.channel.as_ref().ok_or(ApiError::FORBIDDEN)?;
    let descriptor = match &handle.transport {
        ChannelClientDescriptor::Http { .. } => &handle.transport,
        _ => handle.fallback.as_ref().ok_or(ApiError::FORBIDDEN)?,
    };
    let ChannelClientDescriptor::Http { token, .. } = descriptor else {
        return Err(ApiError::FORBIDDEN);
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        format!("Bearer {token}")
            .parse()
            .map_err(|_| ApiError::FORBIDDEN)?,
    );
    let body = ChannelPush {
        channel_id: handle.channel_id.clone(),
        request_id: Some(action.request.id.clone()),
        kind: ChannelPushKind::Reply,
        value: answer,
    };
    let result = crate::routes::channel::push_channel(
        State(state.clone()),
        headers,
        Path(handle.channel_id.clone()),
        Json(body),
    )
    .await;
    action.lease_until = 0;
    let outcome = match result {
        Ok(Json(result))
            if result.accepted
                || result.message.as_deref() == Some("This request was already answered") =>
        {
            action.state = "responded".into();
            Ok("Your response was submitted.")
        }
        Ok(_) => Err(ApiError::gone("The workflow is no longer waiting")),
        Err(error) => Err(error),
    };
    // A rejected or refused push stored nothing, so another answer may follow. A server
    // error leaves the push's outcome unknown and keeps the submitted answer binding.
    if outcome
        .as_ref()
        .is_err_and(|error| error.status().is_client_error())
    {
        action.state = "pending".into();
        action.answer = None;
        action.responder = None;
    }
    store::update(state, &key, &action, revision + 1).await?;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delivery(status: &str, message_id: Option<&str>, lease_until: i64) -> Delivery {
        Delivery {
            status: status.into(),
            message_id: message_id.map(Into::into),
            hash: "digest".into(),
            lease_until,
        }
    }

    #[test]
    fn identities_are_scoped_and_unambiguous() {
        assert_ne!(hash(&["ab", "c"]), hash(&["a", "bc"]));
        assert_ne!(
            hash(&["tenant-a", "message"]),
            hash(&["tenant-b", "message"])
        );
    }

    #[test]
    fn only_activity_hashes_are_teams_runs() {
        assert!(is_teams_run(&hash(&["connection", "activity"])));
        assert!(!is_teams_run("clx7q2k3b0000abcdefghijkl"));
        assert!(!is_teams_run(&"g".repeat(64)));
        assert!(!is_teams_run(""));
    }

    #[test]
    fn retried_messages_dispatch_once_and_recover_failed_dispatches() {
        let at = |status: &str, lease_until| Dispatch {
            status: status.into(),
            lease_until,
        };
        assert_eq!(dispatch_claim(&at("dispatched", 0), 10), Claim::Done);
        assert_eq!(dispatch_claim(&at("dispatching", 11), 10), Claim::InFlight);
        assert_eq!(dispatch_claim(&at("dispatching", 10), 10), Claim::Retry);
        assert_eq!(dispatch_claim(&at("failed", 0), 10), Claim::Retry);
    }

    #[test]
    fn repeated_deliveries_follow_executor_retry_semantics() {
        assert_eq!(
            delivery_retry(&delivery("sent", Some("message"), 0), "digest", 10).unwrap(),
            Some("message".into())
        );
        assert_eq!(
            delivery_retry(&delivery("rejected", None, 0), "digest", 10).unwrap(),
            None
        );
        let status = |result: Result<Option<String>, ApiError>| result.unwrap_err().status();
        assert_eq!(
            status(delivery_retry(&delivery("sending", None, 11), "digest", 10)),
            StatusCode::CONFLICT
        );
        for uncertain in [
            delivery("sending", None, 10),
            delivery("uncertain", None, 0),
        ] {
            assert_eq!(
                status(delivery_retry(&uncertain, "digest", 10)),
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }
        assert_eq!(
            status(delivery_retry(&delivery("sent", None, 0), "digest", 10)),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            status(delivery_retry(&delivery("sent", Some("m"), 0), "other", 10)),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn teams_rejections_separate_transient_from_deterministic_failures() {
        for (teams, api) in [
            (StatusCode::TOO_MANY_REQUESTS, StatusCode::TOO_MANY_REQUESTS),
            (StatusCode::INTERNAL_SERVER_ERROR, StatusCode::BAD_GATEWAY),
            (StatusCode::SERVICE_UNAVAILABLE, StatusCode::BAD_GATEWAY),
            (StatusCode::REQUEST_TIMEOUT, StatusCode::BAD_GATEWAY),
            (StatusCode::BAD_REQUEST, StatusCode::UNPROCESSABLE_ENTITY),
            (StatusCode::FORBIDDEN, StatusCode::UNPROCESSABLE_ENTITY),
            (StatusCode::NOT_FOUND, StatusCode::UNPROCESSABLE_ENTITY),
        ] {
            assert_eq!(teams_rejection(teams, None).status(), api, "{teams}");
        }
        assert!(
            teams_rejection(StatusCode::TOO_MANY_REQUESTS, Some(7))
                .public_message()
                .is_some_and(|message| message.contains("7 seconds"))
        );
    }

    #[test]
    fn interaction_state_never_outlives_the_run_or_a_day() {
        let now = 1_000_000;
        assert_eq!(action_expiry(1_100, None, now), 1_100_000);
        assert_eq!(action_expiry(u64::MAX, None, now), now + DAY_MS);
        assert_eq!(action_expiry(u64::MAX, Some(now + 5), now), now + 5);
    }

    #[test]
    fn replies_record_the_user_message_once_and_ignore_retries() {
        let mut conversation = Conversation::default();
        let user = json!({"role":"user","content":"hi"});
        let local = json!({"topic":"billing"});
        assert!(record_reply(
            &mut conversation,
            "run",
            "node:1",
            Some(&user),
            "first",
            None
        ));
        assert!(!record_reply(
            &mut conversation,
            "run",
            "node:1",
            Some(&user),
            "first",
            None
        ));
        assert!(record_reply(
            &mut conversation,
            "run",
            "chat_out",
            Some(&user),
            "final",
            Some(&local)
        ));
        assert_eq!(
            conversation.messages,
            vec![
                user,
                json!({"role":"assistant","content":"first"}),
                json!({"role":"assistant","content":"final"}),
            ]
        );
        assert_eq!(conversation.local, local);
        assert_eq!(conversation.completed_runs, vec!["run".to_string()]);
    }

    #[test]
    fn history_keeps_only_the_latest_messages() {
        let mut conversation = Conversation::default();
        for index in 0..30 {
            let user = json!({"role":"user","content":index});
            record_reply(
                &mut conversation,
                &format!("run-{index}"),
                &format!("entry-{index}"),
                Some(&user),
                "reply",
                None,
            );
        }
        assert_eq!(conversation.messages.len(), MAX_HISTORY);
        assert_eq!(
            conversation.messages.last(),
            Some(&json!({"role":"assistant","content":"reply"}))
        );
    }
}

//! Channel routes: the HTTP transport's waiter surface (what `HubChannelStore` on an executor
//! talks to) and the client-facing push / grant endpoints.
//!
//! Auth:
//! - waiter endpoints (`messages`, `inbound/drain`, `status`, `DELETE /{cid}`): the run's bearer —
//!   an executor JWT (whose `run_id` must equal the channel) or a user session. Rows are scoped
//!   by that subject, so one run can never read another's.
//! - `push` and `grant`: the channel responder JWT minted with the grant, from `Authorization`
//!   (or the forwarded viewer header), never the user session. Its `transport` claim decides
//!   whether a push flips a row here or is forwarded onto the cloud transport the waiter holds.

use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    http::HeaderMap,
    routing::{delete, get, post},
};
use flow_like_types::Value;
use flow_like_types::channel::{CHANNEL_TRANSPORT_HTTP, ChannelPush, ChannelPushKind, now_unix};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{
    channel::{ForwardOutcome, forward_push, store},
    entity::{
        channel,
        prelude::Channel,
        sea_orm_active_enums::{ChannelMessageKind, ChannelMessageStatus},
    },
    error::ApiError,
    execution::{ChannelClaims, verify_channel_responder},
    middleware::jwt::{AppUser, viewer_authorization},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{channel_id}/messages", post(register_message))
        .route(
            "/{channel_id}/messages/{request_id}",
            get(poll_message).delete(remove_message),
        )
        .route("/{channel_id}/inbound/drain", post(drain_inbound))
        .route("/{channel_id}/status", get(channel_status))
        .route("/{channel_id}", delete(close_channel))
        .route("/{channel_id}/push", post(push_channel))
        .route("/{channel_id}/grant", get(grant_channel))
}

// ============================================================================
// Wire types (mirrors of `flow_like_types::channel::hub`, with OpenAPI schemas)
// ============================================================================

/// Body of `POST /channels/{cid}/messages`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RegisterMessageBody {
    /// Request id the waiter minted; becomes the row id.
    pub request_id: String,
    /// Unix seconds; clamped to at most nine hours from now.
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RegisterMessageResponse {
    pub request_id: String,
    /// The expiry actually stored.
    pub expires_at: i64,
}

/// Result of polling one request.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PollMessageBody {
    Pending,
    Responded {
        #[schema(value_type = Object)]
        value: Value,
    },
    /// No such request for this channel and subject: closed, swept or never registered.
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DrainInboundBody {
    /// Unsolicited client messages since the last drain, oldest first.
    #[schema(value_type = Vec<Object>)]
    pub messages: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChannelStatusBody {
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PushResponse {
    /// False when the push was valid but changed nothing: a duplicate reply, or a cloud
    /// transport reporting that nobody listens for this channel any more.
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl PushResponse {
    fn accepted() -> Json<Self> {
        Json(Self {
            accepted: true,
            message: None,
        })
    }

    fn rejected(message: impl Into<String>) -> Json<Self> {
        Json(Self {
            accepted: false,
            message: Some(message.into()),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GrantSide {
    Client,
    Executor,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct GrantQuery {
    /// Which side of the channel to re-mint credentials for.
    pub side: GrantSide,
}

// ============================================================================
// Auth helpers
// ============================================================================

struct Waiter {
    sub: String,
    app_id: Option<String>,
}

/// The run behind a waiter call. An executor token is bound to its run, which is the channel.
fn waiter_for(user: &AppUser, channel_id: &str) -> Result<Waiter, ApiError> {
    let sub = user.executor_scoped_sub()?;
    let app_id = match user {
        AppUser::Executor(executor) => {
            if executor.run_id != channel_id {
                return Err(ApiError::forbidden(
                    "Executor token is bound to a different run than this channel",
                ));
            }
            Some(executor.app_id.clone())
        }
        _ => None,
    };
    Ok(Waiter { sub, app_id })
}

fn responder_claims(headers: &HeaderMap, channel_id: &str) -> Result<ChannelClaims, ApiError> {
    let authorization = viewer_authorization(headers)
        .ok_or_else(|| ApiError::unauthorized("Missing Authorization header"))?;
    let (scheme, token) = authorization
        .trim()
        .split_once(' ')
        .ok_or_else(|| ApiError::unauthorized("Invalid Authorization header format"))?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Err(ApiError::unauthorized(
            "Invalid Authorization header format",
        ));
    }
    let claims = verify_channel_responder(token.trim())
        .map_err(|e| ApiError::unauthorized(format!("Invalid channel responder token: {e}")))?;
    if claims.channel_id != channel_id {
        return Err(ApiError::forbidden(
            "Channel token does not match this channel",
        ));
    }
    Ok(claims)
}

fn db_error(operation: &str) -> impl FnOnce(sea_orm::DbErr) -> ApiError + '_ {
    move |e| ApiError::internal(format!("Channel {operation} failed: {e}"))
}

// ============================================================================
// Waiter endpoints
// ============================================================================

/// Register a pending request the waiter will block on.
#[utoipa::path(
    post,
    path = "/channels/{channel_id}/messages",
    tag = "channels",
    description = "Register a request the run is about to wait on; must complete before the request is streamed to the client.",
    params(("channel_id" = String, Path, description = "Channel (run) id")),
    request_body = RegisterMessageBody,
    responses(
        (status = 200, description = "Request registered", body = RegisterMessageResponse),
        (status = 403, description = "Token is bound to another run"),
    ),
    security(("executor_jwt" = []), ("bearer_auth" = []))
)]
pub async fn register_message(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(channel_id): Path<String>,
    Json(body): Json<RegisterMessageBody>,
) -> Result<Json<RegisterMessageResponse>, ApiError> {
    if body.request_id.trim().is_empty() {
        return Err(ApiError::bad_request("request_id must not be empty"));
    }
    let waiter = waiter_for(&user, &channel_id)?;
    let expires_at = store::register_request(
        &state.db,
        &channel_id,
        &body.request_id,
        &waiter.sub,
        waiter.app_id.as_deref(),
        body.expires_at,
    )
    .await
    .map_err(db_error("register"))?;
    Ok(Json(RegisterMessageResponse {
        request_id: body.request_id,
        expires_at,
    }))
}

/// Poll one registered request.
#[utoipa::path(
    get,
    path = "/channels/{channel_id}/messages/{request_id}",
    tag = "channels",
    description = "Read the state of one registered request: pending, responded (with the reply) or missing.",
    params(
        ("channel_id" = String, Path, description = "Channel (run) id"),
        ("request_id" = String, Path, description = "Request id"),
    ),
    responses((status = 200, description = "Request state", body = PollMessageBody)),
    security(("executor_jwt" = []), ("bearer_auth" = []))
)]
pub async fn poll_message(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((channel_id, request_id)): Path<(String, String)>,
) -> Result<Json<PollMessageBody>, ApiError> {
    let waiter = waiter_for(&user, &channel_id)?;
    let poll = store::poll_request(&state.db, &channel_id, &request_id, &waiter.sub)
        .await
        .map_err(db_error("poll"))?;
    Ok(Json(match poll {
        flow_like_types::channel::ChannelPoll::Pending => PollMessageBody::Pending,
        flow_like_types::channel::ChannelPoll::Responded(value) => {
            PollMessageBody::Responded { value }
        }
        flow_like_types::channel::ChannelPoll::Missing => PollMessageBody::Missing,
    }))
}

/// Drop a registered request. Idempotent.
#[utoipa::path(
    delete,
    path = "/channels/{channel_id}/messages/{request_id}",
    tag = "channels",
    description = "Remove a registered request the run consumed or gave up on.",
    params(
        ("channel_id" = String, Path, description = "Channel (run) id"),
        ("request_id" = String, Path, description = "Request id"),
    ),
    responses((status = 200, description = "Request removed (or already gone)")),
    security(("executor_jwt" = []), ("bearer_auth" = []))
)]
pub async fn remove_message(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((channel_id, request_id)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let waiter = waiter_for(&user, &channel_id)?;
    store::remove_request(&state.db, &channel_id, &request_id, &waiter.sub)
        .await
        .map_err(db_error("remove"))?;
    Ok(Json(serde_json::json!({ "status": "ok" })))
}

/// Take the unsolicited client messages queued for the channel.
#[utoipa::path(
    post,
    path = "/channels/{channel_id}/inbound/drain",
    tag = "channels",
    description = "Take every unsolicited client message (e.g. steering text) pushed since the last drain, oldest first.",
    params(("channel_id" = String, Path, description = "Channel (run) id")),
    responses((status = 200, description = "Drained messages", body = DrainInboundBody)),
    security(("executor_jwt" = []), ("bearer_auth" = []))
)]
pub async fn drain_inbound(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(channel_id): Path<String>,
) -> Result<Json<DrainInboundBody>, ApiError> {
    let waiter = waiter_for(&user, &channel_id)?;
    let messages = store::drain_inbound(&state.db, &channel_id, &waiter.sub)
        .await
        .map_err(db_error("drain"))?;
    Ok(Json(DrainInboundBody { messages }))
}

/// Whether the client asked the run to stop.
#[utoipa::path(
    get,
    path = "/channels/{channel_id}/status",
    tag = "channels",
    description = "Whether a cancel was pushed into the channel.",
    params(("channel_id" = String, Path, description = "Channel (run) id")),
    responses((status = 200, description = "Channel status", body = ChannelStatusBody)),
    security(("executor_jwt" = []), ("bearer_auth" = []))
)]
pub async fn channel_status(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(channel_id): Path<String>,
) -> Result<Json<ChannelStatusBody>, ApiError> {
    let waiter = waiter_for(&user, &channel_id)?;
    let cancelled = store::is_cancelled(&state.db, &channel_id, &waiter.sub)
        .await
        .map_err(db_error("status"))?;
    Ok(Json(ChannelStatusBody { cancelled }))
}

/// Release everything the channel holds.
#[utoipa::path(
    delete,
    path = "/channels/{channel_id}",
    tag = "channels",
    description = "Delete every row of the channel; called by the run when it finishes.",
    params(("channel_id" = String, Path, description = "Channel (run) id")),
    responses((status = 200, description = "Channel closed")),
    security(("executor_jwt" = []), ("bearer_auth" = []))
)]
pub async fn close_channel(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(channel_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let waiter = waiter_for(&user, &channel_id)?;
    let deleted = store::close_channel(&state.db, &channel_id, &waiter.sub)
        .await
        .map_err(db_error("close"))?;
    Ok(Json(
        serde_json::json!({ "status": "ok", "deleted": deleted }),
    ))
}

// ============================================================================
// Client endpoints
// ============================================================================

/// Outcome of applying a reply push to the row it addresses. Pure, so the state machine is
/// testable without a database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplyDecision {
    Accept,
    AlreadyResponded,
    NotFound,
    Forbidden,
    Expired,
}

fn decide_reply(
    row: Option<&channel::Model>,
    channel_id: &str,
    sub: &str,
    now: i64,
) -> ReplyDecision {
    let Some(row) = row else {
        return ReplyDecision::NotFound;
    };
    if row.channel_id != channel_id || row.kind != ChannelMessageKind::Request {
        return ReplyDecision::NotFound;
    }
    if row.sub != sub {
        return ReplyDecision::Forbidden;
    }
    if row.status == ChannelMessageStatus::Responded {
        return ReplyDecision::AlreadyResponded;
    }
    if now > row.expires_at {
        return ReplyDecision::Expired;
    }
    ReplyDecision::Accept
}

/// Push a reply, an unsolicited message or a cancel into a channel.
#[utoipa::path(
    post,
    path = "/channels/{channel_id}/push",
    tag = "channels",
    description = "Client → run delivery: answer a request (first reply wins), queue an unsolicited message, or cancel the run. Forwarded onto the cloud transport when the channel uses one.",
    params(("channel_id" = String, Path, description = "Channel (run) id")),
    request_body(content = Object, description = "`ChannelPush`: `{ channel_id, request_id?, kind: reply|inbound|cancel, value }`"),
    responses(
        (status = 200, description = "Push processed", body = PushResponse),
        (status = 401, description = "Missing or invalid channel responder token"),
        (status = 403, description = "Token, path and body disagree on the channel, or the request belongs to another subject"),
        (status = 404, description = "No such pending request"),
        (status = 410, description = "The request expired"),
        (status = 429, description = "Too many unsolicited messages are queued"),
        (status = 503, description = "The channel uses a cloud transport this API cannot forward to"),
    ),
    security(("channel_responder_jwt" = []))
)]
pub async fn push_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<String>,
    Json(body): Json<ChannelPush>,
) -> Result<Json<PushResponse>, ApiError> {
    let claims = responder_claims(&headers, &channel_id)?;
    if body.channel_id != channel_id {
        return Err(ApiError::forbidden("Push body does not match this channel"));
    }

    if claims.transport != CHANNEL_TRANSPORT_HTTP {
        return Ok(
            match forward_push(state.channels.forwarder(), &body).await? {
                ForwardOutcome::Delivered => PushResponse::accepted(),
                ForwardOutcome::Undeliverable(message) => PushResponse::rejected(message),
            },
        );
    }

    match body.kind {
        ChannelPushKind::Reply => reply(&state, &claims, &channel_id, body).await,
        ChannelPushKind::Inbound => inbound(&state, &claims, &channel_id, &body.value).await,
        ChannelPushKind::Cancel => {
            store::insert_cancel(
                &state.db,
                &channel_id,
                &claims.sub,
                claims.app_id.as_deref(),
                claims.exp,
            )
            .await
            .map_err(db_error("cancel"))?;
            Ok(PushResponse::accepted())
        }
    }
}

async fn reply(
    state: &AppState,
    claims: &ChannelClaims,
    channel_id: &str,
    body: ChannelPush,
) -> Result<Json<PushResponse>, ApiError> {
    let request_id = body
        .request_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ApiError::bad_request("A reply needs a request_id"))?;
    let row = Channel::find_by_id(request_id)
        .one(&state.db)
        .await
        .map_err(db_error("lookup"))?;
    let row = match (
        decide_reply(row.as_ref(), channel_id, &claims.sub, now_unix()),
        row,
    ) {
        (ReplyDecision::Accept, Some(row)) => row,
        (ReplyDecision::AlreadyResponded, _) => {
            return Ok(PushResponse::rejected("This request was already answered"));
        }
        (ReplyDecision::Forbidden, _) => {
            return Err(ApiError::forbidden(
                "This request belongs to another subject",
            ));
        }
        (ReplyDecision::Expired, _) => return Err(ApiError::gone("This request has expired")),
        (ReplyDecision::NotFound, _) | (ReplyDecision::Accept, None) => {
            return Err(ApiError::not_found("No pending request with this id"));
        }
    };

    let mut active: channel::ActiveModel = row.into();
    active.status = Set(ChannelMessageStatus::Responded);
    active.value = Set(Some(body.value.to_string()));
    active.update(&state.db).await.map_err(db_error("reply"))?;
    Ok(PushResponse::accepted())
}

async fn inbound(
    state: &AppState,
    claims: &ChannelClaims,
    channel_id: &str,
    value: &Value,
) -> Result<Json<PushResponse>, ApiError> {
    let pending = store::count_pending_inbound(&state.db, channel_id, &claims.sub)
        .await
        .map_err(db_error("inbound count"))?;
    if store::inbound_is_full(pending) {
        return Err(ApiError::too_many_requests(
            "Too many messages are already waiting for this run",
        ));
    }
    store::insert_inbound(
        &state.db,
        channel_id,
        &claims.sub,
        claims.app_id.as_deref(),
        claims.exp,
        value,
    )
    .await
    .map_err(db_error("inbound"))?;
    Ok(PushResponse::accepted())
}

/// Re-mint transport credentials for one side of the channel.
#[utoipa::path(
    get,
    path = "/channels/{channel_id}/grant",
    tag = "channels",
    description = "Refresh the client handle (`side=client`) or the executor grant (`side=executor`) with fresh transport credentials; the new expiry never exceeds the channel's.",
    params(
        ("channel_id" = String, Path, description = "Channel (run) id"),
        GrantQuery,
    ),
    responses(
        (status = 200, description = "`ChannelHandle` for `side=client`, `ChannelGrant` for `side=executor`", body = Object),
        (status = 401, description = "Missing or invalid channel responder token"),
        (status = 410, description = "The channel has expired"),
    ),
    security(("channel_responder_jwt" = []))
)]
pub async fn grant_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<String>,
    Query(query): Query<GrantQuery>,
) -> Result<Json<Value>, ApiError> {
    let claims = responder_claims(&headers, &channel_id)?;
    let remaining = claims.exp - now_unix();
    if remaining <= 0 {
        return Err(ApiError::gone("This channel has expired"));
    }
    let issuer = &state.channels;
    let value = match query.side {
        GrantSide::Client => serde_json::to_value(
            issuer
                .client_handle(
                    &channel_id,
                    &claims.sub,
                    claims.app_id.as_deref(),
                    remaining,
                )
                .await
                .map_err(|e| ApiError::internal(format!("Channel grant failed: {e}")))?,
        ),
        GrantSide::Executor => serde_json::to_value(
            issuer
                .grant(
                    &channel_id,
                    &claims.sub,
                    claims.app_id.as_deref(),
                    remaining,
                )
                .await
                .map_err(|e| ApiError::internal(format!("Channel grant failed: {e}")))?,
        ),
    }
    .map_err(|e| ApiError::internal(format!("Channel grant serialization failed: {e}")))?;
    Ok(Json(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::channel::hub;
    use serde_json::json;

    fn row(
        kind: ChannelMessageKind,
        status: ChannelMessageStatus,
        expires_at: i64,
    ) -> channel::Model {
        channel::Model {
            id: "req-1".into(),
            channel_id: "run-1".into(),
            sub: "user-1".into(),
            app_id: None,
            kind,
            status,
            expires_at,
            value: None,
            created_at: chrono::Utc::now().fixed_offset(),
        }
    }

    #[test]
    fn reply_state_machine() {
        let pending = row(
            ChannelMessageKind::Request,
            ChannelMessageStatus::Pending,
            200,
        );
        assert_eq!(
            decide_reply(Some(&pending), "run-1", "user-1", 100),
            ReplyDecision::Accept
        );
        assert_eq!(
            decide_reply(Some(&pending), "run-1", "user-1", 201),
            ReplyDecision::Expired
        );
        assert_eq!(
            decide_reply(Some(&pending), "run-1", "user-2", 100),
            ReplyDecision::Forbidden
        );
        assert_eq!(
            decide_reply(Some(&pending), "run-2", "user-1", 100),
            ReplyDecision::NotFound
        );
        assert_eq!(
            decide_reply(None, "run-1", "user-1", 100),
            ReplyDecision::NotFound
        );
        let responded = row(
            ChannelMessageKind::Request,
            ChannelMessageStatus::Responded,
            200,
        );
        assert_eq!(
            decide_reply(Some(&responded), "run-1", "user-1", 100),
            ReplyDecision::AlreadyResponded
        );
        let inbound = row(
            ChannelMessageKind::Inbound,
            ChannelMessageStatus::Responded,
            200,
        );
        assert_eq!(
            decide_reply(Some(&inbound), "run-1", "user-1", 100),
            ReplyDecision::NotFound
        );
    }

    #[test]
    fn wire_types_match_the_hub_store() {
        let register: RegisterMessageBody = serde_json::from_value(
            serde_json::to_value(hub::RegisterMessageRequest {
                request_id: "r".into(),
                expires_at: 5,
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(register.request_id, "r");
        assert_eq!(register.expires_at, 5);

        for (body, expected) in [
            (PollMessageBody::Pending, json!({ "status": "pending" })),
            (
                PollMessageBody::Responded { value: json!(1) },
                json!({ "status": "responded", "value": 1 }),
            ),
            (PollMessageBody::Missing, json!({ "status": "missing" })),
        ] {
            let value = serde_json::to_value(&body).unwrap();
            assert_eq!(value, expected);
            let parsed: hub::PollMessageResponse = serde_json::from_value(value).unwrap();
            match (body, parsed) {
                (PollMessageBody::Pending, hub::PollMessageResponse::Pending)
                | (PollMessageBody::Missing, hub::PollMessageResponse::Missing) => {}
                (
                    PollMessageBody::Responded { value: a },
                    hub::PollMessageResponse::Responded { value: b },
                ) => assert_eq!(a, b),
                other => panic!("mismatch: {other:?}"),
            }
        }

        let drained: hub::DrainInboundResponse = serde_json::from_value(
            serde_json::to_value(DrainInboundBody {
                messages: vec![json!("steer")],
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(drained.messages, vec![json!("steer")]);
        let status: hub::ChannelStatusResponse = serde_json::from_value(
            serde_json::to_value(ChannelStatusBody { cancelled: true }).unwrap(),
        )
        .unwrap();
        assert!(status.cancelled);
    }

    #[test]
    fn push_response_shape() {
        let accepted = serde_json::to_value(PushResponse::accepted().0).unwrap();
        assert_eq!(accepted, json!({ "accepted": true }));
        let rejected = serde_json::to_value(PushResponse::rejected("dup").0).unwrap();
        assert_eq!(rejected, json!({ "accepted": false, "message": "dup" }));
    }

    #[test]
    fn grant_side_is_snake_case() {
        assert_eq!(
            serde_json::from_str::<GrantSide>("\"executor\"").unwrap(),
            GrantSide::Executor
        );
        assert_eq!(
            serde_json::from_str::<GrantSide>("\"client\"").unwrap(),
            GrantSide::Client
        );
    }
}

//! Human-in-the-loop interaction endpoints
//!
//! Flow:
//! 1. Node/executor POSTs to create an interaction (Bearer: user's OpenID token) → gets back an SSE stream
//! 2. First SSE event contains the interaction ID + responder JWT
//! 3. Node relays the JWT and form details to the user via InterComEvent
//! 4. User submits response via POST /interaction/{id}/respond with the responder JWT
//! 5. SSE polling detects the DB update → streams the response back to the node
//! 6. DB row is deleted, SSE stream closes
//!
//! Security:
//! - Authenticated endpoint — requires valid OpenID/PAT/API-key token (same as embedding proxy)
//! - Max TTL capped at 240s, min 10s
//! - Per-user concurrent interaction limit (MAX_CONCURRENT_PER_USER)
//! - Background cleanup task deletes orphaned rows past expiresAt
//!   (handles client disconnect before stream cleanup)
//! - Responder JWT required for submitting responses

use crate::{
    entity::{interaction, prelude::Interaction, sea_orm_active_enums::InteractionStatus},
    error::ApiError,
    execution::{
        InteractionJwtParams, sign_interaction_responder_jwt, verify_interaction_responder_jwt,
    },
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
    routing::post,
};
use flow_like_types::create_id;
use futures_util::Stream;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder,
};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::time::Duration;
use utoipa::ToSchema;

const MAX_TTL_SECONDS: i64 = 240;
const MIN_TTL_SECONDS: i64 = 10;
/// Per-user ceiling on concurrent pending interactions
const MAX_CONCURRENT_PER_USER: u64 = 10;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", post(create_interaction))
        .route("/{id}/respond", post(respond_to_interaction))
}

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateInteractionRequest {
    /// ID of the app this interaction originates from
    pub app_id: String,
    /// TTL in seconds (defaults to 120)
    #[serde(default = "default_ttl")]
    pub ttl_seconds: i64,
}

fn default_ttl() -> i64 {
    120
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SseCreatedPayload {
    pub id: String,
    pub responder_jwt: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SseRespondedPayload {
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RespondRequest {
    /// The response value (interpretation depends on interaction type)
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RespondResponse {
    pub accepted: bool,
}

// ============================================================================
// Handlers
// ============================================================================

/// Create a human-in-the-loop interaction and stream updates via SSE
///
/// Returns an SSE stream. Events:
/// - `created`: `{ id, responder_jwt, expires_at }` — emitted immediately
/// - `responded`: `{ value }` — emitted when a user submits a response
/// - `expired`: emitted when the TTL elapses without a response
///
/// The DB row is automatically deleted when the stream ends.
#[utoipa::path(
    post,
    path = "/interaction",
    tag = "interaction",
    request_body = CreateInteractionRequest,
    responses(
        (status = 200, description = "SSE stream with interaction lifecycle events"),
        (status = 500, description = "Failed to create interaction"),
    )
)]
#[tracing::instrument(name = "POST /interaction (SSE)", skip(state, user, body))]
pub async fn create_interaction(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<CreateInteractionRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let sub = user.executor_scoped_sub()?;

    let ttl = body.ttl_seconds.max(MIN_TTL_SECONDS).min(MAX_TTL_SECONDS);
    let id = create_id();
    let now = chrono::Utc::now();
    let expires_at = now.timestamp() + ttl;

    // Per-user concurrency limit — uses @@index([sub, expiresAt])
    let open_count = count_open_for_user(&state.db, &sub, now.timestamp()).await?;

    if open_count >= MAX_CONCURRENT_PER_USER {
        // Cleanup expired rows that were never deleted (orphans)
        cleanup_expired_for_user(&state.db, &sub, now.timestamp()).await?;

        let open_after_cleanup = count_open_for_user(&state.db, &sub, now.timestamp()).await?;

        if open_after_cleanup >= MAX_CONCURRENT_PER_USER {
            // Evict the oldest interaction — its SSE stream will see
            // Ok(None) on the next poll and emit "expired" to the consumer.
            evict_oldest(&state.db, &sub).await?;
        }
    }

    let model = interaction::ActiveModel {
        id: Set(id.clone()),
        sub: Set(sub.clone()),
        app_id: Set(body.app_id.clone()),
        status: Set(InteractionStatus::Pending),
        expires_at: Set(expires_at),
        response_value: Set(None),
        created_at: Set(now.naive_utc()),
        updated_at: Set(now.naive_utc()),
    };
    model
        .insert(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to create interaction: {e}")))?;

    let responder_jwt = sign_interaction_responder_jwt(InteractionJwtParams {
        sub: sub.clone(),
        interaction_id: id.clone(),
        ttl_seconds: Some(ttl),
    })
    .map_err(|e| ApiError::internal(format!("Failed to sign responder JWT: {e}")))?;

    let created_payload = SseCreatedPayload {
        id: id.clone(),
        responder_jwt,
        expires_at,
    };

    // Spawn a background task that guarantees DB cleanup even if client disconnects
    let cleanup_db = state.db.clone();
    let cleanup_id = id.clone();
    flow_like_types::tokio::spawn(async move {
        let delay = Duration::from_secs(ttl as u64 + 5); // 5s grace after TTL
        flow_like_types::tokio::time::sleep(delay).await;
        if let Err(e) = Interaction::delete_by_id(&cleanup_id)
            .exec(&cleanup_db)
            .await
        {
            tracing::debug!(
                interaction_id = %cleanup_id,
                error = %e,
                "Background cleanup: row already deleted or error"
            );
        }
    });

    let db = state.db.clone();
    let stream = create_interaction_stream(db, id, created_payload, expires_at);

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .text("keep-alive")
            .interval(Duration::from_secs(1)),
    ))
}

fn create_interaction_stream(
    db: sea_orm::DatabaseConnection,
    id: String,
    created_payload: SseCreatedPayload,
    expires_at: i64,
) -> std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> {
    let stream = async_stream::stream! {
        let created_json = serde_json::to_string(&created_payload).unwrap_or_default();
        yield Ok(Event::default().event("created").data(created_json));

        let poll_interval = Duration::from_millis(500);

        loop {
            flow_like_types::tokio::time::sleep(poll_interval).await;

            let now = chrono::Utc::now().timestamp();
            if now >= expires_at {
                yield Ok(Event::default().event("expired").data("{}"));
                break;
            }

            match Interaction::find_by_id(&id).one(&db).await {
                Ok(Some(row)) => {
                    if row.status == InteractionStatus::Responded {
                        let value: serde_json::Value = row
                            .response_value
                            .as_deref()
                            .and_then(|v| serde_json::from_str(v).ok())
                            .unwrap_or(serde_json::Value::Null);

                        let payload = SseRespondedPayload { value };
                        let json = serde_json::to_string(&payload).unwrap_or_default();
                        yield Ok(Event::default().event("responded").data(json));
                        break;
                    }
                }
                Ok(None) => {
                    yield Ok(Event::default().event("expired").data("{}"));
                    break;
                }
                Err(e) => {
                    tracing::warn!(interaction_id = %id, error = %e, "Failed to poll interaction");
                }
            }
        }

        // Cleanup: delete the row since we've streamed the result
        if let Err(e) = Interaction::delete_by_id(&id).exec(&db).await {
            tracing::warn!(interaction_id = %id, error = %e, "Failed to delete interaction row");
        }
    };

    Box::pin(stream)
}

/// Submit a response to an interaction (first write wins)
///
/// Requires an InteractionResponder JWT in the Authorization header.
/// Only succeeds if the interaction is still in PENDING status.
#[utoipa::path(
    post,
    path = "/interaction/{id}/respond",
    tag = "interaction",
    params(("id" = String, Path, description = "Interaction ID")),
    request_body = RespondRequest,
    responses(
        (status = 200, description = "Response recorded", body = RespondResponse),
        (status = 400, description = "Invalid JWT"),
        (status = 403, description = "JWT does not match this interaction"),
        (status = 404, description = "Interaction not found"),
        (status = 410, description = "Interaction expired"),
    ),
    security(
        ("interaction_responder_jwt" = [])
    )
)]
#[tracing::instrument(name = "POST /interaction/{id}/respond", skip(state, headers, body))]
pub async fn respond_to_interaction(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<RespondRequest>,
) -> Result<Json<RespondResponse>, ApiError> {
    let token = extract_bearer_token(&headers)?;
    let claims = verify_interaction_responder_jwt(token)
        .map_err(|e| ApiError::bad_request(format!("Invalid interaction responder JWT: {e}")))?;

    if claims.interaction_id != id {
        return Err(ApiError::forbidden("JWT does not match this interaction"));
    }

    let row = Interaction::find_by_id(&id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to query interaction: {e}")))?
        .ok_or_else(|| ApiError::not_found("Interaction not found"))?;

    if row.status != InteractionStatus::Pending {
        return Ok(Json(RespondResponse { accepted: false }));
    }

    let now = chrono::Utc::now();
    if now.timestamp() > row.expires_at {
        return Err(ApiError::gone("Interaction has expired"));
    }

    let response_json = serde_json::to_string(&body.value)
        .map_err(|e| ApiError::bad_request(format!("Invalid response value: {e}")))?;

    let mut active: interaction::ActiveModel = row.into();
    active.status = Set(InteractionStatus::Responded);
    active.response_value = Set(Some(response_json));
    active.updated_at = Set(now.naive_utc());
    active
        .update(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to update interaction: {e}")))?;

    Ok(Json(RespondResponse { accepted: true }))
}

// ============================================================================
// Helpers
// ============================================================================

fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("Missing Authorization header"))?
        .strip_prefix("Bearer ")
        .ok_or_else(|| ApiError::bad_request("Invalid Authorization header format"))
}

/// Count non-expired interactions for a user.
/// Hits @@index([sub, expiresAt]).
async fn count_open_for_user(
    db: &sea_orm::DatabaseConnection,
    sub: &str,
    now_ts: i64,
) -> Result<u64, ApiError> {
    Interaction::find()
        .filter(interaction::Column::Sub.eq(sub))
        .filter(interaction::Column::ExpiresAt.gt(now_ts))
        .count(db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to count pending interactions: {e}")))
}

/// Delete all expired rows for a user (orphan cleanup).
async fn cleanup_expired_for_user(
    db: &sea_orm::DatabaseConnection,
    sub: &str,
    now_ts: i64,
) -> Result<(), ApiError> {
    Interaction::delete_many()
        .filter(interaction::Column::Sub.eq(sub))
        .filter(interaction::Column::ExpiresAt.lte(now_ts))
        .exec(db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to cleanup expired interactions: {e}")))?;
    Ok(())
}

/// The evicted row's SSE stream will see `Ok(None)` on its next poll
/// and gracefully emit an "expired" event to the consumer.
async fn evict_oldest(db: &sea_orm::DatabaseConnection, sub: &str) -> Result<(), ApiError> {
    let oldest = Interaction::find()
        .filter(interaction::Column::Sub.eq(sub))
        .order_by_asc(interaction::Column::CreatedAt)
        .one(db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to find oldest interaction: {e}")))?;

    if let Some(row) = oldest {
        Interaction::delete_by_id(&row.id)
            .exec(db)
            .await
            .map_err(|e| ApiError::internal(format!("Failed to evict oldest interaction: {e}")))?;
    }

    Ok(())
}

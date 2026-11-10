//! The hub side of device-hosted bots. A managed device authenticates every call with its own
//! DPoP session (never a user token), polls for the queued turns of the bots bound to it
//! (`Bot.location = device`, `Bot.deviceId = {id}`, same owner), claims one with a lease and
//! ends it with a reply, a silent end, a park for an approval or a release. Every query is
//! scoped to the device and its owner in SQL; a turn of another device's bot reads as missing.
//! Model usage is reserved at the claim and settled by the call that ends the turn.

use super::bots_grant;
use crate::{
    bot_host::{
        approvals,
        ledger::{self, BOT_TURN_LEASE_LOST, ClaimRequest},
        worker,
    },
    devices,
    entity::{bot, bot_turn},
    error::ApiError,
    routes::{
        bots::{
            presence::{forget_device_in, report_device_in},
            turns::{TurnHostRequest, checked_host_id},
            workspace::live_workspace,
        },
        inbox::{entries::validate_send, model::SendEntryRequest, now_millis, wake},
    },
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::{DateTime, Duration, FixedOffset};
use flow_like_types::{
    bot_message::ApprovalRequest,
    bot_turn::{TurnHostKind, failure_message_id, reply_message_id},
    device_bot::{
        BUSY_POLL_MS, DeviceBotInfo, DeviceClaimRequest, DeviceForgetRequest, DeviceForgetResponse,
        DeviceHeartbeatRequest, DeviceHeartbeatResponse, DeviceParkRequest, DevicePollRequest,
        DevicePollResponse, DeviceReleaseRequest, DeviceReplyRequest, DeviceReplyResponse,
        DeviceSilentRequest, DeviceTurnGrant, DeviceTurnState, DeviceWaitRequest,
        DeviceWaitResponse, IDLE_POLL_MS, MAX_POLL_MS, MAX_REPLY_BYTES, TOKEN_REFRESH_SECS,
    },
    inbox::{InboxEntryKind, is_valid_message_id},
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};

const MAX_ERROR_CODE_CHARS: usize = 64;
const REPLY_FAILURE_CODES: [&str; 6] = [
    worker::CODE_TIMEOUT,
    worker::CODE_QUOTA,
    worker::CODE_SETUP,
    worker::CODE_MODEL,
    worker::CODE_FAILED,
    worker::CODE_REPLY_REFUSED,
];

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/{id}/bots/poll", post(poll))
        .route("/{id}/bots/hosts/forget", post(forget))
        .route("/{id}/bots/turns/{turn_id}/claim", post(claim))
        .route(
            "/{id}/bots/turns/{turn_id}/checkpoint",
            get(checkpoint_load)
                .put(checkpoint_save)
                .layer(DefaultBodyLimit::max(
                    flow_like_types::bot_turn::MAX_CHECKPOINT_BYTES + 4096,
                )),
        )
        .route("/{id}/bots/turns/{turn_id}/heartbeat", post(heartbeat))
        .route("/{id}/bots/turns/{turn_id}/silent", post(silent))
        .route("/{id}/bots/turns/{turn_id}/wait", post(wait))
        .route("/{id}/bots/turns/{turn_id}/park", post(park))
        .route("/{id}/bots/turns/{turn_id}/release", post(release))
        .route(
            "/{id}/bots/turns/{turn_id}/reply",
            post(reply).layer(DefaultBodyLimit::max(MAX_REPLY_BYTES)),
        )
}

/// The owner of the device behind a verified device session; the DPoP proof binds `path`.
async fn owner_of(
    state: &AppState,
    id: &str,
    headers: &HeaderMap,
    path: &str,
) -> Result<String, ApiError> {
    Ok(devices::device_principal(
        &devices::context(state),
        id,
        headers,
        "POST",
        &format!("/devices/{id}{path}"),
        false,
    )
    .await?
    .status
    .owner_id)
}

fn host(host_id: String) -> Result<String, ApiError> {
    checked_host_id(TurnHostRequest { host_id })
}

async fn checkpoint_claim(
    state: &AppState,
    id: &str,
    turn_id: &str,
    headers: &HeaderMap,
    method: &str,
    host_id: String,
    attempt: u32,
) -> Result<crate::bot_host::recovery::Claim, ApiError> {
    let owner_id = devices::device_principal(
        &devices::context(state),
        id,
        headers,
        method,
        &format!("/devices/{id}/bots/turns/{turn_id}/checkpoint"),
        false,
    )
    .await?
    .status
    .owner_id;
    held_turn(&state.db, &owner_id, id, turn_id).await?;
    Ok(crate::bot_host::recovery::Claim {
        turn_id: turn_id.to_owned(),
        owner_id,
        host_id: host(host_id)?,
        attempt,
    })
}

#[utoipa::path(get, path = "/devices/{id}/bots/turns/{turn_id}/checkpoint", tag = "devices",
    description = "Reads a bound bot turn's checkpoint under the device's current lease.",
    params(("id" = String, Path), ("turn_id" = String, Path),
        crate::routes::bots::checkpoint::CheckpointQuery),
    responses((status = 200, description = "The saved execution state", body = crate::routes::bots::checkpoint::CheckpointResponse),
        (status = 401, description = "Device session or proof is invalid"),
        (status = 409, description = "The device lost the claim")))]
pub(crate) async fn checkpoint_load(
    State(state): State<AppState>,
    Path((id, turn_id)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Query<crate::routes::bots::checkpoint::CheckpointQuery>,
) -> Result<Json<crate::routes::bots::checkpoint::CheckpointResponse>, ApiError> {
    let claim = checkpoint_claim(
        &state,
        &id,
        &turn_id,
        &headers,
        "GET",
        query.host_id,
        query.attempt,
    )
    .await?;
    if query.monitor_baseline {
        return Ok(Json(crate::routes::bots::checkpoint::CheckpointResponse {
            checkpoint: None,
            monitor_result: crate::bot_host::recovery::previous_monitor_result_in(
                &state.db,
                &claim,
                now_millis(),
            )
            .await?,
        }));
    }
    Ok(Json(crate::routes::bots::checkpoint::CheckpointResponse {
        checkpoint: crate::bot_host::recovery::load_in(&state.db, &claim, now_millis()).await?,
        monitor_result: None,
    }))
}

#[utoipa::path(put, path = "/devices/{id}/bots/turns/{turn_id}/checkpoint", tag = "devices",
    description = "Saves a bound bot turn's checkpoint with its claim attempt and checkpoint revision.",
    params(("id" = String, Path), ("turn_id" = String, Path)),
    request_body = crate::routes::bots::checkpoint::SaveCheckpoint,
    responses((status = 200, description = "The checkpoint was saved"),
        (status = 401, description = "Device session or proof is invalid"),
        (status = 409, description = "The claim or checkpoint revision changed")))]
pub(crate) async fn checkpoint_save(
    State(state): State<AppState>,
    Path((id, turn_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<crate::routes::bots::checkpoint::SaveCheckpoint>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let claim = checkpoint_claim(
        &state,
        &id,
        &turn_id,
        &headers,
        "PUT",
        body.host_id,
        body.attempt,
    )
    .await?;
    crate::bot_host::recovery::save(&state.db, state.db_dialect, &claim, &body.checkpoint).await?;
    Ok(Json(serde_json::json!({"saved": true})))
}

async fn held_turn<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    device_id: &str,
    turn_id: &str,
) -> Result<bot_turn::Model, ApiError> {
    ledger::device_turn(db, turn_id, owner, device_id)
        .await?
        .ok_or_else(|| ledger::Refusal::NotFound.into_api(turn_id))
}

fn lease_lost(host_id: &str, turn_id: &str, action: &str) -> ApiError {
    ApiError::coded(
        StatusCode::CONFLICT,
        BOT_TURN_LEASE_LOST,
        format!("Host '{host_id}' no longer holds turn '{turn_id}'; it can't {action}"),
    )
}

async fn turn_state<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    turn_id: &str,
) -> Result<DeviceTurnState, ApiError> {
    let turn = ledger::find(db, turn_id, Some(owner))
        .await?
        .ok_or_else(|| ledger::Refusal::NotFound.into_api(turn_id))?;
    Ok(DeviceTurnState {
        turn: ledger::turn_json(&turn, Vec::new()),
    })
}

/// The live bots bound to the device, oldest first.
pub(crate) async fn device_bot_infos<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    device_id: &str,
) -> Result<Vec<DeviceBotInfo>, ApiError> {
    Ok(bot::Entity::find()
        .filter(bot::Column::OwnerId.eq(owner))
        .filter(bot::Column::DeviceId.eq(device_id))
        .filter(bot::Column::Location.eq(TurnHostKind::Device.stored()))
        .filter(bot::Column::DeletedAt.is_null())
        .order_by_asc(bot::Column::CreatedAt)
        .order_by_asc(bot::Column::Id)
        .all(db)
        .await?
        .into_iter()
        .map(|row| DeviceBotInfo {
            capabilities: crate::routes::bots::model::BotSettings::from_row(&row).capabilities,
            id: row.id,
            name: row.name,
            updated_at: row.updated_at.to_rfc3339(),
        })
        .collect())
}

const BOT_HOST_SEEN_EVERY_MS: i64 = 60_000;

/// Records that the device's bot host polled, at most once per [`BOT_HOST_SEEN_EVERY_MS`]; the
/// conditional update makes a frequent poll a no-op write-wise.
pub(crate) async fn record_bot_host_seen<C: ConnectionTrait>(
    db: &C,
    device_id: &str,
    now_ms: i64,
) -> Result<bool, ApiError> {
    let updated = db
        .execute_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            r#"UPDATE "ManagedDevice" SET "botHostSeenAt" = $1 WHERE id = $2 AND ("botHostSeenAt" IS NULL OR "botHostSeenAt" <= $3)"#,
            [
                now_ms.into(),
                device_id.into(),
                (now_ms - BOT_HOST_SEEN_EVERY_MS).into(),
            ],
        ))
        .await?;
    Ok(updated.rows_affected() > 0)
}

pub(crate) fn next_poll_ms(waiting: usize) -> u64 {
    if waiting > 0 {
        BUSY_POLL_MS
    } else {
        IDLE_POLL_MS
    }
    .min(MAX_POLL_MS)
}

/// Reports the device's bots online and lists their queued turns.
pub(crate) async fn poll_in<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    device_id: &str,
    host_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<DevicePollResponse, ApiError> {
    let bots = device_bot_infos(db, owner, device_id).await?;
    let ids: Vec<String> = bots.iter().map(|bot| bot.id.clone()).collect();
    report_device_in(db, owner, device_id, host_id, &ids, now).await?;
    let heads = ledger::queued_device_heads(db, owner, device_id, now, ledger::LIST_LIMIT).await?;
    Ok(DevicePollResponse {
        bots,
        next_poll_ms: next_poll_ms(heads.len()),
        turns: heads
            .iter()
            .map(|(turn, batched)| ledger::turn_json(turn, batched.clone()))
            .collect(),
    })
}

#[utoipa::path(
    post,
    path = "/devices/{id}/bots/poll",
    tag = "devices",
    description = "A device reports the bots bound to it as online and lists their queued turns.",
    params(("id" = String, Path, description = "Device ID")),
    request_body = Object,
    responses(
        (status = 200, description = "The device's bots, their queued turns (oldest first) and when to poll again, at most 60 seconds", body = Object),
        (status = 400, description = "host_id is empty, too long or has spaces"),
        (status = 401, description = "The device session or request proof is invalid or revoked"),
        (status = 403, description = "The session belongs to another device"),
        (status = 503, description = "Device enrollment is not enabled on this hub")
    )
)]
pub(crate) async fn poll(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<DevicePollRequest>,
) -> Result<Json<DevicePollResponse>, ApiError> {
    let owner = owner_of(&state, &id, &headers, "/bots/poll").await?;
    let host_id = host(body.host_id)?;
    record_bot_host_seen(&state.db, &id, chrono::Utc::now().timestamp_millis()).await?;
    Ok(Json(
        poll_in(&state.db, &owner, &id, &host_id, now_millis()).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/devices/{id}/bots/turns/{turn_id}/claim",
    tag = "devices",
    description = "A device claims the queued turn of one of its bots and receives the credentials, run spec and models to answer it.",
    params(
        ("id" = String, Path, description = "Device ID"),
        ("turn_id" = String, Path, description = "A turn from the poll")
    ),
    request_body = Object,
    responses(
        (status = 200, description = "The turn is the device's for 60 seconds; the grant holds the turn (the newest of its conversation, older ones in `batched`), tokens bound to it, the bot's run spec and the hosted models", body = Object),
        (status = 400, description = "host_id is invalid, or the bot's model is not a hosted model"),
        (status = 401, description = "The device session or request proof is invalid or revoked"),
        (status = 402, description = "The owner's usage allowance is not available"),
        (status = 404, description = "No such turn for a bot bound to this device, or its bot was deleted"),
        (status = 409, description = "BOT_TURN_NOT_QUEUED, BOT_TURN_BUSY, BOT_TURN_OWNER_BUSY, BOT_TURN_HOST_KIND, or BOT_TURN_ENCRYPTED: the conversation is end-to-end encrypted, which a device bot can't read yet")
    )
)]
pub(crate) async fn claim(
    State(state): State<AppState>,
    Path((id, turn_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<DeviceClaimRequest>,
) -> Result<Json<DeviceTurnGrant>, ApiError> {
    let owner = owner_of(
        &state,
        &id,
        &headers,
        &format!("/bots/turns/{turn_id}/claim"),
    )
    .await?;
    let host_id = host(body.host_id)?;
    let turn = held_turn(&state.db, &owner, &id, &turn_id).await?;
    if turn.host_kind != TurnHostKind::Device.stored() {
        return Err(ledger::Refusal::HostKind(
            TurnHostKind::parse(&turn.host_kind).unwrap_or(TurnHostKind::Server),
        )
        .into_api(&turn_id));
    }
    let claimed = ledger::claim_for_device(
        &state.db,
        state.db_dialect,
        &id,
        ClaimRequest {
            turn_id: turn_id.clone(),
            owner_id: Some(owner.clone()),
            host_kind: TurnHostKind::Device,
            host_id: host_id.clone(),
            now: now_millis(),
        },
    )
    .await?
    .map_err(|refusal| refusal.into_api(&turn_id))?;
    wake(&state, [owner.clone()]).await;
    match grant_for(&state, &claimed, &host_id).await {
        Ok(grant) => Ok(Json(grant)),
        Err(error) => {
            give_up(&state, &claimed.turn, &host_id, &error).await;
            Err(error)
        }
    }
}

async fn grant_for(
    state: &AppState,
    claimed: &ledger::Claimed,
    host_id: &str,
) -> Result<DeviceTurnGrant, ApiError> {
    let conversation = bots_grant::conversation_of(state, &claimed.turn.conversation_id).await?;
    if bots_grant::is_end_to_end(&conversation) {
        ledger::fail_held_in(
            &state.db,
            &claimed.turn.id,
            host_id,
            bots_grant::BOT_TURN_ENCRYPTED,
            now_millis(),
        )
        .await?;
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            bots_grant::BOT_TURN_ENCRYPTED,
            format!(
                "Conversation '{}' is end-to-end encrypted; a device bot can't read it",
                conversation.id
            ),
        ));
    }
    bots_grant::build(state, claimed, host_id, &conversation).await
}

/// The failure a claim that could not become a grant ends the turn with: a permanent one posts
/// its reason as the bot, a transient one goes back to the queue with a backoff, exactly as the
/// server worker treats the same errors.
pub(crate) fn claim_failure(error: &ApiError) -> worker::Failure {
    if error.public_code() == bots_grant::BOT_DEVICE_MODEL_NOT_HOSTED {
        return worker::Failure::new(
            worker::CODE_SETUP,
            bots_grant::MODEL_NOT_HOSTED_TEXT,
            false,
            true,
        );
    }
    worker::Failure::from_api(error)
}

async fn give_up(state: &AppState, turn: &bot_turn::Model, host_id: &str, error: &ApiError) {
    if error.public_code() == bots_grant::BOT_TURN_ENCRYPTED {
        return;
    }
    worker::finish(state, turn, host_id, Err(claim_failure(error)), false).await;
}

/// Fresh tokens for the ones the device reports with less than [`TOKEN_REFRESH_SECS`] left.
/// Whatever it reports, a refreshed token ends with the hold of the turn's allowance.
pub(crate) async fn refreshed<C: ConnectionTrait>(
    db: &C,
    turn: &bot_turn::Model,
    request: &DeviceHeartbeatRequest,
) -> Result<
    (
        Option<flow_like_types::device_bot::DeviceToken>,
        Option<flow_like_types::device_bot::DeviceToken>,
    ),
    ApiError,
> {
    let now = chrono::Utc::now().timestamp();
    let due = |expires_at: Option<i64>| expires_at.is_some_and(|at| at < now + TOKEN_REFRESH_SECS);
    let (bot_due, executor_due) = (
        due(request.bot_token_expires_at),
        due(request.executor_token_expires_at),
    );
    if !bot_due && !executor_due {
        return Ok((None, None));
    }
    let finished = bots_grant::has_finished_checkpoint(db, turn, &request.host_id).await?;
    let ttl = if finished {
        bots_grant::DELIVERY_TTL_SECS
    } else {
        bots_grant::refresh_ttl(bots_grant::hold_end(db, turn).await?, now)
    };
    let bot_token = if bot_due {
        Some(bots_grant::bot_token(db, &turn.owner_id, &turn.bot_id, &turn.id, ttl).await?)
    } else {
        None
    };
    // A completed checkpoint cannot acquire executor credentials through a heartbeat.
    if finished || !executor_due {
        return Ok((bot_token, None));
    }
    let workspace = live_workspace(db, &turn.owner_id, &turn.bot_id).await?;
    let conversation = crate::entity::conversation::Entity::find_by_id(&turn.conversation_id)
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found("The turn's conversation is gone"))?;
    let kind = flow_like_types::inbox::ConversationKind::parse(&conversation.kind)
        .unwrap_or(flow_like_types::inbox::ConversationKind::Group);
    let executor = bots_grant::executor_token(
        &turn.owner_id,
        &turn.bot_id,
        worker::room_of(kind, &turn.conversation_id),
        &turn.id,
        workspace.as_deref(),
        ttl,
    )?;
    Ok((bot_token, Some(executor)))
}

#[utoipa::path(
    post,
    path = "/devices/{id}/bots/turns/{turn_id}/heartbeat",
    tag = "devices",
    description = "A device keeps the lease of the turn it claimed and gets fresh tokens when theirs are about to expire.",
    params(
        ("id" = String, Path, description = "Device ID"),
        ("turn_id" = String, Path, description = "The claimed turn")
    ),
    request_body = Object,
    responses(
        (status = 200, description = "The lease runs 60 seconds from now; `bot_token` and `executor_token` are set when the ones in hand have under 5 minutes left", body = Object),
        (status = 401, description = "The device session or request proof is invalid or revoked"),
        (status = 404, description = "No such turn for a bot bound to this device"),
        (status = 409, description = "BOT_TURN_LEASE_LOST: the device no longer holds the turn; stop working on it")
    )
)]
pub(crate) async fn heartbeat(
    State(state): State<AppState>,
    Path((id, turn_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<DeviceHeartbeatRequest>,
) -> Result<Json<DeviceHeartbeatResponse>, ApiError> {
    let owner = owner_of(
        &state,
        &id,
        &headers,
        &format!("/bots/turns/{turn_id}/heartbeat"),
    )
    .await?;
    let host_id = host(body.host_id.clone())?;
    let held = held_turn(&state.db, &owner, &id, &turn_id).await?;
    bots_grant::require_hold(&state.db, &held, &host_id).await?;
    let turn =
        ledger::heartbeat_in(&state.db, &turn_id, Some(&owner), &host_id, now_millis()).await?;
    let (bot_token, executor_token) = refreshed(&state.db, &turn, &body).await?;
    Ok(Json(DeviceHeartbeatResponse {
        turn: ledger::turn_json(&turn, Vec::new()),
        bot_token,
        executor_token,
    }))
}

/// Ends the held turn `DONE` without a reply.
pub(crate) async fn silent_in<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    device_id: &str,
    turn_id: &str,
    host_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<DeviceTurnState, ApiError> {
    held_turn(db, owner, device_id, turn_id).await?;
    if !ledger::complete_silent_in(db, turn_id, host_id, now).await? {
        return Err(lease_lost(host_id, turn_id, "end silently"));
    }
    turn_state(db, owner, turn_id).await
}

#[utoipa::path(
    post,
    path = "/devices/{id}/bots/turns/{turn_id}/silent",
    tag = "devices",
    description = "A device ends a turn without a reply: the bot read along and chose to stay silent.",
    params(
        ("id" = String, Path, description = "Device ID"),
        ("turn_id" = String, Path, description = "The claimed turn")
    ),
    request_body = Object,
    responses(
        (status = 200, description = "The turn is done with error_code SILENT; nothing is posted", body = Object),
        (status = 401, description = "The device session or request proof is invalid or revoked"),
        (status = 404, description = "No such turn for a bot bound to this device"),
        (status = 409, description = "BOT_TURN_LEASE_LOST: the device no longer holds the turn")
    )
)]
pub(crate) async fn silent(
    State(state): State<AppState>,
    Path((id, turn_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<DeviceSilentRequest>,
) -> Result<Json<DeviceTurnState>, ApiError> {
    let owner = owner_of(
        &state,
        &id,
        &headers,
        &format!("/bots/turns/{turn_id}/silent"),
    )
    .await?;
    let host_id = host(body.host_id)?;
    let ended = silent_in(&state.db, &owner, &id, &turn_id, &host_id, now_millis()).await?;
    bots_grant::settle(&state, &owner, body.usage.as_ref(), true).await;
    wake(&state, [owner]).await;
    Ok(Json(ended))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn wait_in<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    device_id: &str,
    turn_id: &str,
    host_id: &str,
    participant_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<DeviceWaitResponse, ApiError> {
    use crate::bot_host::decision_wait::{ParkOutcome, park_wait_in};
    let turn = held_turn(db, owner, device_id, turn_id).await?;
    let status = match park_wait_in(db, &turn, host_id, participant_id, now).await? {
        ParkOutcome::Parked => "parked",
        ParkOutcome::AlreadyAnswered => "already_answered",
        ParkOutcome::Unavailable => "unavailable",
        ParkOutcome::Superseded => "superseded",
        ParkOutcome::LeaseLost => {
            return Err(lease_lost(host_id, turn_id, "wait for a participant"));
        }
    };
    Ok(DeviceWaitResponse {
        status: status.to_owned(),
    })
}

#[utoipa::path(
    post,
    path = "/devices/{id}/bots/turns/{turn_id}/wait",
    tag = "devices",
    description = "A device defers its bot's response until a named room participant speaks.",
    params(
        ("id" = String, Path, description = "Device ID"),
        ("turn_id" = String, Path, description = "The claimed turn")
    ),
    request_body = super::openapi_registry::wait_schema::DeviceWaitRequest,
    responses(
        (status = 200, description = "Status is parked, already_answered, unavailable or superseded", body = super::openapi_registry::wait_schema::DeviceWaitResponse),
        (status = 401, description = "The device session or request proof is invalid or revoked"),
        (status = 404, description = "No such turn for a bot bound to this device"),
        (status = 409, description = "BOT_TURN_LEASE_LOST: the device no longer holds the turn")
    )
)]
pub(crate) async fn wait(
    State(state): State<AppState>,
    Path((id, turn_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<DeviceWaitRequest>,
) -> Result<Json<DeviceWaitResponse>, ApiError> {
    let owner = owner_of(
        &state,
        &id,
        &headers,
        &format!("/bots/turns/{turn_id}/wait"),
    )
    .await?;
    let host_id = host(body.host_id)?;
    if body.participant_id.is_empty() || body.participant_id.len() > 128 {
        return Err(ApiError::bad_request(
            "participant_id must name a room member",
        ));
    }
    let turn = held_turn(&state.db, &owner, &id, &turn_id).await?;
    let ended = state
        .transaction(|txn| {
            let (owner, id, turn_id, host_id, participant_id, room) = (
                owner.clone(),
                id.clone(),
                turn_id.clone(),
                host_id.clone(),
                body.participant_id.clone(),
                turn.conversation_id.clone(),
            );
            Box::pin(async move {
                crate::routes::inbox::approvals::lock(txn, &room).await?;
                wait_in(
                    txn,
                    &owner,
                    &id,
                    &turn_id,
                    &host_id,
                    &participant_id,
                    now_millis(),
                )
                .await
            })
        })
        .await?;
    if ended.status == "parked" {
        bots_grant::settle(&state, &owner, body.usage.as_ref(), true).await;
        wake(&state, [owner]).await;
    }
    Ok(Json(ended))
}

fn valid_error_code(code: &str) -> bool {
    (1..=MAX_ERROR_CODE_CHARS).contains(&code.len())
        && code
            .chars()
            .all(|char| char.is_ascii_uppercase() || char.is_ascii_digit() || char == '_')
}

/// Gives the held turn up: queued again after a backoff, or `FAILED` with `error_code`. A retry
/// whose budget is spent leaves the turn held, running, for the device to post its failure reply.
pub(crate) async fn release_in<C: ConnectionTrait>(
    db: &C,
    owner: &str,
    device_id: &str,
    turn_id: &str,
    request: &DeviceReleaseRequest,
    now: DateTime<FixedOffset>,
) -> Result<(DeviceTurnState, Option<DateTime<FixedOffset>>), ApiError> {
    if !valid_error_code(&request.error_code) {
        return Err(ApiError::bad_request(format!(
            "error_code is 1 to {MAX_ERROR_CODE_CHARS} characters of A-Z, 0-9 and '_', not '{}'",
            request.error_code.chars().take(80).collect::<String>()
        )));
    }
    let turn = held_turn(db, owner, device_id, turn_id).await?;
    let retry_at = request
        .retry
        .then(|| now + Duration::seconds(worker::backoff_secs(turn.attempts)));
    let held = match retry_at {
        Some(retry_at) => {
            ledger::release_for_retry_in(
                db,
                turn_id,
                &request.host_id,
                retry_at,
                &request.error_code,
                now,
            )
            .await?
                != ledger::RetryRelease::Lost
        }
        None => {
            ledger::fail_held_in(db, turn_id, &request.host_id, &request.error_code, now).await?
        }
    };
    if !held {
        return Err(lease_lost(&request.host_id, turn_id, "release it"));
    }
    let state = turn_state(db, owner, turn_id).await?;
    let retry_at = (state.turn.status == flow_like_types::bot_turn::TurnStatus::Queued)
        .then_some(retry_at)
        .flatten();
    Ok((state, retry_at))
}

/// Whether a released turn is queued again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
pub(crate) struct DeviceReleaseResponse {
    /// `false`: the retries are spent or the turn failed, and the device posts the failure reply.
    pub requeued: bool,
}

#[utoipa::path(
    post,
    path = "/devices/{id}/bots/turns/{turn_id}/release",
    tag = "devices",
    description = "A device gives a turn up: it is queued again after a backoff (requeued true). With its retries spent the turn stays held and requeued is false: the device posts the failure reply. Without retry it fails with the given code and posts nothing.",
    params(
        ("id" = String, Path, description = "Device ID"),
        ("turn_id" = String, Path, description = "The claimed turn")
    ),
    request_body = Object,
    responses(
        (status = 200, description = "Whether the turn is queued again", body = DeviceReleaseResponse),
        (status = 400, description = "host_id or error_code is invalid"),
        (status = 401, description = "The device session or request proof is invalid or revoked"),
        (status = 404, description = "No such turn for a bot bound to this device"),
        (status = 409, description = "BOT_TURN_LEASE_LOST: the device no longer holds the turn")
    )
)]
pub(crate) async fn release(
    State(state): State<AppState>,
    Path((id, turn_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(mut body): Json<DeviceReleaseRequest>,
) -> Result<Json<DeviceReleaseResponse>, ApiError> {
    let owner = owner_of(
        &state,
        &id,
        &headers,
        &format!("/bots/turns/{turn_id}/release"),
    )
    .await?;
    body.host_id = host(std::mem::take(&mut body.host_id))?;
    let (ended, retry_at) = state
        .transaction(|txn| {
            let (owner, id, turn_id, body) =
                (owner.clone(), id.clone(), turn_id.clone(), body.clone());
            Box::pin(
                async move { release_in(txn, &owner, &id, &turn_id, &body, now_millis()).await },
            )
        })
        .await?;
    if let Some(retry_at) = retry_at {
        worker::kick_later(retry_at);
    }
    bots_grant::settle(&state, &owner, body.usage.as_ref(), false).await;
    wake(&state, [owner]).await;
    Ok(Json(DeviceReleaseResponse {
        requeued: ended.turn.status == flow_like_types::bot_turn::TurnStatus::Queued,
    }))
}

/// The approval card of a parked turn names that turn, a tool and a valid card id.
pub(crate) fn checked_card(
    turn: &bot_turn::Model,
    approval: &ApprovalRequest,
) -> Result<(), ApiError> {
    if approval.turn_id != turn.id
        || approval.tool.trim().is_empty()
        || !is_valid_message_id(&approval.id)
    {
        return Err(ApiError::bad_request(format!(
            "The approval card must name turn '{}', a tool and a valid card id",
            turn.id
        )));
    }
    Ok(())
}

#[utoipa::path(
    post,
    path = "/devices/{id}/bots/turns/{turn_id}/park",
    tag = "devices",
    description = "A device parks a turn that needs its owner's approval: the card is posted as the bot and the turn sleeps until answered.",
    params(
        ("id" = String, Path, description = "Device ID"),
        ("turn_id" = String, Path, description = "The claimed turn")
    ),
    request_body = Object,
    responses(
        (status = 200, description = "The turn is waiting; the owner is notified", body = Object),
        (status = 400, description = "The approval card is malformed or names another turn"),
        (status = 401, description = "The device session or request proof is invalid or revoked"),
        (status = 404, description = "No such turn for a bot bound to this device"),
        (status = 409, description = "BOT_TURN_LEASE_LOST: the device no longer holds the turn")
    )
)]
pub(crate) async fn park(
    State(state): State<AppState>,
    Path((id, turn_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<DeviceParkRequest>,
) -> Result<Json<DeviceTurnState>, ApiError> {
    let owner = owner_of(
        &state,
        &id,
        &headers,
        &format!("/bots/turns/{turn_id}/park"),
    )
    .await?;
    let host_id = host(body.host_id)?;
    let turn = held_turn(&state.db, &owner, &id, &turn_id).await?;
    checked_card(&turn, &body.approval)?;
    approvals::park_held(&state, &turn, &host_id, &body.approval).await?;
    bots_grant::settle(&state, &owner, body.usage.as_ref(), true).await;
    Ok(Json(turn_state(&state.db, &owner, &turn_id).await?))
}

/// The reply request of a device answer, checked: the answer's message id, or the failure
/// report's together with a known failure code.
pub(crate) fn checked_reply(
    turn: &bot_turn::Model,
    body: DeviceReplyRequest,
) -> Result<(SendEntryRequest, Option<&'static str>), ApiError> {
    let failure = match body.failure_code.as_deref() {
        None => None,
        Some(code) => Some(
            REPLY_FAILURE_CODES
                .into_iter()
                .find(|known| *known == code)
                .ok_or_else(|| {
                    ApiError::bad_request(format!(
                        "failure_code '{}' is none of {}",
                        code.chars().take(80).collect::<String>(),
                        REPLY_FAILURE_CODES.join(", ")
                    ))
                })?,
        ),
    };
    let expected = match failure {
        None => reply_message_id(&turn.bot_id, &turn.trigger_message_id),
        Some(_) => failure_message_id(&turn.bot_id, &turn.trigger_message_id),
    };
    if body.message_id != expected {
        return Err(ApiError::bad_request(format!(
            "message_id for turn '{}' must be '{expected}'",
            turn.id
        )));
    }
    let request = SendEntryRequest {
        message_id: body.message_id,
        kind: InboxEntryKind::Message,
        envelope: Some(body.envelope),
        envelopes: None,
        turn: None,
    };
    validate_send(request.clone())?;
    Ok((request, failure))
}

#[utoipa::path(
    post,
    path = "/devices/{id}/bots/turns/{turn_id}/reply",
    tag = "devices",
    description = "A device posts the bot's answer (or its failure report) and completes the turn in one step. Sending the same answer again returns the first copy.",
    params(
        ("id" = String, Path, description = "Device ID"),
        ("turn_id" = String, Path, description = "The claimed turn")
    ),
    request_body = Object,
    responses(
        (status = 200, description = "The turn is done (or failed, for a failure report); `posted` is false on a repeat", body = Object),
        (status = 400, description = "message_id is not this turn's answer id, failure_code is unknown, or the envelope is malformed"),
        (status = 401, description = "The device session or request proof is invalid or revoked"),
        (status = 404, description = "No such turn for a bot bound to this device"),
        (status = 409, description = "BOT_TURN_NOT_CLAIMED: the device doesn't hold the turn with a live lease")
    )
)]
pub(crate) async fn reply(
    State(state): State<AppState>,
    Path((id, turn_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<DeviceReplyRequest>,
) -> Result<Json<DeviceReplyResponse>, ApiError> {
    let owner = owner_of(
        &state,
        &id,
        &headers,
        &format!("/bots/turns/{turn_id}/reply"),
    )
    .await?;
    let host_id = host(body.host_id.clone())?;
    let turn = held_turn(&state.db, &owner, &id, &turn_id).await?;
    let usage = body.usage.clone();
    let (request, failure) = checked_reply(&turn, body)?;
    let message_id = request.message_id.clone();
    let sent = worker::commit_reply(
        &state.db,
        state.db_dialect,
        &turn,
        &host_id,
        request,
        failure,
    )
    .await?;
    let posted = !sent.wake.is_empty();
    wake(&state, sent.wake).await;
    crate::bot_host::dispatch(&state, &sent.bots).await;
    if posted {
        crate::bot_notify::after_reply(
            &state,
            &crate::bot_notify::Reply {
                turn_id: &turn.id,
                bot_id: &turn.bot_id,
                message_id: &sent.entry.message_id,
                text: &crate::bot_notify::text_of(Some(&sent.entry.envelope)),
                resumed: false,
            },
        )
        .await;
    }
    bots_grant::settle(&state, &owner, usage.as_ref(), failure.is_none()).await;
    let ended = turn_state(&state.db, &owner, &turn_id).await?;
    Ok(Json(DeviceReplyResponse {
        turn: ended.turn,
        message_id,
        posted,
    }))
}

#[utoipa::path(
    post,
    path = "/devices/{id}/bots/hosts/forget",
    tag = "devices",
    description = "A device that shuts down withdraws its presence: its bots stop reading online.",
    params(("id" = String, Path, description = "Device ID")),
    request_body = Object,
    responses(
        (status = 200, description = "How many presence reports were removed", body = Object),
        (status = 400, description = "host_id is invalid"),
        (status = 401, description = "The device session or request proof is invalid or revoked")
    )
)]
pub(crate) async fn forget(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<DeviceForgetRequest>,
) -> Result<Json<DeviceForgetResponse>, ApiError> {
    let owner = owner_of(&state, &id, &headers, "/bots/hosts/forget").await?;
    let host_id = host(body.host_id)?;
    Ok(Json(DeviceForgetResponse {
        removed: forget_device_in(&state.db, &owner, &id, &host_id).await?,
    }))
}

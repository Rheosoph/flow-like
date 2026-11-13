//! Who is active in a conversation right now: the bot turns still in flight, and realtime access
//! to the conversation's peer room, where people share typing over y-webrtc.

use super::{active_members, active_membership, model::ReaderQuery};
use crate::{
    backend_jwt::{self, TokenType, issuer, make_time_claims},
    entity::{bot_turn, user},
    error::ApiError,
    middleware::jwt::AppUser,
    realtime_ice::RealtimeIceServer,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::header,
    response::IntoResponse,
};
use chrono::NaiveDate;
use flow_like_types::{
    anyhow,
    base64::{Engine, engine::general_purpose::STANDARD},
    bot_turn::{STORED_QUEUED, STORED_RUNNING, STORED_WAITING},
    create_id,
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const MAX_ACTIVE_TURNS: u64 = 50;
const ROOM_KEY_CONTEXT: &str = "flow-like.inbox-room-key/v1";
const MEMBER_DIGEST_HEX: usize = 16;
const SCOPE: &str = "realtime.read";

/// A bot turn that has not finished yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ConversationTurn {
    #[schema(example = "bt_0c3f7a9e1d5b48a2f6e0b9d1c4a7e382")]
    pub id: String,
    pub bot_id: String,
    /// `queued`, `running` or `waiting`.
    #[schema(example = "running")]
    pub status: String,
    /// The message the bot is answering.
    pub trigger_message_id: String,
    /// What the bot is doing right now, one short line it set itself; only while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "Lesson 2 of 6")]
    pub status_text: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ConversationTurnList {
    /// Oldest first, at most 50.
    pub turns: Vec<ConversationTurn>,
}

impl From<bot_turn::Model> for ConversationTurn {
    fn from(turn: bot_turn::Model) -> Self {
        Self {
            id: turn.id,
            bot_id: turn.bot_id,
            status: turn.status.to_ascii_lowercase(),
            trigger_message_id: turn.trigger_message_id,
            status_text: turn.status_text,
            created_at: turn.created_at.to_rfc3339(),
            updated_at: turn.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConversationRealtimeClaims {
    pub sub: String,
    pub name: Option<String>,
    pub conversation_id: String,
    pub scope: String,
    #[serde(rename = "typ")]
    pub token_type: TokenType,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ConversationRealtimeParams {
    /// JWT authorizing the member for this conversation's peer room.
    pub jwt: String,
    /// Base64 256-bit room key.
    pub encryption_key: String,
    /// `{UTC date}.{16 hex}`; changes when the members change.
    pub key_id: String,
    /// Provider-issued browser ICE configuration. Omitted when no provider is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ice_servers: Option<Vec<RealtimeIceServer>>,
    /// Expiry of the ICE credentials as a Unix timestamp in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ice_expires_at: Option<i64>,
}

pub(crate) fn room_member(user: &AppUser, query: &ReaderQuery) -> Result<String, ApiError> {
    if !matches!(user, AppUser::OpenID(_) | AppUser::PAT(_)) {
        return Err(ApiError::forbidden(
            "Only people get live room access, not bots, API keys or running flows",
        ));
    }
    let sub = user.sub()?;
    if super::trimmed(query.as_bot.as_deref()).is_some() {
        return Err(ApiError::bad_request(
            "Bots do not join a conversation's live room",
        ));
    }
    Ok(sub)
}

pub(crate) async fn active_turns_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    sub: &str,
) -> Result<Vec<ConversationTurn>, ApiError> {
    active_membership(db, conversation_id, sub).await?;
    let turns = bot_turn::Entity::find()
        .filter(bot_turn::Column::Status.is_in([STORED_QUEUED, STORED_RUNNING, STORED_WAITING]))
        .filter(bot_turn::Column::ConversationId.eq(conversation_id))
        .order_by_asc(bot_turn::Column::CreatedAt)
        .order_by_asc(bot_turn::Column::Id)
        .limit(MAX_ACTIVE_TURNS)
        .all(db)
        .await?;
    Ok(turns.into_iter().map(ConversationTurn::from).collect())
}

fn length_prefixed(hasher: &mut blake3::Hasher, part: &str) {
    hasher.update(&(part.len() as u64).to_le_bytes());
    hasher.update(part.as_bytes());
}

fn member_digest(mut member_ids: Vec<String>) -> String {
    member_ids.sort();
    let mut hasher = blake3::Hasher::new();
    for id in &member_ids {
        length_prefixed(&mut hasher, id);
    }
    hasher.finalize().to_hex()[..MEMBER_DIGEST_HEX].to_string()
}

pub(crate) fn derive_room_key(master: &[u8; 32], conversation_id: &str, key_id: &str) -> String {
    let mut material = blake3::Hasher::new();
    material.update(master);
    length_prefixed(&mut material, conversation_id);
    length_prefixed(&mut material, key_id);
    let key = blake3::derive_key(ROOM_KEY_CONTEXT, material.finalize().as_bytes());
    STANDARD.encode(key)
}

pub(crate) struct RoomKey {
    pub encryption_key: String,
    pub key_id: String,
}

pub(crate) async fn room_key_in<C: ConnectionTrait>(
    db: &C,
    master: &[u8; 32],
    conversation_id: &str,
    sub: &str,
    today: NaiveDate,
) -> Result<RoomKey, ApiError> {
    let conversation = super::member_conversation(db, conversation_id, sub).await?;
    super::blocks::ensure_direct_allowed(db, &conversation, sub).await?;
    let members = active_members(db, &[conversation_id.to_string()]).await?;
    let ids = members
        .into_iter()
        .map(|member| member.principal_id)
        .collect();
    let key_id = format!("{}.{}", today.format("%Y-%m-%d"), member_digest(ids));
    Ok(RoomKey {
        encryption_key: derive_room_key(master, conversation_id, &key_id),
        key_id,
    })
}

pub(crate) fn claims_for(
    sub: String,
    name: Option<String>,
    conversation_id: &str,
    jti: String,
) -> ConversationRealtimeClaims {
    let time = make_time_claims(TokenType::Realtime, None);
    ConversationRealtimeClaims {
        sub,
        name,
        conversation_id: conversation_id.to_string(),
        scope: SCOPE.to_string(),
        token_type: TokenType::Realtime,
        iss: issuer().to_string(),
        aud: TokenType::Realtime.audience().to_string(),
        iat: time.iat,
        nbf: time.nbf,
        exp: time.exp,
        jti,
    }
}

#[utoipa::path(
    get,
    path = "/inbox/conversations/{conversation_id}/turns",
    tag = "Inbox",
    params(("conversation_id" = String, Path, description = "The conversation")),
    responses(
        (status = 200, description = "The bot turns in this conversation that are queued, running or waiting, oldest first. Use it to show which bots are busy.", body = ConversationTurnList),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You are not in this conversation")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "GET /inbox/conversations/{conversation_id}/turns",
    skip(state, user)
)]
pub async fn list_active_turns(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
) -> Result<Json<ConversationTurnList>, ApiError> {
    let sub = user.sub()?;
    let turns = active_turns_in(&state.db, &conversation_id, &sub).await?;
    Ok(Json(ConversationTurnList { turns }))
}

#[utoipa::path(
    post,
    path = "/inbox/conversations/{conversation_id}/realtime",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        ReaderQuery
    ),
    responses(
        (status = 200, description = "Access to this conversation's live room, where members see each other typing: a short-lived token, the room key and connection servers. Only people in the conversation get it.", body = ConversationRealtimeParams),
        (status = 400, description = "Bots cannot join the live room"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people get live room access, not bots, API keys or running flows"),
        (status = 404, description = "You are not in this conversation"),
        (status = 502, description = "The connection server provider is unavailable")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /inbox/conversations/{conversation_id}/realtime",
    skip(state, user)
)]
pub async fn conversation_realtime(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Query(query): Query<ReaderQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let sub = room_member(&user, &query)?;
    let key = room_key_in(
        &state.db,
        &state.encryption_key,
        &conversation_id,
        &sub,
        chrono::Utc::now().date_naive(),
    )
    .await?;
    let name = user::Entity::find_by_id(&sub)
        .one(&state.db)
        .await?
        .and_then(|model| {
            crate::routes::user::identity::public_display_name(
                model.name.as_deref(),
                model.preferred_username.as_deref(),
                model.email.as_deref(),
            )
        });

    let jti = create_id();
    let issued = state.realtime_ice.issue(&jti).await.map_err(|error| {
        tracing::error!(error = %error, "Failed to issue realtime ICE credentials");
        ApiError::bad_gateway("Realtime ICE credentials are temporarily unavailable")
    })?;
    let (ice_servers, ice_expires_at) = match issued {
        Some(issued) => (Some(issued.ice_servers), Some(issued.expires_at)),
        None => (None, None),
    };

    let claims = claims_for(sub, name, &conversation_id, jti);
    let jwt = backend_jwt::sign(&claims)
        .map_err(|e| ApiError::internal_error(anyhow!("Realtime not configured: {}", e)))?;

    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(ConversationRealtimeParams {
            jwt,
            encryption_key: key.encryption_key,
            key_id: key.key_id,
            ice_servers,
            ice_expires_at,
        }),
    ))
}

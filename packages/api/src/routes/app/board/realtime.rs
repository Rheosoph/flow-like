use crate::{
    backend_jwt::{self, TokenType, get_jwks, issuer, make_time_claims},
    ensure_permission,
    entity::{board_sync, prelude::*},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    realtime_ice::RealtimeIceServer,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::header,
};
use flow_like_types::base64::Engine;
use flow_like_types::base64::engine::general_purpose::STANDARD;
use flow_like_types::{anyhow, create_id};
use sea_orm::TransactionTrait;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ============================================================================
// Realtime collaboration auth (JWT + room key) using unified backend JWT
// ============================================================================

const SCOPE: &str = "realtime.read";

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RealtimeClaims {
    pub sub: String,
    pub name: Option<String>,
    pub app_id: String,
    pub board_id: String,
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

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct RealtimeParams {
    /// JWT authorizing the user for this (app_id, board_id) in y-webrtc
    jwt: String,
    /// Base64 256-bit room key (rotated daily)
    encryption_key: String,
    /// Key identifier (ISO date, e.g. "2025-10-23")
    key_id: String,
    /// Provider-issued browser ICE configuration. Omitted when no provider is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    ice_servers: Option<Vec<RealtimeIceServer>>,
    /// Expiry of the ICE credentials as a Unix timestamp in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    ice_expires_at: Option<i64>,
}

fn generate_encryption_key() -> String {
    use flow_like_types::rand::{TryRngCore, rngs::OsRng};
    let mut key = [0u8; 32];
    let mut rng = OsRng;
    rng.try_fill_bytes(&mut key)
        .expect("Failed to generate random key");
    STANDARD.encode(key)
}

// ============================================================================
// JWKS (no auth) — mount at GET /apps/{app_id}/board/{board_id}/realtime
// ============================================================================
#[utoipa::path(
    get,
    path = "/apps/{app_id}/board/{board_id}/realtime",
    tag = "boards",
    description = "Get JWKS for realtime collaboration.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    responses(
        (status = 200, description = "JWKS", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/board/{board_id}/realtime",
    skip(_state, user)
)]
pub async fn jwks(
    State(_state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((_app_id, _board_id)): Path<(String, String)>,
) -> Result<Json<backend_jwt::Jwks>, ApiError> {
    user.sub()?;

    // Get JWKS from unified backend module
    let jwks = get_jwks()
        .map_err(|e| ApiError::internal_error(anyhow!("Realtime not configured: {}", e)))?;

    Ok(Json(jwks))
}

// ============================================================================
// Access token + room key
// ============================================================================
#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/realtime",
    tag = "boards",
    description = "Get realtime access token, room key, and short-lived ICE configuration.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    responses(
        (status = 200, description = "Realtime access", body = RealtimeParams),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 502, description = "Configured ICE provider unavailable")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/realtime",
    skip(state, user)
)]
pub async fn access(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    // Realtime collaboration mints a room encryption key + session JWT; a
    // machine principal acting through a connection has no use for it, and it
    // would be another arbitrary-board entry point.
    super::ensure_connected_app_board_invoke_denied(&user)?;
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
    let sub = permission.sub()?;

    let user_model = User::find_by_id(&sub)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let jti = create_id();
    let issued_ice = state.realtime_ice.issue(&jti).await.map_err(|error| {
        tracing::error!(error = %error, "Failed to issue realtime ICE credentials");
        ApiError::bad_gateway("Realtime ICE credentials are temporarily unavailable")
    })?;
    let (ice_servers, ice_expires_at) = match issued_ice {
        Some(issued) => (Some(issued.ice_servers), Some(issued.expires_at)),
        None => (None, None),
    };

    let (encryption_key, key_id) = get_or_rotate_room_key(&state, &app_id, &board_id).await?;

    let time = make_time_claims(TokenType::Realtime, None);

    let claims = RealtimeClaims {
        sub: sub.clone(),
        name: user_model.name,
        app_id: app_id.clone(),
        board_id: board_id.clone(),
        scope: SCOPE.to_string(),
        token_type: TokenType::Realtime,
        iss: issuer().to_string(),
        aud: TokenType::Realtime.audience().to_string(),
        iat: time.iat,
        nbf: time.nbf,
        exp: time.exp,
        jti,
    };

    let jwt = backend_jwt::sign(&claims)
        .map_err(|e| ApiError::internal_error(anyhow!("Realtime not configured: {}", e)))?;

    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(RealtimeParams {
            jwt,
            encryption_key,
            key_id,
            ice_servers,
            ice_expires_at,
        }),
    ))
}

// ----------------------------------------------------------------------------
// Helper: get or rotate the per-board room key (daily rotation), returns (key, key_id)
// ----------------------------------------------------------------------------
async fn get_or_rotate_room_key(
    state: &AppState,
    app_id: &str,
    board_id: &str,
) -> Result<(String, String), ApiError> {
    let now = chrono::Utc::now().naive_utc();
    let today = now.date(); // NaiveDate
    let key_id = today.format("%Y-%m-%d").to_string();

    let txn = state.db.begin().await?;

    // Lock row if exists
    let existing = BoardSync::find()
        .filter(board_sync::Column::AppId.eq(app_id))
        .filter(board_sync::Column::BoardId.eq(board_id))
        .one(&txn)
        .await?;

    let encryption_key = match existing {
        Some(sync) => {
            // Rotate if last_synced_at is from a previous day
            if sync.last_synced_at.date() < today {
                let new_key = generate_encryption_key();
                let mut active_sync: board_sync::ActiveModel = sync.into();
                active_sync.sync_encryption_key = Set(new_key.clone());
                active_sync.last_synced_at = Set(now);
                active_sync.updated_at = Set(now);
                active_sync.update(&txn).await?;
                new_key
            } else {
                sync.sync_encryption_key
            }
        }
        None => {
            let new_key = generate_encryption_key();
            let new_sync = board_sync::ActiveModel {
                id: Set(create_id()),
                app_id: Set(app_id.to_string()),
                board_id: Set(board_id.to_string()),
                last_synced_at: Set(now),
                sync_encryption_key: Set(new_key.clone()),
                created_at: Set(now),
                updated_at: Set(now),
            };
            new_sync.insert(&txn).await?;
            new_key
        }
    };

    txn.commit().await?;

    Ok((encryption_key, key_id))
}

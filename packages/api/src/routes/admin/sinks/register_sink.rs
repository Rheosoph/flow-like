//! Register a new sink service token
//!
//! POST /admin/sinks/register
//!
//! Creates a new long-lived JWT for a sink service (cron, discord, telegram, etc.)
//! The token is scoped to a specific sink type and tracked in the database for revocation.

use crate::{
    audit, entity::sink_token, error::ApiError, middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission, state::AppState,
};
use axum::{Extension, Json, extract::State};
use flow_like_types::{anyhow, create_id};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use sea_orm::{ActiveModelTrait, Set};
use serde::{Deserialize, Serialize};

/// Allowed sink types that can be registered
const ALLOWED_SINK_TYPES: &[&str] = &[
    "cron", "discord", "telegram", "github", "rss", "mqtt", "email", "http",
];

#[derive(Debug, Deserialize)]
pub struct RegisterSinkRequest {
    /// The sink type to register (e.g., "cron", "discord", "telegram")
    pub sink_type: String,
    /// Optional human-readable name for the token
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisterSinkResponse {
    /// The generated JWT token - store this securely!
    pub token: String,
    /// The JWT ID (jti) - use this to revoke the token later
    pub jti: String,
    /// The sink type this token is authorized for
    pub sink_type: String,
}

/// JWT claims for sink trigger service tokens
#[derive(Debug, Serialize)]
struct SinkTriggerClaims {
    /// Subject - always "sink-trigger"
    sub: &'static str,
    /// Issuer - always "flow-like"
    iss: &'static str,
    /// JWT ID - unique identifier for revocation
    jti: String,
    /// Which sink types this token can trigger
    sink_types: Vec<String>,
    /// Issued at timestamp
    iat: u64,
}

/// POST /admin/sinks/register
///
/// Register a new sink service and get a JWT token for it.
///
/// # Authentication
/// Requires Admin global permission.
///
/// # Request
/// ```json
/// {
///   "sink_type": "cron",
///   "name": "Production Cron Service"
/// }
/// ```
///
/// # Response
/// ```json
/// {
///   "token": "eyJ...",
///   "jti": "sink_abc123",
///   "sink_type": "cron"
/// }
/// ```
#[tracing::instrument(name = "POST /admin/sinks/register", skip(state, user))]
pub async fn register_sink(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<RegisterSinkRequest>,
) -> Result<Json<RegisterSinkResponse>, ApiError> {
    // Require admin permission
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    // Validate sink type
    let sink_type = request.sink_type.to_lowercase();
    if !ALLOWED_SINK_TYPES.contains(&sink_type.as_str()) {
        return Err(ApiError::bad_request(format!(
            "Invalid sink type '{}'. Allowed types: {}",
            sink_type,
            ALLOWED_SINK_TYPES.join(", ")
        )));
    }

    // Get the signing secret
    let secret = std::env::var("SINK_SECRET")
        .map_err(|_| ApiError::internal_error(anyhow!("SINK_SECRET not configured")))?;

    // Generate unique JTI
    let jti = format!("sink_{}", create_id());

    // Generate JWT claims
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| ApiError::internal_error(anyhow!("Time error: {}", e)))?
        .as_secs();

    let claims = SinkTriggerClaims {
        sub: "sink-trigger",
        iss: "flow-like",
        jti: jti.clone(),
        sink_types: vec![sink_type.clone()],
        iat: now,
    };

    // Sign the JWT (no expiration - long-lived, revocation via DB)
    let key = EncodingKey::from_secret(secret.as_bytes());
    let token = jsonwebtoken::encode(&Header::new(Algorithm::HS256), &claims, &key)
        .map_err(|e| ApiError::internal_error(anyhow!("JWT encoding error: {}", e)))?;

    // Store in database for revocation tracking
    let now_dt = chrono::Utc::now().naive_utc();
    let sink_token = sink_token::ActiveModel {
        id: Set(jti.clone()),
        sink_type: Set(sink_type.clone()),
        name: Set(request.name),
        revoked: Set(false),
        revoked_at: Set(None),
        revoked_by: Set(None),
        created_at: Set(now_dt),
        updated_at: Set(now_dt),
    };

    sink_token
        .insert(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to store sink token: {}", e)))?;

    tracing::info!(jti = %jti, sink_type = %sink_type, "Registered new sink token");

    audit!(
        state,
        user,
        "admin.sink.register",
        "sink_token",
        jti,
        format!("Sink token registered for type: {}", sink_type)
    );
    Ok(Json(RegisterSinkResponse {
        token,
        jti,
        sink_type,
    }))
}

//! Revoke a sink service token
//!
//! DELETE /admin/sinks/{jti}
//!
//! Revokes a previously issued sink token. The token will no longer be
//! accepted for triggering events.

use crate::{
    audit, entity::sink_token, error::ApiError, middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::anyhow;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct RevokeSinkResponse {
    pub success: bool,
    pub jti: String,
    pub message: String,
}

/// DELETE /admin/sinks/{jti}
///
/// Revoke a sink service token by its JTI (JWT ID).
///
/// # Authentication
/// Requires Admin global permission.
///
/// # Response
/// ```json
/// {
///   "success": true,
///   "jti": "sink_abc123",
///   "message": "Token revoked successfully"
/// }
/// ```
#[tracing::instrument(name = "DELETE /admin/sinks/{jti}", skip(state, user))]
pub async fn revoke_sink(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(jti): Path<String>,
) -> Result<Json<RevokeSinkResponse>, ApiError> {
    // Require admin permission
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    // Find the token
    let token = sink_token::Entity::find_by_id(&jti)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Database error: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Sink token '{}' not found", jti)))?;

    // Check if already revoked
    if token.revoked {
        return Ok(Json(RevokeSinkResponse {
            success: true,
            jti,
            message: "Token was already revoked".to_string(),
        }));
    }

    // Get the identifier of who is revoking
    let revoked_by = user.sub().ok();

    // Update to revoked
    let now = chrono::Utc::now().naive_utc();
    let mut active_model: sink_token::ActiveModel = token.into();
    active_model.revoked = Set(true);
    active_model.revoked_at = Set(Some(now));
    active_model.revoked_by = Set(revoked_by.clone());
    active_model.updated_at = Set(now);

    active_model
        .update(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to revoke token: {}", e)))?;

    tracing::info!(jti = %jti, revoked_by = ?revoked_by, "Revoked sink token");

    audit!(
        state,
        user,
        "admin.sink.revoke",
        "sink_token",
        jti,
        format!("Sink token revoked: {}", jti)
    );
    Ok(Json(RevokeSinkResponse {
        success: true,
        jti,
        message: "Token revoked successfully".to_string(),
    }))
}

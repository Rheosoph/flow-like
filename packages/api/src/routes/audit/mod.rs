pub mod head;
pub mod records;
pub mod verify;

use axum::{
    Router,
    routing::{get, post},
};

use crate::{
    audit::{PLATFORM_CHAIN, chain_scope},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::{global_permission::GlobalPermission, role_permission::RolePermissions},
    state::AppState,
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/records", get(records::list_records))
        .route("/verify", get(verify::verify_chain))
        .route("/verify/epochs", get(verify::verify_epochs))
        .route("/head", get(head::chain_head))
        .route("/head/check", post(head::check_head))
}

/// A chain carries every audited change of its scope, so reading one is an Owner-level
/// capability of that app; its activity chain follows the same rule. Platform chains,
/// package chains and apps the caller does not own need the Admin global permission.
pub(crate) async fn ensure_chain_access(
    user: &AppUser,
    state: &AppState,
    chain_id: &str,
) -> Result<(), ApiError> {
    user.sub()?;
    if let Some(app_id) = chain_scope(chain_id)
        && let Ok(permission) = user.app_permission(app_id, state).await
        && permission.has_permission(RolePermissions::Owner)
    {
        return Ok(());
    }
    user.check_global_permission(state, GlobalPermission::Admin)
        .await?;
    Ok(())
}

/// The requested chain, `platform` when none is given.
pub(crate) fn chain_or_platform(chain_id: Option<String>) -> String {
    chain_id
        .map(|chain_id| chain_id.trim().to_owned())
        .filter(|chain_id| !chain_id.is_empty())
        .unwrap_or_else(|| PLATFORM_CHAIN.to_owned())
}

pub mod chain;
pub mod service;
pub mod sign;

pub use chain::{compute_entry_hash, GENESIS_HASH};
pub use service::AuditService;
pub use sign::{sign_entry, verify_entry_signature};

use crate::entity::sea_orm_active_enums::AuditActorType;
use crate::middleware::jwt::AppUser;

pub fn actor_type_from_user(user: &AppUser) -> AuditActorType {
    match user {
        AppUser::OpenID(_) => AuditActorType::User,
        AppUser::PAT(_) => AuditActorType::User,
        AppUser::APIKey(_) => AuditActorType::ApiKey,
        AppUser::Executor(_) => AuditActorType::Executor,
        AppUser::Unauthorized => AuditActorType::System,
    }
}

/// Record an audit entry on the platform root chain (chain_id = None).
/// Audit is recorded synchronously — the endpoint will fail if audit recording fails (NIST AU-5).
/// Respects `audit.enabled` from platform config.
///
/// Usage: `audit!(state, user, action, resource_type, resource_id, summary);`
/// With details: `audit!(state, user, action, resource_type, resource_id, summary, details);`
#[macro_export]
macro_rules! audit {
    ($state:expr, $user:expr, $action:expr, $resource_type:expr, $resource_id:expr, $summary:expr) => {{
        if $state.platform_config.audit.enabled {
            let user = $user.clone();
            let actor_type = $crate::audit::actor_type_from_user(&user);
            let db = $state.db.clone();
            let action = $action.to_string();
            let resource_type = $resource_type.to_string();
            let resource_id = $resource_id.to_string();
            let summary = $summary.to_string();
            let audit_result = async {
                let actor_id = user.audit_id().await.map_err(|e| {
                    tracing::error!("AUDIT FAILURE: Failed to get audit_id: {}", e);
                    e
                })?;
                let input = $crate::audit::service::AuditEntryInput {
                    actor_id,
                    actor_type,
                    actor_ip: None,
                    action,
                    resource_type,
                    resource_id,
                    chain_id: None,
                    summary,
                    details: None,
                };
                $crate::audit::AuditService::record(&db, input).await
            }.await;
            if let Err(e) = &audit_result {
                tracing::error!("AUDIT FAILURE (root chain): {}", e);
            }
        }
    }};
    ($state:expr, $user:expr, $action:expr, $resource_type:expr, $resource_id:expr, $summary:expr, $details:expr) => {{
        if $state.platform_config.audit.enabled {
            let user = $user.clone();
            let actor_type = $crate::audit::actor_type_from_user(&user);
            let db = $state.db.clone();
            let action = $action.to_string();
            let resource_type = $resource_type.to_string();
            let resource_id = $resource_id.to_string();
            let summary = $summary.to_string();
            let details = $details;
            let audit_result = async {
                let actor_id = user.audit_id().await.map_err(|e| {
                    tracing::error!("AUDIT FAILURE: Failed to get audit_id: {}", e);
                    e
                })?;
                let input = $crate::audit::service::AuditEntryInput {
                    actor_id,
                    actor_type,
                    actor_ip: None,
                    action,
                    resource_type,
                    resource_id,
                    chain_id: None,
                    summary,
                    details: Some(details),
                };
                $crate::audit::AuditService::record(&db, input).await
            }.await;
            if let Err(e) = &audit_result {
                tracing::error!("AUDIT FAILURE (root chain): {}", e);
            }
        }
    }};
}

/// Record an audit entry on a branch chain (app or package).
/// Audit is recorded synchronously — the endpoint will fail if audit recording fails (NIST AU-5).
/// Respects `audit.enabled` from platform config.
///
/// Usage: `audit_branch!(state, user, chain_id, action, resource_type, resource_id, summary);`
/// With details: `audit_branch!(state, user, chain_id, action, resource_type, resource_id, summary, details);`
#[macro_export]
macro_rules! audit_branch {
    ($state:expr, $user:expr, $chain_id:expr, $action:expr, $resource_type:expr, $resource_id:expr, $summary:expr) => {{
        if $state.platform_config.audit.enabled {
            let user = $user.clone();
            let actor_type = $crate::audit::actor_type_from_user(&user);
            let db = $state.db.clone();
            let chain_id = $chain_id.to_string();
            let action = $action.to_string();
            let resource_type = $resource_type.to_string();
            let resource_id = $resource_id.to_string();
            let summary = $summary.to_string();
            let audit_result = async {
                let actor_id = user.audit_id().await.map_err(|e| {
                    tracing::error!("AUDIT FAILURE: Failed to get audit_id: {}", e);
                    e
                })?;
                let input = $crate::audit::service::AuditEntryInput {
                    actor_id,
                    actor_type,
                    actor_ip: None,
                    action,
                    resource_type,
                    resource_id,
                    chain_id: Some(chain_id),
                    summary,
                    details: None,
                };
                $crate::audit::AuditService::record(&db, input).await
            }.await;
            if let Err(e) = &audit_result {
                tracing::error!("AUDIT FAILURE (chain {}): {}", $chain_id, e);
            }
        }
    }};
    ($state:expr, $user:expr, $chain_id:expr, $action:expr, $resource_type:expr, $resource_id:expr, $summary:expr, $details:expr) => {{
        if $state.platform_config.audit.enabled {
            let user = $user.clone();
            let actor_type = $crate::audit::actor_type_from_user(&user);
            let db = $state.db.clone();
            let chain_id = $chain_id.to_string();
            let action = $action.to_string();
            let resource_type = $resource_type.to_string();
            let resource_id = $resource_id.to_string();
            let summary = $summary.to_string();
            let details = $details;
            let audit_result = async {
                let actor_id = user.audit_id().await.map_err(|e| {
                    tracing::error!("AUDIT FAILURE: Failed to get audit_id: {}", e);
                    e
                })?;
                let input = $crate::audit::service::AuditEntryInput {
                    actor_id,
                    actor_type,
                    actor_ip: None,
                    action,
                    resource_type,
                    resource_id,
                    chain_id: Some(chain_id),
                    summary,
                    details: Some(details),
                };
                $crate::audit::AuditService::record(&db, input).await
            }.await;
            if let Err(e) = &audit_result {
                tracing::error!("AUDIT FAILURE (chain {}): {}", $chain_id, e);
            }
        }
    }};
}



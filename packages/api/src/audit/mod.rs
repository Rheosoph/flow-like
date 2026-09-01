pub mod chain;
pub mod service;
pub mod sign;

pub use chain::{GENESIS_HASH, compute_entry_hash};
pub use service::AuditService;
pub use sign::{sign_entry, verify_entry_signature};

use crate::entity::sea_orm_active_enums::{AuditActorType, RunMode, RunStatus};
use crate::middleware::jwt::AppUser;
use crate::state::AppState;

#[derive(Clone, Debug)]
pub struct ExecutionAudit {
    pub run_id: String,
    pub app_id: String,
    pub board_id: String,
    pub event_id: Option<String>,
    pub node_id: Option<String>,
    pub version: Option<String>,
    pub board_etag: Option<String>,
    pub mode: RunMode,
    pub status: RunStatus,
    pub input_payload_len: i64,
    pub technical_user_id: Option<String>,
}

pub fn actor_type_from_user(user: &AppUser) -> AuditActorType {
    match user {
        AppUser::OpenID(_) => AuditActorType::User,
        AppUser::PAT(_) => AuditActorType::User,
        AppUser::APIKey(_) => AuditActorType::ApiKey,
        AppUser::Executor(_) => AuditActorType::Executor,
        AppUser::ConnectedApp(_) => AuditActorType::System,
        AppUser::Unauthorized => AuditActorType::System,
    }
}

pub async fn record_execution_start(state: &AppState, user: &AppUser, execution: ExecutionAudit) {
    if !state.platform_config.audit.enabled || !state.platform_config.audit.log_executions {
        return;
    }

    let actor_type = actor_type_from_user(user);
    let actor_id = match user.audit_id().await {
        Ok(actor_id) => actor_id,
        Err(err) => {
            tracing::error!("AUDIT FAILURE: Failed to get audit_id: {}", err);
            return;
        }
    };

    let is_event = execution.event_id.is_some();
    let action = if is_event {
        "execution.event.start"
    } else {
        "execution.board.start"
    };
    let summary = if is_event {
        "Event execution started"
    } else {
        "Board execution started"
    };

    let run_id = execution.run_id.clone();
    let app_id = execution.app_id.clone();
    let details = serde_json::json!({
        "run_id": &execution.run_id,
        "app_id": &execution.app_id,
        "board_id": &execution.board_id,
        "event_id": &execution.event_id,
        "node_id": &execution.node_id,
        "version": &execution.version,
        "board_etag": &execution.board_etag,
        "execution_type": if is_event { "event" } else { "board" },
        "mode": format!("{:?}", execution.mode),
        "status": format!("{:?}", execution.status),
        "input_payload_len": execution.input_payload_len,
        "technical_user_id": &execution.technical_user_id,
    });

    let input = service::AuditEntryInput {
        actor_id,
        actor_type,
        actor_ip: None,
        action: action.to_string(),
        resource_type: "ExecutionRun".to_string(),
        resource_id: execution.run_id,
        chain_id: Some(execution.app_id),
        summary: summary.to_string(),
        details: Some(details),
    };

    if let Err(err) = AuditService::record(&state.db, input).await {
        tracing::error!(
            run_id = %run_id,
            app_id = %app_id,
            "AUDIT FAILURE (execution): {}",
            err
        );
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
            }
            .await;
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
            }
            .await;
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
            }
            .await;
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
            }
            .await;
            if let Err(e) = &audit_result {
                tracing::error!("AUDIT FAILURE (chain {}): {}", $chain_id, e);
            }
        }
    }};
}

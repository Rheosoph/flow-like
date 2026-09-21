//! Audit trail.
//!
//! A request writes one record with a single insert: no read, no lock, no sequence.
//! The audit worker later verifies each pending record's MAC, groups each chain's records
//! into a seal that links to the chain's previous seal, and commits every seal of a run
//! to an epoch signed with the audit key. Monthly archives go to the audit bucket.
//! Design: todo/874-audit-retention.md.

pub mod crypto;
mod execution;
pub mod export;
pub mod keys;
pub mod kms;
pub mod level;
pub mod merkle;
pub mod record;
pub mod request;
pub mod signer;
pub mod verify;
pub mod wire;
pub mod worker;

pub use execution::{
    ExecutionAuditContext, record_execution_dispatch, record_execution_dispatch_failure,
    record_execution_dispatch_for, record_execution_outcome, record_execution_result,
};
pub use level::{AuditLevel, RetentionClass, records, records_executions, required_level};
pub use record::{AuditRecordInput, PLATFORM_CHAIN, WriteMode, chain_for, chain_scope};

use flow_like_types::Value;

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

/// Record with the level gate and the request's failure accounting, for paths that
/// have no `AppUser`: webhooks and background jobs.
pub async fn record_entry(state: &AppState, input: AuditRecordInput) {
    write_gated(state, input, WriteMode::Append).await;
}

/// Like [`record_entry`], once per (chain, action, resource).
pub async fn record_entry_once(state: &AppState, input: AuditRecordInput) {
    write_gated(state, input, WriteMode::Once).await;
}

async fn write_gated(state: &AppState, input: AuditRecordInput, mode: WriteMode) {
    if !records(&state.platform_config.audit, &input.action) {
        return;
    }
    let action = input.action.clone();
    if let Err(error) = record::write(&state.db, input, mode).await {
        request::record_failure();
        tracing::error!(%error, action, "AUDIT FAILURE");
    }
}

/// What the `audit!` and `audit_branch!` macros call.
#[allow(clippy::too_many_arguments)]
pub async fn record_for_user(
    state: &AppState,
    user: &AppUser,
    scope: Option<String>,
    action: String,
    resource_type: String,
    resource_id: String,
    details: Option<Value>,
) {
    let actor_id = match user.audit_id().await {
        Ok(actor_id) => actor_id,
        Err(error) => {
            request::record_failure();
            tracing::error!(%error, action, "AUDIT FAILURE: actor could not be identified");
            return;
        }
    };
    record_entry(
        state,
        AuditRecordInput {
            actor_id,
            actor_type: actor_type_from_user(user),
            actor_ip: request::actor_ip(),
            action,
            resource_type,
            resource_id,
            scope,
            details,
        },
    )
    .await;
}

pub async fn record_execution_start(state: &AppState, user: &AppUser, execution: ExecutionAudit) {
    if !records_executions(&state.platform_config.audit) {
        return;
    }
    let actor_id = match user.audit_id().await {
        Ok(actor_id) => actor_id,
        Err(error) => {
            request::record_failure();
            tracing::error!(%error, "AUDIT FAILURE: actor could not be identified");
            return;
        }
    };
    let is_event = execution.event_id.is_some();
    let input = AuditRecordInput {
        actor_id,
        actor_type: actor_type_from_user(user),
        actor_ip: request::actor_ip(),
        action: if is_event {
            "execution.event.start"
        } else {
            "execution.board.start"
        }
        .to_owned(),
        resource_type: "ExecutionRun".to_owned(),
        resource_id: execution.run_id.clone(),
        scope: Some(execution.app_id.clone()),
        details: Some(serde_json::json!({
            "board_id": execution.board_id,
            "event_id": execution.event_id,
            "node_id": execution.node_id,
            "version": execution.version,
            "board_etag": execution.board_etag,
            "mode": format!("{:?}", execution.mode),
            "status": format!("{:?}", execution.status),
            "input_payload_len": execution.input_payload_len,
            "technical_user_id": execution.technical_user_id,
        })),
    };
    if let Err(error) = record::write(&state.db, input, WriteMode::Once).await {
        request::record_failure();
        tracing::error!(run_id = %execution.run_id, %error, "AUDIT FAILURE (execution start)");
    }
}

/// Record a change on the platform chain.
///
/// `audit!(state, user, action, resource_type, resource_id)`, or with a
/// `serde_json::Value` of ids and short codes: `audit!(state, user, action,
/// resource_type, resource_id, details)`. The UI renders the sentence from the action
/// and resource, so there is no summary. Respects `audit.enabled` and `audit.level`.
#[macro_export]
macro_rules! audit {
    ($state:expr, $user:expr, $action:expr, $resource_type:expr, $resource_id:expr) => {{
        if $crate::audit::records(&$state.platform_config.audit, &$action) {
            $crate::audit::record_for_user(
                &$state,
                &$user,
                None,
                $action.to_string(),
                $resource_type.to_string(),
                $resource_id.to_string(),
                None,
            )
            .await;
        }
    }};
    ($state:expr, $user:expr, $action:expr, $resource_type:expr, $resource_id:expr, $details:expr) => {{
        if $crate::audit::records(&$state.platform_config.audit, &$action) {
            let details: $crate::audit::DetailsValue = $details;
            $crate::audit::record_for_user(
                &$state,
                &$user,
                None,
                $action.to_string(),
                $resource_type.to_string(),
                $resource_id.to_string(),
                Some(details),
            )
            .await;
        }
    }};
}

/// Record a change on an app's or package's chain.
///
/// `audit_branch!(state, user, scope_id, action, resource_type, resource_id)`, or with
/// details as the last argument. Respects `audit.enabled` and `audit.level`.
#[macro_export]
macro_rules! audit_branch {
    ($state:expr, $user:expr, $scope:expr, $action:expr, $resource_type:expr, $resource_id:expr) => {{
        if $crate::audit::records(&$state.platform_config.audit, &$action) {
            $crate::audit::record_for_user(
                &$state,
                &$user,
                Some($scope.to_string()),
                $action.to_string(),
                $resource_type.to_string(),
                $resource_id.to_string(),
                None,
            )
            .await;
        }
    }};
    ($state:expr, $user:expr, $scope:expr, $action:expr, $resource_type:expr, $resource_id:expr, $details:expr) => {{
        if $crate::audit::records(&$state.platform_config.audit, &$action) {
            let details: $crate::audit::DetailsValue = $details;
            $crate::audit::record_for_user(
                &$state,
                &$user,
                Some($scope.to_string()),
                $action.to_string(),
                $resource_type.to_string(),
                $resource_id.to_string(),
                Some(details),
            )
            .await;
        }
    }};
}

/// Type of the details argument of the audit macros.
pub type DetailsValue = Value;

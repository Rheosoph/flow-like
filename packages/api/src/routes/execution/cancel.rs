//! Authorized hard cancellation of isolated Compose executions.

use std::{sync::Arc, time::Duration};

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::anyhow;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    entity::{execution_run, sea_orm_active_enums::AuditActorType},
    error::ApiError,
    execution::{
        ChannelClaims,
        state::{ExecutionRunRecord, ExecutionStateStore, PostgresStateStore, RunMode, RunStatus},
    },
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};

#[derive(Serialize, ToSchema)]
pub struct CancelRunResponse {
    pub run_id: String,
    pub status: String,
    /// True only when the manager confirmed termination and cancellation was persisted.
    pub cancelled: bool,
}

#[derive(Deserialize)]
struct TerminationAcknowledgement {
    run_id: String,
    terminated: bool,
}

fn isolated_execution() -> bool {
    std::env::var("EXECUTION_ISOLATION_MODE").as_deref() == Ok("per_run")
}

fn cancellation_authorized(run_owner: Option<&str>, caller: &str, app_admin: bool) -> bool {
    app_admin || run_owner == Some(caller)
}

fn valid_termination(acknowledgement: &TerminationAcknowledgement, run_id: &str) -> bool {
    acknowledgement.terminated && acknowledgement.run_id == run_id
}

/// DELETE /execution/run/{run_id}
/// Stop the isolated runner before recording cancellation.
#[utoipa::path(
    delete,
    path = "/execution/run/{run_id}",
    tag = "execution",
    params(("run_id" = String, Path, description = "Run to cancel")),
    responses(
        (status = 200, description = "Termination confirmed or run already terminal", body = CancelRunResponse),
        (status = 403, description = "Caller cannot cancel this run"),
        (status = 404, description = "Run not found"),
        (status = 503, description = "Termination could not be confirmed")
    )
)]
pub async fn cancel_run(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<CancelRunResponse>, ApiError> {
    let caller = user.sub()?;
    let store = super::progress::get_state_store(&state).await?;
    let run = store
        .get_run(&run_id)
        .await
        .map_err(|error| ApiError::internal_error(anyhow!("Cannot load execution: {error}")))?
        .ok_or(ApiError::NOT_FOUND)?;
    let permission = user.app_permission_fresh(&run.app_id, &state).await?;
    let can_execute = permission.has_permission(if run.event_id.is_some() {
        RolePermissions::ExecuteEvents
    } else {
        RolePermissions::ExecuteBoards
    });
    if !can_execute
        || !cancellation_authorized(
            run.user_id.as_deref(),
            &caller,
            permission.has_permission(RolePermissions::Admin),
        )
    {
        return Err(ApiError::FORBIDDEN);
    }
    let run = terminate_and_persist(&state, store.as_ref(), &run, &caller).await?;
    Ok(Json(CancelRunResponse {
        run_id: run.id,
        cancelled: run.status == RunStatus::Cancelled,
        status: format!("{:?}", run.status),
    }))
}

/// Channel responder capabilities already authorize cancellation for exactly
/// one channel, subject and app. Local/global-chat channels keep cooperative cancellation.
pub(crate) async fn cancel_channel_run(
    state: &AppState,
    claims: &ChannelClaims,
) -> Result<bool, ApiError> {
    if !isolated_execution() {
        return Ok(false);
    }
    let Some(app_id) = &claims.app_id else {
        return Ok(false);
    };
    let store = super::progress::get_state_store(state).await?;
    let Some(run) = store
        .get_run_for_app(&claims.channel_id, app_id)
        .await
        .map_err(|error| {
            ApiError::internal_error(anyhow!("Cannot load channel execution: {error}"))
        })?
    else {
        return Ok(false);
    };
    if run.mode == RunMode::Local {
        return Ok(false);
    }
    terminate_and_persist(state, store.as_ref(), &run, &claims.sub).await?;
    Ok(true)
}

async fn terminate_and_persist(
    state: &AppState,
    store: &dyn ExecutionStateStore,
    run: &ExecutionRunRecord,
    actor: &str,
) -> Result<ExecutionRunRecord, ApiError> {
    if !isolated_execution()
        || store.backend_name() != "redis"
        || !matches!(run.mode, RunMode::Http | RunMode::Queue)
    {
        return Err(ApiError::service_unavailable(
            "Hard cancellation requires an isolated Compose execution with Redis state",
        ));
    }
    // A terminal callback can originate in the sandbox while child processes
    // remain alive. Only the trusted manager can confirm their termination.
    confirm_termination(&run.id).await?;
    // The manager installs a shared cancellation tombstone before killing
    // active containers, so queued and racing launches cannot start afterward.
    // Redis preserves any terminal outcome that already won the status race.
    let persisted = store
        .cancel_run_after_termination(&run.id, &run.app_id)
        .await
        .map_err(|error| {
            ApiError::internal_error(anyhow!("Cannot persist confirmed cancellation: {error}"))
        })?;
    PostgresStateStore::new(Arc::new(state.db.clone()))
        .mirror_run_update(&persisted)
        .await
        .map_err(|error| {
            ApiError::internal_error(anyhow!("Cannot mirror confirmed cancellation: {error}"))
        })?;
    if persisted.status == RunStatus::Cancelled {
        let sql = execution_run::Entity::find_by_id(&run.id)
            .filter(execution_run::Column::AppId.eq(&run.app_id))
            .one(&state.db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;
        crate::audit::record_execution_result(
            &crate::audit::ExecutionAuditContext::from(state),
            &sql,
            actor,
            AuditActorType::User,
        )
        .await?;
    }
    Ok(persisted)
}

async fn confirm_termination(run_id: &str) -> Result<(), ApiError> {
    if run_id.is_empty()
        || run_id.len() > 128
        || !run_id.as_bytes()[0].is_ascii_alphanumeric()
        || !run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(ApiError::bad_request("Invalid run identifier"));
    }
    let endpoint = std::env::var("EXECUTOR_URL").map_err(|_| {
        ApiError::service_unavailable("Execution manager endpoint is not configured")
    })?;
    let mut url = reqwest::Url::parse(&endpoint)
        .map_err(|_| ApiError::service_unavailable("Execution manager endpoint is invalid"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::service_unavailable(
            "Execution manager endpoint must be an HTTP(S) origin",
        ));
    }
    url.path_segments_mut()
        .map_err(|_| ApiError::service_unavailable("Invalid manager endpoint"))?
        .pop_if_empty()
        .push("execute")
        .push(run_id);
    let token = std::env::var("EXECUTION_MANAGER_TOKEN").map_err(|_| {
        ApiError::service_unavailable("Execution manager authentication is not configured")
    })?;
    if token.len() < 32 {
        return Err(ApiError::service_unavailable(
            "Execution manager authentication is invalid",
        ));
    }
    let mut token = reqwest::header::HeaderValue::from_str(&token).map_err(|_| {
        ApiError::service_unavailable("Execution manager authentication is invalid")
    })?;
    token.set_sensitive(true);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ApiError::service_unavailable("Cannot initialize execution cancellation"))?;
    let mut response = client
        .delete(url)
        .header("X-Execution-Manager-Token", token)
        .send()
        .await
        .map_err(|_| {
            ApiError::service_unavailable(
                "Execution manager did not confirm termination; retry cancellation",
            )
        })?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(ApiError::service_unavailable(
            "Execution manager did not confirm termination; retry cancellation",
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ApiError::service_unavailable("Incomplete termination acknowledgement"))?
    {
        if bytes.len() + chunk.len() > 8192 {
            return Err(ApiError::service_unavailable(
                "Invalid termination acknowledgement",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let acknowledgement: TerminationAcknowledgement = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::service_unavailable("Invalid termination acknowledgement"))?;
    if !valid_termination(&acknowledgement, run_id) {
        return Err(ApiError::service_unavailable(
            "Termination acknowledgement did not match this run",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_run_creator_or_app_admin_can_cancel() {
        assert!(cancellation_authorized(Some("user-a"), "user-a", false));
        assert!(!cancellation_authorized(Some("user-a"), "user-b", false));
        assert!(!cancellation_authorized(None, "user-b", false));
        assert!(cancellation_authorized(None, "admin", true));
    }

    #[test]
    fn termination_requires_matching_positive_acknowledgement() {
        let mut acknowledgement = TerminationAcknowledgement {
            run_id: "run-a".into(),
            terminated: true,
        };
        assert!(valid_termination(&acknowledgement, "run-a"));
        assert!(!valid_termination(&acknowledgement, "run-b"));
        acknowledgement.terminated = false;
        assert!(!valid_termination(&acknowledgement, "run-a"));
        assert!(
            serde_json::from_str::<TerminationAcknowledgement>(r#"{"run_id":"run-a"}"#).is_err()
        );
    }
}

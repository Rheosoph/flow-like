use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::anyhow;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    ensure_permission,
    entity::{
        execution_run, execution_usage_tracking,
        sea_orm_active_enums::{ExecutionStatus, RunMode, RunStatus, RunVariant},
    },
    error::ApiError,
    execution::normalize_run_version_label,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct ReportRunRequest {
    pub run_id: String,
    pub node_id: String,
    pub event_id: Option<String>,
    pub version: Option<String>,
    pub log_level: u8,
    pub start: u64,
    pub end: u64,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ReportRunResponse {
    pub run_id: String,
    pub accepted: bool,
}

fn timestamp_micros(ts: u64) -> Option<i64> {
    if ts == 0 {
        return None;
    }

    let micros = if ts >= 1_000_000_000_000_000 {
        ts
    } else {
        ts.checked_mul(1000)?
    };
    i64::try_from(micros).ok()
}

fn timestamp_datetime(ts: u64) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    let micros = timestamp_micros(ts)?;
    chrono::DateTime::from_timestamp_micros(micros).map(|dt| dt.fixed_offset())
}

fn execution_status_from_log_level(log_level: u8) -> ExecutionStatus {
    match log_level {
        0 => ExecutionStatus::Debug,
        1 => ExecutionStatus::Info,
        2 => ExecutionStatus::Warn,
        3 => ExecutionStatus::Error,
        _ => ExecutionStatus::Fatal,
    }
}

#[allow(clippy::too_many_arguments)]
async fn track_reported_execution_usage(
    state: &AppState,
    run_id: &str,
    board_id: &str,
    node_id: &str,
    microseconds: i64,
    status: ExecutionStatus,
    user_id: Option<&str>,
    app_id: &str,
    created_at: chrono::DateTime<chrono::FixedOffset>,
) -> flow_like_types::Result<()> {
    let existing = execution_usage_tracking::Entity::find()
        .filter(execution_usage_tracking::Column::Version.eq(run_id))
        .one(&state.db)
        .await
        .map_err(|e| anyhow!("Failed to query execution usage: {}", e))?;

    if existing.is_some() {
        return Ok(());
    }

    let instance = std::env::var("INSTANCE_ID").ok();
    execution_usage_tracking::ActiveModel {
        id: Set(flow_like_types::create_id()),
        instance: Set(instance),
        board_id: Set(board_id.to_string()),
        node_id: Set(node_id.to_string()),
        version: Set(run_id.to_string()),
        microseconds: Set(microseconds.max(0)),
        status: Set(status),
        user_id: Set(user_id.map(ToOwned::to_owned)),
        technical_user_id: Set(None),
        app_id: Set(Some(app_id.to_string())),
        created_at: Set(created_at),
        updated_at: Set(chrono::Utc::now().fixed_offset()),
    }
    .insert(&state.db)
    .await
    .map_err(|e| anyhow!("Failed to track execution usage: {}", e))?;

    Ok(())
}

/// POST /apps/{app_id}/board/{board_id}/runs/report
///
/// Report a locally-executed run back to the backend. Used by the desktop app
/// to push run summaries and analytics for online apps.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/runs/report",
    tag = "execution",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
    ),
    request_body = ReportRunRequest,
    responses(
        (status = 200, description = "Run reported successfully", body = ReportRunResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/runs/report",
    skip(state, user, body)
)]
pub async fn report_run(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(body): Json<ReportRunRequest>,
) -> Result<Json<ReportRunResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    let sub = permission.sub()?;

    let run_status = if body.log_level >= 3 {
        RunStatus::Failed
    } else {
        RunStatus::Completed
    };
    let started_at = timestamp_datetime(body.start);
    let completed_at = timestamp_datetime(body.end);

    let now = chrono::Utc::now().fixed_offset();
    let expires_at = now + chrono::Duration::hours(24);

    let existing = execution_run::Entity::find_by_id(&body.run_id)
        .filter(execution_run::Column::AppId.eq(&app_id))
        .filter(execution_run::Column::BoardId.eq(&board_id))
        .filter(execution_run::Column::Mode.eq(RunMode::Local))
        .filter(execution_run::Column::UserId.eq(&sub))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to query run: {}", e)))?;

    let persisted = if let Some(existing) = existing {
        if matches!(existing.status, RunStatus::Pending | RunStatus::Running) {
            let mut update = execution_run::ActiveModel {
                id: Set(body.run_id.clone()),
                ..Default::default()
            };
            update.status = Set(run_status);
            update.log_level = Set(body.log_level as i32);
            update.started_at = Set(started_at);
            update.completed_at = Set(completed_at);
            update.progress = Set(100);
            update.updated_at = Set(now);
            if let Some(ref error) = body.error_message {
                update.error_message = Set(Some(error.clone()));
            }
            execution_run::Entity::update_many()
                .set(update)
                .filter(execution_run::Column::Id.eq(&body.run_id))
                .filter(execution_run::Column::AppId.eq(&app_id))
                .filter(execution_run::Column::BoardId.eq(&board_id))
                .filter(execution_run::Column::Mode.eq(RunMode::Local))
                .filter(execution_run::Column::UserId.eq(&sub))
                .filter(
                    execution_run::Column::Status.is_in([RunStatus::Pending, RunStatus::Running]),
                )
                .exec(&state.db)
                .await
                .map_err(|e| ApiError::internal_error(anyhow!("Failed to update run: {}", e)))?;
            execution_run::Entity::find_by_id(&body.run_id)
                .filter(execution_run::Column::AppId.eq(&app_id))
                .one(&state.db)
                .await?
                .ok_or(ApiError::NOT_FOUND)?
        } else {
            existing
        }
    } else {
        let version_label = body.version.as_deref().map(normalize_run_version_label);
        let run = execution_run::ActiveModel {
            id: Set(body.run_id.clone()),
            board_id: Set(board_id.clone()),
            version: Set(version_label),
            event_id: Set(body.event_id.clone()),
            node_id: Set(Some(body.node_id.clone())),
            status: Set(run_status),
            mode: Set(RunMode::Local),
            run_variant: Set(RunVariant::Primary),
            variant_name: Set(None),
            shadow_of_run_id: Set(None),
            regression_run_id: Set(None),
            log_level: Set(body.log_level as i32),
            input_payload_len: Set(0),
            input_payload_key: Set(None),
            output_payload_len: Set(0),
            error_message: Set(body.error_message.clone()),
            progress: Set(100),
            current_step: Set(None),
            started_at: Set(started_at),
            completed_at: Set(completed_at),
            expires_at: Set(Some(expires_at)),
            user_id: Set(Some(sub.clone())),
            technical_user_id: Set(None),
            caller_app_chain: Set(None),
            trace_id: Set(Some(body.run_id.clone())),
            parent_run_id: Set(None),
            correlation_keys: Set(None),
            app_id: Set(app_id.clone()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        run.insert(&state.db)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to create run: {}", e)))?
    };
    crate::audit::record_execution_result(
        &crate::audit::ExecutionAuditContext::from(&state),
        &persisted,
        &user.audit_id().await?,
        crate::audit::actor_type_from_user(&user),
    )
    .await?;

    let duration_us = match (persisted.started_at, persisted.completed_at) {
        (Some(start), Some(end)) => (end - start).num_microseconds().unwrap_or(0).max(0),
        _ => 0,
    };
    if let Err(error) = track_reported_execution_usage(
        &state,
        &persisted.id,
        &persisted.board_id,
        persisted.node_id.as_deref().unwrap_or_default(),
        duration_us,
        execution_status_from_log_level(persisted.log_level as u8),
        persisted.user_id.as_deref(),
        &persisted.app_id,
        persisted
            .completed_at
            .or(persisted.started_at)
            .unwrap_or(now),
    )
    .await
    {
        tracing::warn!(
            run_id = %body.run_id,
            error = %error,
            "Failed to track reported execution usage"
        );
    }

    Ok(Json(ReportRunResponse {
        run_id: body.run_id,
        accepted: true,
    }))
}

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

fn timestamp_datetime(ts: u64) -> Option<chrono::NaiveDateTime> {
    let micros = timestamp_micros(ts)?;
    chrono::DateTime::from_timestamp_micros(micros).map(|dt| dt.naive_utc())
}

fn reported_duration_us(start: u64, end: u64) -> i64 {
    match (timestamp_micros(start), timestamp_micros(end)) {
        (Some(start), Some(end)) => end.saturating_sub(start).max(0),
        _ => 0,
    }
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
    created_at: chrono::NaiveDateTime,
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
        updated_at: Set(chrono::Utc::now().naive_utc()),
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
    let execution_status = execution_status_from_log_level(body.log_level);
    let started_at = timestamp_datetime(body.start);
    let completed_at = timestamp_datetime(body.end);
    let duration_us = reported_duration_us(body.start, body.end);

    let now = chrono::Utc::now().naive_utc();
    let expires_at = now + chrono::Duration::hours(24);

    let existing = execution_run::Entity::find_by_id(&body.run_id)
        .filter(execution_run::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to query run: {}", e)))?;

    if let Some(_existing) = existing {
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
        update
            .update(&state.db)
            .await
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to update run: {}", e)))?;
    } else {
        let version_label = body.version.as_deref().map(normalize_run_version_label);
        let execution_audit = crate::audit::ExecutionAudit {
            run_id: body.run_id.clone(),
            app_id: app_id.clone(),
            board_id: board_id.clone(),
            event_id: body.event_id.clone(),
            node_id: Some(body.node_id.clone()),
            version: version_label.clone(),
            board_etag: None,
            mode: RunMode::Local,
            status: run_status.clone(),
            input_payload_len: 0,
            technical_user_id: None,
        };
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
            .map_err(|e| ApiError::internal_error(anyhow!("Failed to create run: {}", e)))?;
        crate::audit::record_execution_start(&state, &user, execution_audit).await;
    }

    if let Err(error) = track_reported_execution_usage(
        &state,
        &body.run_id,
        &board_id,
        &body.node_id,
        duration_us,
        execution_status,
        Some(&sub),
        &app_id,
        completed_at.or(started_at).unwrap_or(now),
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

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
        execution_run,
        sea_orm_active_enums::{RunMode, RunStatus},
    },
    error::ApiError,
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

/// POST /apps/{app_id}/board/{board_id}/runs/report
///
/// Report a locally-executed run back to the backend. Used by the desktop app
/// to push run summaries (especially warnings/errors) for online apps.
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
#[tracing::instrument(name = "POST /apps/{app_id}/board/{board_id}/runs/report", skip(state, user, body))]
pub async fn report_run(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(body): Json<ReportRunRequest>,
) -> Result<Json<ReportRunResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    let sub = permission.sub()?;

    let to_datetime = |ts: u64| -> Option<chrono::NaiveDateTime> {
        let millis = if ts >= 1_000_000_000_000_000 {
            (ts / 1000) as i64
        } else {
            ts as i64
        };
        chrono::DateTime::from_timestamp_millis(millis).map(|dt| dt.naive_utc())
    };

    let run_status = if body.log_level >= 3 {
        RunStatus::Failed
    } else {
        RunStatus::Completed
    };

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
        update.started_at = Set(to_datetime(body.start));
        update.completed_at = Set(to_datetime(body.end));
        update.progress = Set(100);
        update.updated_at = Set(now);
        if let Some(ref error) = body.error_message {
            update.error_message = Set(Some(error.clone()));
        }
        update.update(&state.db).await.map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to update run: {}", e))
        })?;
    } else {
        let run = execution_run::ActiveModel {
            id: Set(body.run_id.clone()),
            board_id: Set(board_id.clone()),
            version: Set(body.version.clone()),
            event_id: Set(body.event_id.clone()),
            node_id: Set(Some(body.node_id.clone())),
            status: Set(run_status),
            mode: Set(RunMode::Local),
            log_level: Set(body.log_level as i32),
            input_payload_len: Set(0),
            input_payload_key: Set(None),
            output_payload_len: Set(0),
            error_message: Set(body.error_message.clone()),
            progress: Set(100),
            current_step: Set(None),
            started_at: Set(to_datetime(body.start)),
            completed_at: Set(to_datetime(body.end)),
            expires_at: Set(Some(expires_at)),
            user_id: Set(Some(sub)),
            app_id: Set(app_id.clone()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        run.insert(&state.db).await.map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to create run: {}", e))
        })?;
    }

    Ok(Json(ReportRunResponse {
        run_id: body.run_id,
        accepted: true,
    }))
}

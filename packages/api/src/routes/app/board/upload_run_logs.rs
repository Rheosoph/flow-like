use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use flow_like::flow::execution::{
    log::{LogMessage, StoredLogMessage},
    run_index::runs_base_path,
};
use flow_like_storage::{
    arrow_array::{RecordBatchIterator, RecordBatchReader},
    lance::dataset::WriteMode,
    lancedb::{Connection, table::WriteOptions},
    lancedb_write_options::default_write_options,
};
use flow_like_types::anyhow;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::{
    credentials::CredentialsAccess,
    ensure_permission,
    entity::{execution_run, sea_orm_active_enums::RunMode},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};

pub(crate) const MAX_UPLOADED_RUN_LOGS: usize = 10_000;

#[derive(Debug, Deserialize, ToSchema)]
pub struct UploadRunLogsRequest {
    /// The run's log messages in stored form (`message`, `operation_id`, `node_id`,
    /// `log_level` 0-4, `token_in`, `token_out`, `bit_ids`, `start` and `end` in
    /// microseconds since the Unix epoch), at most 10,000.
    #[schema(value_type = Vec<Object>)]
    pub logs: Vec<StoredLogMessage>,
}

fn is_log_table_name(run_id: &str) -> bool {
    !run_id.is_empty()
        && run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn validate_upload(run_id: &str, logs: &[StoredLogMessage]) -> Result<(), ApiError> {
    if !is_log_table_name(run_id) {
        return Err(ApiError::bad_request(format!(
            "Run id {run_id:?} may only contain ASCII letters, digits, '_', '-' and '.'"
        )));
    }
    if logs.is_empty() {
        return Err(ApiError::bad_request(
            "A log upload needs at least one log message",
        ));
    }
    if logs.len() > MAX_UPLOADED_RUN_LOGS {
        return Err(ApiError::bad_request(format!(
            "A log upload takes at most {MAX_UPLOADED_RUN_LOGS} log messages, got {}",
            logs.len()
        )));
    }
    Ok(())
}

/// Lance's default append mode would add a second copy of the rows when a retry
/// races the first upload; create mode makes the loser fail instead.
fn create_only_write_options() -> WriteOptions {
    let mut options = default_write_options();
    if let Some(params) = options.lance_write_params.as_mut() {
        params.mode = WriteMode::Create;
    }
    options
}

/// Returns whether this call created the table; an existing table is left untouched.
async fn create_run_log_table(
    db: &Connection,
    run_id: &str,
    logs: Vec<StoredLogMessage>,
) -> flow_like_types::Result<bool> {
    if db.open_table(run_id).execute().await.is_ok() {
        return Ok(false);
    }

    let batch = LogMessage::into_arrow(logs.into_iter().map(LogMessage::from))?;
    let schema = batch.schema();
    let reader: Box<dyn RecordBatchReader + Send> =
        Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema));

    match db
        .create_table(run_id, reader)
        .write_options(create_only_write_options())
        .execute()
        .await
    {
        Ok(_) => Ok(true),
        Err(create_err) => {
            db.open_table(run_id).execute().await.map_err(|open_err| {
                anyhow!(
                    "Failed to create log table {run_id} ({create_err}) and no concurrent upload created it: {open_err}"
                )
            })?;
            Ok(false)
        }
    }
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/runs/{run_id}/logs",
    tag = "execution",
    description = "Upload the log messages of a run that executed on your device, so its logs can be inspected like those of a cloud run. Report the run first. Uploading logs for a run that already has them changes nothing.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID"),
        ("run_id" = String, Path, description = "ID of the local run, as reported to /runs/report")
    ),
    request_body = UploadRunLogsRequest,
    responses(
        (status = 204, description = "The run's logs are stored"),
        (status = 400, description = "No log messages, more than 10,000, or an invalid run ID"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "You have not reported a local run with this ID on this board"),
        (status = 413, description = "The request body is larger than 4 MiB")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/runs/{run_id}/logs",
    skip(state, user, body)
)]
pub async fn upload_run_logs(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, board_id, run_id)): Path<(String, String, String)>,
    Json(body): Json<UploadRunLogsRequest>,
) -> Result<StatusCode, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    let sub = permission.sub()?;
    validate_upload(&run_id, &body.logs)?;

    execution_run::Entity::find_by_id(&run_id)
        .select_only()
        .column(execution_run::Column::Id)
        .filter(execution_run::Column::AppId.eq(&app_id))
        .filter(execution_run::Column::BoardId.eq(&board_id))
        .filter(execution_run::Column::Mode.eq(RunMode::Local))
        .filter(execution_run::Column::UserId.eq(&sub))
        .into_tuple::<String>()
        .one(&state.db)
        .await?
        .ok_or_else(|| {
            ApiError::not_found("You have not reported a local run with this id on this board")
        })?;

    let credentials = state
        .scoped_credentials(&sub, &app_id, CredentialsAccess::ServerExecute)
        .await?;
    let logs_db_builder = credentials
        .into_shared_credentials()
        .to_logs_db_builder()
        .map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to create logs db builder: {}", e))
        })?;
    let base_path = runs_base_path(&app_id, &board_id);
    let db = logs_db_builder(base_path.clone())
        .execute()
        .await
        .map_err(|e| {
            ApiError::internal_error(anyhow!("Failed to open log database at {base_path}: {e}"))
        })?;

    let count = body.logs.len();
    let created = create_run_log_table(&db, &run_id, body.logs)
        .await
        .map_err(ApiError::internal_error)?;
    tracing::info!(%run_id, count, created, "Stored uploaded logs of a local run");

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::{
        arrow_array::RecordBatch,
        lancedb::{
            self,
            query::{ExecutableQuery, QueryBase},
        },
        serde_arrow,
    };
    use futures::TryStreamExt;

    fn log(index: usize) -> StoredLogMessage {
        StoredLogMessage {
            message: format!("message {index}"),
            operation_id: Some(format!("op-{index}")),
            node_id: Some("node".to_string()),
            log_level: (index % 5) as u8,
            token_in: Some(index as u64),
            token_out: None,
            bit_ids: Some(vec!["bit".to_string()]),
            start: 1_700_000_000_000_000 + index as u64,
            end: 1_700_000_000_000_100 + index as u64,
            fingerprint: None,
        }
    }

    fn logs(count: usize) -> Vec<StoredLogMessage> {
        (0..count).map(log).collect()
    }

    async fn stored_rows(db: &Connection, run_id: &str) -> Vec<StoredLogMessage> {
        let batches: Vec<RecordBatch> = db
            .open_table(run_id)
            .execute()
            .await
            .expect("log table exists")
            .query()
            .limit(MAX_UPLOADED_RUN_LOGS)
            .execute()
            .await
            .expect("query runs")
            .try_collect()
            .await
            .expect("batches collect");
        batches
            .iter()
            .flat_map(|batch| {
                serde_arrow::from_record_batch::<Vec<StoredLogMessage>>(batch).expect("rows decode")
            })
            .collect()
    }

    #[test]
    fn upload_needs_between_one_and_ten_thousand_logs() {
        assert_eq!(
            validate_upload("run", &[]).unwrap_err().status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            validate_upload("run", &logs(MAX_UPLOADED_RUN_LOGS + 1))
                .unwrap_err()
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert!(validate_upload("run", &logs(1)).is_ok());
        assert!(validate_upload("run", &logs(MAX_UPLOADED_RUN_LOGS)).is_ok());
    }

    #[test]
    fn run_id_must_be_a_valid_log_table_name() {
        for run_id in ["", "a/b", "../x", "run id", "rün"] {
            assert_eq!(
                validate_upload(run_id, &logs(1)).unwrap_err().status(),
                StatusCode::BAD_REQUEST,
                "{run_id:?}"
            );
        }
        for run_id in ["tz4a98xxat96iws9zmbrgj3a", "run_1-2.3"] {
            assert!(validate_upload(run_id, &logs(1)).is_ok(), "{run_id:?}");
        }
    }

    #[test]
    fn request_takes_the_stored_log_message_shape() {
        let body: UploadRunLogsRequest = serde_json::from_value(serde_json::json!({
            "logs": [{
                "message": "failed",
                "operation_id": null,
                "node_id": "node",
                "log_level": 3,
                "token_in": null,
                "token_out": null,
                "bit_ids": null,
                "start": 1_700_000_000_000_000u64,
                "end": 1_700_000_000_000_500u64
            }]
        }))
        .expect("request deserializes");
        assert_eq!(body.logs.len(), 1);
        assert_eq!(body.logs[0].log_level, 3);
        assert_eq!(body.logs[0].end, 1_700_000_000_000_500);
    }

    #[tokio::test]
    async fn creates_the_table_once_and_leaves_an_existing_one_untouched() {
        let db = lancedb::connect("memory://upload-run-logs")
            .execute()
            .await
            .expect("in-memory log database");
        let uploaded = logs(3);

        assert!(
            create_run_log_table(&db, "run-1", uploaded.clone())
                .await
                .expect("first upload")
        );
        assert!(
            !create_run_log_table(&db, "run-1", logs(7))
                .await
                .expect("retried upload")
        );

        let stored = stored_rows(&db, "run-1").await;
        assert_eq!(stored.len(), uploaded.len());
        for (stored, uploaded) in stored.iter().zip(&uploaded) {
            assert_eq!(stored.message, uploaded.message);
            assert_eq!(stored.operation_id, uploaded.operation_id);
            assert_eq!(stored.log_level, uploaded.log_level);
            assert_eq!(stored.token_in, uploaded.token_in);
            assert_eq!(stored.bit_ids, uploaded.bit_ids);
            assert_eq!(stored.start, uploaded.start);
            assert_eq!(stored.end, uploaded.end);
        }
    }
}

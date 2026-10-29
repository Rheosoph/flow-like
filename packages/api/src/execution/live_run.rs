//! Whether an executor's run may still act, from the execution state store.

use crate::{
    error::ApiError, execution::state::ExecutionRunRecord,
    routes::execution::progress::get_state_store, state::AppState,
};

pub fn has_ended(run: &ExecutionRunRecord) -> bool {
    run.status.is_terminal()
        || run
            .expires_at
            .is_some_and(|expiry| expiry <= chrono::Utc::now().timestamp_millis())
}

/// The run's record while it is still going; 403 once it is missing or has ended.
pub async fn live_run(state: &AppState, run_id: &str) -> Result<ExecutionRunRecord, ApiError> {
    let run = get_state_store(state)
        .await?
        .get_run(run_id)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    live(run)
}

fn live(run: Option<ExecutionRunRecord>) -> Result<ExecutionRunRecord, ApiError> {
    let run = run.ok_or_else(|| ApiError::forbidden("The executor's run no longer exists"))?;
    if has_ended(&run) {
        return Err(ApiError::forbidden("The run has already ended"));
    }
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::state::{RunMode, RunStatus, RunVariant};
    use axum::http::StatusCode;

    fn run(status: RunStatus, expires_at: Option<i64>) -> ExecutionRunRecord {
        ExecutionRunRecord {
            id: "run".into(),
            board_id: "board".into(),
            version: None,
            event_id: None,
            status,
            mode: RunMode::Http,
            run_variant: RunVariant::Primary,
            variant_name: None,
            shadow_of_run_id: None,
            regression_run_id: None,
            input_payload_len: 0,
            output_payload_len: 0,
            error_message: None,
            progress: 0,
            current_step: None,
            started_at: None,
            completed_at: None,
            expires_at,
            user_id: None,
            technical_user_id: None,
            app_id: "app".into(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn refusal(run: Option<ExecutionRunRecord>) -> Option<(StatusCode, String)> {
        live(run).err().map(|error| {
            (
                error.status(),
                error.public_message().unwrap_or_default().to_owned(),
            )
        })
    }

    #[test]
    fn only_a_present_unfinished_run_is_live() {
        assert_eq!(refusal(Some(run(RunStatus::Pending, None))), None);
        assert_eq!(refusal(Some(run(RunStatus::Running, Some(i64::MAX)))), None);

        let ended = Some((
            StatusCode::FORBIDDEN,
            "The run has already ended".to_owned(),
        ));
        for status in [
            RunStatus::Completed,
            RunStatus::Failed,
            RunStatus::Cancelled,
            RunStatus::Timeout,
        ] {
            assert_eq!(refusal(Some(run(status, None))), ended);
        }
        assert_eq!(refusal(Some(run(RunStatus::Running, Some(0)))), ended);
        assert_eq!(
            refusal(None),
            Some((
                StatusCode::FORBIDDEN,
                "The executor's run no longer exists".to_owned()
            ))
        );
    }
}

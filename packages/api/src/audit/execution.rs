use std::sync::Arc;

use sea_orm::{ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

use crate::{
    db::DbDialect,
    entity::{
        execution_run,
        sea_orm_active_enums::{AuditActorType, RunStatus},
    },
    state::AppState,
};

use super::{AuditService, service::AuditEntryInput};

/// Retain the audit policy when a response stream outlives its request handler.
#[derive(Clone, Debug)]
pub struct ExecutionAuditContext {
    pub db: Arc<DatabaseConnection>,
    pub dialect: DbDialect,
    pub enabled: bool,
}

impl From<&AppState> for ExecutionAuditContext {
    fn from(state: &AppState) -> Self {
        Self {
            db: Arc::new(state.db.clone()),
            dialect: state.db_dialect,
            enabled: state.platform_config.audit.enabled
                && state.platform_config.audit.log_executions,
        }
    }
}

fn execution_entry(
    run: &execution_run::Model,
    actor_id: &str,
    actor_type: AuditActorType,
    terminal: bool,
) -> Option<AuditEntryInput> {
    let outcome = if terminal {
        match run.status {
            RunStatus::Completed => "complete",
            RunStatus::Failed => "fail",
            RunStatus::Cancelled => "cancel",
            RunStatus::Timeout => "timeout",
            RunStatus::Pending | RunStatus::Running => return None,
        }
    } else {
        "start"
    };
    let kind = if run.event_id.is_some() {
        "event"
    } else {
        "board"
    };
    Some(AuditEntryInput {
        actor_id: actor_id.to_owned(),
        actor_type,
        actor_ip: super::request::actor_ip(),
        action: format!("execution.{kind}.{outcome}"),
        resource_type: "ExecutionRun".to_owned(),
        resource_id: run.id.clone(),
        chain_id: Some(run.app_id.clone()),
        summary: if terminal {
            format!("Execution ended with status {:?}", run.status)
        } else {
            "Execution requested".to_owned()
        },
        details: Some(serde_json::json!({
            "run_id": run.id,
            "app_id": run.app_id,
            "board_id": run.board_id,
            "event_id": run.event_id,
            "node_id": run.node_id,
            "version": run.version,
            "execution_type": kind,
            "mode": format!("{:?}", run.mode),
            "status": format!("{:?}", run.status),
            "input_payload_len": run.input_payload_len,
            "output_payload_len": run.output_payload_len,
            "user_id": run.user_id,
            "technical_user_id": run.technical_user_id,
            "started_at": run.started_at,
            "completed_at": run.completed_at,
            "run_variant": format!("{:?}", run.run_variant),
            "parent_run_id": run.parent_run_id,
        })),
    })
}

/// Record the persisted outcome. Retried callbacks repair a missing entry and
/// share one entry with SSE completion of the same run.
pub async fn record_execution_result(
    context: &ExecutionAuditContext,
    run: &execution_run::Model,
    actor_id: &str,
    actor_type: AuditActorType,
) -> flow_like_types::Result<()> {
    if context.enabled
        && let Some(input) = execution_entry(run, actor_id, actor_type, true)
    {
        AuditService::record_once(&context.db, context.dialect, input)
            .await
            .map_err(|error| {
                super::request::record_failure();
                tracing::error!(run_id = %run.id, %error, "AUDIT FAILURE (execution outcome)");
                error
            })?;
    }
    Ok(())
}

/// Record runs created by inbound, sink, setup and regression dispatchers.
pub async fn record_execution_dispatch(
    state: &AppState,
    run_id: &str,
    source: &str,
) -> flow_like_types::Result<()> {
    let context = ExecutionAuditContext::from(state);
    if !context.enabled {
        return Ok(());
    }
    let run = execution_run::Entity::find_by_id(run_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| flow_like_types::anyhow!("Execution run missing before audit: {run_id}"))?;
    if let Some(input) = execution_entry(&run, source, AuditActorType::System, false) {
        AuditService::record_once(&state.db, context.dialect, input)
            .await
            .map_err(|error| {
                super::request::record_failure();
                tracing::error!(run_id, %error, "AUDIT FAILURE (execution dispatch)");
                error
            })?;
    }
    Ok(())
}

pub async fn record_execution_outcome(
    state: &AppState,
    run_id: &str,
    source: &str,
) -> flow_like_types::Result<()> {
    let context = ExecutionAuditContext::from(state);
    if !context.enabled {
        return Ok(());
    }
    let run = execution_run::Entity::find_by_id(run_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| flow_like_types::anyhow!("Execution run missing before audit: {run_id}"))?;
    record_execution_result(&context, &run, source, AuditActorType::System).await
}

/// A rejected dispatch never receives an executor completion callback.
pub async fn record_execution_dispatch_failure(
    state: &AppState,
    run_id: &str,
    source: &str,
) -> flow_like_types::Result<()> {
    let now = chrono::Utc::now().fixed_offset();
    execution_run::Entity::update_many()
        .set(execution_run::ActiveModel {
            status: Set(RunStatus::Failed),
            completed_at: Set(Some(now)),
            updated_at: Set(now),
            ..Default::default()
        })
        .filter(execution_run::Column::Id.eq(run_id))
        .filter(execution_run::Column::Status.is_in([RunStatus::Pending, RunStatus::Running]))
        .exec(&state.db)
        .await?;
    record_execution_outcome(state, run_id, source).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::sea_orm_active_enums::{RunMode, RunVariant};
    use sea_orm::{ActiveValue::Set, TryIntoModel};

    fn run(status: RunStatus) -> execution_run::Model {
        let now = chrono::Utc::now().fixed_offset();
        execution_run::ActiveModel {
            id: Set("run-1".into()),
            board_id: Set("board-1".into()),
            version: Set(None),
            event_id: Set(None),
            node_id: Set(None),
            status: Set(status),
            mode: Set(RunMode::Http),
            run_variant: Set(RunVariant::Primary),
            variant_name: Set(None),
            shadow_of_run_id: Set(None),
            regression_run_id: Set(None),
            log_level: Set(0),
            input_payload_len: Set(12),
            input_payload_key: Set(Some("private-payload-key".into())),
            output_payload_len: Set(24),
            error_message: Set(Some("secret-bearing error".into())),
            progress: Set(100),
            current_step: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(Some(now)),
            expires_at: Set(None),
            user_id: Set(Some("initiating-user".into())),
            technical_user_id: Set(None),
            caller_app_chain: Set(None),
            trace_id: Set(None),
            parent_run_id: Set(None),
            correlation_keys: Set(None),
            app_id: Set("app-1".into()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .try_into_model()
        .unwrap()
    }

    #[test]
    fn result_actions_cover_every_terminal_status_and_exclude_inflight_runs() {
        for (status, action) in [
            (RunStatus::Completed, "complete"),
            (RunStatus::Failed, "fail"),
            (RunStatus::Cancelled, "cancel"),
            (RunStatus::Timeout, "timeout"),
        ] {
            let entry =
                execution_entry(&run(status), "executor", AuditActorType::Executor, true).unwrap();
            assert_eq!(entry.action, format!("execution.board.{action}"));
            assert_eq!(entry.chain_id.as_deref(), Some("app-1"));
        }
        for status in [RunStatus::Pending, RunStatus::Running] {
            assert!(
                execution_entry(&run(status), "executor", AuditActorType::Executor, true).is_none()
            );
        }
    }

    #[test]
    fn event_result_retains_identity_without_raw_payloads_or_errors() {
        let mut run = run(RunStatus::Failed);
        run.event_id = Some("event-1".into());
        let entry = execution_entry(&run, "executor", AuditActorType::Executor, true).unwrap();
        assert_eq!(entry.action, "execution.event.fail");
        assert_eq!(entry.actor_id, "executor");
        let details = entry.details.unwrap();
        assert_eq!(details["user_id"], "initiating-user");
        assert_eq!(details["input_payload_len"], 12);
        assert!(!details.to_string().contains("private-payload-key"));
        assert!(!details.to_string().contains("secret-bearing error"));
    }

    #[flow_like_types::tokio::test]
    async fn disabled_execution_audit_never_touches_the_database() {
        let context = ExecutionAuditContext {
            db: Arc::new(DatabaseConnection::default()),
            dialect: DbDialect::default(),
            enabled: false,
        };
        record_execution_result(
            &context,
            &run(RunStatus::Completed),
            "executor",
            AuditActorType::Executor,
        )
        .await
        .unwrap();
    }

    #[flow_like_types::tokio::test]
    async fn an_outcome_write_failure_is_returned_and_marks_the_request_incomplete() {
        let context = ExecutionAuditContext {
            db: Arc::new(DatabaseConnection::default()),
            dialect: DbDialect::default(),
            enabled: true,
        };
        let request = super::super::request::RequestAuditContext::default();
        let failures = request.failures.clone();
        super::super::request::REQUEST_AUDIT
            .scope(request, async {
                assert!(
                    record_execution_result(
                        &context,
                        &run(RunStatus::Completed),
                        "executor",
                        AuditActorType::Executor
                    )
                    .await
                    .is_err()
                );
            })
            .await;
        assert_eq!(failures.load(std::sync::atomic::Ordering::Relaxed), 1);
    }
}

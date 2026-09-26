//! The summary an executor reports when a run ends, and its projection back
//! into the `LogMeta` wire shape the desktop and the UI share.
//!
//! Four optional fields travel under the same names on the executor's
//! terminal progress update, the streaming `completed` event and the
//! desktop's run report. They land in the `eventVersion`, `nodes` and
//! `logsCount` columns of `ExecutionRun` (`logLevel` already had one). NULL
//! means "never reported" — an older sender — never "empty".

use flow_like::flow::execution::LogMeta;
use flow_like::flow::execution::rejection::REJECTION_OPERATION_PREFIX;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::anyhow;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, Iterable, QueryFilter,
    QuerySelect, Select,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::credentials::CredentialsAccess;
use crate::entity::execution_run;
use crate::error::ApiError;
use crate::execution::regression::datetime_micros;
use crate::state::AppState;

/// Largest serialized node list kept on a run row, roughly 8,000 visited
/// nodes. The list grows with the distinct nodes a run touched (loops do not
/// add entries), but it rides on the terminal status write and Aurora DSQL
/// rejects values above 1 MiB — an oversized list is recorded as unknown
/// rather than failing the run's completion.
pub const MAX_NODES_SUMMARY_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(default)]
pub struct RunSummary {
    /// Highest log level the run reached (0 = Debug … 4 = Fatal).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_level: Option<u8>,
    /// Dotted `MAJOR.MINOR.PATCH` of the event that triggered the run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_version: Option<String>,
    /// Visited nodes, each with the highest log level it reached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nodes: Option<Vec<(String, u8)>>,
    /// Number of log messages the run wrote.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logs: Option<u64>,
}

impl RunSummary {
    pub fn is_empty(&self) -> bool {
        self.log_level.is_none()
            && self.event_version.is_none()
            && self.nodes.is_none()
            && self.logs.is_none()
    }

    /// Sets the columns this summary carries on `model`; unreported fields
    /// leave their column untouched.
    pub fn apply_to(&self, model: &mut execution_run::ActiveModel) {
        if let Some(level) = self.log_level {
            model.log_level = Set(i32::from(level));
        }
        if let Some(version) = &self.event_version {
            model.event_version = Set(Some(version.clone()));
        }
        if let Some(nodes) = &self.nodes {
            let value = serde_json::to_value(nodes).ok();
            let bytes = value.as_ref().map_or(0, |value| value.to_string().len());
            if bytes <= MAX_NODES_SUMMARY_BYTES {
                model.nodes = Set(value);
            } else {
                tracing::warn!(
                    nodes = nodes.len(),
                    bytes,
                    limit = MAX_NODES_SUMMARY_BYTES,
                    "Run node summary exceeds the stored limit; recording it as unknown"
                );
            }
        }
        if let Some(logs) = self.logs {
            model.logs_count = Set(Some(i64::try_from(logs).unwrap_or(i64::MAX)));
        }
    }
}

/// Persist a summary on its run row. Filtered by id and app only: the summary
/// rides on the terminal update, which may already have moved the row out of
/// `Running` through another path, and a late summary must still land.
pub async fn apply_run_summary(
    db: &DatabaseConnection,
    run_id: &str,
    app_id: &str,
    summary: &RunSummary,
) -> Result<(), sea_orm::DbErr> {
    if summary.is_empty() {
        return Ok(());
    }
    let mut model = execution_run::ActiveModel::default();
    summary.apply_to(&mut model);
    execution_run::Entity::update_many()
        .set(model)
        .filter(execution_run::Column::Id.eq(run_id))
        .filter(execution_run::Column::AppId.eq(app_id))
        .exec(db)
        .await?;
    Ok(())
}

/// Selects every run column except `nodes`, which reads back as NULL, so a
/// listing that does not render the heatmap never loads the node lists.
pub fn without_nodes(select: Select<execution_run::Entity>) -> Select<execution_run::Entity> {
    select
        .select_only()
        .columns(
            execution_run::Column::iter()
                .filter(|column| !matches!(column, execution_run::Column::Nodes)),
        )
        .column_as(Expr::cust("NULL::jsonb"), "nodes")
}

/// The `LogMeta` wire shape of a run row, times in unix micros. Rejected rows
/// (`current_step` starting with `rejected:`) keep the two markers
/// `RejectedRun::log_meta` writes — `start == end` and an empty node list —
/// because the row's `completed_at` is stamped a moment after `created_at`.
pub fn log_meta_from_run(run: &execution_run::Model) -> LogMeta {
    let rejected = run
        .current_step
        .as_deref()
        .is_some_and(|step| step.starts_with(REJECTION_OPERATION_PREFIX));
    let start = datetime_micros(run.started_at.unwrap_or(run.created_at));
    let end = if rejected {
        start
    } else {
        datetime_micros(run.completed_at.unwrap_or(run.updated_at))
    };
    let nodes = if rejected {
        Some(Vec::new())
    } else {
        run.nodes
            .as_ref()
            .and_then(|nodes| serde_json::from_value(nodes.clone()).ok())
    };
    LogMeta {
        app_id: run.app_id.clone(),
        run_id: run.id.clone(),
        board_id: run.board_id.clone(),
        start,
        end,
        log_level: run.log_level as u8,
        version: run.version.clone().unwrap_or_default(),
        nodes,
        logs: run.logs_count.and_then(|count| u64::try_from(count).ok()),
        node_id: run.node_id.clone().unwrap_or_default(),
        event_version: run.event_version.clone(),
        event_id: run.event_id.clone().unwrap_or_default(),
        payload: Vec::new(),
        is_remote: true,
    }
}

/// The logs bucket under read-only credentials — where per-run log tables
/// and payload sidecars live.
pub(crate) async fn logs_store(
    state: &AppState,
    sub: &str,
    app_id: &str,
) -> Result<FlowLikeStore, ApiError> {
    let credentials = state
        .scoped_credentials(sub, app_id, CredentialsAccess::ReadLogs)
        .await?;
    credentials
        .to_store_type(flow_like::credentials::StoreType::Logs)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to open the logs store: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::sea_orm_active_enums::{RunMode, RunStatus, RunVariant};
    use sea_orm::{ActiveValue::NotSet, QueryTrait, TryIntoModel};

    const CREATED_MICROS: i64 = 1_700_000_000_123_456;

    fn run(
        started: bool,
        current_step: Option<&str>,
        nodes: Option<serde_json::Value>,
    ) -> execution_run::Model {
        let created_at = chrono::DateTime::from_timestamp_micros(CREATED_MICROS)
            .unwrap()
            .fixed_offset();
        execution_run::ActiveModel {
            id: Set("run-1".into()),
            board_id: Set("board-1".into()),
            version: Set(Some("v1-2-3".into())),
            event_id: Set(Some("evt-1".into())),
            node_id: Set(Some("node-1".into())),
            status: Set(RunStatus::Completed),
            mode: Set(RunMode::Http),
            run_variant: Set(RunVariant::Primary),
            variant_name: Set(None),
            shadow_of_run_id: Set(None),
            regression_run_id: Set(None),
            log_level: Set(3),
            input_payload_len: Set(0),
            input_payload_key: Set(None),
            output_payload_len: Set(0),
            error_message: Set(None),
            progress: Set(100),
            current_step: Set(current_step.map(str::to_string)),
            started_at: Set(started.then(|| created_at + chrono::Duration::milliseconds(5))),
            completed_at: Set(Some(created_at + chrono::Duration::milliseconds(905))),
            expires_at: Set(None),
            user_id: Set(None),
            technical_user_id: Set(None),
            caller_app_chain: Set(None),
            trace_id: Set(None),
            parent_run_id: Set(None),
            correlation_keys: Set(None),
            app_id: Set("app-1".into()),
            created_at: Set(created_at),
            updated_at: Set(created_at + chrono::Duration::seconds(3)),
            event_version: Set(Some("1.0.3".into())),
            nodes: Set(nodes),
            logs_count: Set(Some(7)),
        }
        .try_into_model()
        .unwrap()
    }

    #[test]
    fn summary_is_empty_only_without_any_field() {
        assert!(RunSummary::default().is_empty());
        assert!(serde_json::from_str::<RunSummary>("{}").unwrap().is_empty());
        assert!(
            serde_json::from_str::<RunSummary>(r#"{"status":"completed","extra":1}"#)
                .unwrap()
                .is_empty()
        );
        for summary in [
            RunSummary {
                log_level: Some(0),
                ..Default::default()
            },
            RunSummary {
                event_version: Some("1.0.0".into()),
                ..Default::default()
            },
            RunSummary {
                nodes: Some(Vec::new()),
                ..Default::default()
            },
            RunSummary {
                logs: Some(0),
                ..Default::default()
            },
        ] {
            assert!(!summary.is_empty());
        }
    }

    #[test]
    fn summary_parses_the_wire_contract_next_to_foreign_keys() {
        let summary: RunSummary = serde_json::from_str(
            r#"{"status":"failed","log_level":3,"event_version":"1.0.3","nodes":[["n1",1],["n2",3]],"logs":12}"#,
        )
        .unwrap();
        assert_eq!(
            summary,
            RunSummary {
                log_level: Some(3),
                event_version: Some("1.0.3".into()),
                nodes: Some(vec![("n1".into(), 1), ("n2".into(), 3)]),
                logs: Some(12),
            }
        );
    }

    #[test]
    fn apply_to_sets_only_the_reported_columns() {
        let mut model = execution_run::ActiveModel::default();
        RunSummary {
            log_level: Some(2),
            logs: Some(4),
            ..Default::default()
        }
        .apply_to(&mut model);
        assert_eq!(model.log_level, Set(2));
        assert_eq!(model.logs_count, Set(Some(4)));
        assert_eq!(model.event_version, NotSet);
        assert_eq!(model.nodes, NotSet);

        let mut model = execution_run::ActiveModel::default();
        RunSummary {
            nodes: Some(vec![("n1".into(), 1)]),
            ..Default::default()
        }
        .apply_to(&mut model);
        assert_eq!(model.nodes, Set(Some(serde_json::json!([["n1", 1]]))));
        assert_eq!(model.log_level, NotSet);
    }

    #[test]
    fn oversized_node_lists_are_recorded_as_unknown() {
        let nodes = |count: usize| RunSummary {
            nodes: Some(
                (0..count)
                    .map(|index| (format!("{index:0>24}"), 1))
                    .collect(),
            ),
            ..Default::default()
        };

        let mut model = execution_run::ActiveModel::default();
        nodes(1_000).apply_to(&mut model);
        assert!(matches!(model.nodes, Set(Some(_))));

        let mut model = execution_run::ActiveModel::default();
        nodes(10_000).apply_to(&mut model);
        assert_eq!(model.nodes, NotSet);
    }

    #[test]
    fn listings_without_nodes_select_null_in_their_place() {
        let statement = without_nodes(execution_run::Entity::find())
            .build(sea_orm::DatabaseBackend::Postgres)
            .to_string();
        assert!(statement.contains("NULL::jsonb AS \"nodes\""));
        assert!(!statement.contains("\"ExecutionRun\".\"nodes\""));
        assert!(statement.contains("\"ExecutionRun\".\"logsCount\""));
        assert!(statement.contains("\"ExecutionRun\".\"eventVersion\""));
    }

    #[test]
    fn log_meta_converts_timestamps_to_micros_and_parses_the_columns() {
        let meta = log_meta_from_run(&run(
            true,
            None,
            Some(serde_json::json!([["n1", 1], ["n2", 3]])),
        ));
        assert_eq!(meta.start, CREATED_MICROS as u64 + 5_000);
        assert_eq!(meta.end, CREATED_MICROS as u64 + 905_000);
        assert_eq!(meta.log_level, 3);
        assert_eq!(meta.version, "v1-2-3");
        assert_eq!(meta.event_id, "evt-1");
        assert_eq!(meta.node_id, "node-1");
        assert_eq!(meta.event_version.as_deref(), Some("1.0.3"));
        assert_eq!(
            meta.nodes,
            Some(vec![("n1".to_string(), 1), ("n2".to_string(), 3)])
        );
        assert_eq!(meta.logs, Some(7));
        assert!(meta.payload.is_empty());
        assert!(meta.is_remote);
    }

    #[test]
    fn log_meta_falls_back_to_row_timestamps_when_the_run_never_started() {
        let meta = log_meta_from_run(&run(false, None, None));
        assert_eq!(meta.start, CREATED_MICROS as u64);
        assert_eq!(meta.end, CREATED_MICROS as u64 + 905_000);
    }

    #[test]
    fn null_nodes_stay_unknown_rather_than_empty() {
        let meta = log_meta_from_run(&run(true, None, None));
        assert_eq!(meta.nodes, None);
        let meta = log_meta_from_run(&run(true, None, Some(serde_json::json!("garbage"))));
        assert_eq!(meta.nodes, None);
    }

    #[test]
    fn rejected_rows_keep_the_two_rejection_markers() {
        let meta = log_meta_from_run(&run(false, Some("rejected:payload"), None));
        assert_eq!(meta.start, CREATED_MICROS as u64);
        assert_eq!(meta.end, meta.start);
        assert_eq!(meta.nodes, Some(Vec::new()));

        let meta = log_meta_from_run(&run(
            true,
            Some("rejected:resolution"),
            Some(serde_json::json!([])),
        ));
        assert_eq!(meta.start, CREATED_MICROS as u64 + 5_000);
        assert_eq!(meta.end, meta.start);
        assert_eq!(meta.nodes, Some(Vec::new()));

        let meta = log_meta_from_run(&run(true, Some("node:abc"), None));
        assert_ne!(meta.end, meta.start);
        assert_eq!(meta.nodes, None);
    }
}

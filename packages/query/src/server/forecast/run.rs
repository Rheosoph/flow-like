//! One forecast: read the rows, prepare the model inputs, stage the checkpoint, run the engine.

use super::checkpoint::CheckpointCache;
use crate::protocol::forecast::ForecastModelSpec;
use crate::protocol::{ErrorCode, QueryShape, SurfaceSpec, ViewSpec, WireError};
use crate::server::ops::{Opened, failed, internal, invalid, surface_of, workbench_source};
use flow_like_catalog_ml::inspection::auto_training_tables::TableBudget;
use flow_like_catalog_ml::inspection::forecasting::studio::{
    self, StudioForecastModel, StudioForecastRequest, StudioForecastResult,
};
use flow_like_storage::databases::workbench::{
    ExecutionBudget, ExternalSchema, SqlQueryResult, WorkbenchView, prepare_workbench_query,
};
use flow_like_storage::lancedb::Connection;
use flow_like_storage_contracts::column_metadata::ColumnReader;
use flow_like_types::Value;
use std::time::{Duration, Instant};

/// How long a forecast waits for the one running before it.
const SLOT_WAIT: Duration = Duration::from_secs(30);

pub(super) async fn forecast(
    opened: &Opened,
    request: Value,
    shape: QueryShape,
    model: ForecastModelSpec,
    budget: &ExecutionBudget,
    started: Instant,
) -> Result<StudioForecastResult, WireError> {
    let request: StudioForecastRequest = serde_json::from_value(request)
        .map_err(|error| invalid(format!("The forecast request is malformed: {error}")))?;
    if request.model_bit_id != model.bit_id {
        return Err(invalid(
            "The forecast request names another model than the one the API authorized",
        ));
    }
    let provenance = provenance(&model)?;
    let table_budget = studio::forecast_budget(&request).map_err(input_or_failed)?;
    let QueryShape::Workbench { surface, views } = shape else {
        return Err(invalid("A forecast reads one app's workbench surface"));
    };
    let (connection, reader) = workbench_source(opened)?;
    let deadline = tokio::time::Instant::from_std(started + budget.timeout);
    let permit =
        studio::forecast_slot(time_left(deadline, "a forecast slot was free")?.min(SLOT_WAIT))
            .await
            .map_err(|error| WireError::new(ErrorCode::Unavailable, format!("{error:#}")))?;
    let reads = Reads {
        connection,
        reader,
        surface: &surface,
        views: &views,
        external: &opened.external,
        budget,
        table_budget: &table_budget,
        deadline,
    };
    let history = reads.rows(&request.sql, &request.params).await?;
    let future = match &request.future_sql {
        Some(sql) => Some(reads.rows(sql, &request.future_params).await?),
        None => None,
    };
    let prepared = studio::prepare_forecast(request, table_budget, history, future)
        .map_err(input_or_failed)?;
    let staged = CheckpointCache::from_env().stage(&model, deadline).await?;
    time_left(deadline, "the forecasting model was loaded")?;
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        prepared.forecast(staged.directory(), provenance)
    });
    tokio::time::timeout_at(deadline, task)
        .await
        .map_err(|_| {
            WireError::new(
                ErrorCode::Timeout,
                "The forecasting model did not finish within the query service's time limit; \
                 shorten the history, the horizon or the backtest",
            )
        })?
        .map_err(|error| internal("The forecast task failed", error))?
        .map_err(inference_error)
}

/// The engine's model description from the API's: Bit id, family, repository and revision.
fn provenance(model: &ForecastModelSpec) -> Result<StudioForecastModel, WireError> {
    let family = serde_json::from_value(Value::String(model.family.clone())).map_err(|_| {
        invalid(format!(
            "'{}' is not a forecasting model family this service knows",
            model.family
        ))
    })?;
    Ok(StudioForecastModel {
        bit_id: model.bit_id.clone(),
        family,
        repository: model.repository.clone(),
        revision: model.revision.clone(),
    })
}

/// Time left before `deadline`, or a timeout naming what had not happened yet.
pub(super) fn time_left(
    deadline: tokio::time::Instant,
    before: &str,
) -> Result<Duration, WireError> {
    let left = deadline.saturating_duration_since(tokio::time::Instant::now());
    if left.is_zero() {
        return Err(WireError::new(
            ErrorCode::Timeout,
            format!("The forecast ran out of time before {before}"),
        ));
    }
    Ok(left)
}

/// Input mistakes keep the engine's exact message; other failures are sorted like query errors.
fn input_or_failed(error: flow_like_types::Error) -> WireError {
    match studio::input_error_message(&error) {
        Some(message) => WireError::new(ErrorCode::InvalidRequest, message),
        None => failed(error),
    }
}

/// After the inputs passed, only a mismatch between them and the model is the caller's to fix.
fn inference_error(error: flow_like_types::Error) -> WireError {
    match studio::input_error_message(&error) {
        Some(message) => WireError::new(ErrorCode::InvalidRequest, message),
        None => WireError::new(
            ErrorCode::Internal,
            format!("Running the forecasting model failed: {error:#}"),
        ),
    }
}

/// The reads of one forecast: the app's workbench surface, the forecast's own row cap and
/// whatever is left of the request's time.
struct Reads<'a> {
    connection: &'a Connection,
    reader: &'a ColumnReader,
    surface: &'a SurfaceSpec,
    views: &'a [ViewSpec],
    external: &'a [ExternalSchema],
    budget: &'a ExecutionBudget,
    table_budget: &'a TableBudget,
    deadline: tokio::time::Instant,
}

impl Reads<'_> {
    async fn rows(&self, sql: &str, params: &Value) -> Result<SqlQueryResult, WireError> {
        let budget = ExecutionBudget {
            rows: self.table_budget.maximum_rows,
            memory_bytes: self.budget.memory_bytes,
            spill: self.budget.spill.clone(),
            timeout: time_left(self.deadline, "its rows were read")?,
        };
        let result = prepare_workbench_query(
            self.connection,
            surface_of(self.surface.clone()),
            self.views.iter().map(WorkbenchView::from).collect(),
            sql,
            params,
            self.reader,
            self.external,
            &budget,
        )
        .await
        .map_err(failed)?
        .collect()
        .await
        .map_err(failed)?;
        studio::checked_rows(result, self.table_budget).map_err(input_or_failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::forecast::ModelFile;
    use flow_like_catalog_ml::inspection::forecasting::studio::StudioForecastInputError;

    fn model(family: &str) -> ForecastModelSpec {
        ForecastModelSpec {
            bit_id: "chronos".to_string(),
            family: family.to_string(),
            repository: Some("amazon/chronos-2".to_string()),
            revision: Some("main".to_string()),
            files: vec![ModelFile {
                name: "config.json".to_string(),
                hash: "config".to_string(),
                verify: false,
                size: 10,
                url: "https://hub.example/config.json".to_string(),
            }],
        }
    }

    #[test]
    fn families_parse_by_their_engine_names() {
        let chronos = provenance(&model("chronos2")).unwrap();
        assert_eq!(chronos.bit_id, "chronos");
        assert_eq!(chronos.repository.as_deref(), Some("amazon/chronos-2"));
        assert_eq!(
            serde_json::to_value(provenance(&model("timesfm2_5")).unwrap().family).unwrap(),
            "timesfm2_5"
        );
        let unknown = provenance(&model("prophet")).unwrap_err();
        assert_eq!(unknown.code, ErrorCode::InvalidRequest);
        assert!(unknown.message.contains("prophet"));
    }

    #[test]
    fn input_mistakes_keep_their_message_and_other_failures_their_class() {
        let input: flow_like_types::Error =
            StudioForecastInputError("Forecast SQL returned no rows".to_string()).into();
        let mapped = input_or_failed(input);
        assert_eq!(mapped.code, ErrorCode::InvalidRequest);
        assert_eq!(mapped.message, "Forecast SQL returned no rows");

        let timed_out = input_or_failed(flow_like_types::anyhow!("SQL query timed out after 5ms"));
        assert_eq!(timed_out.code, ErrorCode::Timeout);

        let model_failure = inference_error(flow_like_types::anyhow!("missing tensor 'proj.w'"));
        assert_eq!(model_failure.code, ErrorCode::Internal);
        assert!(model_failure.message.contains("missing tensor"));

        let unsupported: flow_like_types::Error = StudioForecastInputError(
            "The selected model does not support the mapped covariates".to_string(),
        )
        .into();
        assert_eq!(inference_error(unsupported).code, ErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn deadlines_name_the_step_that_did_not_happen() {
        let passed = tokio::time::Instant::now() - Duration::from_millis(1);
        let error = time_left(passed, "its rows were read").unwrap_err();
        assert_eq!(error.code, ErrorCode::Timeout);
        assert!(error.message.ends_with("before its rows were read"));
        let ahead = tokio::time::Instant::now() + Duration::from_secs(5);
        assert!(time_left(ahead, "anything").unwrap() > Duration::from_secs(4));
    }
}

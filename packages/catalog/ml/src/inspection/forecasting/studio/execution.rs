use super::{preparation::*, *};
use crate::inspection::auto_training_tables::TemporalUnit;
use crate::inspection::forecasting::tables::preparation::PreparedForecast;
use flow_like::{
    bit::{Bit, BitTypes},
    state::FlowLikeState,
};
use flow_like_storage::{
    arrow_schema::{DataType, TimeUnit},
    databases::workbench::{SqlQueryResult, WorkbenchSurface, WorkbenchView, execute_readonly_sql},
    lancedb::Connection,
};
use flow_like_storage_contracts::column_metadata::ColumnReader;
use flow_like_types::{Result, anyhow, bail};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const SLOT_WAIT: Duration = Duration::from_secs(30);

fn input_error(error: impl std::fmt::Display) -> flow_like_types::Error {
    StudioForecastInputError(error.to_string()).into()
}

fn prediction_error(error: flow_like_ml_forecast::Error) -> flow_like_types::Error {
    match error {
        flow_like_ml_forecast::Error::Invalid(message) => input_error(message),
        error => error.into(),
    }
}

fn clone_surface(surface: &WorkbenchSurface) -> WorkbenchSurface {
    match surface {
        WorkbenchSurface::Native => WorkbenchSurface::Native,
        WorkbenchSurface::Overlay(value) => WorkbenchSurface::Overlay(value.clone()),
        WorkbenchSurface::RemoteOverlay(value) => WorkbenchSurface::RemoteOverlay(value.clone()),
    }
}

pub(super) async fn query(
    connection: &Connection,
    surface: &WorkbenchSurface,
    views: &[WorkbenchView],
    reader: &ColumnReader,
    sql: &str,
    params: &Value,
    budget: &TableBudget,
) -> Result<SqlQueryResult> {
    let views = views
        .iter()
        .map(|view| WorkbenchView {
            name: view.name.clone(),
            sql: view.sql.clone(),
        })
        .collect();
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        execute_readonly_sql(
            connection,
            clone_surface(surface),
            views,
            sql,
            params,
            Some(budget.maximum_rows),
            reader,
        ),
    )
    .await
    .map_err(|_| anyhow!("Forecast SQL preparation or execution exceeded 60 seconds"))??;
    checked_rows(result, budget)
}

/// Forecast input rows that are complete, nonempty and within the byte budget; anything else is
/// an input error.
pub fn checked_rows(result: SqlQueryResult, budget: &TableBudget) -> Result<SqlQueryResult> {
    if result.truncated {
        return Err(input_error(format!(
            "Forecast SQL returned more than {} rows; filter or aggregate the complete series before forecasting",
            budget.maximum_rows
        )));
    }
    if result.rows.is_empty() {
        return Err(input_error("Forecast SQL returned no rows"));
    }
    validate_serialized_size(&result.rows, budget.maximum_bytes).map_err(input_error)?;
    Ok(result)
}

/// The request's row, byte and tensor limits; a malformed request is an input error.
pub fn forecast_budget(request: &StudioForecastRequest) -> Result<TableBudget> {
    budget(request).map_err(input_error)
}

/// One forecast per process at a time: checkpoint loading and CPU inference bound memory and
/// cores across every app. Hold the permit until inference returns, even if the caller left.
pub async fn forecast_slot(wait: Duration) -> Result<OwnedSemaphorePermit> {
    static CONCURRENCY: OnceLock<Arc<Semaphore>> = OnceLock::new();
    tokio::time::timeout(
        wait,
        CONCURRENCY
            .get_or_init(|| Arc::new(Semaphore::new(1)))
            .clone()
            .acquire_owned(),
    )
    .await
    .map_err(|_| anyhow!("Forecasting is busy; retry after the active forecast completes"))?
    .map_err(|_| anyhow!("Forecast service is unavailable"))
}

fn numeric_type(kind: &DataType) -> bool {
    matches!(
        kind,
        DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float16
            | DataType::Float32
            | DataType::Float64
            | DataType::Null
    )
}

pub(super) fn temporal_units(
    result: &SqlQueryResult,
    mapping: &ForecastTableMapping,
    future: bool,
) -> Result<BTreeMap<String, TemporalUnit>> {
    let mut units = BTreeMap::new();
    for name in mapping.columns(future) {
        let matches = result
            .columns
            .iter()
            .filter(|column| column.name == name)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            bail!("Forecast SQL must return exactly one column named '{name}'");
        }
        let kind: DataType = matches[0].type_name.parse()?;
        if name == mapping.timestamp {
            let unit = match kind {
                DataType::Timestamp(TimeUnit::Second, _) => Some(TemporalUnit::Seconds),
                DataType::Timestamp(TimeUnit::Microsecond, _) => Some(TemporalUnit::Microseconds),
                DataType::Timestamp(TimeUnit::Nanosecond, _) => Some(TemporalUnit::Nanoseconds),
                DataType::Date32 => Some(TemporalUnit::Days),
                DataType::Timestamp(TimeUnit::Millisecond, _) | DataType::Date64 => {
                    Some(TemporalUnit::Milliseconds)
                }
                DataType::Int8
                | DataType::Int16
                | DataType::Int32
                | DataType::Int64
                | DataType::UInt8
                | DataType::UInt16
                | DataType::UInt32
                | DataType::UInt64
                | DataType::Float16
                | DataType::Float32
                | DataType::Float64
                | DataType::Utf8
                | DataType::LargeUtf8
                | DataType::Utf8View => None,
                _ => bail!(
                    "Timestamp '{name}' must be a native timestamp/date, date text, or numeric Unix epoch column"
                ),
            };
            if let Some(unit) = unit {
                units.insert(name, unit);
            }
        } else if mapping.series_id.as_ref() == Some(&name) {
            if !matches!(
                kind,
                DataType::Utf8
                    | DataType::LargeUtf8
                    | DataType::Utf8View
                    | DataType::Int8
                    | DataType::Int16
                    | DataType::Int32
                    | DataType::Int64
                    | DataType::UInt8
                    | DataType::UInt16
                    | DataType::UInt32
                    | DataType::UInt64
            ) {
                bail!("Series ID '{name}' must be a string or integer column");
            }
        } else if !numeric_type(&kind)
            && !matches!(
                kind,
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
            )
        {
            bail!(
                "Forecast column '{name}' must contain numbers or numeric text; cast other numeric types to DOUBLE in SQL"
            );
        }
    }
    Ok(units)
}

/// A forecast whose rows are read and validated, waiting for its checkpoint.
pub struct PreparedStudioForecast {
    request: StudioForecastRequest,
    budget: TableBudget,
    prepared: PreparedForecast,
    held: Option<PreparedForecast>,
    input_rows: usize,
}

/// Model inputs and the optional holdout from the rows of the request's SQL and future SQL,
/// before any checkpoint is fetched.
pub fn prepare_forecast(
    request: StudioForecastRequest,
    budget: TableBudget,
    history: SqlQueryResult,
    future: Option<SqlQueryResult>,
) -> Result<PreparedStudioForecast> {
    let units = temporal_units(&history, &request.mapping, false).map_err(input_error)?;
    let future_units = future
        .as_ref()
        .map(|result| temporal_units(result, &request.mapping, true))
        .transpose()
        .map_err(input_error)?;
    let input_rows = history.rows.len() + future.as_ref().map_or(0, |result| result.rows.len());
    let prepared = inputs(
        history.rows,
        &units,
        future.map(|result| result.rows).zip(future_units.as_ref()),
        &request,
        &budget,
    )
    .map_err(input_error)?;
    let held = backtest(&prepared, request.backtest_horizon, request.backtest_offset)
        .map_err(input_error)?;
    Ok(PreparedStudioForecast {
        request,
        budget,
        prepared,
        held,
        input_rows,
    })
}

impl PreparedStudioForecast {
    /// Loads the checkpoint in `directory` as `model.family`, runs the holdout and the forecast.
    /// CPU-bound: call it on a blocking thread.
    pub fn forecast(
        self,
        directory: &Path,
        model: StudioForecastModel,
    ) -> Result<StudioForecastResult> {
        let Self {
            request,
            budget,
            prepared,
            held,
            input_rows,
        } = self;
        let loaded = flow_like_ml_forecast::load_cpu(model.family, directory)?;
        let capabilities = loaded.capabilities();
        validate_capabilities(&prepared.request, &capabilities, &budget).map_err(input_error)?;
        validate_chart_budget(
            &prepared,
            &request.mapping,
            request.backtest_horizon,
            capabilities
                .quantile_levels
                .len()
                .max(request.quantile_levels.len()),
            &budget,
        )
        .map_err(input_error)?;
        if let Some(held) = &held {
            validate_capabilities(&held.request, &capabilities, &budget).map_err(input_error)?;
        }
        let holdout = match held {
            Some(held) => {
                let predicted = loaded.forecast(&held.request).map_err(prediction_error)?;
                Some((held, predicted))
            }
            None => None,
        };
        let result = loaded
            .forecast(&prepared.request)
            .map_err(prediction_error)?;
        finish(
            &request,
            prepared,
            result,
            holdout,
            model,
            capabilities,
            input_rows,
            &budget,
        )
        .map_err(input_error)
    }
}

pub(super) async fn run(
    state: Arc<FlowLikeState>,
    bit: Bit,
    connection: &Connection,
    surface: WorkbenchSurface,
    views: Vec<WorkbenchView>,
    reader: &ColumnReader,
    request: StudioForecastRequest,
) -> Result<StudioForecastResult> {
    if bit.id != request.model_bit_id || bit.bit_type != BitTypes::Forecasting {
        return Err(input_error(
            "The selected model must resolve to an authorized Forecasting Bit",
        ));
    }
    let budget = forecast_budget(&request)?;
    let permit = forecast_slot(SLOT_WAIT).await?;
    let history = query(
        connection,
        &surface,
        &views,
        reader,
        &request.sql,
        &request.params,
        &budget,
    )
    .await?;
    let future = match &request.future_sql {
        Some(sql) => Some(
            query(
                connection,
                &surface,
                &views,
                reader,
                sql,
                &request.future_params,
                &budget,
            )
            .await?,
        ),
        None => None,
    };
    let prepared = prepare_forecast(request, budget, history, future)?;
    let staged =
        crate::inspection::forecasting_bit::stage_bit(state, &bit, MAXIMUM_CHECKPOINT_BYTES, None)
            .await?;
    let model = StudioForecastModel {
        bit_id: bit.id,
        family: staged.handle.family,
        repository: staged.handle.repository,
        revision: staged.handle.revision,
    };
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        prepared.forecast(staged.directory.path(), model)
    })
    .await?
}

use super::tables::ForecastTableMapping;
use crate::inspection::auto_training_tables::TableBudget;
use flow_like_ml_forecast::{ForecastCapabilities, ForecastFamily, PointStatistic};
use flow_like_types::Value;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod saved;

/// SQL runs on the caller's authorized workbench surface, including its saved views.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StudioForecastRequest {
    pub model_bit_id: String,
    pub sql: String,
    #[serde(default = "empty_params")]
    pub params: Value,
    pub mapping: ForecastTableMapping,
    pub horizon: usize,
    #[serde(default)]
    pub quantile_levels: Vec<f32>,
    #[serde(default)]
    pub future_sql: Option<String>,
    #[serde(default = "empty_params")]
    pub future_params: Value,
    /// Hold out this many historical observations per series for a second forecast.
    #[serde(default)]
    pub backtest_horizon: usize,
    /// Observations after the evaluated window. Zero selects the trailing window.
    #[serde(default)]
    pub backtest_offset: usize,
    #[serde(default)]
    pub budget: TableBudget,
}

fn empty_params() -> Value {
    flow_like_types::json::json!({})
}

/// User-correctable forecast input failures safe for transport adapters to display.
#[derive(Debug)]
pub struct StudioForecastInputError(pub String);

impl std::fmt::Display for StudioForecastInputError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}
impl std::error::Error for StudioForecastInputError {}

pub fn input_error_message(error: &flow_like_types::Error) -> Option<&str> {
    error
        .downcast_ref::<StudioForecastInputError>()
        .map(|error| error.0.as_str())
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StudioForecastModel {
    pub bit_id: String,
    pub family: ForecastFamily,
    pub repository: Option<String>,
    pub revision: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StudioHistoryPoint {
    pub series_id: String,
    pub timestamp_ms: i64,
    pub target: String,
    /// Null identifies a missing observation, including in backtest metrics.
    pub value: Option<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StudioForecastPoint {
    pub series_id: String,
    pub timestamp_ms: i64,
    pub target: String,
    pub point: f32,
    /// Values follow StudioForecastResult.quantile_levels.
    pub quantiles: Vec<f32>,
    pub actual: Option<f32>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StudioCovariateKind {
    Past,
    Known,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StudioCovariatePoint {
    pub series_id: String,
    pub timestamp_ms: i64,
    pub name: String,
    pub kind: StudioCovariateKind,
    pub value: f32,
    pub future: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StudioBacktestMetric {
    pub series_id: String,
    pub target: String,
    pub observations: usize,
    pub mae: Option<f64>,
    pub rmse: Option<f64>,
    /// Percentage error, absent if no observations or any actual value is zero.
    pub mape: Option<f64>,
    /// Percentage absolute error weighted by actual magnitude; absent for a zero denominator.
    #[serde(default)]
    pub wape: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StudioBacktestResult {
    pub horizon: usize,
    #[serde(default)]
    pub offset: usize,
    pub forecasts: Vec<StudioForecastPoint>,
    pub metrics: Vec<StudioBacktestMetric>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StudioForecastResult {
    pub run_id: String,
    pub model: StudioForecastModel,
    pub capabilities: ForecastCapabilities,
    pub input_rows: usize,
    pub horizon: usize,
    pub quantile_levels: Vec<f32>,
    pub point_statistic: PointStatistic,
    pub history: Vec<StudioHistoryPoint>,
    pub forecasts: Vec<StudioForecastPoint>,
    pub covariates: Vec<StudioCovariatePoint>,
    pub backtest: Option<StudioBacktestResult>,
}

pub const fn inference_available() -> bool {
    cfg!(any(feature = "inference", feature = "forecasting"))
}

/// Largest checkpoint a forecast stages.
pub const MAXIMUM_CHECKPOINT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[cfg(any(feature = "inference", feature = "forecasting"))]
pub use execution::{
    PreparedStudioForecast, checked_rows, forecast_budget, forecast_slot, prepare_forecast,
};

#[cfg(any(
    feature = "studio",
    any(feature = "inference", feature = "forecasting")
))]
pub async fn run_forecast(
    state: std::sync::Arc<flow_like::state::FlowLikeState>,
    bit: flow_like::bit::Bit,
    connection: &flow_like_storage::lancedb::Connection,
    surface: flow_like_storage::databases::workbench::WorkbenchSurface,
    views: Vec<flow_like_storage::databases::workbench::WorkbenchView>,
    reader: &flow_like_storage_contracts::column_metadata::ColumnReader,
    request: StudioForecastRequest,
) -> flow_like_types::Result<StudioForecastResult> {
    #[cfg(any(feature = "inference", feature = "forecasting"))]
    {
        execution::run(state, bit, connection, surface, views, reader, request).await
    }
    #[cfg(not(any(feature = "inference", feature = "forecasting")))]
    {
        let _ = (state, bit, connection, surface, views, reader, request);
        Err(crate::inspection::published::inference_unavailable())
    }
}

#[cfg(any(feature = "inference", feature = "forecasting"))]
mod execution;
#[cfg(any(
    feature = "studio",
    feature = "inference",
    feature = "forecasting",
    test
))]
mod preparation;
#[cfg(test)]
mod tests;

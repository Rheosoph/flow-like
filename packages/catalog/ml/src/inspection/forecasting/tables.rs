use super::ForecastModelHandle;
use crate::inspection::auto_training_tables::TableBudget;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::NodeDBConnection;
use flow_like_ml_forecast::PointStatistic;
use flow_like_storage_contracts::database::DatabaseReference;
use flow_like_types::{Result, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ForecastTableMapping {
    /// Native timestamp/date, date text, or Unix epoch column. Precision must be whole milliseconds.
    pub timestamp: String,
    /// Auto detects text formats and epoch units. Omission retains integer-as-milliseconds compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_format: Option<ForecastTimestampFormat>,
    /// String or integer column identifying independent series. Omit for a single series.
    #[serde(default)]
    pub series_id: Option<String>,
    /// Numeric target columns modeled jointly within each series. Nulls are missing observations.
    pub targets: Vec<String>,
    #[serde(default)]
    pub past_covariates: Vec<String>,
    /// Numeric columns present in both source and future_source.
    #[serde(default)]
    pub known_covariates: Vec<String>,
    /// Positive spacing in milliseconds. Omit to infer it from each series' history.
    #[serde(default)]
    pub frequency_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ForecastTimestampFormat {
    Auto,
    UnixSeconds,
    UnixMilliseconds,
    UnixMicroseconds,
    UnixNanoseconds,
    DayFirst,
    MonthFirst,
}

#[cfg(any(
    feature = "studio",
    feature = "inference",
    feature = "forecasting",
    test
))]
mod timestamps;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ForecastTableMetadata {
    pub run_id: String,
    pub model: ForecastModelHandle,
    pub source: DatabaseReference,
    pub future_source: Option<DatabaseReference>,
    pub output: DatabaseReference,
    pub rows: usize,
    pub series: usize,
    pub horizon: usize,
    /// Quantile output column names mapped to their levels.
    pub quantile_columns: BTreeMap<String, f32>,
    pub point_statistic: PointStatistic,
}

#[cfg(any(
    feature = "studio",
    feature = "inference",
    feature = "forecasting",
    test
))]
pub(crate) mod preparation {
    use super::*;
    use crate::inspection::auto_training_tables::TemporalUnit;
    use flow_like_ml_forecast::{ForecastRequest, ForecastResult, ForecastSeries};
    use flow_like_types::{Value, anyhow};
    use std::collections::BTreeSet;

    #[derive(Debug, Clone)]
    pub(crate) struct PreparedForecast {
        pub request: ForecastRequest,
        pub targets: Vec<String>,
        pub clocks: Vec<(i64, i64)>,
    }

    impl ForecastTableMapping {
        pub(crate) fn columns(&self, future: bool) -> Vec<String> {
            let mut columns = vec![self.timestamp.clone()];
            columns.extend(self.series_id.iter().cloned());
            if !future {
                columns.extend(self.targets.iter().cloned());
                columns.extend(self.past_covariates.iter().cloned());
            }
            columns.extend(self.known_covariates.iter().cloned());
            columns
        }

        pub(crate) fn validate(&self) -> Result<()> {
            if self.targets.is_empty() || self.frequency_ms.is_some_and(|step| step <= 0) {
                return Err(anyhow!(
                    "Map at least one target and use a positive frequency_ms"
                ));
            }
            let columns = self.columns(false);
            let unique: BTreeSet<_> = columns.iter().collect();
            if unique.len() != columns.len()
                || columns.iter().any(|column| column.trim().is_empty())
            {
                return Err(anyhow!(
                    "Forecast column mappings must be nonempty and distinct"
                ));
            }
            Ok(())
        }
    }

    fn cell<'a>(row: &'a Value, column: &str) -> Result<&'a Value> {
        row.get(column)
            .ok_or_else(|| anyhow!("Mapped forecast column '{column}' is missing"))
    }

    fn numeric(row: &Value, column: &str, nullable: bool) -> Result<Option<f32>> {
        let value = cell(row, column)?;
        if value.is_null() && nullable {
            return Ok(None);
        }
        let value = value.as_f64().or_else(|| {
            static NUMBER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            let text = value.as_str()?.trim();
            let pattern = NUMBER.get_or_init(|| regex::Regex::new(r"^[+-]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?$").unwrap());
            pattern.is_match(text).then(|| text.parse::<f64>().ok()).flatten()
        }).ok_or_else(|| {
            anyhow!("Forecast column '{column}' must contain finite numbers or numeric text; remove currency symbols and thousands separators")
        })? as f32;
        if !value.is_finite() {
            return Err(anyhow!(
                "Forecast column '{column}' exceeds finite f32 values"
            ));
        }
        Ok(Some(value))
    }

    type Groups = BTreeMap<String, Vec<(i64, Value)>>;

    fn group_rows(
        rows: Vec<Value>,
        mapping: &ForecastTableMapping,
        units: &BTreeMap<String, TemporalUnit>,
    ) -> Result<(Groups, Option<bool>)> {
        let mut groups = Groups::new();
        let mut id_is_string = None;
        let timestamps = super::timestamps::parse_column(&rows, mapping, units)?;
        for (row, timestamp) in rows.into_iter().zip(timestamps) {
            let id = match &mapping.series_id {
                None => "series".into(),
                Some(column) => {
                    let value = cell(&row, column)?;
                    let is_string = value.is_string();
                    if id_is_string.is_some_and(|kind| kind != is_string) {
                        return Err(anyhow!(
                            "Series IDs must use one consistent string or integer type"
                        ));
                    }
                    id_is_string = Some(is_string);
                    match value {
                        Value::String(id) if !id.trim().is_empty() => id.clone(),
                        Value::Number(id) if id.is_i64() || id.is_u64() => id.to_string(),
                        _ => {
                            return Err(anyhow!(
                                "Series ID column '{column}' must contain nonempty strings or integers"
                            ));
                        }
                    }
                }
            };
            groups.entry(id).or_default().push((timestamp, row));
        }
        for (id, rows) in &mut groups {
            rows.sort_by_key(|(time, _)| *time);
            if rows.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                return Err(anyhow!("Series '{id}' contains duplicate timestamps"));
            }
        }
        Ok((groups, id_is_string))
    }

    pub(crate) fn future_timestamp(last: i64, frequency: i64, step: usize) -> Result<i64> {
        i64::try_from(step)
            .ok()
            .and_then(|step| frequency.checked_mul(step))
            .and_then(|offset| last.checked_add(offset))
            .ok_or_else(|| anyhow!("Forecast timestamps overflow epoch milliseconds"))
    }

    pub(crate) fn prepare(
        rows: Vec<Value>,
        units: &BTreeMap<String, TemporalUnit>,
        future: Option<(Vec<Value>, &BTreeMap<String, TemporalUnit>)>,
        mapping: &ForecastTableMapping,
        horizon: usize,
        quantile_levels: Vec<f32>,
        budget: &TableBudget,
    ) -> Result<PreparedForecast> {
        mapping.validate()?;
        budget.validate()?;
        if horizon == 0 || horizon > flow_like_ml_forecast::MAX_FORECAST_HORIZON {
            return Err(anyhow!(
                "Forecast horizon must be between 1 and {}",
                flow_like_ml_forecast::MAX_FORECAST_HORIZON
            ));
        }
        if mapping.known_covariates.is_empty() != future.is_none() {
            return Err(anyhow!(
                "Provide future_source exactly when known_covariates are mapped"
            ));
        }
        let count = rows
            .len()
            .checked_add(future.as_ref().map_or(0, |(rows, _)| rows.len()))
            .ok_or_else(|| anyhow!("Forecast input row count overflows"))?;
        if count > budget.maximum_rows {
            return Err(anyhow!(
                "Combined history and future rows exceed the table row budget"
            ));
        }
        let (groups, history_id_kind) = group_rows(rows, mapping, units)?;
        let (mut future, future_id_kind) = match future {
            Some((rows, units)) => group_rows(rows, mapping, units)?,
            None => (Groups::new(), None),
        };
        if let (Some(history_kind), Some(future_kind)) = (history_id_kind, future_id_kind)
            && history_kind != future_kind
        {
            return Err(anyhow!(
                "History and future_source series IDs must use the same string or integer type"
            ));
        }
        let output_rows = groups
            .len()
            .checked_mul(mapping.targets.len())
            .and_then(|n| n.checked_mul(horizon))
            .ok_or_else(|| anyhow!("Forecast output row count overflows"))?;
        if output_rows > budget.maximum_rows {
            return Err(anyhow!("Forecast output exceeds the table row budget"));
        }
        let mut series = Vec::with_capacity(groups.len());
        let mut clocks = Vec::with_capacity(groups.len());
        let mut elements = 0usize;
        for (id, rows) in groups {
            let frequency = match mapping.frequency_ms {
                Some(frequency) => frequency,
                None if rows.len() > 1 => rows[1]
                    .0
                    .checked_sub(rows[0].0)
                    .ok_or_else(|| anyhow!("Series '{id}' timestamp spacing overflows"))?,
                None => {
                    return Err(anyhow!(
                        "Series '{id}' needs two historical rows or an explicit frequency_ms"
                    ));
                }
            };
            if frequency <= 0
                || rows
                    .windows(2)
                    .any(|pair| pair[1].0.checked_sub(pair[0].0) != Some(frequency))
            {
                return Err(anyhow!(
                    "Series '{id}' must have regular timestamp spacing; fill missing rows explicitly before forecasting"
                ));
            }
            let last = rows.last().expect("groups contain rows").0;
            future_timestamp(last, frequency, horizon)?;
            let columns = mapping
                .targets
                .len()
                .checked_mul(2)
                .and_then(|n| n.checked_add(mapping.past_covariates.len()))
                .and_then(|n| n.checked_add(mapping.known_covariates.len()))
                .ok_or_else(|| anyhow!("Forecast input dimensions overflow"))?;
            elements = rows
                .len()
                .checked_mul(columns)
                .and_then(|n| {
                    mapping
                        .known_covariates
                        .len()
                        .checked_mul(horizon)
                        .and_then(|future| n.checked_add(future))
                })
                .and_then(|n| elements.checked_add(n))
                .ok_or_else(|| anyhow!("Forecast input dimensions overflow"))?;
            if elements > budget.maximum_tensor_elements {
                return Err(anyhow!(
                    "Forecast values and observation masks exceed the tensor budget"
                ));
            }
            let mut targets = Vec::with_capacity(mapping.targets.len());
            let mut masks = Vec::with_capacity(mapping.targets.len());
            for column in &mapping.targets {
                let values = rows
                    .iter()
                    .map(|(_, row)| numeric(row, column, true))
                    .collect::<Result<Vec<_>>>()?;
                masks.push(values.iter().map(Option::is_some).collect());
                targets.push(values.into_iter().map(|v| v.unwrap_or(0.0)).collect());
            }
            let past_covariates = mapping
                .past_covariates
                .iter()
                .map(|column| {
                    rows.iter()
                        .map(|(_, row)| {
                            numeric(row, column, false).map(|v| v.expect("non-null numeric"))
                        })
                        .collect()
                })
                .collect::<Result<Vec<Vec<_>>>>()?;
            let future_rows = if mapping.known_covariates.is_empty() {
                vec![]
            } else {
                let future_rows = future
                    .remove(&id)
                    .ok_or_else(|| anyhow!("future_source has no rows for series '{id}'"))?;
                if future_rows.len() != horizon {
                    return Err(anyhow!(
                        "future_source must contain exactly {horizon} rows for series '{id}'"
                    ));
                }
                for (step, (time, _)) in future_rows.iter().enumerate() {
                    if *time != future_timestamp(last, frequency, step + 1)? {
                        return Err(anyhow!(
                            "future_source timestamps do not continue series '{id}' at its regular interval"
                        ));
                    }
                }
                future_rows
            };
            let known_covariates = mapping
                .known_covariates
                .iter()
                .map(|column| {
                    rows.iter()
                        .chain(&future_rows)
                        .map(|(_, row)| {
                            numeric(row, column, false).map(|v| v.expect("non-null numeric"))
                        })
                        .collect()
                })
                .collect::<Result<Vec<Vec<_>>>>()?;
            series.push(ForecastSeries {
                id,
                targets,
                observed_mask: Some(masks),
                past_covariates,
                known_covariates,
            });
            clocks.push((last, frequency));
        }
        if !future.is_empty() {
            return Err(anyhow!(
                "future_source contains series absent from the history"
            ));
        }
        let request = ForecastRequest {
            series,
            horizon,
            quantile_levels,
        };
        request.validate()?;
        Ok(PreparedForecast {
            request,
            targets: mapping.targets.clone(),
            clocks,
        })
    }

    pub(super) fn quantile_columns(levels: &[f32]) -> BTreeMap<String, f32> {
        levels
            .iter()
            .map(|level| (format!("q_{}", level.to_string().replace('.', "_")), *level))
            .collect()
    }

    pub(super) fn output_rows(
        prepared: &PreparedForecast,
        result: &ForecastResult,
        model: &ForecastModelHandle,
        run_id: &str,
        budget: &TableBudget,
    ) -> Result<Vec<Value>> {
        result.validate(&prepared.request)?;
        validate_output_budget(&prepared.request, result.quantile_levels.len(), budget)?;
        let mut rows = Vec::new();
        let mut bytes = 0usize;
        let names = result
            .quantile_levels
            .iter()
            .map(|level| format!("q_{}", level.to_string().replace('.', "_")))
            .collect::<Vec<_>>();
        for (series_index, series) in result.series.iter().enumerate() {
            let (last, frequency) = prepared.clocks[series_index];
            for (target_index, target) in prepared.targets.iter().enumerate() {
                for step in 0..prepared.request.horizon {
                    let mut row = json!({
                        "sample_id": format!("{run_id}:{series_index}:{target_index}:{step}"),
                        "run_id": run_id,
                        "series_id": series.id,
                        "timestamp_ms": future_timestamp(last, frequency, step + 1)?,
                        "target": target,
                        "point": series.point[target_index][step],
                        "point_statistic": result.point_statistic,
                        "model_family": model.family,
                        "model_repository": model.repository.as_deref().unwrap_or(""),
                        "model_revision": model.revision.as_deref().unwrap_or(""),
                        "model_bit_id": model.bit_id.as_deref().unwrap_or(""),
                    });
                    for (name, value) in names.iter().zip(&series.quantiles[target_index][step]) {
                        row[name] = json!(value);
                    }
                    bytes = bytes
                        .checked_add(flow_like_types::json::to_vec(&row)?.len())
                        .ok_or_else(|| anyhow!("Forecast output byte count overflows"))?;
                    if bytes > budget.maximum_bytes {
                        return Err(anyhow!("Forecast output exceeds the table byte budget"));
                    }
                    rows.push(row);
                }
            }
        }
        Ok(rows)
    }

    pub(crate) fn validate_output_budget(
        request: &ForecastRequest,
        native_quantiles: usize,
        budget: &TableBudget,
    ) -> Result<()> {
        request.validate_for_quantiles(native_quantiles)?;
        let targets = request
            .series
            .iter()
            .try_fold(0usize, |n, series| n.checked_add(series.targets.len()))
            .ok_or_else(|| anyhow!("Forecast target count overflows"))?;
        let rows = targets
            .checked_mul(request.horizon)
            .ok_or_else(|| anyhow!("Forecast output dimensions overflow"))?;
        let elements = rows
            .checked_mul(
                native_quantiles
                    .max(request.quantile_levels.len())
                    .saturating_add(1),
            )
            .ok_or_else(|| anyhow!("Forecast output dimensions overflow"))?;
        if rows > budget.maximum_rows || elements > budget.maximum_tensor_elements {
            return Err(anyhow!(
                "Forecast output exceeds the table row or tensor budget"
            ));
        }
        Ok(())
    }
}

#[cfg(any(
    any(feature = "inference", feature = "forecasting"),
    all(test, feature = "execute")
))]
fn validate_destination_schema(
    schema: &flow_like_storage::arrow_schema::Schema,
    row: &flow_like_types::Value,
) -> Result<()> {
    use flow_like_storage::arrow_schema::DataType;
    use flow_like_types::anyhow;
    for column in row
        .as_object()
        .ok_or_else(|| anyhow!("Forecast rows must be objects"))?
        .keys()
    {
        let field = schema.field_with_name(column).map_err(|_| anyhow!(
            "Forecast destination lacks output column '{column}'. Omit destination to create a table with the complete forecast schema"
        ))?;
        let compatible = if column == "timestamp_ms" {
            matches!(field.data_type(), DataType::Int64 | DataType::UInt64)
        } else if column == "point" || column.starts_with("q_") {
            matches!(field.data_type(), DataType::Float32 | DataType::Float64)
        } else {
            matches!(
                field.data_type(),
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
            )
        };
        if !compatible {
            return Err(anyhow!(
                "Forecast destination column '{column}' has incompatible type {}. Use strings for IDs/provenance, floating-point values for forecasts and a 64-bit integer for timestamp_ms",
                field.data_type()
            ));
        }
    }
    Ok(())
}

#[cfg(any(
    any(feature = "inference", feature = "forecasting"),
    all(test, feature = "execute")
))]
async fn destination_reference(
    context: &mut ExecutionContext,
    destination: Option<&NodeDBConnection>,
    source: &crate::inspection::auto_training_tables::PinnedTableSource,
    future: Option<&crate::inspection::auto_training_tables::PinnedTableSource>,
) -> Result<Option<DatabaseReference>> {
    let Some(destination) = destination else {
        return Ok(None);
    };
    let cached = destination.load(context).await?;
    let guard = cached.flushed_read().await?;
    let store = guard.inner();
    store.ensure_writable()?;
    let reference = store.reference().await?;
    let connection = flow_like_ml_core::content_digest(store.connection()?.uri().as_bytes());
    for input in std::iter::once(source).chain(future) {
        if input.reference.table == reference.table
            && input
                .locator
                .as_ref()
                .is_some_and(|locator| locator.connection_digest == connection)
        {
            return Err(flow_like_types::anyhow!(
                "Forecast destination must differ from the history and future covariate tables"
            ));
        }
    }
    Ok(Some(reference))
}

#[cfg(any(feature = "inference", feature = "forecasting"))]
async fn optional_table(context: &ExecutionContext, pin: &str) -> Result<Option<NodeDBConnection>> {
    let value: flow_like_types::Value = context.evaluate_pin(pin).await?;
    if value.is_null() || value.as_object().is_some_and(|object| object.is_empty()) {
        Ok(None)
    } else {
        Ok(Some(flow_like_types::json::from_value(value)?))
    }
}

#[cfg(any(feature = "inference", feature = "forecasting"))]
async fn run_table_forecast(
    context: &mut ExecutionContext,
) -> Result<(NodeDBConnection, ForecastTableMetadata)> {
    use crate::inspection::auto_training_tables::{
        TableWritePolicy, pin_table_source, read_columns, write_experiment_rows,
    };
    use flow_like_storage::databases::vector::VectorStore;
    use flow_like_types::anyhow;
    let source: NodeDBConnection = context.evaluate_pin("source").await?;
    let model: ForecastModelHandle = context.evaluate_pin("model").await?;
    let mapping: ForecastTableMapping = context.evaluate_pin("mapping").await?;
    let horizon: usize = context.evaluate_pin("horizon").await?;
    let quantiles: Vec<f32> = context.evaluate_pin("quantile_levels").await?;
    let future_source = optional_table(context, "future_source").await?;
    let destination = optional_table(context, "destination").await?;
    let budget: TableBudget = context.evaluate_pin("budget").await?;
    budget.validate()?;
    mapping.validate()?;
    let capabilities = super::model(context, &model).await?.capabilities.clone();
    if (!mapping.past_covariates.is_empty() && !capabilities.past_covariates)
        || (!mapping.known_covariates.is_empty() && !capabilities.known_covariates)
    {
        return Err(anyhow!(
            "The loaded model does not support the mapped covariates"
        ));
    }
    let source = pin_table_source(context, &source).await?;
    let future = match &future_source {
        Some(future) => Some(pin_table_source(context, future).await?),
        None => None,
    };
    let expected_destination =
        destination_reference(context, destination.as_ref(), &source, future.as_ref()).await?;
    let rows = read_columns(context, &source, &budget, &mapping.columns(false)).await?;
    let future_rows = match &future {
        Some(future) => Some(read_columns(context, future, &budget, &mapping.columns(true)).await?),
        None => None,
    };
    let bytes = rows
        .iter()
        .chain(future_rows.iter().flatten())
        .try_fold(0usize, |n, row| {
            n.checked_add(flow_like_types::json::to_vec(row)?.len())
                .ok_or_else(|| anyhow!("Forecast input bytes overflow"))
        })?;
    if bytes > budget.maximum_bytes {
        return Err(anyhow!(
            "Combined history and future data exceed the table byte budget"
        ));
    }
    let prepared = preparation::prepare(
        rows,
        &source.temporal_units,
        future_rows.zip(future.as_ref().map(|source| &source.temporal_units)),
        &mapping,
        horizon,
        quantiles,
        &budget,
    )?;
    preparation::validate_output_budget(
        &prepared.request,
        capabilities.quantile_levels.len(),
        &budget,
    )?;
    let result = super::forecast_values(context, &model, prepared.request.clone()).await?;
    let run_id = flow_like_types::create_id();
    let rows = preparation::output_rows(&prepared, &result, &model, &run_id, &budget)?;
    if let Some(destination) = &destination {
        let cached = destination.load(context).await?;
        let schema = cached.db.read().await.schema().await?;
        validate_destination_schema(&schema, &rows[0])?;
    }
    let policy = TableWritePolicy {
        table_prefix: Some("ml_forecast".into()),
        allowed_destination: destination,
        expected_destination,
        maximum_rows: budget.maximum_rows,
        maximum_bytes: budget.maximum_bytes,
        ..Default::default()
    };
    let output =
        write_experiment_rows(context, &source, rows, &run_id, "forecast", "", &policy).await?;
    let metadata = ForecastTableMetadata {
        run_id,
        model,
        source: source.reference,
        future_source: future.map(|source| source.reference),
        output: output.reference,
        rows: output.rows,
        series: prepared.request.series.len(),
        horizon,
        quantile_columns: preparation::quantile_columns(&result.quantile_levels),
        point_statistic: result.point_statistic,
    };
    Ok((output.database, metadata))
}

#[crate::register_node]
#[derive(Default)]
pub struct ForecastNode;

#[flow_like_types::async_trait]
impl NodeLogic for ForecastNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ml_forecast",
            "Forecast",
            "Forecast regularly spaced series from a table and write future points and quantiles to a table",
            "AI/ML/Forecasting",
        );
        node.set_flowscript_name("inspection", "forecast");
        node.add_icon("/flow/icons/chart-network.svg");
        node.set_long_running(true);
        node.add_input_pin(
            "exec_in",
            "Input",
            "Run forecasting",
            VariableType::Execution,
        );
        node.add_input_pin(
            "source",
            "Source Table",
            "Historical rows with timestamps, targets and optional covariates",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "model",
            "Model",
            "Loaded forecasting model",
            VariableType::Struct,
        )
        .set_schema::<ForecastModelHandle>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "mapping",
            "Column Mapping",
            "Timestamp, series ID, target and covariate columns; no automatic resampling",
            VariableType::Struct,
        )
        .set_schema::<ForecastTableMapping>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "horizon",
            "Horizon",
            "Number of future steps at the historical interval",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(24)));
        node.add_input_pin(
            "quantile_levels",
            "Quantile Levels",
            "Increasing quantiles between zero and one; empty selects the model's native levels",
            VariableType::Float,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));
        node.add_input_pin("future_source", "Future Covariates", "Required for known covariates: exactly horizon future rows per series, with the same timestamp and ID columns", VariableType::Struct)
            .set_schema::<NodeDBConnection>()
            .set_options(PinOptions::new().set_optional(true).set_enforce_schema(true).build())
            .set_default_value(Some(json!(null)));
        node.add_input_pin("destination", "Destination Table", "Optional writable table with the complete forecast schema; otherwise create a fresh table beside the source", VariableType::Struct)
            .set_schema::<NodeDBConnection>()
            .set_options(PinOptions::new().set_optional(true).set_enforce_schema(true).build())
            .set_default_value(Some(json!(null)));
        node.add_input_pin("budget", "Table Budget", "Limits on combined input rows, input bytes, output rows, output bytes and tensor elements", VariableType::Struct)
            .set_schema::<TableBudget>().set_default_value(Some(json!(TableBudget::default())));
        node.add_output_pin(
            "exec_out",
            "Done",
            "Forecast table is ready",
            VariableType::Execution,
        );
        node.add_output_pin("table", "Forecast Table", "Pinned table with series_id, timestamp_ms, target, point, q_* quantiles and model provenance", VariableType::Struct)
            .set_schema::<NodeDBConnection>();
        node.add_output_pin(
            "metadata",
            "Metadata",
            "Source/output versions, model provenance, row counts and quantile column names",
            VariableType::Struct,
        )
        .set_schema::<ForecastTableMetadata>();
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(any(feature = "inference", feature = "forecasting"))]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let (table, metadata) = run_table_forecast(context).await?;
            context.set_pin_value("table", json!(table)).await?;
            context.set_pin_value("metadata", json!(metadata)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(any(feature = "inference", feature = "forecasting")))]
        {
            let _ = context;
            Err(crate::inspection::published::inference_unavailable())
        }
    }
}

#[cfg(test)]
mod tests;

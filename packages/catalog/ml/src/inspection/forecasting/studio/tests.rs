use super::{preparation::*, *};
use crate::inspection::forecasting::tables::preparation::PreparedForecast;
use flow_like_ml_forecast::{ForecastResult, SeriesForecast};
use flow_like_types::json::json;
use std::collections::BTreeMap;

fn request() -> StudioForecastRequest {
    StudioForecastRequest {
        model_bit_id: "forecast-model".into(),
        sql: "SELECT * FROM history".into(),
        params: json!({}),
        mapping: ForecastTableMapping {
            timestamp: "time".into(),
            timestamp_format: None,
            series_id: Some("id".into()),
            targets: vec!["target".into()],
            past_covariates: vec!["weather".into()],
            known_covariates: vec!["price".into()],
            frequency_ms: None,
        },
        horizon: 2,
        quantile_levels: vec![0.1, 0.5, 0.9],
        future_sql: Some("SELECT * FROM future".into()),
        future_params: json!({}),
        backtest_horizon: 2,
        backtest_offset: 0,
        budget: TableBudget::default(),
    }
}

fn prepared(request: &StudioForecastRequest) -> PreparedForecast {
    let rows = vec![
        json!({"id":"a","time":30,"target":3,"weather":13,"price":7}),
        json!({"id":"a","time":10,"target":1,"weather":11,"price":5}),
        json!({"id":"a","time":40,"target":null,"weather":14,"price":8}),
        json!({"id":"a","time":20,"target":2,"weather":12,"price":6}),
    ];
    let future = vec![
        json!({"id":"a","time":60,"price":10}),
        json!({"id":"a","time":50,"price":9}),
    ];
    inputs(
        rows,
        &BTreeMap::new(),
        Some((future, &BTreeMap::new())),
        request,
        &budget(request).unwrap(),
    )
    .unwrap()
}

fn capabilities() -> ForecastCapabilities {
    ForecastCapabilities {
        family: ForecastFamily::Chronos2,
        max_context: 4,
        max_horizon: Some(16),
        native_horizon: 16,
        quantile_levels: vec![0.1, 0.5, 0.9],
        joint_multivariate: true,
        past_covariates: true,
        known_covariates: true,
    }
}

fn predicted(prepared: &PreparedForecast) -> ForecastResult {
    ForecastResult {
        quantile_levels: vec![0.1, 0.5, 0.9],
        point_statistic: PointStatistic::Median,
        series: prepared
            .request
            .series
            .iter()
            .map(|s| SeriesForecast {
                id: s.id.clone(),
                point: vec![vec![2.0; prepared.request.horizon]; s.targets.len()],
                quantiles: vec![
                    vec![vec![1.0, 2.0, 3.0]; prepared.request.horizon];
                    s.targets.len()
                ],
            })
            .collect(),
    }
}

fn finish_fixture(
    prepared: PreparedForecast,
    request: &StudioForecastRequest,
) -> StudioForecastResult {
    let held = backtest(&prepared, request.backtest_horizon, request.backtest_offset)
        .unwrap()
        .unwrap();
    let result = predicted(&prepared);
    let held_result = predicted(&held);
    finish(
        request,
        prepared,
        result,
        Some((held, held_result)),
        StudioForecastModel {
            bit_id: "forecast-model".into(),
            family: ForecastFamily::Chronos2,
            repository: None,
            revision: None,
        },
        capabilities(),
        6,
        &budget(request).unwrap(),
    )
    .unwrap()
}

#[test]
fn holdout_excludes_targets_and_past_covariates_but_retains_known_covariates() {
    let request = request();
    let mut prepared = prepared(&request);
    let held = backtest(&prepared, 2, 0).unwrap().unwrap();
    assert_eq!(held.request.series[0].targets, [vec![1.0, 2.0]]);
    assert_eq!(held.request.series[0].past_covariates, [vec![11.0, 12.0]]);
    assert_eq!(
        held.request.series[0].known_covariates,
        [vec![5.0, 6.0, 7.0, 8.0]]
    );
    assert_eq!(held.clocks, [(20, 10)]);
    prepared.request.series[0].targets[0][2] = 999.0;
    prepared.request.series[0].past_covariates[0][2] = 999.0;
    assert_eq!(
        flow_like_types::json::to_value(&held.request).unwrap(),
        flow_like_types::json::to_value(backtest(&prepared, 2, 0).unwrap().unwrap().request)
            .unwrap()
    );
    assert!(backtest(&prepared, 3, 0).is_err());
    assert!(backtest(&prepared, 0, 0).unwrap().is_none());
}

fn historical_input(request: &StudioForecastRequest) -> PreparedForecast {
    let values = [
        Some(1),
        Some(2),
        Some(3),
        None,
        Some(20),
        Some(30),
        Some(5000),
        Some(6000),
    ];
    let rows = values
        .into_iter()
        .enumerate()
        .map(|(step, target)| {
            json!({"id":"a","time":10*(step+1),"target":target,"weather":11+step,"price":5+step})
        })
        .collect();
    inputs(
        rows,
        &BTreeMap::new(),
        Some((
            vec![
                json!({"id":"a","time":90,"price":13}),
                json!({"id":"a","time":100,"price":14}),
            ],
            &BTreeMap::new(),
        )),
        request,
        &budget(request).unwrap(),
    )
    .unwrap()
}

#[test]
fn historical_holdout_isolates_targets_past_covariates_and_later_known_values() {
    let mut request = request();
    request.backtest_offset = 2;
    let mut input = historical_input(&request);
    let original = flow_like_types::json::to_value(&input.request).unwrap();
    let held = backtest(&input, 2, 2).unwrap().unwrap();
    assert_eq!(
        flow_like_types::json::to_value(&input.request).unwrap(),
        original
    );
    assert_eq!(held.request.series[0].targets, [vec![1.0, 2.0, 3.0, 0.0]]);
    assert_eq!(
        held.request.series[0].observed_mask,
        Some(vec![vec![true, true, true, false]])
    );
    assert_eq!(
        held.request.series[0].past_covariates,
        [vec![11.0, 12.0, 13.0, 14.0]]
    );
    assert_eq!(
        held.request.series[0].known_covariates,
        [vec![5.0, 6.0, 7.0, 8.0, 9.0, 10.0]]
    );
    assert_eq!(held.clocks, [(40, 10)]);

    for step in 4..8 {
        input.request.series[0].targets[0][step] = 999.0;
        input.request.series[0].past_covariates[0][step] = 999.0;
        input.request.series[0].observed_mask.as_mut().unwrap()[0][step] = false;
    }
    for step in 6..10 {
        input.request.series[0].known_covariates[0][step] = 999.0;
    }
    assert_eq!(
        flow_like_types::json::to_value(&held.request).unwrap(),
        flow_like_types::json::to_value(backtest(&input, 2, 2).unwrap().unwrap().request).unwrap()
    );
    input.request.series[0].known_covariates[0][4] = 123.0;
    assert_eq!(
        backtest(&input, 2, 2).unwrap().unwrap().request.series[0].known_covariates[0][4],
        123.0
    );
}

#[test]
fn historical_holdout_metrics_use_the_exact_window_and_preserve_future_forecasts() {
    let mut request = request();
    request.backtest_offset = 2;
    let input = historical_input(&request);
    let output = finish_fixture(input.clone(), &request);
    assert_eq!(
        output
            .history
            .iter()
            .map(|point| point.timestamp_ms)
            .collect::<Vec<_>>(),
        [10, 20, 30, 40, 50, 60, 70, 80]
    );
    assert_eq!(
        output
            .forecasts
            .iter()
            .map(|point| point.timestamp_ms)
            .collect::<Vec<_>>(),
        [90, 100]
    );
    let held = output.backtest.as_ref().unwrap();
    assert_eq!(held.offset, 2);
    assert_eq!(
        held.forecasts
            .iter()
            .map(|point| (point.timestamp_ms, point.actual))
            .collect::<Vec<_>>(),
        [(50, Some(20.0)), (60, Some(30.0))]
    );
    let metric = &held.metrics[0];
    assert_eq!(metric.observations, 2);
    assert_eq!(metric.mae, Some(23.0));
    assert_eq!(metric.rmse, Some(554.0_f64.sqrt()));
    assert!((metric.mape.unwrap() - 100.0 * (18.0 / 20.0 + 28.0 / 30.0) / 2.0).abs() < 1e-10);
    assert_eq!(metric.wape, Some(92.0));

    let mut later_actuals_changed = input.clone();
    later_actuals_changed.request.series[0].targets[0][6..].fill(-1_000_000.0);
    let changed = finish_fixture(later_actuals_changed, &request);
    assert_eq!(
        flow_like_types::json::to_value(changed.backtest).unwrap(),
        flow_like_types::json::to_value(&output.backtest).unwrap()
    );
    request.backtest_offset = 0;
    let trailing = finish_fixture(input, &request);
    assert_eq!(
        flow_like_types::json::to_value(trailing.forecasts).unwrap(),
        flow_like_types::json::to_value(output.forecasts).unwrap()
    );
    assert_eq!(
        flow_like_types::json::to_value(trailing.history).unwrap(),
        flow_like_types::json::to_value(output.history).unwrap()
    );
    assert_eq!(
        flow_like_types::json::to_value(trailing.covariates).unwrap(),
        flow_like_types::json::to_value(output.covariates).unwrap()
    );
}

#[test]
fn historical_holdout_rejects_offsets_without_training_history_or_evaluation() {
    let mut request = request();
    let input = historical_input(&request);
    assert_eq!(
        backtest(&input, 2, 4).unwrap().unwrap().request.series[0].targets[0].len(),
        2
    );
    assert!(backtest(&input, 2, 5).is_err());
    assert!(backtest(&input, 2, usize::MAX).is_err());
    assert!(backtest(&input, 0, 1).is_err());
    assert!(backtest(&input, 0, 0).unwrap().is_none());
    request.backtest_offset = MAXIMUM_ROWS + 1;
    assert!(budget(&request).is_err());
    request.backtest_offset = 1;
    request.backtest_horizon = 0;
    assert!(budget(&request).is_err());
    request.backtest_offset = 0;
    assert!(budget(&request).is_ok());
}

#[test]
fn legacy_requests_and_results_default_to_trailing_holdout() {
    let request = request();
    let mut legacy = flow_like_types::json::to_value(&request).unwrap();
    legacy.as_object_mut().unwrap().remove("backtest_offset");
    let decoded: StudioForecastRequest = flow_like_types::json::from_value(legacy).unwrap();
    assert_eq!(decoded.backtest_offset, 0);
    let output = finish_fixture(prepared(&decoded), &decoded);
    let held = output.backtest.as_ref().unwrap();
    assert_eq!(held.offset, 0);
    assert_eq!(
        held.forecasts
            .iter()
            .map(|point| point.timestamp_ms)
            .collect::<Vec<_>>(),
        [30, 40]
    );
    let mut legacy = flow_like_types::json::to_value(&output).unwrap();
    let held = legacy["backtest"].as_object_mut().unwrap();
    held.remove("offset");
    held.get_mut("metrics").unwrap()[0]
        .as_object_mut()
        .unwrap()
        .remove("wape");
    let decoded: StudioForecastResult = flow_like_types::json::from_value(legacy).unwrap();
    let held = decoded.backtest.unwrap();
    assert_eq!(held.offset, 0);
    assert_eq!(held.metrics[0].wape, None);
}

#[test]
fn chart_tracks_preserve_nulls_clocks_covariates_and_holdout_metrics() {
    let request = request();
    let output = finish_fixture(prepared(&request), &request);
    assert_eq!(
        output
            .history
            .iter()
            .map(|p| p.timestamp_ms)
            .collect::<Vec<_>>(),
        [10, 20, 30, 40]
    );
    assert_eq!(output.history[3].value, None);
    assert_eq!(
        output
            .forecasts
            .iter()
            .map(|p| p.timestamp_ms)
            .collect::<Vec<_>>(),
        [50, 60]
    );
    assert!(output.forecasts.iter().all(|p| p.actual.is_none()));
    assert_eq!(output.covariates.len(), 10);
    let future = output
        .covariates
        .iter()
        .filter(|p| p.future)
        .collect::<Vec<_>>();
    assert_eq!(future.len(), 2);
    assert_eq!((future[0].timestamp_ms, future[0].value), (50, 9.0));
    let held = output.backtest.unwrap();
    assert_eq!(
        held.forecasts.iter().map(|p| p.actual).collect::<Vec<_>>(),
        [Some(3.0), None]
    );
    assert_eq!(
        held.forecasts
            .iter()
            .map(|p| p.timestamp_ms)
            .collect::<Vec<_>>(),
        [30, 40]
    );
    let metric = &held.metrics[0];
    assert_eq!(metric.observations, 1);
    assert_eq!(metric.mae, Some(1.0));
    assert_eq!(metric.rmse, Some(1.0));
    assert!((metric.mape.unwrap() - 100.0 / 3.0).abs() < 1e-10);
    assert!((metric.wape.unwrap() - 100.0 / 3.0).abs() < 1e-10);
}

#[test]
fn holdout_metrics_handle_zero_and_missing_actuals_without_nonfinite_values() {
    let request = request();
    let mut input = prepared(&request);
    input.request.series[0].targets[0][2] = 0.0;
    let output = finish_fixture(input.clone(), &request);
    let metric = &output.backtest.as_ref().unwrap().metrics[0];
    assert_eq!(metric.mae, Some(2.0));
    assert_eq!(metric.mape, None);
    assert_eq!(metric.wape, None);
    input.request.series[0].observed_mask.as_mut().unwrap()[0][2] = false;
    let output = finish_fixture(input, &request);
    let metric = &output.backtest.as_ref().unwrap().metrics[0];
    assert_eq!(metric.observations, 0);
    assert_eq!(metric.mae, None);
    assert_eq!(metric.rmse, None);
    assert_eq!(metric.mape, None);
    assert_eq!(metric.wape, None);
}

#[test]
fn weighted_error_includes_zero_actuals_and_uses_absolute_negative_values() {
    let points = [Some(0.0), Some(-4.0), None]
        .into_iter()
        .map(|actual| StudioForecastPoint {
            series_id: "a".into(),
            target: "target".into(),
            timestamp_ms: 0,
            point: 2.0,
            quantiles: vec![],
            actual,
        })
        .collect::<Vec<_>>();
    let metrics = metrics(&points);
    assert_eq!(metrics[0].observations, 2);
    assert_eq!(metrics[0].mape, None);
    assert_eq!(metrics[0].wape, Some(200.0));
}

#[test]
fn forecast_studio_budgets_bound_complete_inputs_and_chart_materialization() {
    let mut request = request();
    let input = prepared(&request);
    let effective = budget(&request).unwrap();
    assert_eq!(effective.maximum_rows, MAXIMUM_ROWS);
    let mut caps = capabilities();
    validate_capabilities(&input.request, &caps, &effective).unwrap();
    caps.max_context = 3;
    assert!(validate_capabilities(&input.request, &caps, &effective).is_err());
    caps.max_context = 4;
    caps.known_covariates = false;
    assert!(validate_capabilities(&input.request, &caps, &effective).is_err());
    assert!(validate_serialized_size(&json!({"text":"too large"}), 4).is_err());
    let mut repeated_id = input.clone();
    repeated_id.request.series[0].id = "x".repeat(1024);
    let narrow_bytes = TableBudget {
        maximum_bytes: 8192,
        ..effective.clone()
    };
    assert!(validate_chart_budget(&repeated_id, &request.mapping, 2, 3, &narrow_bytes).is_err());
    request.budget.maximum_rows = 17;
    let result = predicted(&input);
    assert!(
        finish(
            &request,
            input,
            result,
            None,
            StudioForecastModel {
                bit_id: "m".into(),
                family: ForecastFamily::Chronos2,
                repository: None,
                revision: None,
            },
            capabilities(),
            6,
            &budget(&request).unwrap()
        )
        .is_err()
    );
    request.future_sql = None;
    assert!(budget(&request).is_err());
}

#[test]
fn only_typed_input_failures_are_exposed_to_transport_adapters() {
    let safe: flow_like_types::Error =
        StudioForecastInputError("Series needs regular timestamps".into()).into();
    assert_eq!(
        input_error_message(&safe),
        Some("Series needs regular timestamps")
    );
    let internal = flow_like_types::anyhow!("download https://example.com?secret=credential");
    assert_eq!(input_error_message(&internal), None);
}

#[cfg(any(feature = "inference", feature = "forecasting"))]
#[tokio::test]
async fn workbench_forecasts_use_full_joined_results_and_reject_truncation() {
    use flow_like_storage::{
        databases::workbench::{WorkbenchSurface, WorkbenchView},
        lancedb,
    };
    use flow_like_storage_contracts::column_metadata::ColumnReader;
    let temporary = tempfile::tempdir().unwrap();
    let connection = lancedb::connect(temporary.path().join("database").to_str().unwrap())
        .execute()
        .await
        .unwrap();
    let views = vec![
        WorkbenchView {
            name: "sales".into(),
            sql: "SELECT * FROM (VALUES (10, 1), (20, 2), (30, 3)) AS v(time, target)".into(),
        },
        WorkbenchView {
            name: "prices".into(),
            sql: "SELECT * FROM (VALUES (10, 5), (20, 6), (30, 7)) AS v(time, price)".into(),
        },
    ];
    let sql = "SELECT s.time, s.target, p.price FROM sales s JOIN prices p ON s.time = p.time ORDER BY s.time";
    let mut budget = TableBudget {
        maximum_rows: 3,
        ..Default::default()
    };
    let result = execution::query(
        &connection,
        &WorkbenchSurface::Native,
        &views,
        &ColumnReader::Unrestricted,
        sql,
        &json!({}),
        &budget,
    )
    .await
    .unwrap();
    assert_eq!(result.rows.len(), 3);
    assert_eq!(result.rows[2]["price"], json!(7));
    budget.maximum_rows = 2;
    let error = execution::query(
        &connection,
        &WorkbenchSurface::Native,
        &views,
        &ColumnReader::Unrestricted,
        sql,
        &json!({}),
        &budget,
    )
    .await
    .unwrap_err();
    assert!(
        input_error_message(&error)
            .unwrap()
            .contains("more than 2 rows")
    );
    assert!(
        execution::query(
            &connection,
            &WorkbenchSurface::Native,
            &views,
            &ColumnReader::Unrestricted,
            "CREATE TABLE altered AS SELECT 1",
            &json!({}),
            &budget
        )
        .await
        .is_err()
    );
}

#[cfg(any(feature = "inference", feature = "forecasting"))]
#[test]
fn workbench_temporal_types_keep_their_actual_arrow_units() {
    use crate::inspection::auto_training_tables::TemporalUnit;
    use flow_like_storage::{
        arrow_schema::{DataType, TimeUnit},
        databases::workbench::{SqlColumn, SqlQueryResult},
    };
    let mut request = request();
    request.mapping.series_id = None;
    request.mapping.past_covariates.clear();
    request.mapping.known_covariates.clear();
    let mut result = SqlQueryResult {
        columns: vec![
            SqlColumn {
                name: "time".into(),
                type_name: DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
                    .to_string(),
                position: 0,
                metadata: Default::default(),
            },
            SqlColumn {
                name: "target".into(),
                type_name: DataType::Float64.to_string(),
                position: 1,
                metadata: Default::default(),
            },
        ],
        rows: vec![],
        row_count: 0,
        truncated: false,
    };
    assert_eq!(
        execution::temporal_units(&result, &request.mapping, false).unwrap()["time"],
        TemporalUnit::Microseconds
    );
    result.columns[0].type_name = DataType::Date32.to_string();
    assert_eq!(
        execution::temporal_units(&result, &request.mapping, false).unwrap()["time"],
        TemporalUnit::Days
    );
    result.columns[0].type_name = DataType::Utf8.to_string();
    assert!(
        execution::temporal_units(&result, &request.mapping, false)
            .unwrap()
            .is_empty()
    );
    result.columns[1].type_name = DataType::LargeUtf8.to_string();
    assert!(execution::temporal_units(&result, &request.mapping, false).is_ok());
    result.columns[0].type_name =
        DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())).to_string();
    assert_eq!(
        execution::temporal_units(&result, &request.mapping, false).unwrap()["time"],
        TemporalUnit::Milliseconds
    );
    result.columns[1].type_name = DataType::Boolean.to_string();
    assert!(execution::temporal_units(&result, &request.mapping, false).is_err());
}

#[cfg(any(feature = "inference", feature = "forecasting"))]
#[test]
fn rows_read_by_another_runner_get_the_same_checks_and_preparation() {
    use flow_like_storage::{
        arrow_schema::DataType,
        databases::workbench::{SqlColumn, SqlQueryResult},
    };
    let mut request = request();
    request.mapping.series_id = None;
    request.mapping.past_covariates.clear();
    request.mapping.known_covariates.clear();
    request.future_sql = None;
    request.backtest_horizon = 0;
    let column = |name: &str, position| SqlColumn {
        name: name.into(),
        type_name: DataType::Int64.to_string(),
        position,
        metadata: Default::default(),
    };
    let rows = |count: i64| SqlQueryResult {
        columns: vec![column("time", 0), column("target", 1)],
        rows: (0..count)
            .map(|step| json!({"time": step * 10, "target": step}))
            .collect(),
        row_count: count as usize,
        truncated: false,
    };
    let limits = forecast_budget(&request).unwrap();
    let empty = checked_rows(rows(0), &limits).unwrap_err();
    assert_eq!(
        input_error_message(&empty),
        Some("Forecast SQL returned no rows")
    );
    let cut = SqlQueryResult {
        truncated: true,
        ..rows(3)
    };
    let truncated = checked_rows(cut, &limits).unwrap_err();
    assert!(
        input_error_message(&truncated)
            .unwrap()
            .contains("more than")
    );

    let history = checked_rows(rows(4), &limits).unwrap();
    assert!(prepare_forecast(request.clone(), limits.clone(), history, None).is_ok());
    let mut flags = rows(4);
    flags.columns[1].type_name = DataType::Boolean.to_string();
    let error = prepare_forecast(request.clone(), limits, flags, None)
        .err()
        .unwrap();
    assert!(input_error_message(&error).is_some());

    request.model_bit_id.clear();
    assert!(input_error_message(&forecast_budget(&request).unwrap_err()).is_some());
}

#[cfg(any(feature = "inference", feature = "forecasting"))]
#[tokio::test]
#[ignore = "requires an official checkpoint; set FLOWLIKE_TIMESTRAL_CHECKPOINT"]
async fn installed_bit_runs_workbench_forecast_and_holdout_without_network() {
    use flow_like::{
        bit::{Bit, BitPack, BitTypes},
        state::{FlowLikeConfig, FlowLikeState},
        utils::{compression::compress_to_file_json, http::HTTPClient},
    };
    use flow_like_storage::{
        Path,
        databases::workbench::WorkbenchSurface,
        files::store::{FlowLikeStore, local_store::LocalObjectStore},
        lancedb,
    };
    use flow_like_storage_contracts::column_metadata::ColumnReader;
    use std::sync::Arc;
    let checkpoint =
        std::path::PathBuf::from(std::env::var("FLOWLIKE_TIMESTRAL_CHECKPOINT").unwrap());
    let temporary = tempfile::tempdir().unwrap();
    let bits_path = temporary.path().join("bits");
    let mut assets = Vec::new();
    for (id, name) in [
        ("forecast-model", "model.safetensors"),
        ("configuration", "config.json"),
    ] {
        let directory = bits_path.join(id);
        tokio::fs::create_dir_all(&directory).await.unwrap();
        let size = tokio::fs::copy(checkpoint.join(name), directory.join(name))
            .await
            .unwrap();
        assets.push(Bit {
            id: id.into(),
            hash: id.into(),
            hub: "offline.invalid".into(),
            file_name: Some(name.into()),
            size: Some(size),
            ..Bit::default()
        });
    }
    assets[0].bit_type = BitTypes::Forecasting;
    assets[0].dependency_tree_hash = "standalone-studio-smoke".into();
    assets[0].parameters = json!({"model_type":"timestral","assets":[{"bit":"configuration","relative_path":"config.json"}]});
    let bit = assets[0].clone();
    let mut config = FlowLikeConfig::new();
    config.stores.bits_store = Some(FlowLikeStore::Local(
        LocalObjectStore::new(bits_path).unwrap().into(),
    ));
    let state = Arc::new(FlowLikeState::new(
        config,
        HTTPClient::new_without_refetch(),
    ));
    compress_to_file_json(
        FlowLikeState::bit_store(&state).await.unwrap().as_generic(),
        Path::from("deps-cache/bit-deps-standalone-studio-smoke.bin"),
        &BitPack { bits: assets },
    )
    .await
    .unwrap();
    let connection = lancedb::connect(temporary.path().join("database").to_str().unwrap())
        .execute()
        .await
        .unwrap();
    let values = (0..96)
        .map(|step| format!("({}, {})", step * 1000, 10.0 + (step as f64 * 0.15).sin()))
        .collect::<Vec<_>>()
        .join(",");
    let mut request = request();
    request.sql = format!("SELECT * FROM (VALUES {values}) AS measurements(time,target)");
    request.mapping.series_id = None;
    request.mapping.past_covariates.clear();
    request.mapping.known_covariates.clear();
    request.future_sql = None;
    request.horizon = 12;
    request.backtest_horizon = 8;
    request.backtest_offset = 12;
    let result = run_forecast(
        state,
        bit,
        &connection,
        WorkbenchSurface::Native,
        vec![],
        &ColumnReader::Unrestricted,
        request.clone(),
    )
    .await
    .unwrap();
    assert_eq!(result.input_rows, 96);
    assert_eq!(result.history.len(), 96);
    assert_eq!(result.forecasts.len(), 12);
    assert_eq!(result.forecasts[0].timestamp_ms, 96_000);
    assert!(
        result
            .forecasts
            .iter()
            .all(|p| p.point.is_finite() && p.quantiles.windows(2).all(|pair| pair[0] <= pair[1]))
    );
    let snapshot_payload = saved::CreateStudioForecast {
        name: "Installed model forecast".into(),
        description: None,
        request,
        result: result.clone(),
    };
    let snapshot_payload =
        serde_json::from_str(&serde_json::to_string(&snapshot_payload).unwrap()).unwrap();
    let snapshot = saved::create(
        &connection,
        "smoke-app",
        "smoke-user",
        &ColumnReader::Unrestricted,
        snapshot_payload,
    )
    .await
    .unwrap();
    let restored = saved::get(
        &connection,
        "smoke-app",
        &snapshot.id,
        &ColumnReader::Unrestricted,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(restored.result.run_id, result.run_id);
    assert_eq!(
        serde_json::to_value(&restored.result).unwrap(),
        serde_json::to_value(&result).unwrap()
    );
    let held = result.backtest.unwrap();
    assert_eq!(held.offset, 12);
    assert_eq!(held.forecasts.len(), 8);
    assert_eq!(held.forecasts[0].timestamp_ms, 76_000);
    assert_eq!(held.forecasts[7].timestamp_ms, 83_000);
    assert_eq!(held.metrics[0].observations, 8);
    assert!(held.metrics[0].mae.is_some_and(f64::is_finite));
    assert!(held.metrics[0].wape.is_some_and(f64::is_finite));
}

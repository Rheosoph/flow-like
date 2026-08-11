//! Node for Fitting a **Feature Scaler**
//!
//! Learns per-column offsets and scales from a training table using the [`linfa_preprocessing`]
//! crate. The node only fits: the resulting model is replayed on held-out data with the
//! Apply Transform node, so train and test share one set of statistics.

use crate::ml::NodeMLModel;
#[cfg(feature = "execute")]
use crate::ml::{MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, values_to_array2_f64};
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, Value, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::traits::Fit;
#[cfg(feature = "execute")]
use linfa_preprocessing::linear_scaling::LinearScaler;
#[cfg(feature = "execute")]
use std::collections::HashSet;

#[crate::register_node]
#[derive(Default)]
pub struct FitFeatureScalerNode {}

impl FitFeatureScalerNode {
    pub fn new() -> Self {
        FitFeatureScalerNode {}
    }
}

#[async_trait]
impl NodeLogic for FitFeatureScalerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_feature_scaler",
            "Fit Feature Scaler",
            "Learn per-feature offsets and scales from a training table. Distance- and gradient-based models (Logistic Regression, Elastic Net, SVM, KNN, Gaussian Mixture) only behave when their features share a scale.",
            "AI/ML/Preprocessing",
        );
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins fitting the scaler",
            VariableType::Execution,
        );

        node.add_input_pin(
            "source",
            "Data Source",
            "Choose which backend supplies the training data",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Database".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("Database")));

        node.add_input_pin(
            "method",
            "Method",
            "Standard centers each feature and divides it by its standard deviation. MinMax squeezes each feature into the Min..Max range. MaxAbs divides each feature by its largest absolute value, keeping zeros at zero.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Standard".to_string(),
                    "MinMax".to_string(),
                    "MaxAbs".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Standard")));

        node.add_input_pin(
            "min",
            "Min",
            "Lower bound of the target range. Only read when Method is MinMax.",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));

        node.add_input_pin(
            "max",
            "Max",
            "Upper bound of the target range. Only read when Method is MinMax.",
            VariableType::Float,
        )
        .set_default_value(Some(json!(1.0)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the scaler is fitted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the fitted scaler. Feed it to Apply Transform to scale any table with these statistics.",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "offsets",
            "Offsets",
            "Value subtracted from each feature before scaling: the mean for Standard, the minimum for MinMax, zero for MaxAbs",
            VariableType::Float,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "scales",
            "Scales",
            "Multiplier applied to each feature. linfa stores the reciprocal, so this is 1/std for Standard and 1/(max-min) for MinMax, and it stays 1 for constant features.",
            VariableType::Float,
        )
        .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let method: String = context.evaluate_pin("method").await?;
        let min: f64 = context.evaluate_pin("min").await.unwrap_or(0.0);
        let max: f64 = context.evaluate_pin("max").await.unwrap_or(1.0);

        let t0 = std::time::Instant::now();
        let (records, records_col) = match source.as_str() {
            "Database" => {
                let database: NodeDBConnection = context.evaluate_pin("database").await?;
                let records_col: String = context.evaluate_pin("records").await?;

                let records = {
                    let cached_db = database.load(context).await?;
                    cached_db.ensure_flushed().await?;
                    let database = cached_db.db.read().await;
                    let schema = database.schema().await?;
                    let existing_cols: HashSet<String> =
                        schema.fields.iter().map(|f| f.name().clone()).collect();
                    if !existing_cols.contains(&records_col) {
                        return Err(anyhow!(
                            "Database doesn't contain train col `{}`!",
                            records_col
                        ));
                    }
                    database
                        .filter(
                            "true",
                            Some(vec![records_col.clone()]),
                            MAX_ML_PREDICTION_RECORDS,
                            0,
                        )
                        .await?
                };
                (records, records_col)
            }
            _ => return Err(anyhow!("Datasource Not Implemented!")),
        };
        context.log_message(
            &format!("Loaded {} records from database", records.len()),
            LogLevel::Debug,
        );
        context.log_message(
            &format!("Preprocess data: {:?}", t0.elapsed()),
            LogLevel::Debug,
        );

        if records.is_empty() {
            return Err(anyhow!(
                "Column `{records_col}` returned no rows, a scaler cannot learn statistics from an empty table"
            ));
        }
        if records.len() >= MAX_ML_PREDICTION_RECORDS {
            context.log_message(
                &format!(
                    "Hit the {MAX_ML_PREDICTION_RECORDS} row cap, the scaler describes that sample rather than the full table"
                ),
                LogLevel::Warn,
            );
        }

        let array = values_to_array2_f64(&records, &records_col)?;
        let (n_rows, n_features) = array.dim();
        if n_features == 0 {
            return Err(anyhow!(
                "Column `{records_col}` holds empty vectors, there is nothing to scale"
            ));
        }
        let dataset = DatasetBase::from(array);

        let params = match method.as_str() {
            "Standard" => LinearScaler::<f64>::standard(),
            "MaxAbs" => LinearScaler::<f64>::max_abs(),
            "MinMax" => {
                if !min.is_finite() || !max.is_finite() {
                    return Err(anyhow!("MinMax range must be finite, got {min} and {max}"));
                }
                if min > max {
                    return Err(anyhow!(
                        "MinMax range is flipped, Min {min} is greater than Max {max}"
                    ));
                }
                if min == max {
                    context.log_message(
                        &format!(
                            "MinMax range {min}..={max} is empty, every scaled feature collapses to {min}"
                        ),
                        LogLevel::Warn,
                    );
                }
                LinearScaler::<f64>::min_max_range(min, max)
            }
            other => {
                return Err(anyhow!(
                    "Unknown scaling method `{other}`, use Standard, MinMax or MaxAbs"
                ));
            }
        };

        let t0 = std::time::Instant::now();
        let scaler = params.fit(&dataset)?;
        context.log_message(
            &format!(
                "Fitted {} on {} rows x {} features: {:?}",
                scaler.method(),
                n_rows,
                n_features,
                t0.elapsed()
            ),
            LogLevel::Debug,
        );

        // linfa clamps the scale of a zero-spread feature to 1 instead of dividing by zero, so a
        // constant column passes through unscaled rather than turning into NaN.
        let offsets: Vec<f64> = scaler.offsets().to_vec();
        let scales: Vec<f64> = scaler.scales().to_vec();

        let model = MLModel::FeatureScaler(ModelWithMeta {
            model: scaler,
            classes: None,
        });
        let node_model = NodeMLModel::new(context, model).await;
        context.set_pin_value("offsets", json!(offsets)).await?;
        context.set_pin_value("scales", json!(scales)).await?;
        context.set_pin_value("model", json!(node_model)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature. Rebuild with --features execute"
        ))
    }

    #[cfg(feature = "execute")]
    async fn on_update(&self, node: &mut Node, _board: &Board) {
        use flow_like_catalog_core::NodeDBConnection;

        let source_pin: String = node
            .get_pin_by_name("source")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        if source_pin == *"Database" {
            if node.get_pin_by_name("database").is_none() {
                node.add_input_pin(
                    "database",
                    "Database",
                    "Database Connection",
                    VariableType::Struct,
                )
                .set_schema::<NodeDBConnection>()
                .set_options(PinOptions::new().set_enforce_schema(true).build());
            }
            if node.get_pin_by_name("records").is_none() {
                node.add_input_pin(
                    "records",
                    "Train Col",
                    "Column containing the feature vectors the scaler learns from",
                    VariableType::String,
                )
                .set_default_value(Some(json!("vector")));
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}

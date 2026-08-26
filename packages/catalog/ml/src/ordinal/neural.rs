//! Node for fitting a **rank-consistent neural** ordinal model, CORAL or CORN.
//!
//! Every other ordinal trainer in this catalog is linear in the features except Frank & Hall, and
//! that one decomposes the target into `K - 1` independent classifiers that agree on nothing and so
//! carry no probability model at all. This node fills the remaining hole: a small MLP backbone under
//! a head whose `P(y > k)` sequence cannot increase with `k` for ANY parameter values, so the level
//! probabilities are differences of a non-increasing sequence — non-negative and summing to one
//! without clamping or renormalization.
//!
//! The backbone is the entire contribution. With no hidden layer CORAL is the all-threshold
//! proportional-odds fit and CORN is the continuation-ratio fit, exactly, which is why both the node
//! description and a run-time warning point a linear problem back at those simpler nodes.

#[cfg(feature = "execute")]
use crate::ml::{
    MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, OrdinalOrdering, values_to_array1_ordinal,
    values_to_array2_f64,
};
use crate::ml::{NodeMLModel, OrdinalLevels};
use flow_like::flow::board::Board;
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_ordinal::{Activation, OrdinalHead, OrdinalNeural};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
use flow_like_types::Value;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::traits::Fit;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::collections::HashSet;

/// Rows per fitted parameter below which the training set is called out as thin.
///
/// The classical rule for a linear fit is ten observations per estimated parameter, but that targets
/// unbiased coefficient inference, which nobody asks of a network: a network is judged on held-out
/// accuracy and is additionally constrained by the L2 penalty and the iteration cap. The hard
/// failure is the interpolation regime at one row per parameter, where the fit can reproduce the
/// training labels outright and its training score stops carrying information. Three sits far enough
/// above that to warn before the fit is already meaningless, without firing on every reasonable
/// board — the default single 16-unit layer on five features is 115 parameters, tolerated from
/// roughly 345 rows.
#[cfg(feature = "execute")]
const MIN_ROWS_PER_PARAMETER: f64 = 3.0;

/// What the fit actually built, next to how much data it was built from.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrdinalNeuralArchitecture {
    /// Rank-consistent head that was fitted, `Coral` or `Corn`
    pub head: String,
    /// Non-linearity between the hidden layers. Reported even with no hidden layer, where it never
    /// took effect.
    pub activation: String,
    /// Hidden layer widths as fitted, input side first. EMPTY means the backbone was a plain linear
    /// map, so this fit was the linear equivalent of a simpler ordinal node.
    pub hidden_layers: Vec<usize>,
    /// Every fitted number: all weight matrices, all bias vectors, and CORAL's `K - 1` ordering
    /// parameters.
    pub parameter_count: usize,
    /// Rows the model was fitted on
    pub training_samples: usize,
    /// `training_samples / parameter_count`. Below 1 the network has enough freedom to memorize the
    /// training set, which makes a training score meaningless.
    pub samples_per_parameter: f64,
}

#[crate::register_node]
#[derive(Default)]
pub struct FitOrdinalNeuralNode {}

impl FitOrdinalNeuralNode {
    pub fn new() -> Self {
        FitOrdinalNeuralNode {}
    }
}

#[async_trait]
impl NodeLogic for FitOrdinalNeuralNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_ordinal_neural",
            "Train Ordinal Model (Neural CORAL/CORN)",
            "Fit/Train a NEURAL ordinal model on a target whose levels are ORDERED (1 < 2 < ... < 5, or low < medium < high). This is the only trainer in the catalog that is BOTH non-linear in the features AND yields calibrated, rank-consistent per-level probabilities: Frank & Hall is non-linear but votes with K-1 independent classifiers and therefore carries no probability model, while every other ordinal node here is linear in the features. A small network feeds one of two rank-consistent heads, CORAL or CORN, and both are built so that P(y > k) can never rise with k for ANY parameter values — so the level probabilities are non-negative and sum to 1 with nothing patched up afterwards. THE HONEST LIMIT: leave Hidden Layers EMPTY and CORAL becomes exactly Train Ordinal Model (Proportional Odds) with Loss = AllThreshold and Margin = Logistic, and CORN becomes exactly Train Ordinal Model (Continuation Ratio) — the same objective in the same parameters. The hidden layers are the entire contribution, so if your problem is linear in the features prefer those nodes: convex objective, no seed dependence, readable coefficients, better tested. Reach for this one when the level is genuinely not monotone in the features (a boundary that bends back on itself, which no linear ordinal model can represent at all). Two costs come with the network: it has far more parameters than a linear model and so needs far more rows — check the Architecture output — and the objective is not convex, so the Seed changes the fit. Scale your features first with the Fit Feature Scaler node; unscaled columns make this converge slowly or not at all.",
            "AI/ML/Ordinal",
        );
        node.set_flowscript_name("ml", "fitOrdinalNeural");
        node.set_version(1);
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(4) // Full-batch training over a hand-written MLP, one row at a time
                .set_governance(3) // Weights explain nothing; a linear ordinal fit exposes coefficients
                .set_reliability(5) // Non-convex, so the seed genuinely changes the fitted model
                .set_cost(4)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins neural ordinal model training",
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
            "class_order",
            "Class Order",
            "Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one. Note that a declared level the training data never reaches is fine for CORAL but rejected by CORN, whose task for that level would have no rows to fit.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "head",
            "Head",
            "Which rank-consistent head sits on the network. Coral shares ONE latent score across every cut point and lets the cut points differ only by an ordered bias, so a row's whole position on the scale is a single number: fewer parameters, lower variance, and the right choice when the levels really are separated by one underlying quantity or when the top levels are thin. Corn instead asks each step conditionally — given the row reached this level, does it go further? — and gives every step its own weights on the shared representation, which suits a target that is a genuine sequential process (escalation tiers, disease stages, how far a funnel got). Its price is data: step k trains only on the rows that reached level k, so the higher steps rest on the fewest rows, and Corn refuses outright to fit a declared level that nothing reaches.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Coral".to_string(), "Corn".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("Coral")));

        node.add_input_pin(
            "hidden_layers",
            "Hidden Layers",
            "Comma-separated hidden layer widths from the input side, e.g. `16, 8` for two layers. This is the ONLY thing this node adds over the linear ordinal trainers: an EMPTY value collapses the model to its linear equivalent exactly — Coral becomes the All-Threshold proportional-odds fit, Corn becomes the continuation-ratio fit — so if you want an empty value you want one of those simpler, better-tested nodes instead. Wider and deeper buys a boundary that can bend, and costs parameters that have to be paid for in rows: compare the Architecture output's parameter count against your row count. Every width must be at least 1; a zero-width layer would disconnect the head from the features and fit a constant.",
            VariableType::String,
        )
        .set_default_value(Some(json!("16")));

        node.add_input_pin(
            "activation",
            "Activation",
            "Non-linearity between the hidden layers. The head itself is always linear, and this has no effect at all when Hidden Layers is empty. Relu is cheap, and its piecewise-linear folds are exactly what let a small network represent a level that is not monotone in the features. Tanh is smooth and bounded, which often trains more gently on small, well-scaled data, but it saturates on large inputs and then passes almost no gradient — one more reason to scale the features first.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Relu".to_string(), "Tanh".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("Relu")));

        node.add_input_pin(
            "alpha",
            "Alpha (L2 Penalty)",
            "Strength of the L2 penalty on the WEIGHT matrices. Biases and the head's ordering parameters are never penalized: shrinking those would drag the level cut points together and quietly collapse adjacent levels, which changes the model rather than its variance. Raise it when the network memorizes the training rows or the loss blows up; 0 fits unpenalized.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1000.0)).build())
        .set_default_value(Some(json!(1.0)));

        node.add_input_pin(
            "max_iterations",
            "Max Iterations",
            "Iteration cap for the Adam optimizer; each iteration is one full pass over the training set. Training stops here even if the loss is still falling, which is reported on the Converged pin. A network usually needs noticeably more iterations than the linear ordinal fits.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1.0, 1_000_000.0)).build())
        .set_default_value(Some(json!(500)));

        node.add_input_pin(
            "tolerance",
            "Tolerance",
            "Relative change in the loss below which training stops. Smaller values fit tighter and cost iterations; 0 always spends the whole iteration budget.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
        .set_default_value(Some(json!(1e-7)));

        node.add_input_pin(
            "learning_rate",
            "Learning Rate",
            "Adam step size. Lower it if the loss oscillates or goes non-finite, raise it if the model has not converged within Max Iterations. A network wants a smaller step than the linear ordinal fits, because a hidden layer compounds every step.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((1e-6, 10.0)).build())
        .set_default_value(Some(json!(0.05)));

        node.add_input_pin(
            "seed",
            "Seed",
            "Seed for the weight initialization, which is the only randomness in the fit. The objective is NOT convex, so the seed genuinely changes the model you get and an unlucky one can leave the fit in a poor local optimum: refit with two or three seeds to see whether the result is stable. The same seed, data and hyperparameters reproduce a fit exactly.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((0.0, 4294967295.0)).build())
        .set_default_value(Some(json!(42)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once training completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the trained neural ordinal model. Predictions come back as your original level labels, and unlike the threshold losses of the proportional-odds node this family always carries per-level probabilities, so the Predict node reports a confidence.",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "levels",
            "Levels",
            "The level order the model was actually trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when an ordinal model behaves oddly.",
            VariableType::Struct,
        )
        .set_schema::<OrdinalLevels>();

        node.add_output_pin(
            "converged",
            "Converged",
            "False when the optimizer hit Max Iterations before the loss settled. The model is still usable but under-fitted, which on a network is more common than on the linear ordinal fits.",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "architecture",
            "Architecture",
            "What was actually built: the head, the activation, the hidden layer widths as fitted, and the total parameter count next to the number of training rows. Read the rows-per-parameter figure before you trust a training score — with fewer rows than parameters the network can reproduce the training labels outright. Empty hidden layers here means the fit was the linear equivalent, and a simpler ordinal node would have done the same job.",
            VariableType::Struct,
        )
        .set_schema::<OrdinalNeuralArchitecture>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let source: String = context.evaluate_pin("source").await?;
        let class_order: String = context.evaluate_pin("class_order").await?;
        let head: String = context.evaluate_pin("head").await?;
        let hidden_layers_raw: String = context.evaluate_pin("hidden_layers").await?;
        let activation: String = context.evaluate_pin("activation").await?;
        let alpha: f64 = context.evaluate_pin("alpha").await?;
        let max_iterations: i64 = context.evaluate_pin("max_iterations").await?;
        let tolerance: f64 = context.evaluate_pin("tolerance").await?;
        let learning_rate: f64 = context.evaluate_pin("learning_rate").await?;
        let seed: i64 = context.evaluate_pin("seed").await?;

        let head = match head.as_str() {
            "Coral" => OrdinalHead::Coral,
            "Corn" => OrdinalHead::Corn,
            other => {
                return Err(anyhow!(
                    "Unknown head `{other}`, expected `Coral` or `Corn`"
                ));
            }
        };
        let activation = match activation.as_str() {
            "Relu" => Activation::Relu,
            "Tanh" => Activation::Tanh,
            other => {
                return Err(anyhow!(
                    "Unknown activation `{other}`, expected `Relu` or `Tanh`"
                ));
            }
        };

        let mut hidden_layers: Vec<usize> = Vec::new();
        for token in hidden_layers_raw.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            let width = token.parse::<usize>().map_err(|_| {
                anyhow!(
                    "`Hidden Layers` takes comma-separated layer widths such as `16, 8`, but `{token}` is not a whole number. Leave the pin empty for no hidden layer at all."
                )
            })?;
            if width == 0 {
                return Err(anyhow!(
                    "`Hidden Layers` gives layer {} a width of 0 (entry `{token}`); a zero-width layer disconnects the head from the features and can only fit a constant. Give it at least 1 unit, or leave the pin empty for no hidden layer at all.",
                    hidden_layers.len()
                ));
            }
            hidden_layers.push(width);
        }

        if !alpha.is_finite() || alpha < 0.0 {
            return Err(anyhow!(
                "`Alpha (L2 Penalty)` must be a finite value >= 0, got {alpha}"
            ));
        }
        if !(1..=u32::MAX as i64).contains(&max_iterations) {
            return Err(anyhow!(
                "`Max Iterations` must be between 1 and {}, got {max_iterations}",
                u32::MAX
            ));
        }
        if !tolerance.is_finite() || tolerance < 0.0 {
            return Err(anyhow!(
                "`Tolerance` must be a finite value >= 0, got {tolerance}"
            ));
        }
        if !learning_rate.is_finite() || learning_rate <= 0.0 {
            return Err(anyhow!(
                "`Learning Rate` must be a finite value > 0, got {learning_rate}"
            ));
        }
        if !(0..=u32::MAX as i64).contains(&seed) {
            return Err(anyhow!(
                "`Seed` must be between 0 and {}, got {seed}",
                u32::MAX
            ));
        }

        let explicit_order: Vec<String> = class_order
            .split(',')
            .map(|level| level.trim())
            .filter(|level| !level.is_empty())
            .map(ToString::to_string)
            .collect();

        let t0 = std::time::Instant::now();
        let (train_array, ranks, classes, levels) = match source.as_str() {
            "Database" => {
                let database: NodeDBConnection = context.evaluate_pin("database").await?;
                let records_col: String = context.evaluate_pin("records").await?;
                let targets_col: String = context.evaluate_pin("targets").await?;

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
                    if !existing_cols.contains(&targets_col) {
                        return Err(anyhow!(
                            "Database doesn't contain target col `{}`!",
                            targets_col
                        ));
                    }
                    database
                        .filter(
                            "true",
                            Some(vec![records_col.to_string(), targets_col.to_string()]),
                            MAX_ML_PREDICTION_RECORDS,
                            0,
                        )
                        .await?
                };
                context.log_message(
                    &format!("Got {} records for training", records.len()),
                    LogLevel::Debug,
                );
                if records.is_empty() {
                    return Err(anyhow!(
                        "No training records in the database; neural ordinal fitting needs at least one row"
                    ));
                }

                let train_array = values_to_array2_f64(&records, &records_col)?;
                let (ranks, classes, levels) = values_to_array1_ordinal(
                    &records,
                    &targets_col,
                    if explicit_order.is_empty() {
                        None
                    } else {
                        Some(explicit_order.as_slice())
                    },
                )?;
                (train_array, ranks, classes, levels)
            }
            _ => return Err(anyhow!("Datasource Not Implemented!")),
        };
        context.log_message(
            &format!("Preprocess data: {:?}", t0.elapsed()),
            LogLevel::Debug,
        );

        let (n_samples, n_features) = train_array.dim();
        if n_features == 0 {
            return Err(anyhow!(
                "Training records have 0 features, expected at least one value per row"
            ));
        }
        // Adam turns a single NaN into an all-NaN parameter vector, and the crate would only report
        // that "the feature matrix" was non-finite, so the offending cell is resolved here.
        if let Some(((row, col), value)) = train_array
            .indexed_iter()
            .find(|(_, value)| !value.is_finite())
        {
            return Err(anyhow!(
                "Training feature at row {row}, column {col} is {value}; neural ordinal fitting needs finite features. Clean or impute the column before training."
            ));
        }

        let n_classes = levels.labels.len();
        let observed: HashSet<usize> = ranks.iter().copied().collect();
        if observed.len() < 2 {
            let seen = observed
                .iter()
                .filter_map(|rank| levels.labels.get(*rank))
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            return Err(anyhow!(
                "Ordinal models need at least 2 distinct levels in the training data, but only [{seen}] occurs. Widen the training set or check the target column."
            ));
        }

        let ordering_source = match levels.ordering {
            OrdinalOrdering::Explicit => "from your Class Order list",
            OrdinalOrdering::Numeric => "inferred by reading the labels as numbers",
        };
        // Training on a wrong order fails silently — the model just learns the wrong direction — so
        // the resolved order has to be visible in the run log, not only on the output pin.
        context.log_message(
            &format!(
                "Ordinal level order ({ordering_source}): {}",
                levels.labels.join(" < ")
            ),
            LogLevel::Info,
        );

        // CORN's task k trains only on the rows with y >= k, so a declared level nothing reaches
        // leaves that task with an empty subset. The crate rejects it, but only by rank; naming the
        // label and the alternative here is what makes the failure actionable.
        if head == OrdinalHead::Corn
            && let Some(cut) =
                (0..n_classes.saturating_sub(1)).find(|cut| !ranks.iter().any(|rank| *rank >= *cut))
        {
            let unreached = levels
                .labels
                .get(cut)
                .cloned()
                .unwrap_or_else(|| format!("rank {cut}"));
            let highest = observed
                .iter()
                .max()
                .and_then(|rank| levels.labels.get(*rank))
                .cloned()
                .unwrap_or_else(|| "none".to_string());
            return Err(anyhow!(
                "CORN cannot fit level `{unreached}`: its step trains only on rows that reached that level, and no training row does — the highest level present is `{highest}`. Remove the unreached levels from Class Order, widen the training set, or switch Head to Coral, whose steps all see every row and therefore tolerate a declared level the sample never reaches."
            ));
        }

        // The whole reason this node exists is the hidden layers; without them it is a worse-tested
        // copy of an estimator that is convex and interpretable.
        if hidden_layers.is_empty() {
            let equivalent = match head {
                OrdinalHead::Coral => {
                    "Train Ordinal Model (Proportional Odds) with Loss = AllThreshold and Margin = Logistic"
                }
                OrdinalHead::Corn => "Train Ordinal Model (Continuation Ratio) with the Logit link",
            };
            context.log_message(
                &format!(
                    "Hidden Layers is empty, so this {head:?} fit is exactly {equivalent} — the same objective in the same parameters. That node fits it with a convex objective, no seed dependence and readable coefficients, and is better tested; the hidden layers are the only thing this node adds."
                ),
                LogLevel::Warn,
            );
        }

        let t0 = std::time::Instant::now();
        // `n_levels` is declared rather than inferred: an explicit Class Order may name levels the
        // training sample never reached, and CORAL keeps a cut point for them.
        let dataset = DatasetBase::new(train_array, ranks);
        let fitted = OrdinalNeural::params()
            .head(head)
            .hidden_layers(&hidden_layers)
            .activation(activation)
            .alpha(alpha)
            .max_iterations(max_iterations as usize)
            .tolerance(tolerance)
            .learning_rate(learning_rate)
            .seed(seed as u64)
            .n_levels(n_classes)
            .fit(&dataset)
            .map_err(|err| {
                anyhow!(
                    "Neural ordinal fit ({head:?} head, {activation:?} activation, hidden layers {hidden_layers:?}) failed: {err}"
                )
            })?;
        context.log_message(&format!("Fit model: {:?}", t0.elapsed()), LogLevel::Debug);

        let converged = fitted.converged();
        let weight_parameters: usize = fitted.weights().iter().map(|matrix| matrix.len()).sum();
        let bias_parameters: usize = fitted.biases().iter().map(|bias| bias.len()).sum();
        // CORAL holds its K-1 ordering parameters outside the backbone; CORN's per-step parameters
        // are already counted in the output layer.
        let head_parameters = fitted.task_biases().map_or(0, |biases| biases.len());
        let parameter_count = weight_parameters + bias_parameters + head_parameters;
        let samples_per_parameter = n_samples as f64 / parameter_count as f64;

        context.log_message(
            &format!(
                "Neural ordinal fit: {n_features} features -> {:?} -> {n_classes} levels, {head:?} head, {activation:?} activation, {parameter_count} parameters, {} iterations on {n_samples} rows",
                fitted.hidden_layers(),
                fitted.iterations()
            ),
            LogLevel::Info,
        );

        if !converged {
            context.log_message(
                &format!(
                    "Training stopped at the cap of {} iterations without converging. The model is under-fitted: raise Max Iterations, raise Learning Rate, scale the features with the Fit Feature Scaler node, or use a smaller architecture.",
                    fitted.iterations()
                ),
                LogLevel::Warn,
            );
        }

        // A network with more parameters than rows can reproduce the training labels outright, and
        // nothing downstream can tell that apart from a good fit.
        if samples_per_parameter < MIN_ROWS_PER_PARAMETER {
            context.log_message(
                &format!(
                    "Thin training set for this architecture: {n_samples} rows against {parameter_count} parameters, {samples_per_parameter:.1} rows per parameter. A network with fewer rows than parameters memorizes the training set, and its training score then says nothing about new rows. Narrow or remove hidden layers, raise Alpha, add rows — or, if the levels are monotone in the features, use one of the linear ordinal trainers instead."
                ),
                LogLevel::Warn,
            );
        }

        let architecture = OrdinalNeuralArchitecture {
            head: format!("{:?}", fitted.head()),
            activation: format!("{:?}", fitted.activation()),
            hidden_layers: fitted.hidden_layers(),
            parameter_count,
            training_samples: n_samples,
            samples_per_parameter,
        };

        let model = MLModel::OrdinalNeural(ModelWithMeta {
            model: fitted,
            classes: Some(classes),
        });
        let node_model = NodeMLModel::new(context, model).await;

        context.set_pin_value("model", json!(node_model)).await?;
        context.set_pin_value("levels", json!(levels)).await?;
        context.set_pin_value("converged", json!(converged)).await?;
        context
            .set_pin_value("architecture", json!(architecture))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature. Rebuild with --features execute"
        ))
    }

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
                    "Column Containing the Feature Vectors to Train on",
                    VariableType::String,
                )
                .set_default_value(Some(json!("vector")));
            }
            if node.get_pin_by_name("targets").is_none() {
                node.add_input_pin(
                    "targets",
                    "Target Col",
                    "Column Containing the Ordered Level of each Row",
                    VariableType::String,
                );
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}

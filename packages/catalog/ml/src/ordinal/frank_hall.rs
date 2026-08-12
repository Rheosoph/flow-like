//! Node for the **Frank & Hall** ordered-binary decomposition.
//!
//! The ordered target is cut `K - 1` times — "is the level above this cut?" — and each cut is
//! handed to an ordinary binary classifier. The predicted level is the number of cuts answered
//! yes. Nothing about the base learner is assumed beyond separating two classes, which makes this
//! the only ordinal trainer in the catalog that is not linear in the features.

#[cfg(feature = "execute")]
use crate::ml::{
    FrankHallModel, MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta, OrdinalOrdering,
    PersistedBoolEnsemble, values_to_array1_ordinal, values_to_array2_f64,
};
use crate::ml::{NodeMLModel, OrdinalLevels};
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_ordinal::{FrankHall, FrankHallParams};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, Value, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa::DatasetBase;
#[cfg(feature = "execute")]
use linfa::error::Error as LinfaError;
#[cfg(feature = "execute")]
use linfa::traits::Fit;
#[cfg(feature = "execute")]
use linfa_bayes::{GaussianNb, GaussianNbParams, NaiveBayesError};
#[cfg(feature = "execute")]
use linfa_ensemble::RandomForestParams;
#[cfg(feature = "execute")]
use linfa_trees::{DecisionTree as LinfaDecisionTree, DecisionTreeParams, SplitQuality};
#[cfg(feature = "execute")]
use ndarray::{Array1, Array2};
// linfa 0.8 is built against rand 0.8, so the RNG handed to it cannot come from
// `flow_like_types::rand` (0.9) — the `Rng` traits are different types.
#[cfg(feature = "execute")]
use rand_xoshiro::Xoshiro256Plus;
#[cfg(feature = "execute")]
use rand_xoshiro::rand_core::SeedableRng;
#[cfg(feature = "execute")]
use std::collections::HashSet;

const BASE_DECISION_TREE: &str = "Decision Tree";
const BASE_GAUSSIAN_NAIVE_BAYES: &str = "Gaussian Naive Bayes";
const BASE_RANDOM_FOREST: &str = "Random Forest";
const BASE_LEARNERS: [&str; 3] = [
    BASE_DECISION_TREE,
    BASE_GAUSSIAN_NAIVE_BAYES,
    BASE_RANDOM_FOREST,
];

/// Hyperparameter pins owned by each base learner.
///
/// Every base's pins have distinct names even where the knob is conceptually the same, because
/// `on_update` only *adds* a missing pin: a shared name would keep the first base's description and
/// range after the user switches base.
#[cfg(feature = "execute")]
const TREE_PINS: [&str; 3] = ["max_depth", "min_samples_split", "split_quality"];
#[cfg(feature = "execute")]
const BAYES_PINS: [&str; 1] = ["var_smoothing"];
#[cfg(feature = "execute")]
const FOREST_PINS: [&str; 6] = [
    "ensemble_size",
    "bootstrap_proportion",
    "feature_proportion",
    "forest_max_depth",
    "forest_min_weight_split",
    "seed",
];

/// Tree count across the whole decomposition above which the fit and the saved model start to hurt.
#[cfg(feature = "execute")]
const LARGE_FOREST_TOTAL_WARNING: usize = 500;

/// Validated Random Forest knobs, held raw until the feature width is known.
///
/// `feature_proportion` may still carry the 0 sentinel: the textbook sqrt(feature count) default
/// cannot be written as a proportion before the training matrix has been read, and the pins are
/// deliberately resolved *before* the expensive database read so a bad value fails fast.
#[cfg(feature = "execute")]
struct ForestSettings {
    ensemble_size: usize,
    bootstrap_proportion: f64,
    feature_proportion: f64,
    max_depth: i64,
    min_weight_split: f64,
    seed: u64,
}

/// Hyperparameters of the binary classifier backing every cut.
///
/// The variants are exactly the base learners whose *fitted* model can round-trip through storage:
/// `FrankHall<M>` is serializable only when `M` is, and a model that cannot be saved is worthless
/// on a board. The forest qualifies only through [`crate::ml::PersistedBoolEnsemble`], which
/// mirrors linfa's un-serializable ensemble at the IO boundary.
#[cfg(feature = "execute")]
enum BaseParams {
    DecisionTree(DecisionTreeParams<f64, bool>),
    GaussianNaiveBayes(GaussianNbParams<f64, bool>),
    RandomForest(ForestSettings),
}

#[cfg(feature = "execute")]
impl BaseParams {
    fn label(&self) -> &'static str {
        match self {
            BaseParams::DecisionTree(_) => BASE_DECISION_TREE,
            BaseParams::GaussianNaiveBayes(_) => BASE_GAUSSIAN_NAIVE_BAYES,
            BaseParams::RandomForest(_) => BASE_RANDOM_FOREST,
        }
    }

    /// Trees grown per cut, or `None` for the bases that fit a single model per cut.
    fn ensemble_size(&self) -> Option<usize> {
        match self {
            BaseParams::RandomForest(settings) => Some(settings.ensemble_size),
            _ => None,
        }
    }

    /// Substitutes sqrt(feature count) for the 0 sentinel, returning what it resolved to so the
    /// caller can log it; `None` when nothing needed resolving.
    ///
    /// sqrt(p) features per tree is the standard Random Forest default and cannot be expressed as a
    /// fixed proportion, so it can only be applied once the feature width is known.
    fn resolve_feature_proportion(&mut self, n_features: usize) -> Option<f64> {
        match self {
            BaseParams::RandomForest(settings)
                if settings.feature_proportion <= 0.0 && n_features > 0 =>
            {
                let auto =
                    ((n_features as f64).sqrt() / n_features as f64).clamp(f64::MIN_POSITIVE, 1.0);
                settings.feature_proportion = auto;
                Some(auto)
            }
            _ => None,
        }
    }
}

/// Fits the decomposition, naming the base learner's error type at each call site.
///
/// The base params are *unchecked*, and linfa gives every `ParamGuard` a `Fit` impl for each error
/// type that can absorb its own, so `FrankHallParams`'s `E` is ambiguous and cannot be inferred.
/// Each branch therefore names the error its base learner's checked params actually fit with:
/// linfa's own for the tree and for the forest, `NaiveBayesError` for the Bayes model.
#[cfg(feature = "execute")]
fn fit_decomposition(
    base: BaseParams,
    dataset: &DatasetBase<Array2<f64>, Array1<usize>>,
    n_levels: usize,
) -> flow_like_ordinal::Result<FrankHallModel> {
    match base {
        BaseParams::DecisionTree(params) => Ok(FrankHallModel::DecisionTree(
            FrankHallParams::<_, LinfaError>::new(params)
                .n_levels(n_levels)
                .fit(dataset)?,
        )),
        BaseParams::GaussianNaiveBayes(params) => Ok(FrankHallModel::GaussianNaiveBayes(
            FrankHallParams::<_, NaiveBayesError>::new(params)
                .n_levels(n_levels)
                .fit(dataset)?,
        )),
        BaseParams::RandomForest(settings) => {
            let mut tree = LinfaDecisionTree::<f64, bool>::params();
            if settings.max_depth > 0 {
                tree = tree.max_depth(Some(settings.max_depth as usize));
            }
            tree = tree.min_weight_split(settings.min_weight_split as f32);

            // One params object serves all K-1 cuts and the ensemble clones its RNG per fit, so
            // every cut draws the same bootstrap rows and the same feature subsets. The cuts differ
            // only in their targets, which is what makes a fixed seed reproduce the whole model.
            let forest = RandomForestParams::new_fixed_rng(
                tree,
                Xoshiro256Plus::seed_from_u64(settings.seed),
            )
            .ensemble_size(settings.ensemble_size)
            .bootstrap_proportion(settings.bootstrap_proportion)
            .feature_proportion(settings.feature_proportion);

            // linfa's ensemble derives no serde, so the fitted forests are re-wrapped one by one
            // and the decomposition is rebuilt around the persistable type. `from_parts` re-checks
            // that the model count still matches the level count.
            let fitted = FrankHallParams::<_, LinfaError>::new(forest)
                .n_levels(n_levels)
                .fit(dataset)?;
            let (models, n_classes, n_features) = fitted.into_parts();
            let models = models
                .into_iter()
                .map(PersistedBoolEnsemble)
                .collect::<Vec<_>>();
            Ok(FrankHallModel::RandomForest(FrankHall::from_parts(
                models, n_classes, n_features,
            )?))
        }
    }
}

/// Reads a dropdown pin's selected value during `on_update`, where only default values exist.
#[cfg(feature = "execute")]
fn selected_value(node: &Node, name: &str) -> String {
    node.get_pin_by_name(name)
        .and_then(|pin| pin.default_value.clone())
        .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
        .and_then(|json| json.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// Drops the hyperparameter pins of the bases that are not selected.
#[cfg(feature = "execute")]
fn remove_pins(node: &mut Node, names: &[&str]) {
    for name in names {
        flow_like::flow::node::remove_pin_by_name(node, name);
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FitOrdinalFrankHallNode {}

impl FitOrdinalFrankHallNode {
    pub fn new() -> Self {
        FitOrdinalFrankHallNode {}
    }
}

#[async_trait]
impl NodeLogic for FitOrdinalFrankHallNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_ordinal_frank_hall",
            "Train Ordinal Model (Frank & Hall)",
            "Fit/Train an ordinal model by decomposition: the ordered target is cut K-1 times (`is the level above this cut?`) and each cut is handed to an ordinary binary classifier, with the predicted level read back as the number of cuts answered yes. This is the one ordinal trainer here that is not linear in the features, so reach for it when the boundary between levels bends in a way the Proportional Odds and Ridge trainers cannot follow. The price is that the K-1 sub-models are fitted independently: there is no single latent scale, no coefficient vector to read a direction off, and no calibrated per-level probabilities - use Proportional Odds when you need those. Every declared level must occur in the training data at the bottom and at the top of the ordering, otherwise a cut has only one class and cannot be fitted. A Random Forest base is the sturdiest choice and by far the costliest: each cut grows its own full forest, so training costs K-1 forests and the saved model carries every tree of every one of them.",
            "AI/ML/Ordinal",
        );
        node.set_version(2);
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(5) // K-1 independent fits, one full model per cut
                .set_governance(5) // No shared coefficients; each cut explains only itself
                .set_reliability(6)
                .set_cost(5)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins ordinal model training",
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
            "Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one. Listing a level that never occurs at either end of the ordering makes its cut unfittable and is rejected.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "base_learner",
            "Base Learner",
            "Which binary classifier is fitted for each of the K-1 cuts. Decision Tree follows non-linear, non-monotone boundaries and needs no feature scaling, at the cost of overfitting when left deep. Gaussian Naive Bayes is far cheaper and stays stable when rows are few relative to columns, but assumes the features are independent and roughly normal on each side of a cut. Random Forest bags many trees per cut and averages away most of a single tree's variance, usually making it the strongest option here - but it fits one entire forest per cut, so both the training time and the size of the saved model are multiplied by K-1.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(BASE_LEARNERS.iter().map(|base| base.to_string()).collect())
                .build(),
        )
        .set_default_value(Some(json!(BASE_DECISION_TREE)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once training completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the trained decomposition. Predictions come back as your original level labels.",
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

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let source: String = context.evaluate_pin("source").await?;
        let class_order: String = context.evaluate_pin("class_order").await?;
        let base_learner: String = context.evaluate_pin("base_learner").await?;

        // The hyperparameter pins are added by `on_update` for the selected base only, so they are
        // resolved before the expensive database read and never read for the other bases.
        let mut base = match base_learner.as_str() {
            BASE_DECISION_TREE => {
                let max_depth: i64 = context.evaluate_pin("max_depth").await.unwrap_or(10);
                let min_samples_split: i64 =
                    context.evaluate_pin("min_samples_split").await.unwrap_or(2);
                let split_quality: String = context
                    .evaluate_pin("split_quality")
                    .await
                    .unwrap_or_else(|_| "Gini".to_string());

                let split_quality = match split_quality.as_str() {
                    "Gini" => SplitQuality::Gini,
                    "Entropy" => SplitQuality::Entropy,
                    other => {
                        return Err(anyhow!(
                            "Unknown split quality `{other}`, expected `Gini` or `Entropy`"
                        ));
                    }
                };
                if min_samples_split < 2 {
                    return Err(anyhow!(
                        "`Min Samples Split` must be at least 2, got {min_samples_split}"
                    ));
                }
                if max_depth < 0 {
                    return Err(anyhow!(
                        "`Max Depth` must be 0 (unlimited) or a positive depth, got {max_depth}"
                    ));
                }

                let mut params =
                    LinfaDecisionTree::<f64, bool>::params().split_quality(split_quality);
                if max_depth > 0 {
                    params = params.max_depth(Some(max_depth as usize));
                }
                params = params.min_weight_split(min_samples_split as f32);
                BaseParams::DecisionTree(params)
            }
            BASE_GAUSSIAN_NAIVE_BAYES => {
                let var_smoothing: f64 =
                    context.evaluate_pin("var_smoothing").await.unwrap_or(1e-9);
                if !var_smoothing.is_finite() || var_smoothing < 0.0 {
                    return Err(anyhow!(
                        "`Variance Smoothing` must be a finite value >= 0, got {var_smoothing}"
                    ));
                }
                BaseParams::GaussianNaiveBayes(
                    GaussianNb::<f64, bool>::params().var_smoothing(var_smoothing),
                )
            }
            BASE_RANDOM_FOREST => {
                let ensemble_size: i64 = context.evaluate_pin("ensemble_size").await.unwrap_or(100);
                let bootstrap_proportion: f64 = context
                    .evaluate_pin("bootstrap_proportion")
                    .await
                    .unwrap_or(0.7);
                let feature_proportion: f64 = context
                    .evaluate_pin("feature_proportion")
                    .await
                    .unwrap_or(0.0);
                let max_depth: i64 = context.evaluate_pin("forest_max_depth").await.unwrap_or(10);
                let min_weight_split: f64 = context
                    .evaluate_pin("forest_min_weight_split")
                    .await
                    .unwrap_or(2.0);
                let seed: i64 = context.evaluate_pin("seed").await.unwrap_or(42);

                if ensemble_size < 1 {
                    return Err(anyhow!(
                        "`Ensemble Size` must be at least 1, got {ensemble_size}"
                    ));
                }
                // linfa rejects a proportion outside (0, 1] itself, but only after the whole
                // training matrix has been read and copied once per cut.
                if !bootstrap_proportion.is_finite()
                    || bootstrap_proportion <= 0.0
                    || bootstrap_proportion > 1.0
                {
                    return Err(anyhow!(
                        "`Bootstrap Proportion` must be a finite value greater than 0 and at most 1, got {bootstrap_proportion}"
                    ));
                }
                if !feature_proportion.is_finite() || !(0.0..=1.0).contains(&feature_proportion) {
                    return Err(anyhow!(
                        "`Feature Proportion` must be a finite value between 0 and 1, got {feature_proportion}. Use 0 for the sqrt(feature count) default."
                    ));
                }
                if !min_weight_split.is_finite() || min_weight_split <= 0.0 {
                    return Err(anyhow!(
                        "`Min Samples Split` must be a finite value greater than 0, got {min_weight_split}"
                    ));
                }
                if max_depth < 0 {
                    return Err(anyhow!(
                        "`Max Depth` must be 0 (unlimited) or a positive depth, got {max_depth}"
                    ));
                }
                if seed < 0 {
                    return Err(anyhow!("`Seed` must not be negative, got {seed}"));
                }

                BaseParams::RandomForest(ForestSettings {
                    ensemble_size: ensemble_size as usize,
                    bootstrap_proportion,
                    feature_proportion,
                    max_depth,
                    min_weight_split,
                    seed: seed as u64,
                })
            }
            other => {
                return Err(anyhow!(
                    "Unknown base learner `{other}`, expected one of: {}",
                    BASE_LEARNERS.join(", ")
                ));
            }
        };

        let explicit_order: Vec<String> = class_order
            .split(',')
            .map(str::trim)
            .filter(|level| !level.is_empty())
            .map(ToOwned::to_owned)
            .collect();

        let t0 = std::time::Instant::now();
        let (train_array, ranks, classes, levels, targets_col) = match source.as_str() {
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
                        "No training records in the database; the Frank & Hall decomposition needs at least one row per side of every cut"
                    ));
                }

                let train_array = values_to_array2_f64(&records, &records_col)?;
                let (ranks, classes, levels) = values_to_array1_ordinal(
                    &records,
                    &targets_col,
                    (!explicit_order.is_empty()).then_some(explicit_order.as_slice()),
                )?;
                (train_array, ranks, classes, levels, targets_col)
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
        // The decomposition rejects the whole matrix with one message, and the tree splits sort
        // feature values with `partial_cmp`, so the offending cell is resolved here instead.
        if let Some(((row, col), value)) = train_array
            .indexed_iter()
            .find(|(_, value)| !value.is_finite())
        {
            return Err(anyhow!(
                "Training feature at row {row}, column {col} is {value}; ordinal fitting needs finite features. Clean or impute the column before training."
            ));
        }

        if let Some(auto) = base.resolve_feature_proportion(n_features) {
            context.log_message(
                &format!(
                    "Feature Proportion auto-selected as {auto:.4} (sqrt of {n_features} features)"
                ),
                LogLevel::Debug,
            );
        }

        let observed: HashSet<usize> = ranks.iter().copied().collect();
        if observed.len() < 2 {
            let seen = observed
                .iter()
                .filter_map(|rank| levels.labels.get(*rank))
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            return Err(anyhow!(
                "Target col `{targets_col}` holds only [{seen}]. An ordinal model needs at least 2 distinct levels to have an order to learn. Widen the training set or check the target column."
            ));
        }

        let ordering_source = match levels.ordering {
            OrdinalOrdering::Explicit => "from your Class Order list",
            OrdinalOrdering::Numeric => "inferred by reading the labels as numbers",
        };
        // Training on a wrong order fails silently — the cuts are simply placed in the wrong
        // direction — so the resolved order has to be visible in the run log, not only on the pin.
        context.log_message(
            &format!(
                "Ordinal level order ({ordering_source}): {}",
                levels.labels.join(" < ")
            ),
            LogLevel::Info,
        );

        // The rank space is the full level list, not just the observed levels: an explicit order
        // may name a level in the middle that the sample never reached, and its cut stays well
        // posed. A missing lowest or highest level does not, and is rejected by the fit below.
        let n_classes = levels.labels.len();
        let base_label = base.label();
        let n_cuts = n_classes.saturating_sub(1);

        // A forest per cut multiplies both the fit and the stored model, which is the one cost of
        // this node that is easy to trigger by accident.
        if let Some(per_cut) = base.ensemble_size() {
            let total = per_cut.saturating_mul(n_cuts);
            let message = format!(
                "Random Forest base: {per_cut} trees per cut x {n_cuts} cuts = {total} trees to fit and to store"
            );
            if total > LARGE_FOREST_TOTAL_WARNING {
                context.log_message(&message, LogLevel::Warn);
            } else {
                context.log_message(&message, LogLevel::Debug);
            }
        }

        let t0 = std::time::Instant::now();
        let dataset = DatasetBase::new(train_array, ranks);
        let fitted = fit_decomposition(base, &dataset, n_classes).map_err(|err| {
            anyhow!(
                "Frank & Hall fit failed with a {base_label} base: {err}. Levels by rank, lowest first: {}",
                levels.labels.join(" < ")
            )
        })?;
        context.log_message(&format!("Fit model: {:?}", t0.elapsed()), LogLevel::Debug);

        context.log_message(
            &format!(
                "Frank & Hall decomposition fit on {n_samples} samples x {n_features} features: {n_cuts} binary {base_label} models across {n_classes} levels"
            ),
            LogLevel::Debug,
        );
        if observed.len() < n_classes {
            context.log_message(
                &format!(
                    "{} of the {n_classes} declared levels never occur in the training data. Their cuts are still fitted, but no sample supports them and the model will rarely predict them.",
                    n_classes - observed.len()
                ),
                LogLevel::Warn,
            );
        }

        let model = MLModel::OrdinalFrankHall(ModelWithMeta {
            model: fitted,
            classes: Some(classes),
        });
        let node_model = NodeMLModel::new(context, model).await;

        context.set_pin_value("model", json!(node_model)).await?;
        context.set_pin_value("levels", json!(levels)).await?;
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

        // Only the selected base's hyperparameters are shown: a pin the fit would ignore reads as
        // a knob that does nothing.
        match selected_value(node, "base_learner").as_str() {
            BASE_GAUSSIAN_NAIVE_BAYES => {
                if node.get_pin_by_name("var_smoothing").is_none() {
                    node.add_input_pin(
                        "var_smoothing",
                        "Variance Smoothing",
                        "Fraction of the largest feature variance added to every variance estimate. Guards against a feature that is constant on one side of a cut, which would otherwise give a zero variance and an infinite likelihood. Raise it when the fit produces non-finite scores.",
                        VariableType::Float,
                    )
                    .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
                    .set_default_value(Some(json!(1e-9)));
                }
                remove_pins(node, &TREE_PINS);
                remove_pins(node, &FOREST_PINS);
            }
            BASE_DECISION_TREE => {
                if node.get_pin_by_name("max_depth").is_none() {
                    node.add_input_pin(
                        "max_depth",
                        "Max Depth",
                        "Depth limit for each cut's tree; 0 leaves it unlimited. Every cut is fitted independently, so a deep tree overfits K-1 times over. Lower it first when training accuracy far exceeds validation accuracy.",
                        VariableType::Integer,
                    )
                    .set_options(PinOptions::new().set_range((0.0, 1000.0)).build())
                    .set_default_value(Some(json!(10)));
                }
                if node.get_pin_by_name("min_samples_split").is_none() {
                    node.add_input_pin(
                        "min_samples_split",
                        "Min Samples Split",
                        "Fewest samples a node must hold before it may be split. The cuts nearest the ends of the ordering see the most lopsided class balance, so raising this is the cheapest way to stop them splitting on noise.",
                        VariableType::Integer,
                    )
                    .set_options(PinOptions::new().set_range((2.0, 10000.0)).build())
                    .set_default_value(Some(json!(2)));
                }
                if node.get_pin_by_name("split_quality").is_none() {
                    node.add_input_pin(
                        "split_quality",
                        "Split Quality",
                        "Impurity metric that scores candidate splits. Gini is cheaper, Entropy favours balanced information gain.",
                        VariableType::String,
                    )
                    .set_options(
                        PinOptions::new()
                            .set_valid_values(vec!["Gini".to_string(), "Entropy".to_string()])
                            .build(),
                    )
                    .set_default_value(Some(json!("Gini")));
                }
                remove_pins(node, &BAYES_PINS);
                remove_pins(node, &FOREST_PINS);
            }
            BASE_RANDOM_FOREST => {
                if node.get_pin_by_name("ensemble_size").is_none() {
                    node.add_input_pin(
                        "ensemble_size",
                        "Ensemble Size",
                        "Trees grown for each cut. The whole model holds this many trees times K-1 cuts, and both the fit time and the saved model scale with that product, so a 100-tree forest on a 5-level target is 400 trees.",
                        VariableType::Integer,
                    )
                    .set_options(PinOptions::new().set_range((1.0, 2000.0)).build())
                    .set_default_value(Some(json!(100)));
                }
                if node.get_pin_by_name("bootstrap_proportion").is_none() {
                    node.add_input_pin(
                        "bootstrap_proportion",
                        "Bootstrap Proportion",
                        "Share of the training rows drawn (with replacement) for each tree. Must be greater than 0 and at most 1. Lower it to decorrelate the trees, at the cost of showing each one less of an already lopsided cut.",
                        VariableType::Float,
                    )
                    .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
                    .set_default_value(Some(json!(0.7)));
                }
                if node.get_pin_by_name("feature_proportion").is_none() {
                    node.add_input_pin(
                        "feature_proportion",
                        "Feature Proportion",
                        "Share of the features offered to each tree. Must be at most 1. Leave at 0 for the textbook default of sqrt(feature count) features per tree.",
                        VariableType::Float,
                    )
                    .set_options(PinOptions::new().set_range((0.0, 1.0)).build())
                    .set_default_value(Some(json!(0.0)));
                }
                if node.get_pin_by_name("forest_max_depth").is_none() {
                    node.add_input_pin(
                        "forest_max_depth",
                        "Max Depth",
                        "Depth limit for every tree in every forest; 0 leaves it unlimited. Bagging already absorbs a single deep tree's variance, so this matters less here than for a lone Decision Tree - but it is the main lever on the size of the saved model.",
                        VariableType::Integer,
                    )
                    .set_options(PinOptions::new().set_range((0.0, 1000.0)).build())
                    .set_default_value(Some(json!(10)));
                }
                if node.get_pin_by_name("forest_min_weight_split").is_none() {
                    node.add_input_pin(
                        "forest_min_weight_split",
                        "Min Samples Split",
                        "Minimum summed sample weight a node needs before it may be split. Without row weights this is simply the minimum number of samples. Each tree only sees its bootstrap sample, so this counts rows of that sample, not of the full training set.",
                        VariableType::Float,
                    )
                    .set_options(PinOptions::new().set_range((1.0, 100000.0)).build())
                    .set_default_value(Some(json!(2.0)));
                }
                if node.get_pin_by_name("seed").is_none() {
                    node.add_input_pin(
                        "seed",
                        "Seed",
                        "Seed for the bootstrap and feature sampling. One seeded generator serves every cut, so all K-1 forests draw the same rows and features and differ only in the question they answer. Note that the base trees are not bit-exact across processes: linfa resolves modal-class ties in hash-map iteration order, which Rust re-randomizes on every run.",
                        VariableType::Integer,
                    )
                    .set_options(PinOptions::new().set_range((0.0, 4294967295.0)).build())
                    .set_default_value(Some(json!(42)));
                }
                remove_pins(node, &TREE_PINS);
                remove_pins(node, &BAYES_PINS);
            }
            other => {
                node.error = Some(format!(
                    "Unknown base learner `{other}`, expected one of: {}",
                    BASE_LEARNERS.join(", ")
                ));
            }
        }

        if selected_value(node, "source") == *"Database" {
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

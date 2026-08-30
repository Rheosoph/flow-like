// ml — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace ml {
    // === AI/ML ===

    /**
     * Load Trained ML Model from Path
     * @node load_ml_model @alias loadMlModel
     * @param path — Filesystem or storage path pointing at the serialized model JSON
     * @returns model — Handle to the loaded machine learning model
     * @impure has side effects / drives control flow
     */
    function load({ path: Struct }): Struct;

    /**
     * Load Trained ML Model from Path using fast binary format (Fory)
     * @node load_ml_model_binary @alias loadMlModelBinary
     * @param path — Filesystem or storage path pointing at the serialized model binary (.flmodel)
     * @returns model — Handle to the loaded machine learning model
     * @impure has side effects / drives control flow
     */
    function loadBinary({ path: Struct }): Struct;

    /**
     * Predict with Machine Learning Model
     * @node ml_predict @receiver model @alias mlPredict
     * @param model — Trained ML model to use for inference (receiver: `this` in `x.predict(...)`)
     * @param source (optional) — Choose the input type for prediction (database rows or raw vector)
     * @param batchSize (optional) — Number of records to process per batch (default: 5000, 0 = process all at once)
     * @impure has side effects / drives control flow
     */
    function predict(this: NodeMLModel, { model: Struct, source?: string, batchSize?: int }): void;

    /**
     * Save Trained ML Model to Path
     * @node save_ml_model @receiver model @alias saveMlModel
     * @param model — Any trained ML model handle to persist (receiver: `this` in `x.save(...)`)
     * @param path — Destination path where the model JSON should be written
     * @impure has side effects / drives control flow
     */
    function save(this: NodeMLModel, { model: Struct, path: Struct }): void;

    /**
     * Save Trained ML Model to Path using fast binary format (Fory)
     * @node save_ml_model_binary @receiver model @alias saveMlModelBinary
     * @param model — Any trained ML model handle to persist (receiver: `this` in `x.saveBinary(...)`)
     * @param path — Destination path where the model binary should be written (.flmodel)
     * @impure has side effects / drives control flow
     */
    function saveBinary(this: NodeMLModel, { model: Struct, path: Struct }): void;

    // === AI/ML/Classification ===

    /**
     * Fit/Train an AdaBoost classifier using multi-class SAMME boosting over shallow Decision Trees. Each learner focuses on the rows its predecessors got wrong, so boosting usually beats a single tree on weak signal, but it is far more sensitive to label noise and outliers than Random Forest. Estimators is a maximum, not a guarantee: boosting stops early once a learner is no better than random guessing.
     * @node fit_adaboost @alias fitAdaboost
     * @param source (optional) — Choose which backend supplies the training data
     * @param nEstimators (optional) — Maximum number of boosting rounds. Boosting stops early once a learner performs no better than random guessing, so the fitted model may hold fewer estimators than requested.
     * @param learningRate (optional) — Shrinkage applied to each learner's vote. Must be positive. Values below 1 regularize the ensemble but need more estimators; 0.1 with 500 estimators is a common pairing.
     * @param maxDepth (optional) — Depth of each weak learner. AdaBoost is designed around shallow trees; 1 gives classic decision stumps. Deep base trees defeat the point of boosting and overfit quickly.
     * @param seed (optional) — Seed for the base learner sampling. Fixing it makes the sampling reproducible. Note that the base trees are not bit-exact across processes: linfa resolves modal-class ties in hash-map iteration order, which Rust re-randomizes on every run.
     * @returns model — Thread-safe handle to the trained AdaBoost classifier
     * @returns estimatorsKept — Number of estimators actually retained after early stopping, which may be lower than the requested maximum
     * @impure has side effects / drives control flow
     */
    function fitAdaboost({ source?: string, nEstimators?: int, learningRate?: float, maxDepth?: int, seed?: int }): { model: Struct, estimatorsKept: int };

    /**
     * Fit/Train a Decision Tree classifier. Native multi-class support with interpretable rules.
     * @node fit_decision_tree @alias fitDecisionTree
     * @param source (optional) — Choose which backend supplies the training data
     * @param maxDepth (optional) — Maximum depth of the tree. None means unlimited.
     * @param minSamplesSplit (optional) — Minimum number of samples required to split a node
     * @param splitQuality (optional) — Impurity metric that scores candidate splits. Gini is cheaper, Entropy favours balanced information gain.
     * @param minWeightLeaf (optional) — Minimum number of samples (total sample weight) a split has to place in each leaf
     * @param minImpurityDecrease (optional) — Minimum impurity decrease a split has to bring to be applied. Must be greater than zero; larger values prune harder.
     * @returns model — Thread-safe handle to the trained Decision Tree classifier
     * @impure has side effects / drives control flow
     */
    function fitDecisionTree({ source?: string, maxDepth?: int, minSamplesSplit?: int, splitQuality?: string, minWeightLeaf?: float, minImpurityDecrease?: float }): Struct;

    /**
     * Fit a K-Nearest-Neighbours classifier. Non-parametric and instance based: the fitted model embeds a verbatim copy of the whole training set instead of learned coefficients, so every training row (and any personal data in it) travels with the model, is written into every saved model file and can be reconstructed by anyone holding it. Treat the model with the same care as the source table.
     * @node fit_knn_classifier @alias fitKnnClassifier
     * @param source (optional) — Choose which backend supplies the training data
     * @param k (optional) — How many nearest training rows vote on each prediction. Must be at least 1 and cannot exceed the number of training rows. Larger values smooth the decision boundary.
     * @param distanceWeighted (optional) — Weight each neighbour by the inverse of its distance instead of counting every neighbour equally. Helps when k is large or classes overlap.
     * @returns model — Thread-safe handle to the trained KNN classifier. Contains a full copy of the training set.
     * @impure has side effects / drives control flow
     */
    function fitKnnClassifier({ source?: string, k?: int, distanceWeighted?: bool }): Struct;

    /**
     * Fit/Train a Logistic Regression classifier with L2 regularization. Handles binary and multi-class targets and yields interpretable coefficients plus calibrated probabilities. The solver expects features on a comparable scale - fit a Feature Scaler first if your columns have very different ranges.
     * @node fit_logistic_regression @alias fitLogisticRegression
     * @param source (optional) — Choose which backend supplies the training data
     * @param mode (optional) — Auto picks the binary solver for two classes and the multinomial (softmax) solver for more. Binary and Multinomial force one of them.
     * @param alpha (optional) — Weight of the L2 penalty on the coefficients. 0 disables regularization, larger values shrink the model harder.
     * @param fitIntercept (optional) — Fit a bias term. Disable only when the features are already centered.
     * @param maxIterations (optional) — Upper bound on LBFGS iterations. Raise it when training accuracy stays at the baseline.
     * @param gradientTolerance (optional) — Smallest gradient norm that still continues the solver. Smaller means a tighter fit and more iterations.
     * @param threshold (optional) — Probability above which linfa's positive class is predicted. You do not choose that class: linfa assigns it to whichever label sorts second, which for a typical imbalanced dataset is the majority class. Raising the threshold therefore makes the OTHER class — usually the rare one — more likely to be predicted. The class the threshold actually governs is logged when training runs. Binary mode only, ignored for multinomial targets.
     * @returns model — Thread-safe handle to the trained Logistic Regression classifier
     * @impure has side effects / drives control flow
     */
    function fitLogisticRegression({ source?: string, mode?: string, alpha?: float, fitIntercept?: bool, maxIterations?: int, gradientTolerance?: float, threshold?: float }): Struct;

    /**
     * Fit/Train a Multinomial Naive Bayes classifier, the standard baseline for text and other count data. Features must be non-negative counts or TF-IDF weights, which is what the Fit TF-IDF Vectorizer node produces. Native multi-class support and a single pass over the data.
     * @node fit_multinomial_naive_bayes @alias fitMultinomialNaiveBayes
     * @param source (optional) — Choose which backend supplies the training data
     * @param alpha (optional) — Additive (Laplace/Lidstone) smoothing added to every feature count. 1.0 is the usual choice; smaller values trust the training counts more, and 0 disables smoothing so any term unseen in a class makes that class impossible.
     * @returns model — Thread-safe handle to the trained Multinomial Naive Bayes classifier
     * @impure has side effects / drives control flow
     */
    function fitMultinomialNaiveBayes({ source?: string, alpha?: float }): Struct;

    /**
     * Fit/Train a Gaussian Naive Bayes classifier. Native multi-class support - no need for One-vs-All.
     * @node fit_naive_bayes @alias fitNaiveBayes
     * @param source (optional) — Choose which backend supplies the training data
     * @returns model — Thread-safe handle to the trained Naive Bayes classifier
     * @impure has side effects / drives control flow
     */
    function fitNaiveBayes({ source?: string }): Struct;

    /**
     * Fit a One-Class SVM on normal observations only. Predictions flag whether a new row is an inlier (1) or an outlier (0).
     * @node fit_one_class_svm @alias fitOneClassSvm
     * @param source (optional) — Choose which backend supplies the training data
     * @param nu (optional) — Upper bound on the fraction of training rows the model is allowed to treat as outliers, in (0, 1]. Raise it when the training set is known to be contaminated.
     * @param kernel (optional) — Feature-space mapping. Gaussian wraps a tight non-linear boundary around the data, Linear yields a half-space, Polynomial adds interaction terms.
     * @param kernelParam (optional) — Gaussian: the eps in exp(-||x - x'||^2 / eps), larger means a looser boundary. Polynomial: the degree of (<x, x'> + 1)^degree. Ignored for Linear.
     * @param tolerance (optional) — Stopping threshold of the SMO solver. Smaller values train longer for a more precise boundary.
     * @returns model — Thread-safe handle to the trained One-Class SVM
     * @returns supportVectors — Number of training rows that define the learned boundary
     * @impure has side effects / drives control flow
     */
    function fitOneClassSvm({ source?: string, nu?: float, kernel?: string, kernelParam?: float, tolerance?: float }): { model: Struct, supportVectors: int };

    /**
     * Fit/Train a Random Forest classifier: many Decision Trees, each grown on a bootstrapped sample of the rows and a random subset of the features, combined by majority vote. Far more robust to overfitting than a single tree, at the price of interpretability. Model size and fit time grow linearly with Ensemble Size, so a forest of 500 trees costs roughly 500x a single tree.
     * @node fit_random_forest @alias fitRandomForest
     * @param source (optional) — Choose which backend supplies the training data
     * @param ensembleSize (optional) — Number of Decision Trees to grow. Both fit time and the size of the saved model scale linearly with this value.
     * @param bootstrapProportion (optional) — Share of the training rows drawn (with replacement) for each tree. Must be greater than 0 and at most 1.
     * @param featureProportion (optional) — Share of the features offered to each tree. Must be at most 1. Leave at 0 for the textbook default of sqrt(feature count) features per tree.
     * @param maxDepth (optional) — Maximum depth of each tree. 0 or less means unlimited, which grows deeper trees and a larger model.
     * @param minWeightSplit (optional) — Minimum summed sample weight a node needs before it may be split. Without row weights this is simply the minimum number of samples.
     * @param seed (optional) — Seed for the bootstrap and feature sampling. Fixing it makes the row and feature draws reproducible. Note that the base trees are not bit-exact across processes: linfa resolves modal-class ties in hash-map iteration order, which Rust re-randomizes on every run.
     * @returns model — Thread-safe handle to the trained Random Forest classifier
     * @impure has side effects / drives control flow
     */
    function fitRandomForest({ source?: string, ensembleSize?: int, bootstrapProportion?: float, featureProportion?: float, maxDepth?: int, minWeightSplit?: float, seed?: int }): Struct;

    /**
     * Fit/Train Support Vector Machines (SVM) for Multi-Class Classification
     * @node fit_svm_multi_class @alias fitSvmMultiClass
     * @param source (optional) — Choose which backend supplies the training data
     * @param kernel (optional) — Feature-space mapping. Gaussian separates non-linear classes, Linear is the plain SVM, Polynomial adds interaction terms.
     * @param kernelParam (optional) — Gaussian: the eps in exp(-||x - x'||^2 / eps), larger means smoother boundaries. Polynomial: the degree of (<x, x'> + 1)^degree. Ignored for Linear.
     * @param c (optional) — Penalty for misclassified training rows, applied to both the positive and the negative side. Higher values fit the training data harder and risk overfitting.
     * @returns model — Thread-safe handle to the trained SVM classifier
     * @impure has side effects / drives control flow
     */
    function fitSvmMultiClass({ source?: string, kernel?: string, kernelParam?: float, c?: float }): Struct;

    // === AI/ML/Clustering ===

    /**
     * Fit/Train DBSCAN Density-Based Clustering
     * @node fit_dbscan @alias fitDbscan
     * @param epsilon (optional) — Maximum distance between points in the same cluster
     * @param minPoints (optional) — Minimum points required to form a dense region
     * @param source (optional) — Choose which backend supplies the training data
     * @returns nClusters — Number of clusters found (excluding noise)
     * @returns nNoise — Number of points classified as noise
     * @impure has side effects / drives control flow
     */
    function fitDbscan({ epsilon?: float, minPoints?: int, source?: string }): { nClusters: int, nNoise: int };

    /**
     * Fit/Train a Gaussian Mixture Model. Soft clustering with per-component covariances and mixture weights, fitted by Expectation-Maximization.
     * @node fit_gaussian_mixture @alias fitGaussianMixture
     * @param source (optional) — Choose which backend supplies the training data
     * @param nClusters (optional) — Number of Gaussian components (k) in the mixture. Each component costs a full d x d covariance matrix.
     * @param covarianceType (optional) — Shape of each component's covariance. linfa 0.8 implements full covariances only - scikit-learn's diag, tied and spherical variants do not exist here, so every component always costs d x d parameters.
     * @param initMethod (optional) — How initial responsibilities are built: KMeans runs a KMeans pass first (usually the better optimum), Random draws them uniformly.
     * @param nRuns (optional) — Number of EM passes. Note: linfa 0.8 continues each pass from the previous parameters instead of re-initializing, so this multiplies the iteration budget (Runs x Max Iterations) rather than performing independent restarts. Vary the Seed for a genuinely different start.
     * @param tolerance (optional) — EM stops once the average log-likelihood gain per iteration falls below this value
     * @param regCovariance (optional) — Non-negative value added to each covariance diagonal to keep it positive definite. Raise it when the fit reports a singular covariance; 0 makes duplicate or constant rows fail outright.
     * @param maxNIterations (optional) — Maximum number of EM iterations per run
     * @param seed (optional) — Seed for the training row order. linfa 0.8 hard-codes its internal RNG (seed 42) and exposes no seeding hook on this entry point, so changing the seed re-orders the rows, which is what changes the initial responsibilities. Keep 42 to reproduce linfa's stock ordering.
     * @returns model — Thread-safe handle to the trained Gaussian Mixture model
     * @returns weights — Fitted mixture proportions, one per component, summing to 1. A tiny weight means that component captured almost no data.
     * @impure has side effects / drives control flow
     */
    function fitGaussianMixture({ source?: string, nClusters?: int, covarianceType?: string, initMethod?: string, nRuns?: int, tolerance?: float, regCovariance?: float, maxNIterations?: int, seed?: int }): { model: Struct, weights: float[] };

    /**
     * Fit/Train KMeans Clustering
     * @node fit_kmeans @alias fitKmeans
     * @param cluster (optional) — Choose how many centroids to fit
     * @param source (optional) — Choose which backend supplies the training data
     * @returns model — Thread-safe handle to the trained KMeans model
     * @impure has side effects / drives control flow
     */
    function fitKmeans({ cluster?: int, source?: string }): Struct;

    // === AI/ML/Dataset ===

    /**
     * Generate K train/test splits for cross-validation. Each fold uses (K-1)/K data for training and 1/K for validation, and runs the connected fold branch once per fold.
     * @node ai_ml_dataset_kfold @alias aiMlDatasetKfold
     * @param k (optional) — Number of folds for cross-validation (typically 5 or 10)
     * @param shuffle (optional) — Randomly shuffle data before splitting
     * @param source — Source database containing the dataset. It is only read, never modified.
     * @param trainDb — Database to receive training data for each fold (will be cleared and filled K times)
     * @param testDb — Database to receive validation data for each fold (will be cleared and filled K times)
     * @returns foldIndex — Current fold index (0 to K-1)
     * @returns info — Information about the K-fold split
     * @impure has side effects / drives control flow
     */
    function kfold({ k?: int, shuffle?: bool, source: Struct, trainDb: Struct, testDb: Struct }): { foldIndex: int, info: Struct };

    /**
     * Random sample N records or a ratio from a dataset
     * @node ai_ml_dataset_sample @alias aiMlDatasetSample
     * @param sampleCount (optional) — Number of records to sample (if set, takes precedence over ratio)
     * @param sampleRatio (optional) — Ratio of records to sample (0.0 to 1.0, used if sample_count is 0)
     * @param source — Data Source (DB or CSV)
     * @param target — Destination database connection that receives the sampled rows
     * @returns sampledCount — Number of records that were sampled
     * @impure has side effects / drives control flow
     */
    function sampleDataset({ sampleCount?: int, sampleRatio?: float, source: Struct, target: Struct }): int;

    /**
     * Shuffle dataset rows randomly
     * @node ai_ml_dataset_shuffle @alias aiMlDatasetShuffle
     * @param source — Data Source (DB or CSV)
     * @param target — Destination database connection that receives the shuffled rows
     * @impure has side effects / drives control flow
     */
    function shuffleDataset({ source: Struct, target: Struct }): void;

    /**
     * Split a dataset into training and testing subsets
     * @node ai_ml_dataset_split @alias aiMlDatasetSplit
     * @param split (optional) — Ratio used for assigning rows to the training set (rest goes to test)
     * @param source — Data Source (DB or CSV)
     * @param train — Destination database connection that receives the training rows
     * @param test — Destination database connection that receives the testing rows
     * @impure has side effects / drives control flow
     */
    function splitDataset({ split?: float, source: Struct, train: Struct, test: Struct }): void;

    /**
     * Split a dataset into training and testing subsets, keeping every class at its original proportion in both subsets
     * @node ai_ml_dataset_stratified_split @alias aiMlDatasetStratifiedSplit
     * @param split (optional) — Share of each class that goes to the training set (rest goes to test). Must be between 0 and 1, exclusive
     * @param labelColumn (optional) — Name of the column containing class labels for stratification
     * @param seed (optional) — Seed for the per-class shuffle. Any non-zero value makes the split reproducible; 0 draws a fresh seed each run and logs it
     * @param source — Data Source (DB or CSV)
     * @param train — Destination database that receives the training rows. It is cleared before every run
     * @param test — Destination database that receives the testing rows. It is cleared before every run
     * @impure has side effects / drives control flow
     */
    function stratifiedSplit({ split?: float, labelColumn?: string, seed?: int, source: Struct, train: Struct, test: Struct }): void;

    // === AI/ML/Metrics ===

    /**
     * Calculate classification accuracy by comparing predictions to actual values
     * @node ml_eval_accuracy @alias mlEvalAccuracy
     * @param database — Database connection containing predictions and actuals
     * @param predictionsCol (optional) — Column name containing predicted values
     * @param actualsCol (optional) — Column name containing actual/true values
     * @returns result — Accuracy metrics including score and counts
     * @impure has side effects / drives control flow
     */
    function evalAccuracy({ database: Struct, predictionsCol?: string, actualsCol?: string }): Struct;

    /**
     * Build confusion matrix and calculate precision, recall, and F1 score
     * @node ml_eval_confusion_matrix @alias mlEvalConfusionMatrix
     * @param database — Database connection containing predictions and actuals
     * @param predictionsCol (optional) — Column name containing predicted values
     * @param actualsCol (optional) — Column name containing actual/true values
     * @returns result — Confusion matrix with precision, recall, and F1 metrics
     * @impure has side effects / drives control flow
     */
    function evalConfusionMatrix({ database: Struct, predictionsCol?: string, actualsCol?: string }): Struct;

    /**
     * Calculate MSE, RMSE, MAE, and R² for regression predictions
     * @node ml_eval_regression @alias mlEvalRegression
     * @param database — Database connection containing predictions and actuals
     * @param predictionsCol (optional) — Column name containing predicted float values
     * @param actualsCol (optional) — Column name containing actual/true float values
     * @returns result — Regression metrics (MSE, RMSE, MAE, R²)
     * @impure has side effects / drives control flow
     */
    function evalRegression({ database: Struct, predictionsCol?: string, actualsCol?: string }): Struct;

    /**
     * Threshold-free evaluation of a binary classifier: area under the ROC curve, log loss and the curve points. This is the payoff for Logistic Regression producing calibrated probabilities instead of bare class labels.
     * @node ml_roc_auc @alias mlRocAuc
     * @param database — Database connection containing the predicted probabilities and the true labels
     * @param probabilitiesCol (optional) — Column holding P(positive class) for each row, between 0 and 1 — the probability of the class named in Positive Label, NOT the probability of whichever class was predicted. No node writes this column for you: Predict in Database mode writes the predicted class only, and `confidence` is a field on the struct its Vector mode returns for one row, so build the column by looping rows through Vector mode. Convert as you go, because `confidence` is the winning class's probability: use it directly where the prediction is the positive class, and 1 - confidence elsewhere. A raw decision value or an uncalibrated score produces a meaningless curve.
     * @param actualsCol (optional) — Column holding the true binary label of each sample
     * @param positiveLabel (optional) — Value of the actuals column that counts as the positive class. Strings are compared literally, numbers numerically; booleans are always taken as-is.
     * @returns auc — Area under the ROC curve (0.5 = random, 1.0 = perfect)
     * @returns logLoss — Mean binary cross-entropy of the predicted probabilities (lower is better)
     * @returns result — AUC, log loss and the ROC curve points ordered by ascending false positive rate
     * @impure has side effects / drives control flow
     */
    function rocAuc({ database: Struct, probabilitiesCol?: string, actualsCol?: string, positiveLabel?: string }): { auc: float, logLoss: float, result: Struct };

    /**
     * Evaluate clustering quality: how much closer each sample sits to its own cluster than to the nearest other one (-1 to +1)
     * @node ml_silhouette_score @alias mlSilhouetteScore
     * @param database — Database connection containing the feature vectors and their cluster assignments
     * @param featuresCol (optional) — Column holding the feature vectors the clustering was computed on. Distances are euclidean, so scale the features first if their ranges differ.
     * @param labelsCol (optional) — Column holding the cluster assignment of each sample, as a string name or a non-negative integer id
     * @param maxSamples (optional) — Upper bound on the samples used. The metric compares every sample with every other one, so the cost grows quadratically; larger sets are sub-sampled evenly.
     * @returns score — Mean silhouette score across all evaluated samples (-1 to +1, higher is better)
     * @returns nSamples — Number of samples the score was computed on after sub-sampling
     * @returns nClusters — Number of distinct clusters found in the cluster column
     * @impure has side effects / drives control flow
     */
    function silhouetteScore({ database: Struct, featuresCol?: string, labelsCol?: string, maxSamples?: int }): { score: float, nSamples: int, nClusters: int };

    // === AI/ML/Model Info ===

    /**
     * Extract per-feature importance from a Decision Tree, Random Forest or AdaBoost model
     * @node ml_feature_importance @receiver model @alias mlFeatureImportance
     * @param model — Trained tree model (Decision Tree, Random Forest or AdaBoost) (receiver: `this` in `x.featureImportance(...)`)
     * @param featureNames (optional) — Optional column labels in training order. Unnamed columns fall back to feature_<index>.
     * @returns result — Per-feature importance with leaf and depth statistics
     * @returns importances — Normalized importance per feature, in column order
     * @returns topFeature — Name of the most important feature
     * @impure has side effects / drives control flow
     */
    function featureImportance(this: NodeMLModel, { model: Struct, featureNames?: string[] }): { result: Struct, importances: float[], topFeature: string };

    /**
     * Extract cluster centroids from a trained KMeans model
     * @node ml_get_kmeans_centroids @receiver model @alias mlGetKmeansCentroids
     * @param model — Trained KMeans model (receiver: `this` in `x.getKmeansCentroids(...)`)
     * @returns result — Cluster centroids with metadata
     * @impure has side effects / drives control flow
     */
    function getKmeansCentroids(this: NodeMLModel, { model: Struct }): Struct;

    /**
     * Extract coefficients and intercept from a trained Linear Regression model
     * @node ml_get_linear_coefficients @receiver model @alias mlGetLinearCoefficients
     * @param model — Trained Linear Regression model (receiver: `this` in `x.getLinearCoefficients(...)`)
     * @returns result — Regression coefficients with intercept
     * @impure has side effects / drives control flow
     */
    function getLinearCoefficients(this: NodeMLModel, { model: Struct }): Struct;

    /**
     * Get general information about any ML model
     * @node ml_model_info @receiver model @alias mlModelInfo
     * @param model — Any trained ML model (receiver: `this` in `x.info(...)`)
     * @returns info — Model information structure
     * @returns modelType — Model type as string
     * @impure has side effects / drives control flow
     */
    function info(this: NodeMLModel, { model: Struct }): { info: Struct, modelType: string };

    // === AI/ML/Ordinal ===

    /**
     * Fit/Train an ordinal model that compares each level with the one directly below it: `log( P(level k+1) / P(level k) ) = contrast_k + x . beta`. Its coefficients answer `what does one more unit of this feature do to my rating?` - `exp(coefficient)` is the factor on the odds of scoring one level higher rather than staying put, the same factor at every step. That is NOT what Train Ordinal Model (Proportional Odds) reports: a cumulative coefficient is the log odds ratio of everything AT OR BELOW a cut point against everything above it, pooling levels instead of comparing two neighbours. The same fitted number therefore means different things in the two families, and since one shared coefficient applies once per step here, the bottom-to-top effect is (levels - 1) times the per-step effect. Pick this for ratings, severity grades and Likert answers, where the question really is about one step; pick proportional odds when the question is about crossing a threshold (`does this case escalate past level 2?`). Fitted by penalized maximum likelihood over all levels jointly, so per-level probabilities are calibrated and the Predict node returns a confidence. Scale your features first with the Fit Feature Scaler node: this is a gradient fit, and unscaled columns make it converge slowly or not at all.
     * @node fit_ordinal_adjacent_category @alias fitOrdinalAdjacentCategory
     * @param source (optional) — Choose which backend supplies the training data
     * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one. Levels listed here but never seen in training still keep their slot in the ordering, so the contrasts stay comparable across runs.
     * @param alpha (optional) — Strength of the L2 penalty on the shared coefficients. The level contrasts are never penalized: shrinking those would pull neighbouring levels toward equal frequency, which asserts something about your data rather than limiting model complexity. 0 fits unpenalized. Raise it when the fit diverges or the coefficients blow up.
     * @param maxIterations (optional) — Iteration cap for the Adam optimizer. Training stops here even if the objective is still moving, which is reported on the Converged pin.
     * @param tolerance (optional) — Relative change in the objective below which training stops. Smaller values fit tighter but need more iterations; 0 always runs the full iteration budget.
     * @param learningRate (optional) — Adam step size. Lower it if training oscillates or produces non-finite values; raise it if the model has not converged within Max Iterations. Level scores here carry a factor of the level index, so a badly scaled step travels further than it would in a cumulative fit.
     * @returns model — Thread-safe handle to the trained adjacent-category model. Predictions come back as your original level labels, and because the fit maximizes a likelihood the Predict node also returns a per-level confidence.
     * @returns levels — The level order the model was actually trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when an ordinal model behaves oddly.
     * @returns converged — False when the optimizer hit Max Iterations before the objective settled. The model is still usable but under-fitted.
     * @returns coefficients — The shared per-feature coefficients together with the level contrasts, both of them PER-STEP quantities: `exp(coefficient)` multiplies the odds of landing one level higher rather than on the current one, which is a single step and not the cumulative `above this cut` odds ratio a proportional-odds model prints. The struct also carries `bottom_to_top_effect`, the same coefficient times (levels - 1), which is the magnitude to quote when someone asks about the full range. The contrasts are the same log odds at a zero score, one per adjacent pair; unlike cumulative cut points they are free intercepts and may DECREASE.
     * @impure has side effects / drives control flow
     */
    function fitOrdinalAdjacentCategory({ source?: string, classOrder?: string, alpha?: float, maxIterations?: int, tolerance?: float, learningRate?: float }): { model: Struct, levels: Struct, converged: bool, coefficients: Struct };

    /**
     * Fit/Train a continuation-ratio model on an ORDERED target that is really a process that can halt. It fits K-1 sub-models, where sub-model k answers `given this row reached level k, did it STOP there?`, so the model describes a progression through the levels instead of placing cut points on a latent scale. Reach for it when the levels are genuinely sequential and each one had to be passed to get to the next: escalation tiers, disease stages, how far a signup funnel got, how far an incident escalated before it was contained. Each sub-model carries its own coefficient vector, so nothing assumes proportional odds, and the per-level probabilities are exact by the chain rule rather than differences of two fits. The cost is strictness: because each sub-model is conditioned on having reached its level, EVERY level must occur in the training data, middle ones included. Scale your features first with the Fit Feature Scaler node: these are gradient fits, and unscaled columns make them converge slowly or not at all.
     * @node fit_ordinal_continuation_ratio @alias fitOrdinalContinuationRatio
     * @param source (optional) — Choose which backend supplies the training data
     * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one. Unlike the other ordinal nodes, a level you list here that never occurs in the data is rejected instead of merely left unpredicted: its sub-model would have no rows to separate.
     * @param link (optional) — The CDF each conditional stopping probability is read through. CLogLog is the standout pairing here: with it this model IS the discrete-time proportional-hazards (grouped survival) model, each sub-model's output is the hazard of stopping at that step, and a shared feature effect multiplies every hazard by the same factor — so for `how long / how far until something stopped` targets, pick CLogLog and read the fit as a survival model. Logit gives conditional log-odds, the classical continuation-ratio logit, and is the safe default. Probit assumes a normal latent variable per step. Cauchit is heavy-tailed, so extreme rows pull each sub-model far less.
     * @param alpha (optional) — Strength of the L2 penalty on each sub-model's coefficients; the intercepts are never penalized. Because the penalty is a fixed amount added to a summed log-likelihood, one value shrinks the high levels harder than the low ones — which is what you want, since those are the sub-models fitted on the fewest rows. Raise it when Subset Sizes shows a thin top end.
     * @param maxIterations (optional) — Iteration cap for the Adam optimizer, applied to EACH sub-model separately. A single sub-model stopping here makes Converged false.
     * @param tolerance (optional) — Relative change in a sub-model's objective below which its fit stops. The test is relative, so it means the same thing on the large bottom subset and the small top one. 0 always runs the full iteration budget.
     * @param learningRate (optional) — Adam step size, shared by every sub-model. Lower it if training oscillates or produces non-finite values; raise it if the model has not converged within Max Iterations.
     * @returns model — Thread-safe handle to the trained continuation-ratio model. Predictions come back as your original level labels, and the per-level probabilities behind them sum to exactly 1 because the chain rule telescopes.
     * @returns levels — The level order the model was actually trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when an ordinal model behaves oddly.
     * @returns subsetSizes — How many training rows each sub-model actually saw, lowest level first: entry k counts the rows that reached level k. It only ever decreases, so the LAST entry is the evidence behind your top level — the honest measure of how much to trust the high end of the fit. A small tail there means the top coefficients are noise, not a subtle effect.
     * @returns converged — True only when EVERY sub-model's objective settled before Max Iterations. One stubborn sub-model — usually the top one, fitted on the fewest rows — makes it false; the run log names which levels stalled.
     * @impure has side effects / drives control flow
     */
    function fitOrdinalContinuationRatio({ source?: string, classOrder?: string, link?: string, alpha?: float, maxIterations?: int, tolerance?: float, learningRate?: float }): { model: Struct, levels: Struct, subsetSizes: int[], converged: bool };

    /**
     * Fit/Train an ordinal model by decomposition: the ordered target is cut K-1 times (`is the level above this cut?`) and each cut is handed to an ordinary binary classifier, with the predicted level read back as the number of cuts answered yes. This is the one ordinal trainer here that is not linear in the features, so reach for it when the boundary between levels bends in a way the Proportional Odds and Ridge trainers cannot follow. The price is that the K-1 sub-models are fitted independently: there is no single latent scale, no coefficient vector to read a direction off, and no calibrated per-level probabilities - use Proportional Odds when you need those. Every declared level must occur in the training data at the bottom and at the top of the ordering, otherwise a cut has only one class and cannot be fitted. A Random Forest base is the sturdiest choice and by far the costliest: each cut grows its own full forest, so training costs K-1 forests and the saved model carries every tree of every one of them.
     * @node fit_ordinal_frank_hall @alias fitOrdinalFrankHall
     * @param source (optional) — Choose which backend supplies the training data
     * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one. Listing a level that never occurs at either end of the ordering makes its cut unfittable and is rejected.
     * @param baseLearner (optional) — Which binary classifier is fitted for each of the K-1 cuts. Decision Tree follows non-linear, non-monotone boundaries and needs no feature scaling, at the cost of overfitting when left deep. Gaussian Naive Bayes is far cheaper and stays stable when rows are few relative to columns, but assumes the features are independent and roughly normal on each side of a cut. Random Forest bags many trees per cut and averages away most of a single tree's variance, usually making it the strongest option here - but it fits one entire forest per cut, so both the training time and the size of the saved model are multiplied by K-1.
     * @returns model — Thread-safe handle to the trained decomposition. Predictions come back as your original level labels.
     * @returns levels — The level order the model was actually trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when an ordinal model behaves oddly.
     * @impure has side effects / drives control flow
     */
    function fitOrdinalFrankHall({ source?: string, classOrder?: string, baseLearner?: string }): { model: Struct, levels: Struct };

    /**
     * Fit/Train a proportional-odds model on a target whose levels are ORDERED (1 < 2 < ... < 5, or low < medium < high). Use this instead of a classifier, which treats the levels as unrelated names and so counts predicting `low` for `high` as no worse than predicting `medium`. Use it instead of a regressor, which treats the levels as real numbers and so invents distances the levels do not carry (`high` is not exactly twice `medium`). The model learns one coefficient vector plus ordered cut points, which keeps predictions monotone in the score and, under the default loss, yields calibrated per-level probabilities. Link Function, Loss and Margin widen it to the whole threshold-model family, up to support vector ordinal regression, while Free Features relaxes the shared coefficient into one slope per cut point. Scale your features first with the Fit Feature Scaler node: this is a gradient fit, and unscaled columns make it converge slowly or not at all.
     * @node fit_ordinal_logistic @alias fitOrdinalLogistic
     * @param source (optional) — Choose which backend supplies the training data
     * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one.
     * @param link (optional) — The CDF sitting behind the cut points, i.e. which latent distribution you assume produced the levels. Logit gives the proportional-odds model and coefficients that read as log odds ratios. Probit assumes a normally distributed latent variable and is the convention in econometrics and the social sciences. CLogLog is asymmetric — it leaves the bottom level quickly and approaches the top one slowly — which is the right shape for `time until something escalates` targets. Cauchit is heavy-tailed, so extreme rows pull the fit far less than they do under Logit or Probit. Applies to the CumulativeLink loss only: the two threshold losses use a logistic margin and ignore this.
     * @param loss (optional) — What the optimizer actually minimizes. CumulativeLink maximizes the likelihood of each level and is the ONLY choice that carries a probability model — the confidence value on the Predict node comes from it. AllThreshold penalizes every cut point that falls on the wrong side of the observation, ImmediateThreshold only the two bracketing it; both drop the proportional-odds assumption and are often more robust when it fails, but they fit cut-point placement rather than a likelihood, so the resulting model yields NO per-level probabilities and Predict returns no confidence.
     * @param margin (optional) — Shape of the penalty a cut point pays for sitting on the wrong side of an observation. Hinge charges nothing once the cut point clears the margin, so only the observations NEAR a cut point influence the fit at all: Hinge together with the AllThreshold loss IS support vector ordinal regression (Chu & Keerthi's implicit-constraint SVOR), and with ImmediateThreshold it is the explicit-constraint variant. SquaredHinge is the differentiable version of that kink — smoother gradients, but distant violations are punished quadratically, so single outliers drag the cut points. Logistic is smooth everywhere and charges even well-placed cut points a little. IGNORED by the default CumulativeLink loss, which maximizes a likelihood and has no margin.
     * @param freeFeatures (optional) — Comma-separated feature INDICES (0-based, e.g. `0, 3`) that get their own coefficient at EVERY cut point instead of one shared across all of them — the partial proportional-odds model. Empty is the standard model, where a single slope describes every cut point; that is an assumption. Free a feature when you suspect it violates it, then check the Effective Coefficients output: a feature whose per-cut slopes barely differ gained nothing by being freed. Freeing only the ones that do differ keeps every other feature parsimonious. Listing every index gives the fully generalized ordinal model. The price shows up on Crossing Rate: unconstrained per-cut slopes let the cumulative curves cross, which is no longer a valid probability model.
     * @param alpha (optional) — Strength of the L2 penalty on the coefficients; the cut points are never penalized. 0 fits unpenalized. Raise it when the fit diverges or the coefficients blow up.
     * @param maxIterations (optional) — Iteration cap for the Adam optimizer. Training stops here even if the objective is still moving, which is reported on the Converged pin.
     * @param tolerance (optional) — Relative change in the objective below which training stops. Smaller values fit tighter but need more iterations; 0 always runs the full iteration budget.
     * @param learningRate (optional) — Adam step size. Lower it if training oscillates or produces non-finite values; raise it if the model has not converged within Max Iterations.
     * @returns model — Thread-safe handle to the trained proportional-odds model. Predictions come back as your original level labels.
     * @returns levels — The level order the model was actually trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when an ordinal model behaves oddly.
     * @returns converged — False when the optimizer hit Max Iterations before the objective settled. The model is still usable but under-fitted.
     * @returns crossingRate — Share of training rows (0.0 to 1.0) whose cumulative curves crossed, i.e. where the fit put P(y <= k) ABOVE P(y <= k+1) and so implied a negative probability for a level. Always 0.0 without Free Features, because a shared slope cannot cross. Anything above 0 means the generalized fit is no longer a clean probability model: prediction clamps and renormalizes so nothing downstream sees a negative number, but the per-level probabilities stop being trustworthy — free fewer features, or go back to the shared model.
     * @returns effectiveCoefficients — The coefficient of every feature at every cut point, one row per cut point from lowest to highest, next to the cut points themselves. Shared features repeat the same value down every row; freed ones vary, and the reported spread (largest minus smallest over the cut points) is how you tell whether freeing a feature bought anything — a spread near zero means one shared slope fitted it just as well and the extra parameters were wasted.
     * @impure has side effects / drives control flow
     */
    function fitOrdinalLogistic({ source?: string, classOrder?: string, link?: string, loss?: string, margin?: string, freeFeatures?: string, alpha?: float, maxIterations?: int, tolerance?: float, learningRate?: float }): { model: Struct, levels: Struct, converged: bool, crossingRate: float, effectiveCoefficients: Struct };

    /**
     * Fit/Train a NEURAL ordinal model on a target whose levels are ORDERED (1 < 2 < ... < 5, or low < medium < high). This is the only trainer in the catalog that is BOTH non-linear in the features AND yields calibrated, rank-consistent per-level probabilities: Frank & Hall is non-linear but votes with K-1 independent classifiers and therefore carries no probability model, while every other ordinal node here is linear in the features. A small network feeds one of two rank-consistent heads, CORAL or CORN, and both are built so that P(y > k) can never rise with k for ANY parameter values — so the level probabilities are non-negative and sum to 1 with nothing patched up afterwards. THE HONEST LIMIT: leave Hidden Layers EMPTY and CORAL becomes exactly Train Ordinal Model (Proportional Odds) with Loss = AllThreshold and Margin = Logistic, and CORN becomes exactly Train Ordinal Model (Continuation Ratio) — the same objective in the same parameters. The hidden layers are the entire contribution, so if your problem is linear in the features prefer those nodes: convex objective, no seed dependence, readable coefficients, better tested. Reach for this one when the level is genuinely not monotone in the features (a boundary that bends back on itself, which no linear ordinal model can represent at all). Two costs come with the network: it has far more parameters than a linear model and so needs far more rows — check the Architecture output — and the objective is not convex, so the Seed changes the fit. Scale your features first with the Fit Feature Scaler node; unscaled columns make this converge slowly or not at all.
     * @node fit_ordinal_neural @alias fitOrdinalNeural
     * @param source (optional) — Choose which backend supplies the training data
     * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so training fails rather than guessing one. Note that a declared level the training data never reaches is fine for CORAL but rejected by CORN, whose task for that level would have no rows to fit.
     * @param head (optional) — Which rank-consistent head sits on the network. Coral shares ONE latent score across every cut point and lets the cut points differ only by an ordered bias, so a row's whole position on the scale is a single number: fewer parameters, lower variance, and the right choice when the levels really are separated by one underlying quantity or when the top levels are thin. Corn instead asks each step conditionally — given the row reached this level, does it go further? — and gives every step its own weights on the shared representation, which suits a target that is a genuine sequential process (escalation tiers, disease stages, how far a funnel got). Its price is data: step k trains only on the rows that reached level k, so the higher steps rest on the fewest rows, and Corn refuses outright to fit a declared level that nothing reaches.
     * @param hiddenLayers (optional) — Comma-separated hidden layer widths from the input side, e.g. `16, 8` for two layers. This is the ONLY thing this node adds over the linear ordinal trainers: an EMPTY value collapses the model to its linear equivalent exactly — Coral becomes the All-Threshold proportional-odds fit, Corn becomes the continuation-ratio fit — so if you want an empty value you want one of those simpler, better-tested nodes instead. Wider and deeper buys a boundary that can bend, and costs parameters that have to be paid for in rows: compare the Architecture output's parameter count against your row count. Every width must be at least 1; a zero-width layer would disconnect the head from the features and fit a constant.
     * @param activation (optional) — Non-linearity between the hidden layers. The head itself is always linear, and this has no effect at all when Hidden Layers is empty. Relu is cheap, and its piecewise-linear folds are exactly what let a small network represent a level that is not monotone in the features. Tanh is smooth and bounded, which often trains more gently on small, well-scaled data, but it saturates on large inputs and then passes almost no gradient — one more reason to scale the features first.
     * @param alpha (optional) — Strength of the L2 penalty on the WEIGHT matrices. Biases and the head's ordering parameters are never penalized: shrinking those would drag the level cut points together and quietly collapse adjacent levels, which changes the model rather than its variance. Raise it when the network memorizes the training rows or the loss blows up; 0 fits unpenalized.
     * @param maxIterations (optional) — Iteration cap for the Adam optimizer; each iteration is one full pass over the training set. Training stops here even if the loss is still falling, which is reported on the Converged pin. A network usually needs noticeably more iterations than the linear ordinal fits.
     * @param tolerance (optional) — Relative change in the loss below which training stops. Smaller values fit tighter and cost iterations; 0 always spends the whole iteration budget.
     * @param learningRate (optional) — Adam step size. Lower it if the loss oscillates or goes non-finite, raise it if the model has not converged within Max Iterations. A network wants a smaller step than the linear ordinal fits, because a hidden layer compounds every step.
     * @param seed (optional) — Seed for the weight initialization, which is the only randomness in the fit. The objective is NOT convex, so the seed genuinely changes the model you get and an unlucky one can leave the fit in a poor local optimum: refit with two or three seeds to see whether the result is stable. The same seed, data and hyperparameters reproduce a fit exactly.
     * @returns model — Thread-safe handle to the trained neural ordinal model. Predictions come back as your original level labels, and unlike the threshold losses of the proportional-odds node this family always carries per-level probabilities, so the Predict node reports a confidence.
     * @returns levels — The level order the model was actually trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when an ordinal model behaves oddly.
     * @returns converged — False when the optimizer hit Max Iterations before the loss settled. The model is still usable but under-fitted, which on a network is more common than on the linear ordinal fits.
     * @returns architecture — What was actually built: the head, the activation, the hidden layer widths as fitted, and the total parameter count next to the number of training rows. Read the rows-per-parameter figure before you trust a training score — with fewer rows than parameters the network can reproduce the training labels outright. Empty hidden layers here means the fit was the linear equivalent, and a simpler ordinal node would have done the same job.
     * @impure has side effects / drives control flow
     */
    function fitOrdinalNeural({ source?: string, classOrder?: string, head?: string, hiddenLayers?: string, activation?: string, alpha?: float, maxIterations?: int, tolerance?: float, learningRate?: float, seed?: int }): { model: Struct, levels: Struct, converged: bool, architecture: Struct };

    /**
     * Fit/Train an ordinal model the cheap way: ridge-regress the level rank on the features, then cut the score at thresholds learned from the training distribution instead of rounding it. Closed-form, so it stays fast exactly where the proportional-odds model gets expensive - many levels, many features, or when you just want a quick ordinal baseline to beat. It also degrades gracefully when the proportional-odds assumption does not hold. Unlike the proportional-odds model it yields no probabilities: you get the predicted level and the latent score behind it, nothing calibrated.
     * @node fit_ordinal_ridge @alias fitOrdinalRidge
     * @param source (optional) — Choose which backend supplies the training data
     * @param classOrder (optional) — Level labels from LOWEST to HIGHEST, comma separated - e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is already the one you want (`1, 2, 10` sorts as numbers, not as text). Non-numeric labels carry no inferable order, so training fails rather than guessing unless you list them here.
     * @param alpha (optional) — Strength of the L2 penalty. Must be strictly greater than 0: the penalty is added to the diagonal of the normal equations and is the only thing keeping them positive definite, so the Cholesky solve has a unique answer even with collinear or wide features. Larger values shrink the coefficients harder.
     * @returns model — Thread-safe handle to the trained ordinal ridge model. Predictions come back as the original level labels.
     * @returns levels — The resolved level order the model was trained on, lowest first, plus whether that order came from `Class Order` or from reading the labels as numbers. Check it before trusting the model - a wrong order trains a wrong model without ever failing.
     * @returns coefficients — Fitted coefficients and intercept on the rank scale. The SIGN tells you which way a feature pushes the level: positive moves samples toward the higher levels, negative toward the lower ones. The magnitude is only comparable across features when they share a scale.
     * @impure has side effects / drives control flow
     */
    function fitOrdinalRidge({ source?: string, classOrder?: string, alpha?: float }): { model: Struct, levels: Struct, coefficients: Struct };

    /**
     * Evaluate predictions for an ordered target with distance-aware metrics. Plain accuracy is inadequate here: it treats "predicted high when the truth was medium" exactly as harshly as "predicted low", so a model that is reliably one level off scores like one that guesses. Quadratic weighted kappa is the standard headline metric because it weights every miss by how far off it was and corrects for chance agreement, but it answers only one of three questions: the linear kappa and the macro-averaged error say how far off the model is under a different cost structure and on the rare levels, while Kendall's tau-b and the Spearman correlation say whether it orders the rows correctly at all.
     * @node ml_ordinal_metrics @alias mlOrdinalMetrics
     * @param database — Database connection containing the predicted levels and the true levels
     * @param predictionsCol (optional) — Column holding the predicted level of each row. The labels must be the same ones the actuals column uses, since both columns are ranked against one shared level order.
     * @param actualsCol (optional) — Column holding the true level of each row. When no Class Order is given, the level order is inferred from this column, and a predicted level that never occurs here is an error rather than a silent extra rank.
     * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order (sorting them alphabetically would rank high < low < medium), so they have to be listed here.
     * @returns quadraticWeightedKappa — Headline ordinal metric: chance-corrected agreement weighted by the squared level distance. 1.0 perfect, 0.0 chance, negative worse than chance.
     * @returns linearWeightedKappa — The same chance-corrected agreement with every level of distance costing the same. Read this one instead of the quadratic kappa when a level is a level — grading scales, severity tiers, anything where two steps off is exactly twice as bad as one. Quadratic weighting charges a near miss only a quarter of a two-level miss, so it flatters a model that merely hovers next to the truth; where that discount is not real, this is the honest number and it will be the lower of the two.
     * @returns meanAbsoluteRankError — Average miss in levels. 0.0 is perfect, 1.0 means being off by one level on average.
     * @returns macroMeanAbsoluteError — The mean absolute rank error computed per true level and averaged with one vote per level. Look here whenever the levels are imbalanced: the plain error averages over rows, so the majority level speaks for the model and a predictor that collapses onto it still scores well while missing every rare level. This metric gives the rare levels equal weight, so it is the one that moves when that happens. Levels absent from the actuals are skipped rather than counted as perfect.
     * @returns accuracyExact — Share of predictions hitting the exact level. Reported for reference; it ignores how far the misses are off.
     * @returns accuracyWithinOne — Share of predictions landing on the true level or one of its direct neighbours
     * @returns kendallTauB — Tie-corrected rank association: +1.0 orders the rows exactly as the truth does, 0.0 no association, -1.0 exactly backwards. This answers "does the model rank the rows correctly", which is a different question from "does it land on the right level" — a model whose every prediction is one level too high ranks perfectly and scores 1.0 here while the kappas drop. Consult it when the output feeds a sort, a triage queue or a threshold you can recalibrate, and read it against kappa to tell a miscalibrated model from a model that has learned nothing.
     * @returns spearmanRankCorrelation — The same ordering question as tau-b, computed as a correlation on midranks. It is the less conservative of the two under the heavy ties ordinal data always has, so it reads higher than tau-b on the same predictions; prefer tau-b when you need a defensible figure and this one when comparing against Spearman values reported elsewhere. Like tau-b it ignores calibration entirely.
     * @returns nSamples — Number of rows evaluated
     * @returns nLevels — Number of distinct levels both columns were ranked against
     * @returns result — All ordinal metrics plus the resolved level order they were computed against
     * @impure has side effects / drives control flow
     */
    function ordinalMetrics({ database: Struct, predictionsCol?: string, actualsCol?: string, classOrder?: string }): { quadraticWeightedKappa: float, linearWeightedKappa: float, meanAbsoluteRankError: float, macroMeanAbsoluteError: float, accuracyExact: float, accuracyWithinOne: float, kendallTauB: float, spearmanRankCorrelation: float, nSamples: int, nLevels: int, result: Struct };

    // === AI/ML/Preprocessing ===

    /**
     * Apply a fitted transformer (Feature Scaler, TF-IDF) to a table, writing one vector per row. A Feature Scaler replays the exact offsets and scales learned at fit time, so applying it to train and test gives both the same statistics. TF-IDF is different: linfa recomputes the inverse document frequencies from the table being transformed, so vectors are only comparable within a single Apply Transform run.
     * @node ml_apply_transform @receiver model @alias mlApplyTransform
     * @param model — Fitted transformer to apply. Classifiers and regressors belong on the Predict node. (receiver: `this` in `x.applyTransform(...)`)
     * @param source (optional) — Choose which backend supplies the rows to transform
     * @param batchSize (optional) — Number of records to transform per batch (default: 5000, 0 = process all at once)
     * @impure has side effects / drives control flow
     */
    function applyTransform(this: NodeMLModel, { model: Struct, source?: string, batchSize?: int }): void;

    /**
     * Learn per-feature offsets and scales from a training table. Distance- and gradient-based models (Logistic Regression, Elastic Net, SVM, KNN, Gaussian Mixture) only behave when their features share a scale.
     * @node fit_feature_scaler @alias fitFeatureScaler
     * @param source (optional) — Choose which backend supplies the training data
     * @param method (optional) — Standard centers each feature and divides it by its standard deviation. MinMax squeezes each feature into the Min..Max range. MaxAbs divides each feature by its largest absolute value, keeping zeros at zero.
     * @param min (optional) — Lower bound of the target range. Only read when Method is MinMax.
     * @param max (optional) — Upper bound of the target range. Only read when Method is MinMax.
     * @returns model — Thread-safe handle to the fitted scaler. Feed it to Apply Transform to scale any table with these statistics.
     * @returns offsets — Value subtracted from each feature before scaling: the mean for Standard, the minimum for MinMax, zero for MaxAbs
     * @returns scales — Multiplier applied to each feature. linfa stores the reciprocal, so this is 1/std for Standard and 1/(max-min) for MinMax, and it stays 1 for constant features.
     * @impure has side effects / drives control flow
     */
    function fitFeatureScaler({ source?: string, method?: string, min?: float, max?: float }): { model: Struct, offsets: float[], scales: float[] };

    /**
     * Learn a vocabulary from a text column and turn documents into numeric vectors weighted by term frequency times inverse document frequency. Feed the fitted vectorizer to Apply Transform to vectorize a column, then train a classifier such as Multinomial Naive Bayes on the result. Tokenization always uses the built-in regex tokenizer, because a custom tokenizer function cannot be persisted and would make the saved model unloadable.
     * @node fit_tfidf_vectorizer @alias fitTfidfVectorizer
     * @param source (optional) — Choose which backend supplies the documents
     * @param method (optional) — Weighting formula. Smooth: log((1+n)/(1+df))+1, never divides by zero. Non-Smooth: log(n/df)+1, sharper but requires every term to appear at least once. Textbook: log(n/(1+df)), which discounts terms appearing in nearly every document down to a negative weight, so it cannot feed Multinomial Naive Bayes.
     * @param nGramMin (optional) — Smallest number of adjacent tokens forming a vocabulary entry (1 = single words)
     * @param nGramMax (optional) — Largest number of adjacent tokens forming a vocabulary entry. Must not be smaller than Min N-Gram.
     * @param convertToLowercase (optional) — Lowercase every document before tokenizing, so casing variants collapse into one vocabulary entry
     * @param maxFeatures (optional) — Keep only the most frequent N vocabulary entries, which caps the width of the produced vectors. 0 keeps all of them.
     * @param minDocumentFrequency (optional) — Drop terms appearing in a smaller share of documents than this (0-1). Useful to remove typos and one-off tokens.
     * @param maxDocumentFrequency (optional) — Drop terms appearing in a larger share of documents than this (0-1). Useful to remove boilerplate that carries no signal.
     * @param stopwords (optional) — Comma separated words to exclude from the vocabulary, e.g. `the, and, of`. Leave empty to keep every term.
     * @returns model — Thread-safe handle to the fitted TF-IDF vectorizer, for use with Apply Transform
     * @returns vocabulary — Learned vocabulary entries, in the same order as the columns of the produced vectors
     * @impure has side effects / drives control flow
     */
    function fitTfidfVectorizer({ source?: string, method?: string, nGramMin?: int, nGramMax?: int, convertToLowercase?: bool, maxFeatures?: int, minDocumentFrequency?: float, maxDocumentFrequency?: float, stopwords?: string }): { model: Struct, vocabulary: string[] };

    // === AI/ML/Reduction ===

    /**
     * Principal Component Analysis for dimensionality reduction
     * @node fit_pca @alias fitPca
     * @param nComponents (optional) — Number of principal components to keep
     * @param source (optional) — Choose which backend supplies the data
     * @returns explainedVariance — Variance explained by each principal component
     * @impure has side effects / drives control flow
     */
    function fitPca({ nComponents?: int, source?: string }): float[];

    /**
     * t-Distributed Stochastic Neighbor Embedding. Projects high-dimensional vectors into 2-3 dimensions for visualization and writes the embedding back into the source table. t-SNE is transductive, so it produces no reusable model.
     * @node fit_tsne @alias fitTsne
     * @param source (optional) — Choose which backend supplies the data
     * @param embeddingSize (optional) — Dimensionality of the embedding. Must not exceed the width of the input vectors; values above 3 require the exact gradient (Approx Threshold = 0).
     * @param perplexity (optional) — Effective number of neighbors per point (typically 5-50). t-SNE requires 3 * perplexity <= rows - 1, so small tables need a small perplexity.
     * @param approxThreshold (optional) — Barnes-Hut theta. 0 runs the exact O(n^2) gradient, larger values approximate distant points by their cell centroid and run faster.
     * @param maxIter (optional) — Number of gradient descent iterations. Fewer iterations finish sooner but may leave the embedding unconverged.
     * @impure has side effects / drives control flow
     */
    function fitTsne({ source?: string, embeddingSize?: int, perplexity?: float, approxThreshold?: float, maxIter?: int }): void;

    // === AI/ML/Regression ===

    /**
     * Fit/Train a penalized linear regression model. Ridge shrinks all coefficients, Lasso drives irrelevant ones to exactly zero (feature selection), Elastic Net mixes both.
     * @node fit_elastic_net @alias fitElasticNet
     * @param source (optional) — Choose which backend supplies the training data
     * @param penaltyType (optional) — Ridge = pure L2 (keeps all features, handles correlated ones well), Lasso = pure L1 (zeroes out weak features), ElasticNet = a blend controlled by L1 Ratio
     * @param penalty (optional) — Overall regularization strength. 0 means ordinary least squares, larger values shrink the coefficients harder.
     * @param l1Ratio (optional) — Share of the penalty spent on L1 vs L2. Only used when Penalty Type is ElasticNet; Ridge forces 0.0 and Lasso forces 1.0.
     * @param withIntercept (optional) — Fit a bias term. Disable only when the data is already centered.
     * @param maxIterations (optional) — Upper bound on coordinate descent passes. The solver stops silently at this cap, so a convergence warning is logged when it is hit.
     * @param tolerance (optional) — Convergence tolerance for coordinate descent. Smaller values give a tighter fit at the cost of more iterations.
     * @returns model — Thread-safe handle to the trained penalized regression model
     * @returns coefficients — Fitted coefficients and intercept. With Lasso, coefficients that are exactly zero mark features the model discarded.
     * @impure has side effects / drives control flow
     */
    function fitElasticNet({ source?: string, penaltyType?: string, penalty?: float, l1Ratio?: float, withIntercept?: bool, maxIterations?: int, tolerance?: float }): { model: Struct, coefficients: Struct };

    /**
     * Fit/Train a Generalized Linear Model. Pick the distribution that matches the target: Normal for unbounded values, Poisson for counts, Gamma for positive skewed amounts, Inverse Gaussian for heavy tails.
     * @node fit_glm @alias fitGlm
     * @param source (optional) — Choose which backend supplies the training data
     * @param distribution (optional) — Target distribution: Normal (power 0, any value), Poisson (power 1, counts >= 0), Gamma (power 2, values > 0), Inverse Gaussian (power 3, values > 0), or Custom to set the Tweedie power directly
     * @param power (optional) — Free Tweedie power, only used when Distribution is Custom. Values in (0, 1) do not describe any distribution and are rejected; (1, 2) is compound Poisson-Gamma.
     * @param alpha (optional) — Strength of the L2 penalty on the coefficients. 0 fits an unpenalized GLM.
     * @param fitIntercept (optional) — Fit a bias term. Disable only when the data is already centered.
     * @param maxIter (optional) — Iteration cap for the L-BFGS solver. Defaults to 1000 instead of the library default of 100, which is too low to converge on unscaled real-world features.
     * @param tol (optional) — Gradient tolerance that stops the L-BFGS solver. Smaller values fit tighter but need more iterations.
     * @returns model — Thread-safe handle to the trained generalized linear model
     * @impure has side effects / drives control flow
     */
    function fitGlm({ source?: string, distribution?: string, power?: float, alpha?: float, fitIntercept?: bool, maxIter?: int, tol?: float }): Struct;

    /**
     * Fit a K-Nearest-Neighbours regressor that averages the target of the nearest training rows. Non-parametric and instance based: the fitted model embeds a verbatim copy of the whole training set instead of learned coefficients, so every training row (and any personal data in it) travels with the model, is written into every saved model file and can be reconstructed by anyone holding it. Treat the model with the same care as the source table.
     * @node fit_knn_regressor @alias fitKnnRegressor
     * @param source (optional) — Choose which backend supplies the training data
     * @param k (optional) — How many nearest training rows are averaged for each prediction. Must be at least 1 and cannot exceed the number of training rows. Larger values smooth the response.
     * @param distanceWeighted (optional) — Weight each neighbour by the inverse of its distance instead of taking a plain mean. Reduces the pull of distant neighbours when k is large.
     * @returns model — Thread-safe handle to the trained KNN regressor. Contains a full copy of the training set.
     * @impure has side effects / drives control flow
     */
    function fitKnnRegressor({ source?: string, k?: int, distanceWeighted?: bool }): Struct;

    /**
     * Fit/Train Linear Regression Model
     * @node fit_linear_regression @alias fitLinearRegression
     * @param source (optional) — Choose where training data should be loaded from
     * @returns model — Thread-safe handle to the trained linear regression model
     * @impure has side effects / drives control flow
     */
    function fitLinearRegression({ source?: string }): Struct;

    /**
     * Fit/Train a Support Vector Regressor. Learns non-linear targets through a kernel, with epsilon-SVR or nu-SVR.
     * @node fit_svm_regression @alias fitSvmRegression
     * @param source (optional) — Choose which backend supplies the training data
     * @param mode (optional) — Epsilon-SVR penalises deviations larger than Epsilon. Nu-SVR replaces Epsilon with Nu, the target fraction of support vectors.
     * @param kernel (optional) — Feature-space mapping. Gaussian for smooth non-linear targets, Linear for the plain SVR, Polynomial for interaction terms.
     * @param kernelParam (optional) — Gaussian: the eps in exp(-||x - x'||^2 / eps), larger means smoother. Polynomial: the degree of (<x, x'> + 1)^degree. Ignored for Linear.
     * @param c (optional) — Penalty for deviations outside the tolerated margin. Higher values fit the training data harder and risk overfitting. Used by both modes.
     * @param epsilon (optional) — Width of the insensitive tube: errors smaller than this are not penalised. Epsilon-SVR only.
     * @param nu (optional) — Upper bound on the fraction of training errors and lower bound on the fraction of support vectors, in (0, 1]. Nu-SVR only.
     * @param tolerance (optional) — Stopping threshold of the SMO solver. Smaller values train longer for a more precise solution.
     * @returns model — Thread-safe handle to the trained support vector regressor
     * @returns supportVectors — Number of training rows that ended up contributing to the regression
     * @impure has side effects / drives control flow
     */
    function fitSvmRegression({ source?: string, mode?: string, kernel?: string, kernelParam?: float, c?: float, epsilon?: float, nu?: float, tolerance?: float }): { model: Struct, supportVectors: int };

    // === AI/ML/Tuning ===

    /**
     * Automatically finds the best classification model. Cross-validates Naive Bayes, Decision Tree, Logistic Regression, Random Forest and SVM, then retrains the winner on the full dataset. The reported Best Model Type can be fed straight into Grid Search to tune it further.
     * @node ai_ml_tuning_auto_classifier @alias aiMlTuningAutoClassifier
     * @param cvFolds (optional) — Number of cross-validation folds
     * @param metric (optional) — Metric the leaderboard is ranked by. Accuracy is the share of correct rows; Macro F1 averages per-class F1 with equal weight per class, which is the right choice when the classes are imbalanced.
     * @param includeSvm (optional) — Include SVM in comparison (slower but often more accurate)
     * @param includeLogistic (optional) — Include Logistic Regression. Fast, and the only candidate that yields calibrated probabilities, but it expects scaled features — fit a Feature Scaler first for a fair comparison.
     * @param includeRandomForest (optional) — Include Random Forest. Usually the strongest candidate here, at the cost of training one tree per ensemble member on every fold.
     * @param source (optional) — Data source type
     * @returns results — Complete AutoML results with leaderboard
     * @returns bestModel — The best model trained on full data
     * @returns bestModelType — Name of the best algorithm
     * @impure has side effects / drives control flow
     */
    function autoClassifier({ cvFolds?: int, metric?: string, includeSvm?: bool, includeLogistic?: bool, includeRandomForest?: bool, source?: string }): { results: Struct, bestModel: Struct, bestModelType: string };

    /**
     * Automatically finds the best model for a target whose levels are ORDERED (1 < 2 < ... < 5, or low < medium < high). Cross-validates the ordinal families - Proportional Odds and Ordered Probit, the all-threshold model and its support-vector form, Ordinal Ridge, Continuation Ratio and Adjacent Category, plus an optional rank-consistent neural family that is off by default because it costs far more than all the others combined - on identical folds, ranks them by an ordinal metric that knows how far a miss was, then retrains the winner on the full data. Use this rather than Auto Classifier, which resolves the target without its order and ranks by accuracy or macro-F1, scoring a five-level miss exactly like a one-level one. Every candidate here is a gradient or a least-squares fit on the raw columns, so scale your features with the Fit Feature Scaler node first: unscaled columns change which family wins, not just how fast it converges.
     * @node ai_ml_tuning_auto_ordinal @alias aiMlTuningAutoOrdinal
     * @param source (optional) — Choose which backend supplies the training data
     * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so the search fails rather than guessing one. The level set is resolved once here and handed to every family, so a level that a fold happens to miss cannot renumber the ranks for that fold.
     * @param cvFolds (optional) — How many folds the rows are split into. Every family is scored on the SAME folds, so the comparison is paired rather than a race between different splits. More folds mean a less noisy score and proportionally more fitting, since the whole sweep is repeated once per fold.
     * @param metric (optional) — What the leaderboard is ranked by. Quadratic Kappa is chance-corrected agreement that forgives a near miss and punishes a distant one four times as hard - the standard headline metric for ordered targets. Linear Kappa charges the same for every step along the scale, which is what you want when one level is one unit of loss. Mean Rank Error is the average number of levels a prediction is off by, and Macro Rank Error is the same averaged per true level so a rare level counts as much as the majority one - both are ERROR metrics, so the leaderboard ranks their smallest value first. Kendall Tau-b and Spearman ask only whether the rows come out in the right order and ignore calibration entirely, so a model whose levels are all shifted by one still scores perfectly.
     * @param seed (optional) — Seed for the fold shuffle. The same seed reproduces the same folds and therefore the same leaderboard; change it to check whether a narrow win survives a different split.
     * @param includeProportionalOdds (optional) — Try the cumulative-link model under a logit and a probit link. The only family here that yields calibrated per-level probabilities and coefficients that read as a direction along the ordering, but it assumes one shared effect across all cut points.
     * @param includeAllThreshold (optional) — Try the all-threshold model under a logistic and a hinge margin. It drops the proportional-odds assumption by fitting cut-point placement instead of a likelihood, which is often more robust when that assumption fails; the hinge entry is support vector ordinal regression. Neither yields per-level probabilities.
     * @param includeOrdinalRidge (optional) — Try rank regression with learned cut points across a small L2 sweep. Closed-form, so it is by far the cheapest candidate and stays cheap as levels and features grow - but it treats the ranks as numbers, so it is the family most likely to be beaten when the levels are not evenly spaced.
     * @param includeContinuationRatio (optional) — Try the sequential model, `P(stop at level k | reached level k)`. The right shape when reaching a level genuinely requires passing the ones below it (stages, escalation, dropout). It fits K-1 sub-models on shrinking subsets and refuses to fit at all when a middle level is missing from a fold, in which case it is dropped from the leaderboard and the other families continue.
     * @param includeAdjacentCategory (optional) — Try the adjacent-category model, which contrasts neighbouring levels instead of splitting the scale cumulatively. Reach for it when the interesting comparison is `this level versus the next one` rather than `at most this level versus above it`.
     * @param includeNeural (optional) — Try a small neural network under a rank-consistent head, as two candidates: a CORAL head, which shares one latent score across the cut points and lets them differ only by biases that cannot cross, and a CORN head, which fits one conditional task per cut point on the rows that reached it. OFF by default, unlike every other family here, and the default is the recommendation: a network is orders of magnitude more expensive to fit than the linear families, it is refitted from scratch on EVERY fold, and it is the one candidate that can dominate the runtime of the whole sweep. Switch it on when you suspect the levels are not separated by a single monotone direction in the features - the hidden layer is the entire contribution, and it is the only thing here that can represent such a boundary at all. On a problem that is linear in the features it can only match the simpler families, never beat them: with no hidden layer CORAL is EXACTLY the all-threshold model with a logistic margin and CORN is EXACTLY Continuation Ratio, so prefer those better-tested candidates when they win. Both use a fixed initialization seed, so the leaderboard stays reproducible. CORN is dropped from the leaderboard on any fold that omits a level nothing reaches, since its task for that level would have no rows; CORAL has no such failure mode.
     * @returns results — Leaderboard of every configuration that finished, best first, plus the ones that were dropped and why. `higher_is_better` states which end of `cv_score` won.
     * @returns bestModel — The winning configuration retrained on the full dataset. Predictions come back as your original level labels.
     * @returns bestModelType — Model kind of the winner, e.g. `OrdinalLogistic`. Read back off the retrained model, so it always matches what the rest of the catalog calls it.
     * @returns levels — The level order every candidate was trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when the leaderboard looks upside down.
     * @impure has side effects / drives control flow
     */
    function autoOrdinal({ source?: string, classOrder?: string, cvFolds?: int, metric?: string, seed?: int, includeProportionalOdds?: bool, includeAllThreshold?: bool, includeOrdinalRidge?: bool, includeContinuationRatio?: bool, includeAdjacentCategory?: bool, includeNeural?: bool }): { results: Struct, bestModel: Struct, bestModelType: string, levels: Struct };

    /**
     * Exhaustive search over parameter combinations with cross-validation. Returns the best parameters found. Model Type accepts the same names the Auto Classifier reports as its best model, so the two nodes chain directly.
     * @node ai_ml_tuning_grid_search @alias aiMlTuningGridSearch
     * @param modelType (optional) — Type of model to tune
     * @param cvFolds (optional) — Number of cross-validation folds
     * @param source (optional) — Database containing the training data
     * @returns results — Complete grid search results with all combinations tried
     * @returns bestModel — The model trained with the best parameters on full training data
     * @impure has side effects / drives control flow
     */
    function gridSearch({ modelType?: string, cvFolds?: int, source?: string }): { results: Struct, bestModel: Struct };

    /**
     * Exhaustively searches the hyperparameters of ONE ordinal model family with cross-validation, for a target whose levels are ORDERED (1 < 2 < ... < 5, or low < medium < high). Every combination in the Parameter Grid is scored on the SAME folds and ranked by an ordinal metric that knows how far a miss was. Use this rather than Grid Search, which resolves the target without its order and tunes against accuracy, scoring a five-level miss exactly like a one-level one. Model Type accepts the names Auto Ordinal reports as its best model, so the usual chain is Auto Ordinal to pick the family, then this node to tune it. Every family here is a gradient or a least-squares fit on the raw columns, so scale your features with the Fit Feature Scaler node first: unscaled columns change which hyperparameters win, not just how fast they converge.
     * @node ai_ml_tuning_ordinal_grid_search @alias aiMlTuningOrdinalGridSearch
     * @param source (optional) — Choose which backend supplies the training data
     * @param modelType (optional) — Which ordinal family to tune. OrdinalLogistic is the threshold model, the widest family here: it takes a link, a loss and a margin, and covers proportional odds, ordered probit and support vector ordinal regression. OrdinalRidge is rank regression with learned cut points, closed-form and so by far the cheapest to sweep, but it has only a penalty to tune. OrdinalContinuationRatio models a sequential progression, `P(stop at k | reached k)`. OrdinalAdjacentCategory contrasts neighbouring levels instead of splitting the scale cumulatively. OrdinalNeural is a small network under a rank-consistent CORAL or CORN head, the only family here that is not linear in the features and the only one that can represent a level that is not monotone in them - and by a wide margin the most expensive to sweep, since every combination trains a whole network from scratch on every fold, so keep its grid small. Switching this after the Parameter Grid was seeded does NOT rewrite the grid - the run rejects parameters the new family does not consume rather than ignoring them silently.
     * @param classOrder (optional) — Comma-separated level labels from LOWEST to HIGHEST, e.g. `low, medium, high`. Leave empty when the levels are numeric and their numeric order is the order you want. Non-numeric labels have no inferable order, so the search fails rather than guessing one. The level set is resolved once here and handed to every fit, so a level that a fold happens to miss cannot renumber the ranks for that fold.
     * @param cvFolds (optional) — How many folds the rows are split into. Every combination is scored on the SAME folds, so the comparison is paired rather than a race between different splits. More folds mean a less noisy score and proportionally more fitting, since the whole grid is refitted once per fold.
     * @param metric (optional) — What the sweep is ranked by. Quadratic Kappa is chance-corrected agreement that forgives a near miss and punishes a distant one four times as hard - the standard headline metric for ordered targets. Linear Kappa charges the same for every step along the scale, which is what you want when one level is one unit of loss. Mean Rank Error is the average number of levels a prediction is off by, and Macro Rank Error is the same averaged per true level so a rare level counts as much as the majority one - both are ERROR metrics, so the SMALLEST value wins and the `higher_is_better` output says so. Kendall Tau-b and Spearman ask only whether the rows come out in the right order and ignore calibration entirely, so a model whose levels are all shifted by one still scores perfectly.
     * @param seed (optional) — Seed for the fold shuffle, and for the weight initialization when Model Type is OrdinalNeural - the two sources of randomness in the sweep, tied to one value so the same seed reproduces the same folds, the same fits and therefore the same winner. Change it to check whether a narrow win survives a different split, which for the neural family also re-rolls the starting point of a non-convex fit. The winner is retrained from the same initialization it was scored at.
     * @returns results — Every combination that completed all folds with its mean and spread across the folds, plus the ones that were dropped and why. `higher_is_better` states which end of `mean_score` won.
     * @returns bestModel — The winning combination retrained on the full dataset. Predictions come back as your original level labels.
     * @returns bestScore — Mean cross-validated score of the winner, in the units of the chosen metric. Meaningless without Higher Is Better: for the two error metrics this is the SMALLEST score in the sweep, not the largest.
     * @returns higherIsBetter — Direction of the chosen metric: true for the agreement measures, false for MeanAbsoluteRankError and MacroMeanAbsoluteError, where a smaller score is the better model. Branch on this rather than assuming, otherwise a comparison downstream will rank the sweep upside down.
     * @returns levels — The level order every configuration was trained on, lowest first, plus whether it came from your Class Order list (Explicit) or from reading the labels as numbers (Numeric). Check this first when the results look upside down.
     * @impure has side effects / drives control flow
     */
    function ordinalGridSearch({ source?: string, modelType?: string, classOrder?: string, cvFolds?: int, metric?: string, seed?: int }): { results: Struct, bestModel: Struct, bestScore: float, higherIsBetter: bool, levels: Struct };
}

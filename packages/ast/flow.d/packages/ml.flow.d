// ml — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === AI/ML ===

/**
 * Load Trained ML Model from Path
 * @param path — Filesystem or storage path pointing at the serialized model JSON
 * @returns model — Handle to the loaded machine learning model
 * @impure has side effects / drives control flow
 */
declare function loadMlModel({ path: Struct }): Struct;

/**
 * Load Trained ML Model from Path using fast binary format (Fory)
 * @param path — Filesystem or storage path pointing at the serialized model binary (.flmodel)
 * @returns model — Handle to the loaded machine learning model
 * @impure has side effects / drives control flow
 */
declare function loadMlModelBinary({ path: Struct }): Struct;

/**
 * Predict with Machine Learning Model
 * @param model — Trained ML model to use for inference
 * @param source (optional) — Choose the input type for prediction (database rows or raw vector)
 * @param batchSize (optional) — Number of records to process per batch (default: 5000, 0 = process all at once)
 * @impure has side effects / drives control flow
 */
declare function mlPredict({ model: Struct, source?: string, batchSize?: int }): void;

/**
 * Save Trained ML Model to Path
 * @param model — Any trained ML model handle to persist
 * @param path — Destination path where the model JSON should be written
 * @impure has side effects / drives control flow
 */
declare function saveMlModel({ model: Struct, path: Struct }): void;

/**
 * Save Trained ML Model to Path using fast binary format (Fory)
 * @param model — Any trained ML model handle to persist
 * @param path — Destination path where the model binary should be written (.flmodel)
 * @impure has side effects / drives control flow
 */
declare function saveMlModelBinary({ model: Struct, path: Struct }): void;


// === AI/ML/Classification ===

/**
 * Fit/Train a Decision Tree classifier. Native multi-class support with interpretable rules.
 * @param source (optional) — Choose which backend supplies the training data
 * @param maxDepth (optional) — Maximum depth of the tree. None means unlimited.
 * @param minSamplesSplit (optional) — Minimum number of samples required to split a node
 * @returns model — Thread-safe handle to the trained Decision Tree classifier
 * @impure has side effects / drives control flow
 */
declare function fitDecisionTree({ source?: string, maxDepth?: int, minSamplesSplit?: int }): Struct;

/**
 * Fit/Train a Gaussian Naive Bayes classifier. Native multi-class support - no need for One-vs-All.
 * @param source (optional) — Choose which backend supplies the training data
 * @returns model — Thread-safe handle to the trained Naive Bayes classifier
 * @impure has side effects / drives control flow
 */
declare function fitNaiveBayes({ source?: string }): Struct;

/**
 * Fit/Train Support Vector Machines (SVM) for Multi-Class Classification
 * @param source (optional) — Choose which backend supplies the training data
 * @returns model — Thread-safe handle to the trained SVM classifier
 * @impure has side effects / drives control flow
 */
declare function fitSvmMultiClass({ source?: string }): Struct;


// === AI/ML/Clustering ===

/**
 * Fit/Train DBSCAN Density-Based Clustering
 * @param epsilon (optional) — Maximum distance between points in the same cluster
 * @param minPoints (optional) — Minimum points required to form a dense region
 * @param source (optional) — Choose which backend supplies the training data
 * @returns nClusters — Number of clusters found (excluding noise)
 * @returns nNoise — Number of points classified as noise
 * @impure has side effects / drives control flow
 */
declare function fitDbscan({ epsilon?: float, minPoints?: int, source?: string }): { nClusters: int, nNoise: int };

/**
 * Fit/Train KMeans Clustering
 * @param cluster (optional) — Choose how many centroids to fit
 * @param source (optional) — Choose which backend supplies the training data
 * @returns model — Thread-safe handle to the trained KMeans model
 * @impure has side effects / drives control flow
 */
declare function fitKmeans({ cluster?: int, source?: string }): Struct;


// === AI/ML/Dataset ===

/**
 * Generate K train/test splits for cross-validation. Each fold uses (K-1)/K data for training and 1/K for validation.
 * @param k (optional) — Number of folds for cross-validation (typically 5 or 10)
 * @param shuffle (optional) — Randomly shuffle data before splitting
 * @param source — Source database containing the dataset
 * @param trainDb — Database to receive training data for each fold (will be cleared and filled K times)
 * @param testDb — Database to receive validation data for each fold (will be cleared and filled K times)
 * @returns foldIndex — Current fold index (0 to K-1)
 * @returns info — Information about the K-fold split
 * @impure has side effects / drives control flow
 */
declare function aiMlDatasetKfold({ k?: int, shuffle?: bool, source: Struct, trainDb: Struct, testDb: Struct }): { foldIndex: int, info: Struct };

/**
 * Random sample N records or a ratio from a dataset
 * @param sampleCount (optional) — Number of records to sample (if set, takes precedence over ratio)
 * @param sampleRatio (optional) — Ratio of records to sample (0.0 to 1.0, used if sample_count is 0)
 * @param source — Data Source (DB or CSV)
 * @param target — Destination database connection that receives the sampled rows
 * @returns sampledCount — Number of records that were sampled
 * @impure has side effects / drives control flow
 */
declare function aiMlDatasetSample({ sampleCount?: int, sampleRatio?: float, source: Struct, target: Struct }): int;

/**
 * Shuffle dataset rows randomly
 * @param source — Data Source (DB or CSV)
 * @param target — Destination database connection that receives the shuffled rows
 * @impure has side effects / drives control flow
 */
declare function aiMlDatasetShuffle({ source: Struct, target: Struct }): void;

/**
 * Split a dataset into training and testing subsets
 * @param split (optional) — Ratio used for assigning rows to the training set (rest goes to test)
 * @param source — Data Source (DB or CSV)
 * @param train — Destination database connection that receives the training rows
 * @param test — Destination database connection that receives the testing rows
 * @impure has side effects / drives control flow
 */
declare function aiMlDatasetSplit({ split?: float, source: Struct, train: Struct, test: Struct }): void;

/**
 * Split a dataset into training and testing subsets while maintaining class distribution
 * @param split (optional) — Ratio used for assigning rows to the training set (rest goes to test)
 * @param labelColumn (optional) — Name of the column containing class labels for stratification
 * @param source — Data Source (DB or CSV)
 * @param train — Destination database connection that receives the training rows
 * @param test — Destination database connection that receives the testing rows
 * @impure has side effects / drives control flow
 */
declare function aiMlDatasetStratifiedSplit({ split?: float, labelColumn?: string, source: Struct, train: Struct, test: Struct }): void;


// === AI/ML/Metrics ===

/**
 * Calculate classification accuracy by comparing predictions to actual values
 * @param database — Database connection containing predictions and actuals
 * @param predictionsCol (optional) — Column name containing predicted values
 * @param actualsCol (optional) — Column name containing actual/true values
 * @returns result — Accuracy metrics including score and counts
 * @impure has side effects / drives control flow
 */
declare function mlEvalAccuracy({ database: Struct, predictionsCol?: string, actualsCol?: string }): Struct;

/**
 * Build confusion matrix and calculate precision, recall, and F1 score
 * @param database — Database connection containing predictions and actuals
 * @param predictionsCol (optional) — Column name containing predicted values
 * @param actualsCol (optional) — Column name containing actual/true values
 * @returns result — Confusion matrix with precision, recall, and F1 metrics
 * @impure has side effects / drives control flow
 */
declare function mlEvalConfusionMatrix({ database: Struct, predictionsCol?: string, actualsCol?: string }): Struct;

/**
 * Calculate MSE, RMSE, MAE, and R² for regression predictions
 * @param database — Database connection containing predictions and actuals
 * @param predictionsCol (optional) — Column name containing predicted float values
 * @param actualsCol (optional) — Column name containing actual/true float values
 * @returns result — Regression metrics (MSE, RMSE, MAE, R²)
 * @impure has side effects / drives control flow
 */
declare function mlEvalRegression({ database: Struct, predictionsCol?: string, actualsCol?: string }): Struct;


// === AI/ML/Model Info ===

/**
 * Extract cluster centroids from a trained KMeans model
 * @param model — Trained KMeans model
 * @returns result — Cluster centroids with metadata
 * @impure has side effects / drives control flow
 */
declare function mlGetKmeansCentroids({ model: Struct }): Struct;

/**
 * Extract coefficients and intercept from a trained Linear Regression model
 * @param model — Trained Linear Regression model
 * @returns result — Regression coefficients with intercept
 * @impure has side effects / drives control flow
 */
declare function mlGetLinearCoefficients({ model: Struct }): Struct;

/**
 * Get general information about any ML model
 * @param model — Any trained ML model
 * @returns info — Model information structure
 * @returns modelType — Model type as string
 * @impure has side effects / drives control flow
 */
declare function mlModelInfo({ model: Struct }): { info: Struct, modelType: string };


// === AI/ML/Reduction ===

/**
 * Principal Component Analysis for dimensionality reduction
 * @param nComponents (optional) — Number of principal components to keep
 * @param source (optional) — Choose which backend supplies the data
 * @returns explainedVariance — Variance explained by each principal component
 * @impure has side effects / drives control flow
 */
declare function fitPca({ nComponents?: int, source?: string }): float[];

/**
 * t-Distributed Stochastic Neighbor Embedding for dimensionality reduction (placeholder - not yet implemented)
 * @param nComponents (optional) — Number of dimensions to reduce to (typically 2 or 3)
 * @param perplexity (optional) — Related to the number of nearest neighbors (typical values: 5-50)
 * @impure has side effects / drives control flow
 */
declare function fitTsne({ nComponents?: int, perplexity?: float }): void;


// === AI/ML/Regression ===

/**
 * Fit/Train Linear Regression Model
 * @param source (optional) — Choose where training data should be loaded from
 * @returns model — Thread-safe handle to the trained linear regression model
 * @impure has side effects / drives control flow
 */
declare function fitLinearRegression({ source?: string }): Struct;


// === AI/ML/Tuning ===

/**
 * Automatically finds the best classification model. Tries Naive Bayes, Decision Tree, and SVM with cross-validation.
 * @param cvFolds (optional) — Number of cross-validation folds
 * @param metric (optional) — Optimization metric
 * @param includeSvm (optional) — Include SVM in comparison (slower but often more accurate)
 * @param source (optional) — Data source type
 * @returns results — Complete AutoML results with leaderboard
 * @returns bestModel — The best model trained on full data
 * @returns bestModelType — Name of the best algorithm
 * @impure has side effects / drives control flow
 */
declare function aiMlTuningAutoClassifier({ cvFolds?: int, metric?: string, includeSvm?: bool, source?: string }): { results: Struct, bestModel: Struct, bestModelType: string };

/**
 * Exhaustive search over parameter combinations with cross-validation. Returns the best parameters found.
 * @param modelType (optional) — Type of model to tune
 * @param cvFolds (optional) — Number of cross-validation folds
 * @param source (optional) — Database containing the training data
 * @returns results — Complete grid search results with all combinations tried
 * @returns bestModel — The model trained with the best parameters on full training data
 * @impure has side effects / drives control flow
 */
declare function aiMlTuningGridSearch({ modelType?: string, cvFolds?: int, source?: string }): { results: Struct, bestModel: Struct };


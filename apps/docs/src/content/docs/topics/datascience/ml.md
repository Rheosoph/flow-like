---
title: Machine Learning
description: Prepare data, train supported models, evaluate them, and run inference in Flow-Like
sidebar:
  order: 4
---

Flow-Like provides workflow nodes for classical machine learning and ONNX inference. Keep data preparation, splitting, training, evaluation, persistence, and prediction as explicit stages so the result can be reproduced and reviewed.

## Supported workflow families

| Task | Current catalog examples |
|------|--------------------------|
| Classification | Decision tree, naive Bayes, multi-class SVM |
| Regression | Linear regression |
| Clustering | KMeans, DBSCAN |
| Dimensionality reduction | PCA, t-SNE |
| Evaluation | Accuracy, confusion matrix, regression metrics |
| Tuning | Grid search, auto classifier |
| Inference | Saved Flow-Like models and ONNX models |

Browse the [Machine Learning node catalog](/nodes/ai/ml/) for current pins, schemas, and model requirements.

## Prepare data

### Define the observation grain

Each row should represent the same kind of observation. Before training:

- choose a stable record identifier;
- remove leakage from fields created after the prediction target;
- define numeric or categorical feature handling;
- document missing-value behavior;
- fix the target definition and evaluation window;
- retain the source version or query window.

### Split before fitting

| Need | Node |
|------|------|
| General train/test split | [Split Dataset](/nodes/ai/ml/dataset/ai-ml-dataset-split/) |
| Preserve class proportions | [Stratified Split](/nodes/ai/ml/dataset/ai-ml-dataset-stratified-split/) |
| Repeated fold evaluation | [K-Fold Split](/nodes/ai/ml/dataset/ai-ml-dataset-kfold/) |
| Shuffle or sample | [Shuffle Dataset](/nodes/ai/ml/dataset/ai-ml-dataset-shuffle/), [Sample Dataset](/nodes/ai/ml/dataset/ai-ml-dataset-sample/) |

Perform transformations that learn from data, such as scaling statistics or category vocabularies, using the training portion only. Reuse the fitted transformation for validation and inference.

For time-dependent data, use a chronological split rather than randomizing future observations into the training set.

## Classification

| Model | Node | Useful starting point |
|-------|------|-----------------------|
| Decision tree | [Train Classifier (Decision Tree)](/nodes/ai/ml/classification/fit-decision-tree/) | Interpretable non-linear rules |
| Naive Bayes | [Train Classifier (Naive Bayes)](/nodes/ai/ml/classification/fit-naive-bayes/) | Fast baseline under its feature assumptions |
| Multi-class SVM | [Train Classifier (SVM)](/nodes/ai/ml/classification/fit-svm-multi-class/) | Margin-based classification on prepared features |

Start with a baseline and compare models on the same untouched evaluation set. Accuracy alone can hide poor minority-class performance.

Use [Accuracy](/nodes/ai/ml/metrics/ml-eval-accuracy/) and [Confusion Matrix](/nodes/ai/ml/metrics/ml-eval-confusion-matrix/) together. Add task-specific precision, recall, or cost analysis downstream when false positives and false negatives have different consequences.

## Regression

[Train Regression (Linear)](/nodes/ai/ml/regression/fit-linear-regression/) fits a linear regression model. [Get Coefficients](/nodes/ai/ml/model-info/ml-get-linear-coefficients/) exposes the learned coefficients for interpretation, and [Regression Metrics](/nodes/ai/ml/metrics/ml-eval-regression/) evaluates predictions.

Check residual patterns and target distribution, not only one aggregate score. Large outliers, time drift, and extrapolation beyond the training range can make a plausible average metric misleading.

## Clustering

| Model | Node | Main decision |
|-------|------|---------------|
| KMeans | [Train Clustering (KMeans)](/nodes/ai/ml/clustering/fit-kmeans/) | Number of clusters and feature scaling |
| DBSCAN | [Train Clustering (DBSCAN)](/nodes/ai/ml/clustering/fit-dbscan/) | Neighborhood radius and minimum density |

[Get Centroids](/nodes/ai/ml/model-info/ml-get-kmeans-centroids/) helps inspect KMeans clusters. Clusters do not acquire business meaning automatically; review representative records and stability before assigning labels.

DBSCAN can identify noise without choosing a cluster count, but its behavior depends strongly on scale and density. Standardize comparable numeric features before tuning distance-based models.

## Dimensionality reduction

| Method | Node | Typical use |
|--------|------|-------------|
| PCA | [PCA Reduction](/nodes/ai/ml/reduction/fit-pca/) | Linear compression and variance analysis |
| t-SNE | [t-SNE Reduction](/nodes/ai/ml/reduction/fit-tsne/) | Exploratory low-dimensional visualization |

Fit reduction on training data when it is part of a predictive pipeline. Treat t-SNE layouts as exploratory views; distances and cluster shapes should not be interpreted as a validated predictive model.

## Tune models

[Grid Search](/nodes/ai/ml/tuning/ai-ml-tuning-grid-search/) evaluates configured parameter combinations. [Auto Classifier](/nodes/ai/ml/tuning/ai-ml-tuning-auto-classifier/) compares supported classifier choices.

Use a validation strategy that remains separate from the final test set. Record:

- candidate parameter ranges;
- selected metric;
- random seed where available;
- split definition;
- winning configuration and score;
- training data version.

## Predict

[Predict](/nodes/ai/ml/ml-predict/) runs a saved or newly trained model on prepared features. [Prediction Class/Label](/nodes/ai/ml/ai-ml-pred-class-or-label/) and [Prediction Score](/nodes/ai/ml/teachable-machine/ai-ml-pred-score/) expose task-specific prediction details where applicable.

Inference must reproduce the training feature order, types, missing-value handling, and transformations. Validate the input schema before invoking the model.

## Save and load models

| Need | Node |
|------|------|
| Save model through the path abstraction | [Save Model](/nodes/ai/ml/save-ml-model/) |
| Load model through the path abstraction | [Load Model](/nodes/ai/ml/load-ml-model/) |
| Save raw binary model data | [Save Model (Binary)](/nodes/ai/ml/save-ml-model-binary/) |
| Load raw binary model data | [Load Model (Binary)](/nodes/ai/ml/load-ml-model-binary/) |
| Inspect metadata | [Model Info](/nodes/ai/ml/model-info/ml-model-info/) |

Store a small model card with the artifact: training data version, feature schema, target, metrics, intended use, limitations, and owner.

## ONNX inference

The [ONNX node family](/nodes/ai/ml/onnx/) covers model loading plus several prepared tasks:

| Area | Examples |
|------|----------|
| Vision | Image classification, object detection, segmentation, depth, pose |
| OCR | Text detection, region cropping, text recognition |
| Face | Detection, embeddings, comparison |
| Audio | Loading, resampling, spectrograms, voice activity detection |
| NLP | Named-entity recognition |
| Batch | Batch image inference |

Start with [Load ONNX](/nodes/ai/ml/onnx/load-onnx/) and inspect [ONNX Model Info](/nodes/ai/ml/onnx/onnx-model-info/) and [Session Info](/nodes/ai/ml/onnx/onnx-session-info/). Model compatibility depends on input tensors, preprocessing, output tensors, and supported operators. Validate those contracts instead of assuming any ONNX file will work with a task-specific node.

[Teachable Machine](/nodes/ai/ml/ai-ml-teachable-machine/) is available for compatible exported models.

## Example: churn model

| Stage | Operation |
|-------|-----------|
| Source | Query one row per customer with a fixed observation window |
| Validate | Check target, features, missing values, and class distribution |
| Split | Stratify train and test data by churn label |
| Baseline | Train a decision tree with a documented configuration |
| Evaluate | Review accuracy and the confusion matrix |
| Persist | Save the model and its feature schema |
| Serve | Validate new customer features, predict, and record model version |
| Monitor | Compare outcome drift and class balance over time |

## Production checklist

- [ ] Observation grain and target are documented
- [ ] Leakage fields are removed
- [ ] Train, validation, and test data are separate
- [ ] Preprocessing is fitted on training data and reused at inference
- [ ] Baseline and selected model use the same evaluation set
- [ ] Multiple task-relevant metrics are reviewed
- [ ] Model, feature schema, and data version are saved together
- [ ] Inference validates feature order and types
- [ ] Drift, failures, and model version are observable

## Next steps

- [Data loading and storage](/topics/datascience/loading/)
- [DataFusion and SQL](/topics/datascience/datafusion/)
- [Data visualization](/topics/datascience/visualization/)
- [AI-powered analysis](/topics/datascience/ai-analysis/)

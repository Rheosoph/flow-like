//! ML Evaluation Metrics Module
//!
//! This module provides nodes for evaluating machine learning model performance:
//! - **accuracy**: Classification accuracy (correct predictions / total)
//! - **confusion_matrix**: Confusion matrix with precision, recall, and F1 score
//! - **regression_metrics**: MSE, RMSE, MAE, and R² for regression models
//! - **roc_auc**: ROC-AUC and log loss for probabilistic binary classifiers
//! - **silhouette**: Silhouette score for clustering quality

pub mod accuracy;
pub mod confusion_matrix;
pub mod regression_metrics;
pub mod roc_auc;
pub mod silhouette;

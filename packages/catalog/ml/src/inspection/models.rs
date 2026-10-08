use super::operation;
use flow_like_ml_native::{dataset::StandardScaler, models::*};
use flow_like_types::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct IsolationForestRequest {
    pub rows: Vec<Vec<f64>>,
    pub config: IsolationForestConfig,
}
fn isolation(input: IsolationForestRequest) -> Result<NativeModel> {
    Ok(NativeModel::IsolationForest(IsolationForest::fit(
        &input.rows,
        &input.config,
    )?))
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct BoostRequest {
    pub rows: Vec<Vec<f64>>,
    pub targets: Vec<f64>,
    pub config: BoostConfig,
}
fn boost(input: BoostRequest) -> Result<NativeModel> {
    Ok(NativeModel::HistogramGradientBoosting(
        HistogramGradientBoosting::fit(&input.rows, &input.targets, &input.config)?,
    ))
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PatchCoreRequest {
    pub normal_patches: Vec<Vec<f64>>,
    pub coreset_size: usize,
}
fn patchcore(input: PatchCoreRequest) -> Result<NativeModel> {
    Ok(NativeModel::PatchCore(PatchCore::fit(
        &input.normal_patches,
        input.coreset_size,
    )?))
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PadimRequest {
    pub normal_images: Vec<Vec<Vec<f64>>>,
    pub regularization: f64,
}
fn padim(input: PadimRequest) -> Result<NativeModel> {
    Ok(NativeModel::Padim(Padim::fit(
        &input.normal_images,
        input.regularization,
    )?))
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "architecture", content = "model", rename_all = "snake_case")]
pub enum NativeModel {
    IsolationForest(IsolationForest),
    HistogramGradientBoosting(HistogramGradientBoosting),
    PatchCore(PatchCore),
    Padim(Padim),
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct NativePredictionRequest {
    pub model: NativeModel,
    pub rows: Vec<Vec<f64>>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct NativePrediction {
    pub scores: Vec<f64>,
    pub image_score: Option<f64>,
}
fn predict(input: NativePredictionRequest) -> Result<NativePrediction> {
    let (scores, image_score) = match input.model {
        NativeModel::IsolationForest(model) => (
            input
                .rows
                .iter()
                .map(|row| model.score(row))
                .collect::<flow_like_ml_core::Result<Vec<_>>>()?,
            None,
        ),
        NativeModel::HistogramGradientBoosting(model) => (
            input
                .rows
                .iter()
                .map(|row| model.predict(row))
                .collect::<flow_like_ml_core::Result<Vec<_>>>()?,
            None,
        ),
        NativeModel::PatchCore(model) => {
            let scores = model.score_patches(&input.rows)?;
            let max = scores.iter().copied().reduce(f64::max);
            (scores, max)
        }
        NativeModel::Padim(model) => {
            let scores = model.score_patches(&input.rows)?;
            let max = scores.iter().copied().reduce(f64::max);
            (scores, max)
        }
    };
    Ok(NativePrediction {
        scores,
        image_score,
    })
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct FitScalerRequest {
    pub training_rows: Vec<Vec<f64>>,
}
fn scaler(input: FitScalerRequest) -> Result<StandardScaler> {
    Ok(StandardScaler::fit(&input.training_rows)?)
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScaleRequest {
    pub scaler: StandardScaler,
    pub rows: Vec<Vec<f64>>,
}
fn scale(input: ScaleRequest) -> Result<Vec<Vec<f64>>> {
    Ok(input
        .rows
        .iter()
        .map(|r| input.scaler.transform(r))
        .collect::<flow_like_ml_core::Result<_>>()?)
}

#[crate::register_node]
#[derive(Default)]
pub struct TrainIsolationForestNode;
operation!(
    TrainIsolationForestNode,
    "ml_train_isolation_forest",
    "Train Isolation Forest",
    "Fit isolation trees to normal sensor or embedding features",
    "trainIsolationForest",
    "AI/ML/Anomaly",
    IsolationForestRequest,
    NativeModel,
    isolation
);
#[crate::register_node]
#[derive(Default)]
pub struct TrainGradientBoostedTreesNode;
operation!(
    TrainGradientBoostedTreesNode,
    "ml_train_gradient_boosted_trees",
    "Train Gradient Boosted Trees",
    "Fit native histogram trees with squared-error or binary logistic loss",
    "trainGradientBoostedTrees",
    "AI/ML/Training",
    BoostRequest,
    NativeModel,
    boost
);
#[crate::register_node]
#[derive(Default)]
pub struct TrainPatchCoreNode;
operation!(
    TrainPatchCoreNode,
    "ml_train_patchcore",
    "Train PatchCore Memory Bank",
    "Select a farthest-first coreset of normal frozen-encoder patch embeddings",
    "trainPatchCore",
    "AI/ML/Anomaly",
    PatchCoreRequest,
    NativeModel,
    patchcore
);
#[crate::register_node]
#[derive(Default)]
pub struct TrainPadimNode;
operation!(
    TrainPadimNode,
    "ml_train_padim",
    "Train PaDiM",
    "Fit a regularized Gaussian per spatial patch of normal image embeddings",
    "trainPadim",
    "AI/ML/Anomaly",
    PadimRequest,
    NativeModel,
    padim
);
#[crate::register_node]
#[derive(Default)]
pub struct NativeModelInferenceNode;
operation!(
    NativeModelInferenceNode,
    "ml_native_model_inference",
    "Native Model Inference",
    "Predict with a serialized isolation forest, boosted trees, PatchCore or PaDiM model",
    "predictNative",
    "AI/ML/Inference",
    NativePredictionRequest,
    NativePrediction,
    predict
);
#[crate::register_node]
#[derive(Default)]
pub struct FitStandardScalerNode;
operation!(
    FitStandardScalerNode,
    "ml_fit_standard_scaler",
    "Fit Standard Scaler",
    "Fit feature means and scales using only the training partition",
    "fitScaler",
    "AI/ML/Dataset",
    FitScalerRequest,
    StandardScaler,
    scaler
);
#[crate::register_node]
#[derive(Default)]
pub struct ApplyStandardScalerNode;
operation!(
    ApplyStandardScalerNode,
    "ml_apply_standard_scaler",
    "Apply Standard Scaler",
    "Normalize features with saved training statistics",
    "scale",
    "AI/ML/Dataset",
    ScaleRequest,
    Vec<Vec<f64>>,
    scale
);

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::{node::NodeLogic, pin::Pin};
    use flow_like_types::json::{from_value, json};

    #[test]
    fn training_results_feed_native_inference_without_a_model_wrapper() {
        let rows = vec![vec![0.0], vec![0.1], vec![0.2], vec![0.3]];
        let models = [
            isolation(IsolationForestRequest {
                rows: rows.clone(),
                config: IsolationForestConfig {
                    trees: 4,
                    sample_size: 4,
                    seed: 7,
                },
            })
            .unwrap(),
            boost(BoostRequest {
                rows: rows.clone(),
                targets: vec![0.0, 0.1, 0.2, 0.3],
                config: BoostConfig::default(),
            })
            .unwrap(),
            patchcore(PatchCoreRequest {
                normal_patches: rows.clone(),
                coreset_size: 2,
            })
            .unwrap(),
            padim(PadimRequest {
                normal_images: vec![rows.clone(), rows.clone()],
                regularization: 0.01,
            })
            .unwrap(),
        ];
        for model in models {
            let value = json!(model);
            assert!(value.get("architecture").is_some());
            let request: NativePredictionRequest =
                from_value(json!({"model":value,"rows":rows})).unwrap();
            let prediction = predict(request).unwrap();
            assert_eq!(prediction.scores.len(), rows.len());
            assert!(prediction.scores.iter().all(|value| value.is_finite()));
        }
        for node in [
            TrainIsolationForestNode.get_node(),
            TrainGradientBoostedTreesNode.get_node(),
            TrainPatchCoreNode.get_node(),
            TrainPadimNode.get_node(),
        ] {
            assert_eq!(
                node.get_pin_by_name("result").unwrap().schema,
                Pin::schema_string_for::<NativeModel>()
            );
        }
    }
}

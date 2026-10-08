use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LegacyAlgorithm {
    GaussianNaiveBayes,
    DecisionTree { max_depth: usize, min_leaf: usize },
    Logistic { alpha: f64, max_iterations: u64 },
    OrdinalRidge { alpha: f64 },
    OrdinalLogistic { alpha: f64, max_iterations: usize },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LegacyRecipe {
    pub algorithm: LegacyAlgorithm,
    pub labels: Vec<String>,
}

#[cfg(feature = "training")]
mod execution {
    use super::*;
    use crate::ml::{MLModel, ModelWithMeta};
    use flow_like_ml_core::{Annotation, ComputeBackend, ComputeConfig, Sample, TensorData};
    use flow_like_ml_runtime::{
        self as runtime, TrainingRepository,
        auto_training::{NativeSearchPredictorFactory, SearchPredictor, SearchPredictorFactory},
        worker::{EngineOutput, TrainingEngine, TrainingWork, TrainingWorker, WorkerControl},
    };
    use flow_like_types::{
        Value,
        json::{self, json},
    };
    use linfa::{DatasetBase, traits::Fit};
    use ndarray::{Array1, Array2};
    use std::{
        collections::{HashMap, HashSet},
        sync::Arc,
    };

    fn error(value: impl std::fmt::Display) -> runtime::Error {
        runtime::Error::Engine(value.to_string())
    }

    fn decode_model(bytes: &[u8]) -> runtime::Result<MLModel> {
        let value: Value = json::from_slice(bytes)?;
        // Dispatch before serde buffers an internally tagged enum. GaussianNb stores usize
        // class-map keys, which JSON must deserialize directly from string map keys.
        Ok(match value["type"].as_str() {
            Some("GaussianNaiveBayes") => MLModel::GaussianNaiveBayes(json::from_value(value)?),
            Some("DecisionTree") => MLModel::DecisionTree(json::from_value(value)?),
            Some("LogisticRegression") => MLModel::LogisticRegression(json::from_value(value)?),
            Some("MultinomialLogisticRegression") => {
                MLModel::MultinomialLogisticRegression(json::from_value(value)?)
            }
            Some("OrdinalLogistic") => MLModel::OrdinalLogistic(json::from_value(value)?),
            Some("OrdinalRidge") => MLModel::OrdinalRidge(json::from_value(value)?),
            _ => return Err(error("Unsupported classical weight type")),
        })
    }

    fn validate_compute(compute: &ComputeConfig) -> runtime::Result<()> {
        compute.validate().map_err(error)?;
        if !matches!(compute.backend, ComputeBackend::Auto | ComputeBackend::Cpu)
            && !compute.allow_cpu_fallback
        {
            return Err(error("Classical models require CPU or backend fallback"));
        }
        Ok(())
    }

    impl LegacyRecipe {
        pub fn validate(&self) -> runtime::Result<()> {
            flow_like_ml_core::validate_labels(&self.labels).map_err(error)?;
            if !(2..=1024).contains(&self.labels.len()) {
                return Err(error(
                    "Classical classification requires 2 to 1024 ordered labels",
                ));
            }
            let valid = match self.algorithm {
                LegacyAlgorithm::GaussianNaiveBayes => true,
                LegacyAlgorithm::DecisionTree {
                    max_depth,
                    min_leaf,
                } => (1..=64).contains(&max_depth) && (1..=1_000_000).contains(&min_leaf),
                LegacyAlgorithm::Logistic {
                    alpha,
                    max_iterations,
                } => alpha.is_finite() && alpha >= 0. && (1..=10_000).contains(&max_iterations),
                LegacyAlgorithm::OrdinalRidge { alpha } => alpha.is_finite() && alpha > 0.,
                LegacyAlgorithm::OrdinalLogistic {
                    alpha,
                    max_iterations,
                } => alpha.is_finite() && alpha >= 0. && (1..=10_000).contains(&max_iterations),
            };
            if !valid {
                return Err(error(
                    "Invalid classical model regularization or iteration limits",
                ));
            }
            Ok(())
        }
    }

    fn fit_model(
        recipe: &LegacyRecipe,
        inputs: Vec<f64>,
        targets: Vec<usize>,
        width: usize,
    ) -> runtime::Result<MLModel> {
        recipe.validate()?;
        if width == 0
            || width > 4096
            || targets.len() < 2
            || targets.len().checked_mul(width) != Some(inputs.len())
            || inputs.iter().any(|v| !v.is_finite())
            || targets.iter().any(|v| *v >= recipe.labels.len())
        {
            return Err(error(
                "Classical training requires a finite rectangular feature matrix and valid classes",
            ));
        }
        let found: HashSet<_> = targets.iter().copied().collect();
        if found.len() != recipe.labels.len() {
            return Err(error(
                "Each configured class needs training examples; change the split or collect more labels",
            ));
        }
        if matches!(recipe.algorithm, LegacyAlgorithm::GaussianNaiveBayes)
            && !(0..width).any(|column| {
                inputs
                    .chunks_exact(width)
                    .any(|row| row[column] != inputs[column])
            })
        {
            return Err(error(
                "Gaussian Naive Bayes requires at least one varying training feature",
            ));
        }
        let dataset = DatasetBase::new(
            Array2::from_shape_vec((targets.len(), width), inputs).map_err(error)?,
            Array1::from(targets),
        );
        let classes = Some(
            recipe
                .labels
                .iter()
                .cloned()
                .enumerate()
                .collect::<HashMap<_, _>>(),
        );
        Ok(match recipe.algorithm {
            LegacyAlgorithm::GaussianNaiveBayes => MLModel::GaussianNaiveBayes(ModelWithMeta {
                model: linfa_bayes::GaussianNb::params()
                    .fit(&dataset)
                    .map_err(error)?,
                classes,
            }),
            LegacyAlgorithm::DecisionTree {
                max_depth,
                min_leaf,
            } => MLModel::DecisionTree(ModelWithMeta {
                model: linfa_trees::DecisionTree::params()
                    .max_depth(Some(max_depth))
                    .min_weight_leaf(min_leaf as f32)
                    .fit(&dataset)
                    .map_err(error)?,
                classes,
            }),
            LegacyAlgorithm::Logistic {
                alpha,
                max_iterations,
            } if recipe.labels.len() == 2 => MLModel::LogisticRegression(ModelWithMeta {
                model: linfa_logistic::LogisticRegression::default()
                    .alpha(alpha)
                    .max_iterations(max_iterations)
                    .fit(&dataset)
                    .map_err(error)?,
                classes,
            }),
            LegacyAlgorithm::Logistic {
                alpha,
                max_iterations,
            } => MLModel::MultinomialLogisticRegression(ModelWithMeta {
                model: linfa_logistic::MultiLogisticRegression::default()
                    .alpha(alpha)
                    .max_iterations(max_iterations)
                    .fit(&dataset)
                    .map_err(error)?,
                classes,
            }),
            LegacyAlgorithm::OrdinalRidge { alpha } => MLModel::OrdinalRidge(ModelWithMeta {
                model: flow_like_ordinal::OrdinalRidge::params()
                    .alpha(alpha)
                    .n_levels(recipe.labels.len())
                    .fit(&dataset)
                    .map_err(error)?,
                classes,
            }),
            LegacyAlgorithm::OrdinalLogistic {
                alpha,
                max_iterations,
            } => MLModel::OrdinalLogistic(ModelWithMeta {
                model: flow_like_ordinal::OrdinalLogistic::params()
                    .alpha(alpha)
                    .max_iterations(max_iterations)
                    .n_levels(recipe.labels.len())
                    .fit(&dataset)
                    .map_err(error)?,
                classes,
            }),
        })
    }

    #[derive(Default)]
    pub struct LegacyTrainingEngine;
    impl TrainingEngine for LegacyTrainingEngine {
        fn estimated_memory_bytes(&self, work: &TrainingWork) -> runtime::Result<u64> {
            let recipe: LegacyRecipe = json::from_value(work.job.request.recipe.clone())?;
            recipe.validate()?;
            let mut elements = 0u64;
            let mut width = 0u64;
            for sample in &work.snapshot.train {
                let raw = sample.payload.get("sample").unwrap_or(&sample.payload);
                let values = raw["input"]["values"]
                    .as_array()
                    .ok_or_else(|| error("Training sample has no feature values"))?;
                width = width.max(values.len() as u64);
                elements = elements
                    .checked_add(values.len() as u64)
                    .ok_or_else(|| error("Feature matrix size overflows"))?;
            }
            if elements > 32 * 1024 * 1024 || width > 4096 || work.snapshot.train.len() > 1_000_000
            {
                return Err(error(
                    "Classical training matrix exceeds the bounded adapter capacity",
                ));
            }
            let gram = if matches!(recipe.algorithm, LegacyAlgorithm::OrdinalRidge { .. }) {
                width * width * 24
            } else {
                0
            };
            Ok(elements * 32 + gram + 8 * 1024 * 1024)
        }
        fn resource_key(&self, work: &TrainingWork) -> runtime::Result<String> {
            let compute: ComputeConfig = json::from_value(work.job.request.compute.clone())?;
            validate_compute(&compute)?;
            Ok("cpu:0".into())
        }
        fn train(
            &self,
            work: &TrainingWork,
            control: &WorkerControl,
        ) -> runtime::Result<EngineOutput> {
            self.resource_key(work)?;
            self.estimated_memory_bytes(work)?;
            let recipe: LegacyRecipe = json::from_value(work.job.request.recipe.clone())?;
            recipe.validate()?;
            control.progress(
                json!({"stage":"loading_classical_matrix","rows":work.snapshot.train.len()}),
            )?;
            let mut inputs = Vec::new();
            let mut targets = Vec::new();
            let mut width = None;
            let mut seen = HashSet::new();
            for stored in &work.snapshot.train {
                control.check_cancel()?;
                let sample: Sample = json::from_value(
                    stored
                        .payload
                        .get("sample")
                        .unwrap_or(&stored.payload)
                        .clone(),
                )?;
                sample.validate(recipe.labels.len(), 4096).map_err(error)?;
                if sample.id != stored.id
                    || sample.group_id != stored.group_id
                    || !seen.insert(sample.id.clone())
                    || !stored.accepted
                {
                    return Err(error(
                        "Training sample identity/group or acceptance differs from its ledger",
                    ));
                }
                if sample.input.shape.len() != 1
                    || width.is_some_and(|v| v != sample.input.values.len())
                {
                    return Err(error(
                        "Classical models require a fixed one-dimensional feature vector per sample",
                    ));
                }
                width = Some(sample.input.values.len());
                let Annotation::Class { class_id } = sample.annotation else {
                    return Err(error("Classical classifier requires class annotations"));
                };
                inputs.extend(sample.input.values.into_iter().map(f64::from));
                targets.push(class_id as usize);
            }
            control.progress(json!({"stage":"fitting_classical_model","restart_from_snapshot":work.resume_checkpoint.is_some()}))?;
            // These deterministic batch solvers have no optimizer checkpoint API. A retry fits
            // the immutable training partition again; cancellation is checked around the fit.
            let model = fit_model(&recipe, inputs, targets, width.unwrap_or(0))?;
            control.check_cancel()?;
            let model_bytes = model.to_json_vec().map_err(error)?;
            if model_bytes.len() as u64 > control.limits().maximum_artifact_bytes {
                return Err(error("Classical model exceeds artifact byte budget"));
            }
            Ok(EngineOutput {
                model_bytes,
                manifest: json!({"engine":"legacy_classical","format_version":1,"serialization":"mlmodel_json","model_kind":model.kind(),"input_shape":[width.unwrap_or(0)],"labels":recipe.labels,"algorithm":recipe.algorithm,"backend":{"backend":"cpu"},"resume_semantics":"deterministic_refit"}),
            })
        }
    }

    pub fn register_legacy_engine(worker: &mut TrainingWorker) -> runtime::Result<()> {
        worker.register("legacy_classical", Arc::new(LegacyTrainingEngine))
    }

    pub struct LegacyPredictor {
        model: MLModel,
        width: usize,
        labels: Vec<String>,
        maximum_elements: usize,
    }
    impl SearchPredictor for LegacyPredictor {
        fn predict(&mut self, input: &TensorData) -> runtime::Result<Value> {
            input.validate(self.maximum_elements).map_err(error)?;
            let [batch, width] = input.shape.as_slice() else {
                return Err(error("Classical inference requires [batch,features]"));
            };
            if *width != self.width {
                return Err(error(
                    "Inference feature width differs from the fitted preprocessing contract",
                ));
            }
            if batch
                .checked_mul(self.width + self.labels.len())
                .is_none_or(|elements| elements > self.maximum_elements)
            {
                return Err(error(
                    "Classical prediction outputs exceed the inference memory budget",
                ));
            }
            let mut classes = Vec::with_capacity(*batch);
            let mut confidence = Vec::with_capacity(*batch);
            let mut probabilities = Vec::new();
            for row in input.values.chunks_exact(self.width) {
                let vector: Vec<f64> = row.iter().copied().map(f64::from).collect();
                let prediction = self
                    .model
                    .predict_on_vector(vector.clone())
                    .map_err(error)?;
                if !prediction.score.is_finite()
                    || prediction.score.fract() != 0.
                    || prediction.score < 0.
                    || prediction.score as usize >= self.labels.len()
                {
                    return Err(error(
                        "Model returned a class outside its persisted label order",
                    ));
                }
                classes.push(prediction.score as usize);
                confidence.push(prediction.confidence);
                let array = Array2::from_shape_vec((1, self.width), vector).map_err(error)?;
                match &self.model {
                    MLModel::LogisticRegression(model) => {
                        let p = model.model.predict_probabilities(&array)[0];
                        let mut scores = vec![0.; self.labels.len()];
                        scores[model.model.labels().pos.class] = p;
                        scores[model.model.labels().neg.class] = 1. - p;
                        probabilities.extend(scores);
                    }
                    MLModel::MultinomialLogisticRegression(model) => {
                        let scores = model.model.predict_probabilities(&array);
                        let mut ordered = vec![0.; self.labels.len()];
                        for (index, class) in model.model.classes().iter().enumerate() {
                            ordered[*class] = scores[[0, index]];
                        }
                        probabilities.extend(ordered);
                    }
                    MLModel::OrdinalLogistic(model) => probabilities.extend(
                        model
                            .model
                            .predict_probabilities(&array.row(0))
                            .map_err(error)?,
                    ),
                    _ => {}
                }
            }
            let mut result = json!({"classes":classes,"labels":self.labels,"confidence":confidence,"backend":{"backend":"cpu"}});
            if !probabilities.is_empty() {
                if probabilities.len() != batch * self.labels.len()
                    || probabilities
                        .iter()
                        .any(|v| !v.is_finite() || *v < 0. || *v > 1.)
                {
                    return Err(error("Model returned invalid class probabilities"));
                }
                result["output"] =
                    json!({"shape":[batch,self.labels.len()],"values":probabilities});
                result["score_semantics"] = json!("class_probabilities");
            } else {
                result["score_semantics"] = json!("hard_classes");
            }
            Ok(result)
        }
    }

    #[derive(Default)]
    pub struct CatalogSearchPredictorFactory;
    impl SearchPredictorFactory for CatalogSearchPredictorFactory {
        fn load(
            &self,
            repository: &TrainingRepository,
            artifact_id: &str,
            compute: &ComputeConfig,
        ) -> runtime::Result<Box<dyn SearchPredictor>> {
            let artifact = repository.get_artifact(artifact_id)?;
            if artifact.manifest["engine"] != "legacy_classical" {
                return NativeSearchPredictorFactory.load(repository, artifact_id, compute);
            }
            Ok(Box::new(LegacyPredictor::load(
                repository,
                artifact_id,
                compute,
            )?))
        }
    }

    impl LegacyPredictor {
        pub fn load(
            repository: &TrainingRepository,
            artifact_id: &str,
            compute: &ComputeConfig,
        ) -> runtime::Result<Self> {
            let artifact = repository.get_artifact(artifact_id)?;
            if artifact.manifest["engine"] != "legacy_classical" {
                return Err(error("Expected a classical model artifact"));
            }
            validate_compute(compute)?;
            if artifact.manifest["format_version"] != 1
                || artifact.manifest["serialization"] != "mlmodel_json"
            {
                return Err(error("Unsupported classical artifact format"));
            }
            let labels: Vec<String> = json::from_value(artifact.manifest["labels"].clone())?;
            let algorithm: LegacyAlgorithm =
                json::from_value(artifact.manifest["algorithm"].clone())?;
            LegacyRecipe {
                algorithm,
                labels: labels.clone(),
            }
            .validate()?;
            let shape: Vec<usize> = json::from_value(artifact.manifest["input_shape"].clone())?;
            let [width] = shape.as_slice() else {
                return Err(error("Classical artifact input shape is invalid"));
            };
            if *width == 0 || *width > 4096 {
                return Err(error(
                    "Classical artifact feature width is outside capacity",
                ));
            }
            let bytes = repository.read_blob_limited(
                &artifact.blob,
                (compute.memory_limit_bytes / 4).min(256 * 1024 * 1024),
            )?;
            let model = decode_model(&bytes)?;
            let reserved = artifact
                .blob
                .bytes
                .checked_mul(4)
                .ok_or_else(|| error("Classical artifact memory estimate overflows"))?;
            let maximum_elements = compute
                .memory_limit_bytes
                .checked_sub(reserved)
                .ok_or_else(|| error("Classical artifact exceeds the inference memory budget"))?
                / 32;
            if maximum_elements < (*width + labels.len()) as u64 {
                return Err(error(
                    "Inference memory budget has no room for one prediction",
                ));
            }
            let supported = matches!(
                model,
                MLModel::GaussianNaiveBayes(_)
                    | MLModel::DecisionTree(_)
                    | MLModel::LogisticRegression(_)
                    | MLModel::MultinomialLogisticRegression(_)
                    | MLModel::OrdinalLogistic(_)
                    | MLModel::OrdinalRidge(_)
            );
            if !supported
                || artifact.manifest["model_kind"] != model.kind()
                || model
                    .expected_features()
                    .is_some_and(|found| found != *width)
                || model.classes().is_none_or(|found| {
                    found.len() != labels.len()
                        || labels
                            .iter()
                            .enumerate()
                            .any(|(i, label)| found.get(&i) != Some(label))
                })
            {
                return Err(error("Classical artifact model and manifest disagree"));
            }
            Ok(LegacyPredictor {
                model,
                width: *width,
                labels,
                maximum_elements: maximum_elements.min(32 * 1024 * 1024) as usize,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn classical_and_ordinal_models_learn_and_preserve_predictions_after_serialization() {
            let algorithms = [
                LegacyAlgorithm::GaussianNaiveBayes,
                LegacyAlgorithm::DecisionTree {
                    max_depth: 4,
                    min_leaf: 1,
                },
                LegacyAlgorithm::Logistic {
                    alpha: 0.01,
                    max_iterations: 200,
                },
                LegacyAlgorithm::OrdinalRidge { alpha: 0.01 },
                LegacyAlgorithm::OrdinalLogistic {
                    alpha: 0.01,
                    max_iterations: 1000,
                },
            ];
            for algorithm in algorithms {
                let labels = vec!["low".into(), "high".into()];
                let inputs: Vec<f64> = (0..40).map(|i| (i as f64 - 19.5) / 10.).collect();
                let targets = (0..40).map(|i| usize::from(i >= 20)).collect();
                let model = fit_model(
                    &LegacyRecipe {
                        algorithm,
                        labels: labels.clone(),
                    },
                    inputs,
                    targets,
                    1,
                )
                .unwrap();
                let restored = decode_model(&model.to_json_vec().unwrap()).unwrap();
                let mut original = LegacyPredictor {
                    model,
                    width: 1,
                    labels: labels.clone(),
                    maximum_elements: 100,
                };
                let mut loaded = LegacyPredictor {
                    model: restored,
                    width: 1,
                    labels,
                    maximum_elements: 100,
                };
                let input = TensorData {
                    shape: vec![2, 1],
                    values: vec![-1.5, 1.5],
                };
                let expected = original.predict(&input).unwrap();
                assert_eq!(expected["classes"], json!([0, 1]));
                assert_eq!(expected, loaded.predict(&input).unwrap());
                assert!(
                    loaded
                        .predict(&TensorData {
                            shape: vec![1, 2],
                            values: vec![0., 1.]
                        })
                        .is_err()
                );
            }
        }
        #[test]
        fn multinomial_probabilities_follow_persisted_label_order() {
            let labels = vec!["lowest".into(), "middle".into(), "highest".into()];
            let mut input = Vec::new();
            let mut targets = Vec::new();
            for class in 0..3 {
                for i in 0..15 {
                    input.extend([
                        (class as f64 - 1.) * 3. + i as f64 / 100.,
                        if class == 1 { 2. } else { 0. },
                    ]);
                    targets.push(class);
                }
            }
            let model = fit_model(
                &LegacyRecipe {
                    algorithm: LegacyAlgorithm::Logistic {
                        alpha: 0.01,
                        max_iterations: 200,
                    },
                    labels: labels.clone(),
                },
                input,
                targets,
                2,
            )
            .unwrap();
            let mut predictor = LegacyPredictor {
                model,
                width: 2,
                labels,
                maximum_elements: 100,
            };
            let result = predictor
                .predict(&TensorData {
                    shape: vec![3, 2],
                    values: vec![-3., 0., 0., 2., 3., 0.],
                })
                .unwrap();
            assert_eq!(result["classes"], json!([0, 1, 2]));
            let values: Vec<f64> = json::from_value(result["output"]["values"].clone()).unwrap();
            for (class, row) in values.chunks_exact(3).enumerate() {
                assert!(row[class] > 0.9);
                assert!((row.iter().sum::<f64>() - 1.).abs() < 1e-6);
            }
        }

        #[test]
        fn durable_worker_publishes_reloadable_classical_weights() {
            use flow_like_ml_core::LabelProvenance;
            use runtime::{
                LabelSource, SplitPolicy, StreamKey, TrainingRequest, TrainingSample, WorkerLimits,
            };
            struct Directory(std::path::PathBuf);
            impl Drop for Directory {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
            let directory = Directory(
                std::env::temp_dir().join(format!("flow-like-legacy-{}", uuid::Uuid::new_v4())),
            );
            std::fs::create_dir_all(&directory.0).unwrap();
            let path = directory.0.join("training.sqlite");
            let repository = TrainingRepository::open(&path).unwrap();
            let stream = StreamKey {
                project_id: "project".into(),
                stream_id: "classical".into(),
                inspection_version: "1".into(),
            };
            for i in 0..40 {
                let class_id = u32::from(i >= 20);
                let sample = Sample {
                    id: format!("row-{i}"),
                    group_id: format!("group-{i}"),
                    stream_id: stream.stream_id.clone(),
                    timestamp_ms: i,
                    window_start_ms: i,
                    window_end_ms: i,
                    input: TensorData {
                        shape: vec![1],
                        values: vec![(i as f32 - 19.5) / 10.],
                    },
                    annotation: Annotation::Class { class_id },
                    provenance: LabelProvenance::Reviewed {
                        reviewer: "tester".into(),
                        reviewed_at_ms: i,
                    },
                    outcome: None,
                };
                repository.record_sample(&stream,&TrainingSample {id:sample.id.clone(),annotation_revision:1,group_id:sample.group_id.clone(),captured_at_ms:i,label_available_at_ms:i,content_digest:flow_like_ml_core::content_digest(&json::to_vec(&sample).unwrap()),source:LabelSource::Reviewed,accepted:true,payload:json!({"sample":sample,"label":if class_id==0 {"low"} else {"high"}})}).unwrap();
            }
            let job = repository
                .trigger_after_n(
                    &stream,
                    40,
                    TrainingRequest {
                        engine: "legacy_classical".into(),
                        recipe: json::to_value(LegacyRecipe {
                            algorithm: LegacyAlgorithm::Logistic {
                                alpha: 0.01,
                                max_iterations: 200,
                            },
                            labels: vec!["low".into(), "high".into()],
                        })
                        .unwrap(),
                        compute: json!({}),
                    },
                    SplitPolicy::Group {
                        train_fraction: 0.6,
                        validation_fraction: 0.2,
                        seed: 7,
                    },
                    1000,
                )
                .unwrap()
                .unwrap();
            let mut worker =
                TrainingWorker::new(repository.clone(), WorkerLimits::default()).unwrap();
            register_legacy_engine(&mut worker).unwrap();
            let artifact = worker.run(&job.id).unwrap();
            assert_eq!(artifact.manifest["backend"], json!({"backend":"cpu"}));
            drop(worker);
            drop(repository);
            let reopened = TrainingRepository::open(&path).unwrap();
            let mut predictor = CatalogSearchPredictorFactory
                .load(&reopened, &artifact.id, &ComputeConfig::default())
                .unwrap();
            let cached = crate::inspection::lifecycle::CatalogLoadedPredictor::load(
                &reopened,
                &artifact.id,
                &ComputeConfig::default(),
            )
            .unwrap();
            fn assert_send_sync<T: Send + Sync>() {}
            assert_send_sync::<crate::inspection::lifecycle::CatalogLoadedPredictor>();
            let batch = TensorData {
                shape: vec![2, 1],
                values: vec![-1.5, 1.5],
            };
            assert_eq!(
                cached.predict(&batch).unwrap(),
                predictor.predict(&batch).unwrap()
            );
            assert!(cached.features(&batch).is_err());
            assert_eq!(
                predictor
                    .predict(&TensorData {
                        shape: vec![2, 1],
                        values: vec![-1.5, 1.5]
                    })
                    .unwrap()["classes"],
                json!([0, 1])
            );
            let strict = ComputeConfig {
                backend: ComputeBackend::Wgpu,
                allow_cpu_fallback: false,
                ..ComputeConfig::default()
            };
            assert!(
                CatalogSearchPredictorFactory
                    .load(&reopened, &artifact.id, &strict)
                    .is_err()
            );
        }

        #[test]
        fn training_rows_after_twenty_thousand_are_used() {
            let recipe = LegacyRecipe {
                algorithm: LegacyAlgorithm::GaussianNaiveBayes,
                labels: vec!["normal".into(), "late".into()],
            };
            let inputs = (0..20_100)
                .map(|i| {
                    if i < 20_000 {
                        (i % 11) as f64 / 100.
                    } else {
                        5. + (i % 11) as f64 / 100.
                    }
                })
                .collect();
            let targets = (0..20_100).map(|i| usize::from(i >= 20_000)).collect();
            let model = fit_model(&recipe, inputs, targets, 1).unwrap();
            assert_eq!(
                model
                    .predict_on_vector(vec![5.05])
                    .unwrap()
                    .class
                    .as_deref(),
                Some("late")
            );
        }
    }
}

#[cfg(feature = "training")]
pub use execution::{
    CatalogSearchPredictorFactory, LegacyPredictor, LegacyTrainingEngine, register_legacy_engine,
};

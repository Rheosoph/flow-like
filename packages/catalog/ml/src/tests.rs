//! Tests for ML core functionality
//!
//! Tests data conversion functions, serialization, and utility helpers.

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod ml_tests {
    use crate::NodeLogic;
    use crate::ml::{
        AccuracyMetrics, ConfusionMatrixResult, GridSearchEntry, GridSearchResult, KMeansCentroids,
        LinearCoefficients, ParameterSpec, RegressionMetrics, make_new_field,
        prediction::MLPredictNode, values_to_array1_f64, values_to_array1_target,
        values_to_array1_usize, values_to_array2_f64,
    };
    use flow_like_types::Value;
    use flow_like_types::json::{self, json};
    use std::collections::HashMap;

    // ============================================================================
    // values_to_array2_f64 tests
    // ============================================================================

    #[test]
    fn test_values_to_array2_f64_basic() {
        let values = vec![
            json!({"features": [1.0, 2.0, 3.0]}),
            json!({"features": [4.0, 5.0, 6.0]}),
            json!({"features": [7.0, 8.0, 9.0]}),
        ];

        let result = values_to_array2_f64(&values, "features").unwrap();
        assert_eq!(result.shape(), &[3, 3]);
        assert_eq!(result[[0, 0]], 1.0);
        assert_eq!(result[[1, 1]], 5.0);
        assert_eq!(result[[2, 2]], 9.0);
    }

    #[test]
    fn test_values_to_array2_f64_single_row() {
        let values = vec![json!({"vec": [1.5, 2.5]})];

        let result = values_to_array2_f64(&values, "vec").unwrap();
        assert_eq!(result.shape(), &[1, 2]);
        assert_eq!(result[[0, 0]], 1.5);
        assert_eq!(result[[0, 1]], 2.5);
    }

    #[test]
    fn test_values_to_array2_f64_missing_column() {
        let values = vec![json!({"other": [1.0, 2.0]})];

        let result = values_to_array2_f64(&values, "features");
        assert!(result.is_err());
    }

    #[test]
    fn test_values_to_array2_f64_inconsistent_lengths() {
        let values = vec![
            json!({"features": [1.0, 2.0, 3.0]}),
            json!({"features": [4.0, 5.0]}), // Wrong length
        ];

        let result = values_to_array2_f64(&values, "features");
        assert!(result.is_err());
    }

    #[test]
    fn test_values_to_array2_f64_empty() {
        let values: Vec<Value> = vec![];

        let result = values_to_array2_f64(&values, "features");
        assert!(result.is_err());
    }

    // ============================================================================
    // values_to_array1_f64 tests
    // ============================================================================

    #[test]
    fn test_values_to_array1_f64_basic() {
        let values = vec![
            json!({"score": 1.5}),
            json!({"score": 2.5}),
            json!({"score": 3.5}),
        ];

        let result = values_to_array1_f64(&values, "score").unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], 1.5);
        assert_eq!(result[1], 2.5);
        assert_eq!(result[2], 3.5);
    }

    #[test]
    fn test_values_to_array1_f64_integers_as_floats() {
        let values = vec![json!({"val": 1}), json!({"val": 2}), json!({"val": 3})];

        let result = values_to_array1_f64(&values, "val").unwrap();
        assert_eq!(result[0], 1.0);
        assert_eq!(result[1], 2.0);
        assert_eq!(result[2], 3.0);
    }

    #[test]
    fn test_values_to_array1_f64_missing_column() {
        let values = vec![json!({"other": 1.0})];

        let result = values_to_array1_f64(&values, "score");
        assert!(result.is_err());
    }

    // ============================================================================
    // values_to_array1_usize tests (categorical string mapping)
    // ============================================================================

    #[test]
    fn test_values_to_array1_usize_basic() {
        let values = vec![
            json!({"label": "cat"}),
            json!({"label": "dog"}),
            json!({"label": "cat"}),
            json!({"label": "bird"}),
        ];

        let (arr, mapping) = values_to_array1_usize(&values, "label").unwrap();
        assert_eq!(arr.len(), 4);

        // First occurrence of "cat" should get ID 0
        assert_eq!(arr[0], 0);
        // "dog" should get ID 1
        assert_eq!(arr[1], 1);
        // Second "cat" should reuse ID 0
        assert_eq!(arr[2], 0);
        // "bird" should get ID 2
        assert_eq!(arr[3], 2);

        // Mapping should have 3 unique classes
        assert_eq!(mapping.len(), 3);
        assert_eq!(mapping.get(&0), Some(&"cat".to_string()));
        assert_eq!(mapping.get(&1), Some(&"dog".to_string()));
        assert_eq!(mapping.get(&2), Some(&"bird".to_string()));
    }

    #[test]
    fn test_values_to_array1_usize_single_class() {
        let values = vec![
            json!({"label": "same"}),
            json!({"label": "same"}),
            json!({"label": "same"}),
        ];

        let (arr, mapping) = values_to_array1_usize(&values, "label").unwrap();
        assert_eq!(arr.len(), 3);
        assert!(arr.iter().all(|&x| x == 0));
        assert_eq!(mapping.len(), 1);
    }

    // ============================================================================
    // values_to_array1_target tests (auto-detect type)
    // ============================================================================

    #[test]
    fn test_values_to_array1_target_strings() {
        let values = vec![
            json!({"target": "A"}),
            json!({"target": "B"}),
            json!({"target": "A"}),
        ];

        let (arr, mapping) = values_to_array1_target(&values, "target").unwrap();
        assert_eq!(arr.len(), 3);
        assert!(mapping.is_some());

        let mapping = mapping.unwrap();
        assert_eq!(mapping.len(), 2);
    }

    #[test]
    fn test_values_to_array1_target_integers() {
        let values = vec![
            json!({"class": 0}),
            json!({"class": 1}),
            json!({"class": 2}),
            json!({"class": 1}),
        ];

        let (arr, mapping) = values_to_array1_target(&values, "class").unwrap();
        assert_eq!(arr.len(), 4);
        assert_eq!(arr[0], 0);
        assert_eq!(arr[1], 1);
        assert_eq!(arr[2], 2);
        assert_eq!(arr[3], 1);
        // No mapping for integer targets
        assert!(mapping.is_none());
    }

    #[test]
    fn test_values_to_array1_target_negative_integers_error() {
        let values = vec![json!({"class": -1}), json!({"class": 0})];

        let result = values_to_array1_target(&values, "class");
        assert!(result.is_err());
    }

    #[test]
    fn test_values_to_array1_target_floats_error() {
        let values = vec![json!({"target": 1.5}), json!({"target": 2.5})];

        let result = values_to_array1_target(&values, "target");
        assert!(result.is_err());
    }

    #[test]
    fn test_values_to_array1_target_empty_error() {
        let values: Vec<Value> = vec![];

        let result = values_to_array1_target(&values, "target");
        assert!(result.is_err());
    }

    // ============================================================================
    // make_new_field tests
    // ============================================================================

    #[test]
    fn test_make_new_field_float() {
        let value = json!({"prediction": 3.14});
        let field = make_new_field(&value, "prediction").unwrap();
        assert_eq!(field.name(), "prediction");
    }

    #[test]
    fn test_make_new_field_integer() {
        let value = json!({"count": 42});
        let field = make_new_field(&value, "count").unwrap();
        assert_eq!(field.name(), "count");
    }

    #[test]
    fn test_make_new_field_string() {
        let value = json!({"label": "hello"});
        let field = make_new_field(&value, "label").unwrap();
        assert_eq!(field.name(), "label");
    }

    #[test]
    fn test_make_new_field_missing() {
        let value = json!({"other": 1.0});
        let result = make_new_field(&value, "prediction");
        assert!(result.is_err());
    }

    // ============================================================================
    // Schema struct serialization tests
    // ============================================================================

    #[test]
    fn test_kmeans_centroids_serde() {
        let centroids = KMeansCentroids {
            k: 3,
            dimensions: 2,
            centroids: vec![vec![1.0, 2.0], vec![3.0, 4.0], vec![5.0, 6.0]],
        };

        let json = json::to_string(&centroids).unwrap();
        let parsed: KMeansCentroids = json::from_str(&json).unwrap();

        assert_eq!(parsed.k, 3);
        assert_eq!(parsed.dimensions, 2);
        assert_eq!(parsed.centroids.len(), 3);
    }

    #[test]
    fn test_linear_coefficients_serde() {
        let coeffs = LinearCoefficients {
            coefficients: vec![0.5, -0.3, 1.2],
            intercept: 0.1,
            n_features: 3,
        };

        let json = json::to_string(&coeffs).unwrap();
        let parsed: LinearCoefficients = json::from_str(&json).unwrap();

        assert_eq!(parsed.coefficients, vec![0.5, -0.3, 1.2]);
        assert_eq!(parsed.intercept, 0.1);
        assert_eq!(parsed.n_features, 3);
    }

    #[test]
    fn test_accuracy_metrics_serde() {
        let metrics = AccuracyMetrics {
            accuracy: 0.95,
            correct_count: 95,
            total_count: 100,
        };

        let json = json::to_string(&metrics).unwrap();
        let parsed: AccuracyMetrics = json::from_str(&json).unwrap();

        assert_eq!(parsed.accuracy, 0.95);
        assert_eq!(parsed.correct_count, 95);
        assert_eq!(parsed.total_count, 100);
    }

    #[test]
    fn test_regression_metrics_serde() {
        let metrics = RegressionMetrics {
            mse: 0.01,
            rmse: 0.1,
            mae: 0.08,
            r2: 0.99,
            n_samples: 1000,
        };

        let json = json::to_string(&metrics).unwrap();
        let parsed: RegressionMetrics = json::from_str(&json).unwrap();

        assert!((parsed.mse - 0.01).abs() < 1e-10);
        assert!((parsed.r2 - 0.99).abs() < 1e-10);
    }

    #[test]
    fn test_confusion_matrix_result_serde() {
        let cm = ConfusionMatrixResult {
            matrix: vec![vec![50, 5], vec![3, 42]],
            labels: vec!["positive".to_string(), "negative".to_string()],
            precision: 0.91,
            recall: 0.93,
            f1_score: 0.92,
            total_samples: 100,
        };

        let json = json::to_string(&cm).unwrap();
        let parsed: ConfusionMatrixResult = json::from_str(&json).unwrap();

        assert_eq!(parsed.matrix.len(), 2);
        assert_eq!(parsed.labels.len(), 2);
    }

    #[test]
    fn test_grid_search_result_serde() {
        let mut params = HashMap::new();
        params.insert("max_depth".to_string(), json!(10));

        let entry = GridSearchEntry {
            params: params.clone(),
            mean_score: 0.85,
            std_score: 0.02,
            fold_scores: vec![0.84, 0.86, 0.85],
            train_time_secs: 1.5,
        };

        let result = GridSearchResult {
            results: vec![entry],
            best_index: 0,
            best_params: params,
            best_score: 0.85,
            total_time_secs: 5.0,
            n_combinations: 1,
            n_folds: 3,
        };

        let json = json::to_string(&result).unwrap();
        let parsed: GridSearchResult = json::from_str(&json).unwrap();

        assert_eq!(parsed.best_score, 0.85);
        assert_eq!(parsed.n_folds, 3);
    }

    #[test]
    fn test_parameter_spec_serde() {
        let spec = ParameterSpec {
            name: "max_depth".to_string(),
            values: vec![json!(5), json!(10), json!(15)],
        };

        let json = json::to_string(&spec).unwrap();
        let parsed: ParameterSpec = json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "max_depth");
        assert_eq!(parsed.values.len(), 3);
    }

    #[test]
    fn test_ml_predict_node_is_versioned_for_schema_sync() {
        let node = MLPredictNode::new().get_node();

        assert_eq!(node.version, Some(1));
    }
}

// ============================================================================
// Tests that require the execute feature (ML model operations)
// ============================================================================

#[cfg(all(test, feature = "execute"))]
mod execute_tests {
    use crate::ml::{MLModel, ModelWithMeta, ParameterSpec};
    use flow_like_types::json::{self, json};
    use linfa::prelude::*;
    use linfa_clustering::KMeans;
    use linfa_svm::Svm;
    use ndarray::{Array1, Array2};
    use std::collections::HashMap;

    // ============================================================================
    // MLModel serialization round-trip tests
    // ============================================================================

    #[test]
    fn test_kmeans_json_roundtrip() {
        // Create a simple dataset
        let data = Array2::from_shape_vec(
            (6, 2),
            vec![1.0, 1.0, 1.1, 1.1, 5.0, 5.0, 5.1, 5.1, 9.0, 9.0, 9.1, 9.1],
        )
        .unwrap();

        let dataset = linfa::DatasetBase::from(data);
        let model = KMeans::params(3)
            .fit(&dataset)
            .expect("KMeans fitting failed");

        let ml_model = MLModel::KMeans(ModelWithMeta {
            model,
            classes: None,
        });

        // JSON round-trip
        let json_bytes = ml_model.to_json_vec().unwrap();
        assert!(!json_bytes.is_empty());

        // Verify it's valid JSON
        let parsed: flow_like_types::Value = json::from_slice(&json_bytes).unwrap();
        assert!(parsed.get("type").is_some());
    }

    #[test]
    fn test_kmeans_fory_roundtrip() {
        let data = Array2::from_shape_vec(
            (6, 2),
            vec![1.0, 1.0, 1.1, 1.1, 5.0, 5.0, 5.1, 5.1, 9.0, 9.0, 9.1, 9.1],
        )
        .unwrap();

        let dataset = linfa::DatasetBase::from(data);
        let model = KMeans::params(3)
            .fit(&dataset)
            .expect("KMeans fitting failed");

        let ml_model = MLModel::KMeans(ModelWithMeta {
            model,
            classes: None,
        });

        // Fory round-trip
        let fory_bytes = ml_model.to_fory_vec().unwrap();
        assert!(!fory_bytes.is_empty());

        let restored = MLModel::from_fory_slice(&fory_bytes).unwrap();

        // Verify type preserved
        match restored {
            MLModel::KMeans(_) => {}
            _ => panic!("Expected KMeans model"),
        }
    }

    /// Every persisted model must survive BOTH serialization paths. The binary path is the one at
    /// risk: `MLModel` is an internally tagged enum and the payload goes through MessagePack, which
    /// encodes structs as arrays by default — a combination that breaks tag lookup on the way back.
    /// Nested tagged enums (Frank & Hall carries its own `#[serde(tag = "base")]`) are the case most
    /// likely to fail, so it is pinned here explicitly rather than assumed.
    #[test]
    fn test_frank_hall_roundtrips_through_both_formats() {
        use flow_like_ordinal::FrankHallParams;
        use linfa::error::Error as LinfaError;
        use linfa_trees::DecisionTree as LinfaDecisionTree;

        // Four rows, three ordered levels, cleanly separable on one feature.
        let records = Array2::from_shape_vec((6, 1), vec![0.0, 0.5, 2.0, 2.5, 4.0, 4.5]).unwrap();
        let targets = ndarray::Array1::from(vec![0usize, 0, 1, 1, 2, 2]);
        let dataset = linfa::DatasetBase::new(records, targets);

        let fitted =
            FrankHallParams::<_, LinfaError>::new(LinfaDecisionTree::<f64, bool>::params())
                .n_levels(3)
                .fit(&dataset)
                .expect("Frank & Hall fit failed");

        let ml_model = MLModel::OrdinalFrankHall(ModelWithMeta {
            model: crate::ml::FrankHallModel::DecisionTree(fitted),
            classes: None,
        });

        // Predictions before, to compare against. Matching the variant alone would pass even if
        // every fitted tree came back empty.
        let probe = Array2::from_shape_vec((3, 1), vec![0.25, 2.25, 4.25]).unwrap();
        let expected = match &ml_model {
            MLModel::OrdinalFrankHall(model) => model.model.predict_levels(&probe),
            _ => unreachable!(),
        };
        assert_eq!(
            expected.to_vec(),
            vec![0, 1, 2],
            "fit did not separate the levels"
        );

        for (format, restored) in [
            (
                "JSON",
                json::from_slice::<MLModel>(&ml_model.to_json_vec().expect("JSON serialize"))
                    .expect("JSON round-trip"),
            ),
            (
                "binary",
                MLModel::from_fory_slice(&ml_model.to_fory_vec().expect("binary serialize"))
                    .expect("binary round-trip"),
            ),
        ] {
            match restored {
                MLModel::OrdinalFrankHall(model) => {
                    assert_eq!(model.model.n_classes(), 3, "{format}: level count lost");
                    assert_eq!(
                        model.model.predict_levels(&probe).to_vec(),
                        expected.to_vec(),
                        "{format}: predictions changed across the round-trip"
                    );
                }
                other => panic!("{format}: expected OrdinalFrankHall, got {other}"),
            }
        }
    }

    #[test]
    fn test_svm_fory_roundtrip() {
        let data = Array2::from_shape_vec(
            (9, 2),
            vec![
                0.0, 0.0, 0.1, 0.0, 0.0, 0.1, 5.0, 5.0, 5.1, 5.0, 5.0, 5.1, 10.0, 0.0, 10.1, 0.0,
                10.0, 0.1,
            ],
        )
        .unwrap();
        let targets = Array1::from(vec![0usize, 0, 0, 1, 1, 1, 2, 2, 2]);
        let dataset = linfa::DatasetBase::from(data).with_targets(targets);
        let params = Svm::<_, Pr>::params().gaussian_kernel(30.0);
        let svm_models = dataset
            .one_vs_all()
            .unwrap()
            .into_iter()
            .map(|(label, dataset)| (label, params.fit(&dataset).unwrap()))
            .collect::<Vec<_>>();

        let mut classes = HashMap::new();
        classes.insert(0, "a".to_string());
        classes.insert(1, "b".to_string());
        classes.insert(2, "c".to_string());

        let ml_model = MLModel::SVMMultiClass(ModelWithMeta {
            model: svm_models,
            classes: Some(classes),
        });

        let fory_bytes = ml_model.to_fory_vec().unwrap();
        assert!(!fory_bytes.is_empty());

        let restored = MLModel::from_fory_slice(&fory_bytes).unwrap();
        match restored {
            MLModel::SVMMultiClass(_) => {}
            other => panic!("Expected SVMMultiClass model, got {other}"),
        }
    }

    #[test]
    fn test_from_fory_slice_rejects_json_instead_of_defaulting() {
        // Fory treats bit 0 of the leading byte as a null marker, and `{` (0x7B) has it set,
        // so JSON used to decode into an all-default wrapper reported as "version: 0".
        for input in [
            &br#"{"model_ref":"h12a5fbkujvy7w2otit56hlg"}"#[..],
            &br#"{"type":"SVMMultiClass","model":[[0,{"alpha":[1.0]}]]}"#[..],
        ] {
            let err = MLModel::from_fory_slice(input).unwrap_err().to_string();
            assert!(
                err.contains("Not a valid .flmodel") && err.contains("JSON"),
                "expected an actionable JSON error, got: {err}"
            );
        }

        assert!(MLModel::from_fory_slice(&[]).is_err());
    }

    #[test]
    fn test_fory_is_smaller_than_json() {
        let data = Array2::from_shape_vec((100, 10), (0..1000).map(|i| i as f64 * 0.01).collect())
            .unwrap();

        let dataset = linfa::DatasetBase::from(data);
        let model = KMeans::params(5).fit(&dataset).unwrap();

        let ml_model = MLModel::KMeans(ModelWithMeta {
            model,
            classes: None,
        });

        let json_bytes = ml_model.to_json_vec().unwrap();
        let fory_bytes = ml_model.to_fory_vec().unwrap();

        // Fory should be smaller (or at least not significantly larger)
        // JSON has text overhead, Fory uses binary
        assert!(
            fory_bytes.len() <= json_bytes.len(),
            "Fory ({} bytes) should be <= JSON ({} bytes)",
            fory_bytes.len(),
            json_bytes.len()
        );
    }

    #[test]
    fn test_model_display() {
        let data =
            Array2::from_shape_vec((4, 2), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]).unwrap();
        let dataset = linfa::DatasetBase::from(data);
        let model = KMeans::params(2).fit(&dataset).unwrap();

        let ml_model = MLModel::KMeans(ModelWithMeta {
            model,
            classes: None,
        });

        let display = format!("{}", ml_model);
        assert!(display.contains("KMeans"));
    }

    // ============================================================================
    // predict_on_values tests (public API)
    // ============================================================================

    #[test]
    fn test_kmeans_predict_on_values() {
        let data = Array2::from_shape_vec((4, 2), vec![0.0, 0.0, 0.1, 0.1, 10.0, 10.0, 10.1, 10.1])
            .unwrap();

        let dataset = linfa::DatasetBase::from(data);
        let model = KMeans::params(2).fit(&dataset).unwrap();

        let ml_model = MLModel::KMeans(ModelWithMeta {
            model,
            classes: None,
        });

        let mut values = vec![
            json!({"features": [0.05, 0.05]}),
            json!({"features": [10.05, 10.05]}),
        ];

        ml_model
            .predict_on_values(&mut values, "features", "cluster")
            .unwrap();

        // Check that predictions were added
        assert!(values[0].get("cluster").is_some());
        assert!(values[1].get("cluster").is_some());

        // Predictions should be different (different clusters)
        let cluster0 = values[0].get("cluster").unwrap().as_u64().unwrap();
        let cluster1 = values[1].get("cluster").unwrap().as_u64().unwrap();
        assert_ne!(cluster0, cluster1);
    }

    #[test]
    fn test_kmeans_predict_on_values_multiple_rows() {
        let data = Array2::from_shape_vec(
            (6, 2),
            vec![
                0.0, 0.0, 0.1, 0.1, 0.2, 0.2, 10.0, 10.0, 10.1, 10.1, 10.2, 10.2,
            ],
        )
        .unwrap();

        let dataset = linfa::DatasetBase::from(data);
        let model = KMeans::params(2).fit(&dataset).unwrap();

        let ml_model = MLModel::KMeans(ModelWithMeta {
            model,
            classes: None,
        });

        let mut values = vec![
            json!({"features": [0.0, 0.0]}),
            json!({"features": [0.1, 0.1]}),
            json!({"features": [10.0, 10.0]}),
            json!({"features": [10.1, 10.1]}),
        ];

        ml_model
            .predict_on_values(&mut values, "features", "cluster")
            .unwrap();

        // First two should be in same cluster
        let c0 = values[0].get("cluster").unwrap().as_u64().unwrap();
        let c1 = values[1].get("cluster").unwrap().as_u64().unwrap();
        assert_eq!(c0, c1);

        // Last two should be in same cluster
        let c2 = values[2].get("cluster").unwrap().as_u64().unwrap();
        let c3 = values[3].get("cluster").unwrap().as_u64().unwrap();
        assert_eq!(c2, c3);

        // But different from first cluster
        assert_ne!(c0, c2);
    }

    #[test]
    fn test_svm_predict_on_vector_confidence_uses_winning_probability() {
        let data = Array2::from_shape_vec(
            (9, 2),
            vec![
                0.0, 0.0, 0.1, 0.0, 0.0, 0.1, 5.0, 5.0, 5.1, 5.0, 5.0, 5.1, 10.0, 0.0, 10.1, 0.0,
                10.0, 0.1,
            ],
        )
        .unwrap();
        let targets = Array1::from(vec![0usize, 0, 0, 1, 1, 1, 2, 2, 2]);
        let dataset = linfa::DatasetBase::from(data).with_targets(targets);
        let params = Svm::<_, Pr>::params().gaussian_kernel(30.0);
        let svm_models = dataset
            .one_vs_all()
            .unwrap()
            .into_iter()
            .map(|(label, dataset)| (label, params.fit(&dataset).unwrap()))
            .collect::<Vec<_>>();

        let mut classes = HashMap::new();
        classes.insert(0, "a".to_string());
        classes.insert(1, "b".to_string());
        classes.insert(2, "c".to_string());

        let ml_model = MLModel::SVMMultiClass(ModelWithMeta {
            model: svm_models,
            classes: Some(classes),
        });
        let input = vec![0.05, 0.05];
        let prediction = ml_model.predict_on_vector(input.clone()).unwrap();

        let expected_confidence = match &ml_model {
            MLModel::SVMMultiClass(model) => {
                let sample = Array1::from(input);
                model
                    .model
                    .iter()
                    .map(|(_, classifier)| f64::from(*classifier.predict(sample.view())))
                    .fold(f64::NEG_INFINITY, f64::max)
            }
            _ => unreachable!(),
        };

        let confidence = prediction.confidence.unwrap();
        assert!(
            (confidence - expected_confidence).abs() < 1e-6,
            "confidence {confidence} should match winning SVM probability {expected_confidence}"
        );
        assert!(
            (confidence - (1.0 / 3.0)).abs() > 1e-6,
            "confidence should not be the old class-count placeholder"
        );
    }

    // ============================================================================
    // Grid search parameter combination tests
    // ============================================================================

    #[test]
    fn test_generate_param_combinations_empty() {
        let grid: Vec<ParameterSpec> = vec![];
        let combos = generate_param_combinations(&grid);

        assert_eq!(combos.len(), 1);
        assert!(combos[0].is_empty());
    }

    #[test]
    fn test_generate_param_combinations_single_param() {
        let grid = vec![ParameterSpec {
            name: "max_depth".to_string(),
            values: vec![json!(5), json!(10), json!(15)],
        }];

        let combos = generate_param_combinations(&grid);

        assert_eq!(combos.len(), 3);
        assert_eq!(combos[0].get("max_depth"), Some(&json!(5)));
        assert_eq!(combos[1].get("max_depth"), Some(&json!(10)));
        assert_eq!(combos[2].get("max_depth"), Some(&json!(15)));
    }

    #[test]
    fn test_generate_param_combinations_multiple_params() {
        let grid = vec![
            ParameterSpec {
                name: "max_depth".to_string(),
                values: vec![json!(5), json!(10)],
            },
            ParameterSpec {
                name: "min_samples".to_string(),
                values: vec![json!(1), json!(2), json!(3)],
            },
        ];

        let combos = generate_param_combinations(&grid);

        // 2 * 3 = 6 combinations
        assert_eq!(combos.len(), 6);

        // Check that all combinations are present
        let has_5_1 = combos.iter().any(|c| {
            c.get("max_depth") == Some(&json!(5)) && c.get("min_samples") == Some(&json!(1))
        });
        let has_10_3 = combos.iter().any(|c| {
            c.get("max_depth") == Some(&json!(10)) && c.get("min_samples") == Some(&json!(3))
        });

        assert!(has_5_1);
        assert!(has_10_3);
    }

    #[test]
    fn test_generate_param_combinations_three_params() {
        let grid = vec![
            ParameterSpec {
                name: "a".to_string(),
                values: vec![json!(1), json!(2)],
            },
            ParameterSpec {
                name: "b".to_string(),
                values: vec![json!(3), json!(4)],
            },
            ParameterSpec {
                name: "c".to_string(),
                values: vec![json!(5), json!(6)],
            },
        ];

        let combos = generate_param_combinations(&grid);

        // 2 * 2 * 2 = 8 combinations
        assert_eq!(combos.len(), 8);
    }

    // ============================================================================
    // Accuracy computation tests
    // ============================================================================

    #[test]
    fn test_compute_accuracy_perfect() {
        let predictions = ndarray::Array1::from(vec![0, 1, 2, 0, 1]);
        let targets = vec![0, 1, 2, 0, 1];

        let acc = compute_accuracy(&predictions, &targets);
        assert!((acc - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_compute_accuracy_partial() {
        let predictions = ndarray::Array1::from(vec![0, 1, 2, 0, 1]);
        let targets = vec![0, 1, 0, 0, 0]; // 3 out of 5 correct

        let acc = compute_accuracy(&predictions, &targets);
        assert!((acc - 0.6).abs() < 1e-10);
    }

    #[test]
    fn test_compute_accuracy_zero() {
        let predictions = ndarray::Array1::from(vec![0, 0, 0]);
        let targets = vec![1, 1, 1]; // All wrong

        let acc = compute_accuracy(&predictions, &targets);
        assert_eq!(acc, 0.0);
    }

    #[test]
    fn test_compute_accuracy_empty() {
        let predictions = ndarray::Array1::from(vec![]);
        let targets: Vec<usize> = vec![];

        let acc = compute_accuracy(&predictions, &targets);
        assert_eq!(acc, 0.0);
    }

    #[test]
    fn test_compute_accuracy_mismatched_lengths() {
        let predictions = ndarray::Array1::from(vec![0, 1, 2]);
        let targets = vec![0, 1]; // Different length

        let acc = compute_accuracy(&predictions, &targets);
        assert_eq!(acc, 0.0);
    }

    // ============================================================================
    // Helper functions duplicated here for testing
    // ============================================================================

    fn generate_param_combinations(
        grid: &[ParameterSpec],
    ) -> Vec<HashMap<String, flow_like_types::Value>> {
        if grid.is_empty() {
            return vec![HashMap::new()];
        }

        let mut result = vec![HashMap::new()];

        for spec in grid {
            let mut new_result = Vec::with_capacity(result.len() * spec.values.len());
            for existing in &result {
                for value in &spec.values {
                    let mut combo = existing.clone();
                    combo.insert(spec.name.clone(), value.clone());
                    new_result.push(combo);
                }
            }
            result = new_result;
        }

        result
    }

    fn compute_accuracy(predictions: &ndarray::Array1<usize>, targets: &[usize]) -> f64 {
        if predictions.len() != targets.len() || predictions.is_empty() {
            return 0.0;
        }
        let correct = predictions
            .iter()
            .zip(targets.iter())
            .filter(|(p, t)| p == t)
            .count();
        correct as f64 / predictions.len() as f64
    }
}

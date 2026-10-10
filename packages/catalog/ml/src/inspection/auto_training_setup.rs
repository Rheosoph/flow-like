#[cfg(any(feature = "training", test))]
use super::{
    auto_training::AutoTrainRequest,
    auto_training_tables::{
        FeatureSource, PinnedTableSource, TableDatasetRequest, TableMapping, TableSplitPolicy,
        TargetSource,
    },
};
use flow_like_catalog_core::NodeDBConnection;
#[cfg(any(feature = "training", test))]
use flow_like_ml_core::{InspectionSpec, LabelProvenance, TaskKind};
#[cfg(any(feature = "training", test))]
use flow_like_ml_native::preprocessing::{Imputation, TabularOptions};
#[cfg(any(feature = "training", test))]
use flow_like_types::{Result, Value, anyhow};
#[cfg(any(feature = "training", test))]
use std::collections::{BTreeSet, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabularAutoTrainTask {
    Classification,
    Regression,
}

/// Direct setup expects measured target values already present in the source table.
#[derive(Clone, Debug)]
pub struct TabularAutoTrainSetup {
    pub database: NodeDBConnection,
    pub task: TabularAutoTrainTask,
    pub target_column: String,
    pub row_id_column: String,
    pub numeric_columns: Vec<String>,
    pub categorical_columns: Vec<String>,
    pub group_column: Option<String>,
}

#[cfg(any(feature = "training", test))]
fn validate_setup(setup: &TabularAutoTrainSetup) -> Result<()> {
    for (label, column) in [
        ("Target Column", Some(&setup.target_column)),
        ("Row ID Column", Some(&setup.row_id_column)),
        ("Group Column", setup.group_column.as_ref()),
    ] {
        if column.is_some_and(|name| name.trim().is_empty() || name.len() > 1024) {
            return Err(anyhow!("{label} must name a column of 1..1024 bytes"));
        }
    }
    if setup.target_column == setup.row_id_column
        || setup.group_column.as_ref() == Some(&setup.target_column)
    {
        return Err(anyhow!(
            "Target Column must differ from the row ID and group columns"
        ));
    }
    let mut features = HashSet::new();
    for column in setup
        .numeric_columns
        .iter()
        .chain(&setup.categorical_columns)
    {
        if column.trim().is_empty() || column.len() > 1024 {
            return Err(anyhow!("Feature columns must have names of 1..1024 bytes"));
        }
        if column == &setup.target_column
            || column == &setup.row_id_column
            || setup.group_column.as_ref() == Some(column)
        {
            return Err(anyhow!(
                "Feature column '{column}' is a target, row ID or group column; remove it from the feature lists"
            ));
        }
        if !features.insert(column) {
            return Err(anyhow!(
                "Feature column '{column}' is listed more than once; choose either Numeric Columns or Categorical Columns"
            ));
        }
    }
    if features.is_empty() || features.len() > 4096 {
        return Err(anyhow!(
            "Select between 1 and 4096 numeric or categorical feature columns"
        ));
    }
    Ok(())
}

#[cfg(any(feature = "training", test))]
fn infer_class_labels(rows: &[Value], target_column: &str) -> Result<Vec<String>> {
    let mut labels = BTreeSet::new();
    for row in rows {
        let label = match row.get(target_column) {
            Some(Value::String(value)) if !value.trim().is_empty() => value.clone(),
            Some(value @ (Value::Number(_) | Value::Bool(_))) => value.to_string(),
            _ => {
                return Err(anyhow!(
                    "Classification target '{target_column}' must contain a nonempty string, number or boolean in every row"
                ));
            }
        };
        labels.insert(label);
        if labels.len() > 256 {
            return Err(anyhow!(
                "Classification target '{target_column}' has more than 256 classes; choose Regression for a continuous target or use Advanced Configuration"
            ));
        }
    }
    if labels.len() < 2 {
        return Err(anyhow!(
            "Classification target '{target_column}' needs at least two distinct classes"
        ));
    }
    Ok(labels.into_iter().collect())
}

#[cfg(any(feature = "training", test))]
fn build_request(
    setup: TabularAutoTrainSetup,
    source: PinnedTableSource,
    labels: Vec<String>,
) -> Result<AutoTrainRequest> {
    validate_setup(&setup)?;
    let task = match setup.task {
        TabularAutoTrainTask::Classification => TaskKind::SensorClassification,
        TabularAutoTrainTask::Regression => TaskKind::SensorRegression,
    };
    let identity = format!("{}:{}", source.reference.table, setup.target_column);
    let measurement_source = format!(
        "table:{}@{}:{}#{}",
        source.reference.table,
        source.reference.branch,
        source.reference.version,
        setup.target_column
    );
    let spec = InspectionSpec {
        id: format!("auto:{identity}"),
        description: format!(
            "Predict {} from measured rows in {}",
            setup.target_column, source.reference.table
        ),
        task,
        labels,
        input_shape: vec![1],
        prediction_horizon_ms: None,
        minimum_examples: 3,
        minimum_examples_per_class: 1,
    };
    spec.validate()?;
    Ok(AutoTrainRequest {
        dataset: TableDatasetRequest {
            source: source.database,
            stream_id: format!("auto:{identity}"),
            spec,
            mapping: TableMapping {
                row_id: setup.row_id_column,
                group_id: setup.group_column,
                timestamp_ms: None,
                window_start_ms: None,
                window_end_ms: None,
                outcome: None,
                features: FeatureSource::Tabular {
                    options: TabularOptions {
                        numeric_columns: setup.numeric_columns,
                        categorical_columns: setup.categorical_columns,
                        imputation: Imputation::Median,
                        standardize: true,
                        maximum_categories_per_column: 256,
                        maximum_output_features: 4096,
                    },
                },
                target: TargetSource::Column {
                    column: setup.target_column,
                },
                provenance: LabelProvenance::Measured {
                    source: measurement_source,
                },
            },
            split: TableSplitPolicy::Group {
                train_fraction: 0.7,
                validation_fraction: 0.15,
                seed: 42,
            },
            budget: Default::default(),
            selection: Default::default(),
            label_overrides: Default::default(),
        },
        compute: Default::default(),
        search: Default::default(),
        budget: Default::default(),
        goals: None,
        tables: Default::default(),
        ordered_target: false,
        initial_candidates: Vec::new(),
        pretrained: None,
        feature_plan: None,
        feature_sources: Vec::new(),
        learning_cycle_id: None,
    })
}

#[cfg(feature = "training")]
pub async fn build_tabular_auto_train_request(
    context: &mut flow_like::flow::execution::context::ExecutionContext,
    setup: TabularAutoTrainSetup,
) -> Result<AutoTrainRequest> {
    use super::auto_training_tables::{TableBudget, pin_table_source, read_rows};

    validate_setup(&setup)?;
    let source = pin_table_source(context, &setup.database).await?;
    let labels = match setup.task {
        TabularAutoTrainTask::Classification => {
            let rows = read_rows(context, &source, &TableBudget::default(), 512).await?;
            // This establishes class identity only. Preprocessing is fitted after the split.
            infer_class_labels(&rows, &setup.target_column)?
        }
        TabularAutoTrainTask::Regression => Vec::new(),
    };
    build_request(setup, source, labels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspection::auto_training_tables::{TablePreprocessing, prepare_rows};
    use flow_like_ml_core::Annotation;
    use flow_like_ml_native::preprocessing::FittedColumn;
    use flow_like_storage_contracts::database::DatabaseReference;
    use flow_like_types::json::json;

    fn setup(task: TabularAutoTrainTask) -> TabularAutoTrainSetup {
        TabularAutoTrainSetup {
            database: NodeDBConnection {
                cache_key: "live".into(),
            },
            task,
            target_column: "target".into(),
            row_id_column: "id".into(),
            numeric_columns: vec!["temperature".into()],
            categorical_columns: vec!["machine".into()],
            group_column: Some("batch".into()),
        }
    }

    fn source() -> PinnedTableSource {
        PinnedTableSource {
            database: NodeDBConnection {
                cache_key: "pinned".into(),
            },
            reference: DatabaseReference {
                table: "measurements".into(),
                branch: "main".into(),
                version: 7,
                read_only: true,
                pinned: true,
            },
            locator: None,
        }
    }

    #[test]
    fn direct_classification_setup_prepares_grouped_rows_and_fits_training_only() {
        let rows: Vec<Value> = (0..60)
            .map(|index| {
                json!({
                    "id": index,
                    "batch": index / 2,
                    "temperature": index as f64,
                    "machine": format!("machine-{}", index / 2),
                    "target": if index % 2 == 0 { "pass" } else { "fail" }
                })
            })
            .collect();
        let labels = infer_class_labels(&rows, "target").unwrap();
        let request = build_request(
            setup(TabularAutoTrainTask::Classification),
            source(),
            labels,
        )
        .unwrap();
        assert_eq!(request.dataset.source.cache_key, "pinned");
        assert_eq!(request.dataset.spec.labels, ["fail", "pass"]);
        assert_eq!(request.dataset.mapping.row_id, "id");
        assert_eq!(request.dataset.mapping.group_id.as_deref(), Some("batch"));
        assert!(matches!(
            request.dataset.mapping.provenance,
            LabelProvenance::Measured { .. }
        ));
        let prepared = prepare_rows(source(), &request.dataset, &rows).unwrap();
        prepared.split.validate(&prepared.samples).unwrap();
        assert_eq!(prepared.split.train.len(), 42);
        assert!(!prepared.split.validation.is_empty());
        assert!(!prepared.split.test.is_empty());
        let TablePreprocessing::Tabular { fitted } = prepared.preprocessing else {
            panic!("Expected tabular preprocessing");
        };
        let training_mean = prepared
            .split
            .train
            .iter()
            .map(|index| *index as f64 / prepared.split.train.len() as f64)
            .sum::<f64>();
        assert!(matches!(
            &fitted.columns[0],
            FittedColumn::Numeric { offset, .. } if (*offset - training_mean).abs() < 1e-10
        ));
        let FittedColumn::Categorical { vocabulary, .. } = &fitted.columns[1] else {
            panic!("Expected categorical preprocessing");
        };
        for index in &prepared.split.test {
            assert!(!vocabulary.contains(&rows[*index]["machine"].to_string()));
        }
    }

    #[test]
    fn direct_regression_setup_preserves_numeric_targets_and_row_groups() {
        let mut setup = setup(TabularAutoTrainTask::Regression);
        setup.group_column = None;
        setup.categorical_columns.clear();
        let request = build_request(setup, source(), Vec::new()).unwrap();
        assert!(request.dataset.spec.labels.is_empty());
        assert_eq!(request.dataset.spec.task, TaskKind::SensorRegression);
        let rows: Vec<Value> = (0..12)
            .map(|index| json!({"id":index,"temperature":index,"target":index as f64 * 0.5}))
            .collect();
        let prepared = prepare_rows(source(), &request.dataset, &rows).unwrap();
        for (index, sample) in prepared.samples.iter().enumerate() {
            assert_eq!(sample.group_id, sample.id);
            assert!(matches!(
                sample.annotation,
                Annotation::Scalar { value } if value == index as f64 * 0.5
            ));
        }
    }

    #[test]
    fn direct_setup_rejects_protected_duplicate_and_missing_feature_columns() {
        for column in ["target", "id", "batch"] {
            let mut setup = setup(TabularAutoTrainTask::Classification);
            setup.numeric_columns.push(column.into());
            assert!(
                validate_setup(&setup)
                    .unwrap_err()
                    .to_string()
                    .contains("remove it")
            );
        }
        let mut duplicate = setup(TabularAutoTrainTask::Classification);
        duplicate.categorical_columns.push("temperature".into());
        assert!(
            validate_setup(&duplicate)
                .unwrap_err()
                .to_string()
                .contains("more than once")
        );
        let mut empty = setup(TabularAutoTrainTask::Classification);
        empty.numeric_columns.clear();
        empty.categorical_columns.clear();
        assert!(validate_setup(&empty).is_err());
        for invalid in ["", " "] {
            let mut setup = setup(TabularAutoTrainTask::Classification);
            setup.row_id_column = invalid.into();
            assert!(validate_setup(&setup).is_err());
        }
    }

    #[test]
    fn class_labels_are_sorted_scalar_names_and_reject_invalid_targets() {
        let rows = vec![
            json!({"target": "z"}),
            json!({"target": true}),
            json!({"target": 2}),
            json!({"target": "a"}),
            json!({"target": "z"}),
        ];
        assert_eq!(
            infer_class_labels(&rows, "target").unwrap(),
            ["2", "a", "true", "z"]
        );
        for invalid in [json!(null), json!([]), json!({}), json!(" ")] {
            assert!(infer_class_labels(&[json!({"target": invalid})], "target").is_err());
        }
        assert!(infer_class_labels(&[json!({})], "target").is_err());
        assert!(infer_class_labels(&[], "target").is_err());
        assert!(infer_class_labels(&[json!({"target": "only"})], "target").is_err());
        let rows: Vec<Value> = (0..257).map(|label| json!({"target":label})).collect();
        assert_eq!(
            infer_class_labels(&rows[..256], "target").unwrap().len(),
            256
        );
        assert!(
            infer_class_labels(&rows, "target")
                .unwrap_err()
                .to_string()
                .contains("more than 256")
        );
    }
}

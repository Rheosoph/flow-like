use crate::inspection::{
    auto_training_legacy::CatalogSearchPredictorFactory,
    auto_training_tables::{
        FeatureSource, TableMapping, TablePreprocessing, raw_input_digest, transform_table_rows,
    },
};
use flow_like_ml_core::{Annotation, InspectionSpec, LabelProvenance, Sample, TensorData};
use flow_like_ml_runtime::{
    LabelSource, TrainingRepository, TrainingSample,
    auto_training::{SearchPredictorFactory, prediction_record_from_output},
    learning::{LearningObservation, LearningProject, LearningReview, LearningState},
};
use flow_like_types::{Result, Value, anyhow, json::json};
use serde::de::DeserializeOwned;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

fn decode<T: DeserializeOwned>(value: &Value) -> Result<T> {
    Ok(if let Some(text) = value.as_str() {
        flow_like_types::json::from_str(text)?
    } else {
        flow_like_types::json::from_value(value.clone())?
    })
}
fn field<'a>(row: &'a Value, name: &str) -> Result<&'a Value> {
    row.get(name).ok_or_else(|| {
        anyhow!("Canary replay requires raw source column '{name}' in observation.sample")
    })
}
fn identifier(value: &Value) -> Result<String> {
    match value {
        Value::String(value) if !value.is_empty() => Ok(value.clone()),
        Value::Number(value) => Ok(value.to_string()),
        _ => Err(anyhow!(
            "Canary source identifiers must be nonempty strings or numbers"
        )),
    }
}
fn timestamp(row: &Value, column: &Option<String>, fallback: i64) -> Result<i64> {
    column.as_ref().map_or(Ok(fallback), |column| {
        field(row, column)?
            .as_i64()
            .ok_or_else(|| anyhow!("Canary timestamp '{column}' must use integer milliseconds"))
    })
}
fn original_sample(
    observation: &LearningObservation,
    mapping: &TableMapping,
    stream: &str,
) -> Result<Sample> {
    let row = &observation.sample;
    if !row.is_object() || row.as_object().is_some_and(|row| row.is_empty()) {
        return Err(anyhow!(
            "Canary sample '{}' has no raw source row; populate observation.sample before submitting it",
            observation.sample_id
        ));
    }
    if identifier(field(row, &mapping.row_id)?)? != observation.sample_id {
        return Err(anyhow!("Canary source row ID differs from its observation"));
    }
    if !matches!(mapping.features, FeatureSource::Sample { .. }) && mapping.timestamp_ms.is_none() {
        return Err(anyhow!(
            "Canary replay requires a source timestamp mapping or a serialized sample"
        ));
    }
    let mut sample = match &mapping.features {
        FeatureSource::Sample { column } => decode::<Sample>(field(row, column)?)?,
        features => Sample {
            id: identifier(field(row, &mapping.row_id)?)?,
            group_id: mapping
                .group_id
                .as_ref()
                .map(|column| identifier(field(row, column)?))
                .transpose()?
                .unwrap_or_else(|| observation.sample_id.clone()),
            stream_id: stream.into(),
            timestamp_ms: timestamp(row, &mapping.timestamp_ms, observation.captured_at_ms)?,
            window_start_ms: timestamp(row, &mapping.window_start_ms, observation.captured_at_ms)?,
            window_end_ms: timestamp(row, &mapping.window_end_ms, observation.captured_at_ms)?,
            input: match features {
                FeatureSource::Tensor { column } => decode(field(row, column)?)?,
                _ => TensorData {
                    shape: vec![1],
                    values: vec![0.],
                },
            },
            annotation: Annotation::Unlabeled,
            provenance: LabelProvenance::Unlabeled,
            outcome: None,
        },
    };
    if sample.id != observation.sample_id
        || sample.group_id != observation.group_id
        || sample.timestamp_ms != observation.captured_at_ms
    {
        return Err(anyhow!(
            "Canary raw identity, group or timestamp differs from its observation"
        ));
    }
    sample.stream_id = stream.into();
    Ok(sample)
}
fn has_windows(preprocessing: &TablePreprocessing) -> bool {
    match preprocessing {
        TablePreprocessing::Engineered {
            pipeline,
            preprocessing,
            ..
        } => {
            pipeline.pipeline.steps.iter().any(|step| {
                matches!(
                    step,
                    flow_like_ml_native::feature_engineering::FeatureStep::WindowFeatures { .. }
                )
            }) || has_windows(preprocessing)
        }
        _ => false,
    }
}
fn reviewed_sample(
    observation: &LearningObservation,
    review: &LearningReview,
    mapping: &TableMapping,
    stream: &str,
    input: TensorData,
    spec: &InspectionSpec,
    at_ms: i64,
) -> Result<TrainingSample> {
    let mut sample = original_sample(observation, mapping, stream)?;
    let digest = raw_input_digest(mapping, &observation.sample, &sample)?;
    sample.input = input;
    sample.annotation = flow_like_types::json::from_value(review.annotation.clone())?;
    sample.outcome = review
        .outcome
        .clone()
        .map(flow_like_types::json::from_value)
        .transpose()?;
    sample.provenance = match review.source {
        LabelSource::Reviewed => LabelProvenance::Reviewed {
            reviewer: review.reviewer.clone(),
            reviewed_at_ms: review.available_at_ms,
        },
        LabelSource::ObservedOutcome => LabelProvenance::Measured {
            source: review.reviewer.clone(),
        },
        LabelSource::Teacher => {
            return Err(anyhow!(
                "Canary audit requires independently reviewed or measured outcomes"
            ));
        }
    };
    let available = review.available_at_ms.max(
        sample
            .outcome
            .as_ref()
            .map_or(review.available_at_ms, |outcome| outcome.available_at_ms),
    );
    if available > at_ms || available < sample.timestamp_ms {
        return Err(anyhow!("Canary outcome is unavailable at the audit time"));
    }
    spec.validate_sample(&sample)?;
    let mut payload = json!(sample);
    payload["feature_row"] = observation.sample.clone();
    if let Annotation::Class { class_id } = &sample.annotation {
        payload["label"] = json!(
            spec.labels
                .get(*class_id as usize)
                .ok_or_else(|| anyhow!("Canary class exceeds artifact label order"))?
        );
    }
    Ok(TrainingSample {
        id: sample.id,
        annotation_revision: review
            .revision
            .checked_add(1)
            .ok_or_else(|| anyhow!("Canary annotation revision overflow"))?,
        group_id: sample.group_id,
        captured_at_ms: sample.timestamp_ms,
        label_available_at_ms: available,
        content_digest: digest,
        source: review.source,
        accepted: true,
        payload,
    })
}

pub(super) fn audit_canary(repo: &TrainingRepository, project: &LearningProject) -> Result<()> {
    if project.state != LearningState::Canary {
        return Ok(());
    }
    let at_ms = flow_like_ml_runtime::now_ms();
    let cohort: BTreeSet<_> = repo
        .learning_canary_sample_ids(&project.id, at_ms)?
        .into_iter()
        .collect();
    if cohort.is_empty() {
        return Ok(());
    }
    let template = super::training_template(project)?;
    let budget = &template.dataset.budget;
    budget.validate()?;
    let started = Instant::now();
    let check_time = || -> Result<()> {
        if started.elapsed().as_millis() > template.budget.worker_limits.maximum_duration_ms as u128
        {
            return Err(anyhow!(
                "Canary replay reached its per-tick duration budget; saved predictions will resume next tick"
            ));
        }
        Ok(())
    };
    let observations = repo.learning_observations(&project.id)?;
    let reviews: BTreeMap<_, _> = repo
        .learning_training_reviews_at(&project.id, at_ms)?
        .into_iter()
        .filter(|review| review.available_at_ms <= at_ms)
        .map(|review| (review.sample_id.clone(), review))
        .collect();
    let latest = observations
        .iter()
        .filter(|row| cohort.contains(&row.sample_id))
        .map(|row| row.captured_at_ms)
        .max()
        .ok_or_else(|| anyhow!("Canary audit observations are missing"))?;
    if latest > at_ms {
        return Err(anyhow!("Canary observations cannot come from the future"));
    }
    let mut scales = BTreeMap::new();
    for cycle in repo.learning_cycles(&project.id)? {
        if let Some(experiment) = cycle.experiment_id {
            for trial in repo.experiment_result(&experiment)?.trials {
                if let Some(artifact) = trial.artifact_id {
                    scales.insert(
                        artifact,
                        trial
                            .validation_metrics
                            .get("anomaly_scale")
                            .copied()
                            .unwrap_or(1.),
                    );
                }
            }
        }
    }
    for artifact_id in [
        &project.candidate_artifact_id,
        &project.champion_artifact_id,
    ]
    .into_iter()
    .flatten()
    {
        check_time()?;
        let artifact = repo.get_artifact(artifact_id)?;
        if artifact.stream != project.request.stream {
            return Err(anyhow!(
                "Canary artifact belongs to another inspection stream"
            ));
        }
        let preprocessing: TablePreprocessing =
            flow_like_types::json::from_value(artifact.manifest["preprocessing_manifest"].clone())?;
        let spec: InspectionSpec =
            flow_like_types::json::from_value(artifact.manifest["inspection_spec"].clone())?;
        let recorded: BTreeSet<_> = repo
            .predictions(artifact_id)?
            .into_iter()
            .map(|row| row.sample_id)
            .collect();
        if cohort.is_subset(&recorded) {
            continue;
        }
        let windows = has_windows(&preprocessing);
        let mut history: Vec<_> = observations
            .iter()
            .filter(|row| {
                row.captured_at_ms <= latest && (windows || cohort.contains(&row.sample_id))
            })
            .collect();
        history.sort_by(|a, b| {
            (a.captured_at_ms, &a.sample_id).cmp(&(b.captured_at_ms, &b.sample_id))
        });
        if history.len() > budget.maximum_rows {
            return Err(anyhow!(
                "Canary replay history exceeds the configured table row budget"
            ));
        }
        let mut bytes = 0usize;
        let mut rows = Vec::with_capacity(history.len());
        for observation in &history {
            check_time()?;
            original_sample(
                observation,
                &template.dataset.mapping,
                &project.request.stream.stream_id,
            )?;
            bytes = bytes
                .checked_add(flow_like_types::json::to_vec(&observation.sample)?.len())
                .ok_or_else(|| anyhow!("Canary byte count overflow"))?;
            if bytes > budget.maximum_bytes {
                return Err(anyhow!(
                    "Canary replay history exceeds the configured table byte budget"
                ));
            }
            rows.push(observation.sample.clone());
        }
        let inputs = transform_table_rows(&preprocessing, &rows, budget.maximum_tensor_elements)?;
        let mut predictor =
            CatalogSearchPredictorFactory.load(repo, artifact_id, &template.compute)?;
        for (observation, input) in history.iter().zip(inputs) {
            if !cohort.contains(&observation.sample_id) || recorded.contains(&observation.sample_id)
            {
                continue;
            }
            check_time()?;
            let review = reviews.get(&observation.sample_id).ok_or_else(|| {
                anyhow!(
                    "Canary sample '{}' needs an available independent review",
                    observation.sample_id
                )
            })?;
            let row = reviewed_sample(
                observation,
                review,
                &template.dataset.mapping,
                &project.request.stream.stream_id,
                input.clone(),
                &spec,
                at_ms,
            )?;
            repo.record_prepared_sample(&project.request.stream, &row)?;
            let mut batched = input;
            batched.shape.insert(0, 1);
            let inference = Instant::now();
            let output = predictor.predict(&batched)?;
            let record = prediction_record_from_output(
                &row,
                &spec,
                &project.request.goals.task,
                artifact_id,
                &output,
                scales.get(artifact_id).copied().unwrap_or(1.),
                inference.elapsed().as_secs_f64() * 1000.,
            )?;
            repo.record_prediction(&record)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reviewed_canary_rows_preserve_raw_identity_and_use_the_reviewed_target() {
        let mapping: TableMapping = flow_like_types::json::from_value(json!({
            "row_id":"id","group_id":"group","timestamp_ms":"time",
            "features":{"kind":"tabular","options":{"numeric_columns":["x"],"categorical_columns":[]}},
            "target":{"kind":"column","column":"target"},"provenance":{"kind":"measured","source":"gauge"}
        })).unwrap();
        let observation = LearningObservation {
            sample_id: "part".into(),
            group_id: "batch".into(),
            captured_at_ms: 10,
            student: None,
            teacher: None,
            embedding: vec![],
            slices: BTreeMap::new(),
            sample: json!({"id":"part","group":"batch","time":10,"x":12.,"target":"pass"}),
        };
        let review = LearningReview {
            sample_id: "part".into(),
            annotation: json!({"kind":"class","class_id":1}),
            source: LabelSource::Reviewed,
            reviewer: "operator".into(),
            available_at_ms: 20,
            revision: 2,
            outcome: None,
        };
        let spec = InspectionSpec {
            id: "quality".into(),
            description: "Quality".into(),
            task: flow_like_ml_core::TaskKind::SensorClassification,
            labels: vec!["pass".into(), "fail".into()],
            input_shape: vec![2],
            prediction_horizon_ms: None,
            minimum_examples: 1,
            minimum_examples_per_class: 1,
        };
        let first = reviewed_sample(
            &observation,
            &review,
            &mapping,
            "stream",
            TensorData {
                shape: vec![2],
                values: vec![1., 2.],
            },
            &spec,
            20,
        )
        .unwrap();
        let second = reviewed_sample(
            &observation,
            &review,
            &mapping,
            "stream",
            TensorData {
                shape: vec![2],
                values: vec![3., 4.],
            },
            &spec,
            20,
        )
        .unwrap();
        assert_eq!(first.content_digest, second.content_digest);
        assert_eq!(first.annotation_revision, 3);
        assert_eq!(first.payload["label"], "fail");
        assert_eq!(first.payload["feature_row"]["target"], "pass");
        assert_eq!(first.label_available_at_ms, 20);
        assert_eq!(first.source, LabelSource::Reviewed);
        let input = TensorData {
            shape: vec![2],
            values: vec![0.; 2],
        };
        assert!(
            reviewed_sample(
                &observation,
                &review,
                &mapping,
                "stream",
                input.clone(),
                &spec,
                19
            )
            .is_err()
        );
        let mut invalid = observation.clone();
        invalid.sample["time"] = json!(11);
        assert!(reviewed_sample(&invalid, &review, &mapping, "stream", input, &spec, 20).is_err());
    }
}

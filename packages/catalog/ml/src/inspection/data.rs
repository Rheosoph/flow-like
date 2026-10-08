use super::operation;
use flow_like_ml_core::{
    Annotation, DatasetSnapshot, InspectionSpec, LabelProvenance, Outcome, Sample, TensorData,
};
use flow_like_ml_native::{
    dataset::{DatasetSplit, Readiness},
    signals::{SpectrumBin, TimedValue, WindowFeatures},
};
use flow_like_types::{Result, anyhow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const LIMIT: usize = 16_777_216;

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct MakeSampleRequest {
    pub spec: InspectionSpec,
    pub stream_id: String,
    pub group_id: String,
    pub source_id: Option<String>,
    pub timestamp_ms: i64,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub input: TensorData,
}
fn make_sample(input: MakeSampleRequest) -> Result<Sample> {
    input.spec.validate()?;
    if input.spec.input_shape != input.input.shape {
        return Err(anyhow!(
            "Input shape differs from the inspection specification"
        ));
    }
    let id = match input.source_id {
        Some(id) => id,
        None => flow_like_ml_core::content_digest(&flow_like_types::json::to_vec(&(
            &input.stream_id,
            input.window_start_ms,
            input.window_end_ms,
            &input.input,
        ))?),
    };
    let sample = Sample {
        id,
        group_id: input.group_id,
        stream_id: input.stream_id,
        timestamp_ms: input.timestamp_ms,
        window_start_ms: input.window_start_ms,
        window_end_ms: input.window_end_ms,
        input: input.input,
        annotation: Annotation::Unlabeled,
        provenance: LabelProvenance::Unlabeled,
        outcome: None,
    };
    sample.validate(input.spec.labels.len(), LIMIT)?;
    Ok(sample)
}
#[crate::register_node]
#[derive(Default)]
pub struct MakeInspectionSampleNode;
operation!(
    MakeInspectionSampleNode,
    "ml_make_inspection_sample",
    "Make Inspection Sample",
    "Attach stable source identity, timestamps and leakage group to an image or sensor tensor",
    "makeSample",
    "AI/ML/Dataset",
    MakeSampleRequest,
    Sample,
    make_sample
);

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct VisualSequenceRequest {
    pub frames: Vec<Sample>,
    pub maximum_gap_ms: u64,
}
fn visual_sequence(input: VisualSequenceRequest) -> Result<Sample> {
    let first = input
        .frames
        .first()
        .ok_or_else(|| anyhow!("A visual sequence needs frames"))?;
    if first.input.shape.len() != 3 {
        return Err(anyhow!("Video frames require channel-first image tensors"));
    }
    let count = input
        .frames
        .len()
        .checked_mul(first.input.values.len())
        .ok_or_else(|| anyhow!("Video tensor size overflows"))?;
    if count > LIMIT {
        return Err(anyhow!("Video tensor exceeds element budget"));
    }
    let mut result = first.clone();
    let mut values = Vec::with_capacity(count);
    let mut previous = first.timestamp_ms;
    for frame in &input.frames {
        frame.input.validate(LIMIT)?;
        if frame.input.shape != first.input.shape
            || frame.group_id != first.group_id
            || frame.stream_id != first.stream_id
            || frame.timestamp_ms < previous
            || frame.timestamp_ms as i128 - previous as i128 > input.maximum_gap_ms as i128
        {
            return Err(anyhow!(
                "Video frames must share dimensions, stream and group with ordered timestamps within the gap limit"
            ));
        }
        previous = frame.timestamp_ms;
        values.extend_from_slice(&frame.input.values);
    }
    result.id = flow_like_ml_core::content_digest(&flow_like_types::json::to_vec(
        &input.frames.iter().map(|s| &s.id).collect::<Vec<_>>(),
    )?);
    result.window_end_ms = input.frames.last().unwrap().window_end_ms;
    result.timestamp_ms = result.window_end_ms;
    result.input.shape.insert(0, input.frames.len());
    result.input.values = values;
    result.annotation = Annotation::Unlabeled;
    result.provenance = LabelProvenance::Unlabeled;
    result.outcome = None;
    Ok(result)
}
#[crate::register_node]
#[derive(Default)]
pub struct BuildVisualSequenceNode;
operation!(
    BuildVisualSequenceNode,
    "ml_build_visual_sequence",
    "Build Visual Sequence",
    "Stack ordered frames into a temporal image tensor for a CNN-LSTM",
    "visualSequence",
    "AI/ML/Sequence",
    VisualSequenceRequest,
    Sample,
    visual_sequence
);

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ValidateAnnotationRequest {
    pub spec: InspectionSpec,
    pub sample: Sample,
}
fn validate_annotation(input: ValidateAnnotationRequest) -> Result<Sample> {
    input.spec.validate_sample(&input.sample)?;
    input.sample.input.validate(LIMIT)?;
    Ok(input.sample)
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReviewAnnotationRequest {
    pub spec: InspectionSpec,
    pub sample: Sample,
    pub annotation: Annotation,
    pub reviewer: String,
    pub reviewed_at_ms: i64,
}
fn review_annotation(mut input: ReviewAnnotationRequest) -> Result<Sample> {
    if input.reviewer.trim().is_empty() || input.reviewed_at_ms < input.sample.timestamp_ms {
        return Err(anyhow!(
            "Reviewer and a review time after capture are required"
        ));
    }
    input.sample.annotation = input.annotation;
    input.sample.provenance = LabelProvenance::Reviewed {
        reviewer: input.reviewer,
        reviewed_at_ms: input.reviewed_at_ms,
    };
    validate_annotation(ValidateAnnotationRequest {
        spec: input.spec,
        sample: input.sample,
    })
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotRequest {
    pub id: String,
    pub labels: Vec<String>,
    pub samples: Vec<Sample>,
}
fn snapshot(input: SnapshotRequest) -> Result<DatasetSnapshot> {
    let snapshot = DatasetSnapshot::new(input.id, input.labels, input.samples)?;
    snapshot.validate(LIMIT)?;
    Ok(snapshot)
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SplitStrategy {
    Group {
        train_fraction: f64,
        validation_fraction: f64,
        seed: u64,
    },
    Time {
        train_end_ms: i64,
        validation_end_ms: i64,
        embargo_ms: u64,
        label_horizon_ms: u64,
    },
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct SplitRequest {
    pub dataset: DatasetSnapshot,
    pub strategy: SplitStrategy,
}
fn split(input: SplitRequest) -> Result<DatasetSplit> {
    input.dataset.validate(LIMIT)?;
    Ok(match input.strategy {
        SplitStrategy::Group {
            train_fraction,
            validation_fraction,
            seed,
        } => flow_like_ml_native::dataset::split_grouped(
            &input.dataset.samples,
            train_fraction,
            validation_fraction,
            seed,
        )?,
        SplitStrategy::Time {
            train_end_ms,
            validation_end_ms,
            embargo_ms,
            label_horizon_ms,
        } => flow_like_ml_native::dataset::split_temporal(
            &input.dataset.samples,
            train_end_ms,
            validation_end_ms,
            embargo_ms,
            label_horizon_ms,
        )?,
    })
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadinessRequest {
    pub dataset: DatasetSnapshot,
    pub minimum_new: usize,
    pub minimum_per_class: usize,
    pub consumed_ids: HashSet<String>,
}
fn readiness(input: ReadinessRequest) -> Result<Readiness> {
    input.dataset.validate(LIMIT)?;
    if input.minimum_new == 0 {
        return Err(anyhow!("Minimum new samples must be positive"));
    }
    Ok(flow_like_ml_native::dataset::training_readiness(
        &input.dataset.samples,
        input.minimum_new,
        input.minimum_per_class,
        input.dataset.labels.len(),
        &input.consumed_ids,
    ))
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct SignalRequest {
    pub values: Vec<f64>,
    pub sample_rate_hz: f64,
}
fn features(input: SignalRequest) -> Result<WindowFeatures> {
    if input.values.len() > LIMIT {
        return Err(anyhow!("Signal exceeds element budget"));
    }
    Ok(flow_like_ml_native::signals::window_features(
        &input.values,
        input.sample_rate_hz,
    )?)
}
fn spectrum(input: SignalRequest) -> Result<Vec<SpectrumBin>> {
    if input.values.len() > LIMIT {
        return Err(anyhow!("Signal exceeds element budget"));
    }
    Ok(flow_like_ml_native::signals::real_spectrum(
        &input.values,
        input.sample_rate_hz,
    )?)
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct SpectrogramRequest {
    pub values: Vec<f64>,
    pub sample_rate_hz: f64,
    pub window_samples: usize,
    pub hop_samples: usize,
    pub window: flow_like_ml_native::signals::WindowFunction,
}
fn spectrogram(input: SpectrogramRequest) -> Result<flow_like_ml_native::signals::Spectrogram> {
    if input.values.len() > LIMIT {
        return Err(anyhow!("Signal exceeds element budget"));
    }
    Ok(flow_like_ml_native::signals::stft(
        &input.values,
        input.sample_rate_hz,
        input.window_samples,
        input.hop_samples,
        input.window,
    )?)
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct BandEnergyRequest {
    pub values: Vec<f64>,
    pub sample_rate_hz: f64,
    pub bands_hz: Vec<(f64, f64)>,
}
fn band_energy(input: BandEnergyRequest) -> Result<Vec<f64>> {
    if input.values.len() > LIMIT {
        return Err(anyhow!("Signal exceeds element budget"));
    }
    Ok(flow_like_ml_native::signals::band_energy(
        &input.values,
        input.sample_rate_hz,
        &input.bands_hz,
    )?)
}

#[crate::register_node]
#[derive(Default)]
pub struct SignalSpectrogramNode;
operation!(
    SignalSpectrogramNode,
    "ml_signal_spectrogram",
    "Signal STFT",
    "Compute a short-time spectrum with a selected window and hop",
    "spectrogram",
    "AI/ML/Signal",
    SpectrogramRequest,
    flow_like_ml_native::signals::Spectrogram,
    spectrogram
);
#[crate::register_node]
#[derive(Default)]
pub struct SignalBandEnergyNode;
operation!(
    SignalBandEnergyNode,
    "ml_signal_band_energy",
    "Signal Band Energy",
    "Measure mean-square energy in explicit frequency bands",
    "bandEnergy",
    "AI/ML/Signal",
    BandEnergyRequest,
    Vec<f64>,
    band_energy
);

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResampleRequest {
    pub values: Vec<TimedValue>,
    pub start_ms: i64,
    pub step_ms: u64,
    pub count: usize,
    pub max_gap_ms: u64,
}
fn resample(input: ResampleRequest) -> Result<Vec<f64>> {
    if input.count > LIMIT || input.values.len() > LIMIT {
        return Err(anyhow!("Resampling exceeds element budget"));
    }
    Ok(flow_like_ml_native::signals::resample_linear(
        &input.values,
        input.start_ms,
        input.step_ms,
        input.count,
        input.max_gap_ms,
    )?)
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct SequenceRequest {
    pub stream_id: String,
    pub group_id: String,
    pub start_ms: i64,
    pub step_ms: u64,
    /// Time-major measurements. Each row contains the same sensor channels.
    pub values: Vec<Vec<f32>>,
    pub window: usize,
    pub stride: usize,
}
fn sequences(input: SequenceRequest) -> Result<Vec<Sample>> {
    let width = input.values.first().map_or(0, Vec::len);
    if input.stream_id.is_empty()
        || input.group_id.is_empty()
        || input.window == 0
        || input.stride == 0
        || input.step_ms == 0
        || width == 0
        || input.window > input.values.len()
    {
        return Err(anyhow!(
            "A sequence requires stream/group IDs, positive timing, a valid window and channels"
        ));
    }
    if !input
        .values
        .iter()
        .all(|v| v.len() == width && v.iter().all(|x| x.is_finite()))
    {
        return Err(anyhow!(
            "Sensor rows must have equal widths and finite values"
        ));
    }
    let count = (input.values.len() - input.window) / input.stride + 1;
    let total = count
        .checked_mul(input.window)
        .and_then(|n| n.checked_mul(width))
        .ok_or_else(|| anyhow!("Sequence dimensions overflow"))?;
    if total > LIMIT {
        return Err(anyhow!("Sequence windows exceed the element budget"));
    }
    let timestamp = |index: usize| -> Result<i64> {
        i64::try_from(input.start_ms as i128 + index as i128 * input.step_ms as i128)
            .map_err(|_| anyhow!("Sequence timestamp overflows"))
    };
    (0..count)
        .map(|i| {
            let start = i * input.stride;
            let end = start + input.window;
            let first = timestamp(start)?;
            let last = timestamp(end - 1)?;
            Ok(Sample {
                id: format!("{}:{}:{}", input.stream_id, first, last),
                group_id: input.group_id.clone(),
                stream_id: input.stream_id.clone(),
                timestamp_ms: last,
                window_start_ms: first,
                window_end_ms: last,
                input: TensorData {
                    shape: vec![input.window, width],
                    values: input.values[start..end].iter().flatten().copied().collect(),
                },
                annotation: Annotation::Unlabeled,
                provenance: LabelProvenance::Unlabeled,
                outcome: None,
            })
        })
        .collect()
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutcomeRequest {
    pub spec: InspectionSpec,
    pub sample: Sample,
    pub annotation: Annotation,
    pub source: String,
    pub target_start_ms: i64,
    pub target_end_ms: i64,
    pub available_at_ms: i64,
    pub now_ms: i64,
}
fn attach_outcome(mut input: OutcomeRequest) -> Result<Sample> {
    if input.source.trim().is_empty()
        || input.target_start_ms < input.sample.window_end_ms
        || input.available_at_ms > input.now_ms
    {
        return Err(anyhow!(
            "An observed outcome requires a source, a target after the feature window and a mature observation"
        ));
    }
    if let Some(horizon) = input.spec.prediction_horizon_ms {
        let expected = input.sample.window_end_ms as i128 + horizon as i128;
        if expected != input.target_end_ms as i128 {
            return Err(anyhow!(
                "Observed target interval does not match the prediction horizon"
            ));
        }
    }
    input.sample.annotation = input.annotation;
    input.sample.provenance = LabelProvenance::Measured {
        source: input.source,
    };
    input.sample.outcome = Some(Outcome {
        available_at_ms: input.available_at_ms,
        target_start_ms: input.target_start_ms,
        target_end_ms: input.target_end_ms,
    });
    validate_annotation(ValidateAnnotationRequest {
        spec: input.spec,
        sample: input.sample,
    })
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct AlignedReading {
    pub timestamp_ms: i64,
    pub features: Vec<f32>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct AlignRequest {
    pub timestamp_ms: i64,
    pub maximum_age_ms: u64,
    pub readings: Vec<AlignedReading>,
    pub image_features: Vec<f32>,
}
fn align(input: AlignRequest) -> Result<TensorData> {
    // Restrict to measurements available at inspection time to preserve causality.
    let row = input
        .readings
        .iter()
        .filter(|r| r.timestamp_ms <= input.timestamp_ms)
        .max_by_key(|r| r.timestamp_ms)
        .ok_or_else(|| anyhow!("No sensor reading is available at the image timestamp"))?;
    if input.timestamp_ms as i128 - row.timestamp_ms as i128 > input.maximum_age_ms as i128 {
        return Err(anyhow!("The most recent sensor reading is too old"));
    }
    let values: Vec<f32> = input
        .image_features
        .into_iter()
        .chain(row.features.iter().copied())
        .collect();
    let output = TensorData {
        shape: vec![values.len()],
        values,
    };
    output.validate(LIMIT)?;
    Ok(output)
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PatchTensorRequest {
    pub feature_map: TensorData,
}
fn patches(input: PatchTensorRequest) -> Result<TensorData> {
    input.feature_map.validate(LIMIT)?;
    let dims = input.feature_map.shape.as_slice();
    let (channels, height, width) = match dims {
        [c, h, w] => (*c, *h, *w),
        [1, c, h, w] => (*c, *h, *w),
        _ => {
            return Err(anyhow!(
                "Patch conversion requires [channels,height,width] or a single NCHW image"
            ));
        }
    };
    let positions = height * width;
    let mut values = vec![0.0; channels * positions];
    for position in 0..positions {
        for channel in 0..channels {
            values[position * channels + channel] =
                input.feature_map.values[channel * positions + position];
        }
    }
    Ok(TensorData {
        shape: vec![positions, channels],
        values,
    })
}
#[crate::register_node]
#[derive(Default)]
pub struct FeatureMapToPatchesNode;
operation!(
    FeatureMapToPatchesNode,
    "ml_feature_map_to_patches",
    "Feature Map to Patches",
    "Convert a channel-first spatial feature map to position-major PatchCore or PaDiM embeddings",
    "patches",
    "AI/ML/Anomaly",
    PatchTensorRequest,
    TensorData,
    patches
);

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportDatasetRequest {
    pub dataset: DatasetSnapshot,
    pub format: AnnotationFormat,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationFormat {
    NativeJson,
    YoloLabels,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportedDataset {
    pub format: String,
    pub files: std::collections::BTreeMap<String, String>,
}
fn export_annotations(input: ExportDatasetRequest) -> Result<ExportedDataset> {
    input.dataset.validate(LIMIT)?;
    let mut files = std::collections::BTreeMap::new();
    match input.format {
        AnnotationFormat::NativeJson => {
            files.insert(
                "dataset.json".into(),
                flow_like_types::json::to_string(&input.dataset)?,
            );
            Ok(ExportedDataset {
                format: "native_json".into(),
                files,
            })
        }
        AnnotationFormat::YoloLabels => {
            let mut index = std::collections::BTreeMap::new();
            files.insert(
                "classes.json".into(),
                flow_like_types::json::to_string(&input.dataset.labels)?,
            );
            for sample in input.dataset.samples {
                let Annotation::Boxes { boxes } = sample.annotation else {
                    return Err(anyhow!("YOLO label export requires box annotations"));
                };
                let text = boxes
                    .iter()
                    .map(|b| {
                        format!(
                            "{} {} {} {} {}",
                            b.class_id,
                            (b.x_min + b.x_max) / 2.0,
                            (b.y_min + b.y_max) / 2.0,
                            b.x_max - b.x_min,
                            b.y_max - b.y_min
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let filename = format!(
                    "{}.txt",
                    flow_like_ml_core::content_digest(sample.id.as_bytes())
                );
                index.insert(sample.id, filename.clone());
                files.insert(filename, text);
            }
            files.insert(
                "sample-index.json".into(),
                flow_like_types::json::to_string(&index)?,
            );
            Ok(ExportedDataset {
                format: "yolo_labels".into(),
                files,
            })
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ValidateAnnotationNode;
operation!(
    ValidateAnnotationNode,
    "ml_validate_annotation",
    "Validate Annotation",
    "Check tensor dimensions, labels and annotation geometry",
    "validateAnnotation",
    "AI/ML/Inspection",
    ValidateAnnotationRequest,
    Sample,
    validate_annotation
);
#[crate::register_node]
#[derive(Default)]
pub struct ReviewAnnotationNode;
operation!(
    ReviewAnnotationNode,
    "ml_review_annotation",
    "Review Annotation",
    "Apply a corrected annotation with reviewer provenance",
    "reviewAnnotation",
    "AI/ML/Inspection",
    ReviewAnnotationRequest,
    Sample,
    review_annotation
);
#[crate::register_node]
#[derive(Default)]
pub struct DatasetSnapshotNode;
operation!(
    DatasetSnapshotNode,
    "ml_dataset_snapshot",
    "Dataset Snapshot",
    "Create a canonical dataset with a content digest",
    "snapshot",
    "AI/ML/Dataset",
    SnapshotRequest,
    DatasetSnapshot,
    snapshot
);
#[crate::register_node]
#[derive(Default)]
pub struct GroupTimeSplitNode;
operation!(
    GroupTimeSplitNode,
    "ml_group_time_split",
    "Group/Time Split",
    "Keep related samples together and purge temporal boundary overlap",
    "split",
    "AI/ML/Dataset",
    SplitRequest,
    DatasetSplit,
    split
);
#[crate::register_node]
#[derive(Default)]
pub struct TrainingReadinessNode;
operation!(
    TrainingReadinessNode,
    "ml_training_readiness",
    "Training Readiness Gate",
    "Check new labelled samples and class coverage",
    "readiness",
    "AI/ML/Training",
    ReadinessRequest,
    Readiness,
    readiness
);
#[crate::register_node]
#[derive(Default)]
pub struct SignalFeaturesNode;
operation!(
    SignalFeaturesNode,
    "ml_signal_features",
    "Extract Signal Features",
    "Calculate RMS, moments and frequency features from a sensor window",
    "signalFeatures",
    "AI/ML/Signal",
    SignalRequest,
    WindowFeatures,
    features
);
#[crate::register_node]
#[derive(Default)]
pub struct SignalSpectrumNode;
operation!(
    SignalSpectrumNode,
    "ml_signal_spectrum",
    "Signal FFT",
    "Calculate a real one-sided frequency spectrum",
    "spectrum",
    "AI/ML/Signal",
    SignalRequest,
    Vec<SpectrumBin>,
    spectrum
);
#[crate::register_node]
#[derive(Default)]
pub struct ResampleSensorNode;
operation!(
    ResampleSensorNode,
    "ml_resample_sensor",
    "Resample Sensor",
    "Interpolate timestamped readings with a maximum gap",
    "resample",
    "AI/ML/Signal",
    ResampleRequest,
    Vec<f64>,
    resample
);
#[crate::register_node]
#[derive(Default)]
pub struct BuildSequenceDatasetNode;
operation!(
    BuildSequenceDatasetNode,
    "ml_build_sequence_dataset",
    "Build Sequence Dataset",
    "Build timestamped multichannel windows for temporal models",
    "sequences",
    "AI/ML/Sequence",
    SequenceRequest,
    Vec<Sample>,
    sequences
);
#[crate::register_node]
#[derive(Default)]
pub struct SensorWindowNode;
operation!(
    SensorWindowNode,
    "ml_sensor_window",
    "Sensor Window",
    "Window an ordered sensor batch with explicit stride and sampling interval",
    "sensorWindow",
    "AI/ML/Sequence",
    SequenceRequest,
    Vec<Sample>,
    sequences
);
#[crate::register_node]
#[derive(Default)]
pub struct AttachObservedOutcomeNode;
operation!(
    AttachObservedOutcomeNode,
    "ml_attach_observed_outcome",
    "Attach Observed Outcome",
    "Join a mature measured outcome to its earlier feature window",
    "attachOutcome",
    "AI/ML/Sequence",
    OutcomeRequest,
    Sample,
    attach_outcome
);
#[crate::register_node]
#[derive(Default)]
pub struct AlignModalitiesNode;
operation!(
    AlignModalitiesNode,
    "ml_align_modalities",
    "Align Modalities",
    "Combine image features with the latest eligible sensor reading",
    "align",
    "AI/ML/Fusion",
    AlignRequest,
    TensorData,
    align
);
#[crate::register_node]
#[derive(Default)]
pub struct ExportAnnotationDatasetNode;
operation!(
    ExportAnnotationDatasetNode,
    "ml_export_annotation_dataset",
    "Export Annotation Dataset",
    "Materialize native dataset JSON or YOLO box-label files",
    "exportAnnotations",
    "AI/ML/Dataset",
    ExportDatasetRequest,
    ExportedDataset,
    export_annotations
);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequence_timestamps_and_causal_alignment() {
        let result = sequences(SequenceRequest {
            stream_id: "s".into(),
            group_id: "part".into(),
            start_ms: 10,
            step_ms: 5,
            values: vec![vec![1.0], vec![2.0], vec![3.0]],
            window: 2,
            stride: 1,
        })
        .unwrap();
        assert_eq!(result[1].window_start_ms, 15);
        assert_eq!(result[1].timestamp_ms, 20);
        assert_eq!(result[1].input.values, vec![2.0, 3.0]);
        let fused = align(AlignRequest {
            timestamp_ms: 15,
            maximum_age_ms: 10,
            image_features: vec![1.0],
            readings: vec![
                AlignedReading {
                    timestamp_ms: 10,
                    features: vec![2.0],
                },
                AlignedReading {
                    timestamp_ms: 20,
                    features: vec![99.0],
                },
            ],
        })
        .unwrap();
        assert_eq!(fused.values, vec![1.0, 2.0]);
    }
}

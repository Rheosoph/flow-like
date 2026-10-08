use flow_like_catalog_core::NodeDBConnection;
use flow_like_ml_core::{
    Annotation, InspectionSpec, LabelProvenance, Outcome, Sample, TaskKind, TensorData,
    content_digest,
};
use flow_like_ml_native::{
    dataset::{DatasetSplit, split_grouped, split_temporal},
    preprocessing::{FittedTabularPreprocessor, TabularOptions},
};
use flow_like_storage_contracts::database::DatabaseReference;
use flow_like_types::{Result, Value, anyhow, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PinnedTableSource {
    pub database: NodeDBConnection,
    pub reference: DatabaseReference,
    #[serde(default)]
    pub locator: Option<TableLocator>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TableLocator {
    pub app_id: String,
    pub board_id: String,
    pub connection_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TableBudget {
    pub maximum_rows: usize,
    pub maximum_bytes: usize,
    pub maximum_tensor_elements: usize,
}
impl Default for TableBudget {
    fn default() -> Self {
        Self {
            maximum_rows: 100_000,
            maximum_bytes: 256 * 1024 * 1024,
            maximum_tensor_elements: 32 * 1024 * 1024,
        }
    }
}
impl TableBudget {
    pub fn validate(&self) -> Result<()> {
        if !(1..=1_000_000).contains(&self.maximum_rows)
            || !(1024..=1024 * 1024 * 1024).contains(&self.maximum_bytes)
            || !(1..=64 * 1024 * 1024).contains(&self.maximum_tensor_elements)
        {
            return Err(anyhow!("Invalid table row, byte or tensor budget"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeatureSource {
    Tabular { options: TabularOptions },
    Tensor { column: String },
    Sample { column: String },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetSource {
    Column { column: String },
    Columns { columns: Vec<String> },
    Annotation { column: String },
    Sample,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TableMapping {
    pub row_id: String,
    pub group_id: Option<String>,
    pub timestamp_ms: Option<String>,
    pub window_start_ms: Option<String>,
    pub window_end_ms: Option<String>,
    pub outcome: Option<String>,
    pub features: FeatureSource,
    pub target: TargetSource,
    pub provenance: LabelProvenance,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TableSplitPolicy {
    Group {
        train_fraction: f64,
        validation_fraction: f64,
        seed: u64,
    },
    Time {
        train_end_ms: i64,
        validation_end_ms: i64,
        embargo_ms: u64,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TableDatasetRequest {
    pub source: NodeDBConnection,
    pub stream_id: String,
    pub spec: InspectionSpec,
    pub mapping: TableMapping,
    pub split: TableSplitPolicy,
    #[serde(default)]
    pub budget: TableBudget,
    #[serde(default)]
    pub selection: DatasetSelection,
    #[serde(default)]
    pub label_overrides: BTreeMap<String, TableLabelOverride>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DatasetSelection {
    pub excluded_ids: Vec<String>,
    pub excluded_groups: Vec<String>,
    pub ineligible_test_ids: Vec<String>,
    pub ineligible_test_groups: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TableLabelOverride {
    pub annotation: Annotation,
    pub provenance: LabelProvenance,
    pub annotation_revision: i64,
    #[serde(default)]
    pub outcome: Option<Outcome>,
    #[serde(default)]
    pub available_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ColumnProfile {
    pub name: String,
    pub missing: usize,
    pub numeric: usize,
    pub boolean: usize,
    pub text: usize,
    pub other: usize,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub examples: Vec<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TableProfile {
    pub partition: String,
    pub rows: usize,
    pub columns: Vec<ColumnProfile>,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TablePreprocessing {
    Engineered {
        pipeline: flow_like_ml_native::feature_engineering::FittedFeaturePipeline,
        preprocessing: Box<TablePreprocessing>,
        #[serde(default)]
        projection: Option<flow_like_ml_native::reduction::FittedPca>,
        #[serde(default)]
        plan: Option<Box<super::feature_engineering::FeaturePlan>>,
    },
    Tabular {
        fitted: FittedTabularPreprocessor,
    },
    Tensor {
        column: String,
        input_shape: Vec<usize>,
    },
    Sample {
        column: String,
        input_shape: Vec<usize>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PreparedTableDataset {
    pub source: PinnedTableSource,
    pub spec: InspectionSpec,
    pub split: DatasetSplit,
    pub samples: Vec<Sample>,
    pub preprocessing: TablePreprocessing,
    pub profile: TableProfile,
    pub digest: String,
    #[serde(default)]
    pub rows: Vec<Value>,
    #[serde(default)]
    pub raw_rows: Vec<Value>,
    #[serde(default)]
    pub annotation_revisions: BTreeMap<String, i64>,
    #[serde(default)]
    pub raw_content_digests: BTreeMap<String, String>,
    #[serde(default)]
    pub label_available_at_ms: BTreeMap<String, i64>,
    #[serde(default)]
    pub feature_sources: Vec<super::feature_engineering::PinnedFeatureSource>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TableWritePolicy {
    pub create_tables: bool,
    pub table_prefix: Option<String>,
    pub allowed_destination: Option<NodeDBConnection>,
    pub expected_destination: Option<DatabaseReference>,
    pub maximum_rows: usize,
    pub maximum_bytes: usize,
}
impl Default for TableWritePolicy {
    fn default() -> Self {
        Self {
            create_tables: true,
            table_prefix: None,
            allowed_destination: None,
            expected_destination: None,
            maximum_rows: 100_000,
            maximum_bytes: 256 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MaterializedTable {
    pub partition: String,
    pub database: NodeDBConnection,
    pub reference: DatabaseReference,
    pub source: DatabaseReference,
    pub rows: usize,
    pub preprocessing_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TableColumnSchema {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SourceTableSchema {
    pub source: PinnedTableSource,
    pub columns: Vec<TableColumnSchema>,
}

fn field<'a>(row: &'a Value, name: &str) -> Result<&'a Value> {
    row.as_object()
        .and_then(|row| row.get(name))
        .ok_or_else(|| anyhow!("Missing table column '{name}'"))
}
fn identifier(value: &Value) -> Result<String> {
    let result = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) if value.is_i64() || value.is_u64() => value.to_string(),
        _ => {
            return Err(anyhow!(
                "Row/group identifiers must be non-null strings or integers"
            ));
        }
    };
    if result.is_empty() || result.len() > 4096 {
        return Err(anyhow!(
            "Row/group identifier is empty or exceeds 4096 bytes"
        ));
    }
    Ok(result)
}
fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
    Ok(if let Some(text) = value.as_str() {
        flow_like_types::json::from_str(text)?
    } else {
        flow_like_types::json::from_value(value.clone())?
    })
}
fn timestamp(row: &Value, column: &Option<String>, default: i64) -> Result<i64> {
    match column {
        Some(column) => field(row, column)?
            .as_i64()
            .ok_or_else(|| anyhow!("Timestamp '{column}' must be integer milliseconds")),
        None => Ok(default),
    }
}
fn finite(value: &Value) -> Result<f64> {
    value
        .as_f64()
        .filter(|value| value.is_finite())
        .ok_or_else(|| anyhow!("Numeric targets must be finite"))
}
fn target(row: &Value, spec: &InspectionSpec, mapping: &TargetSource) -> Result<Annotation> {
    match mapping {
        TargetSource::Annotation { column } => decode(field(row, column)?),
        TargetSource::Columns { columns } => {
            if columns.is_empty() || columns.len() > 65536 {
                return Err(anyhow!("Dense targets require bounded, nonempty columns"));
            }
            Ok(Annotation::Values {
                values: columns
                    .iter()
                    .map(|name| {
                        let value = finite(field(row, name)?)? as f32;
                        if !value.is_finite() {
                            return Err(anyhow!("Dense target exceeds float32 range"));
                        }
                        Ok(value)
                    })
                    .collect::<Result<_>>()?,
            })
        }
        TargetSource::Sample => Err(anyhow!("Sample targets require serialized sample features")),
        TargetSource::Column { column } => {
            let value = field(row, column)?;
            Ok(match spec.task {
                TaskKind::ImageClassification
                | TaskKind::SensorClassification
                | TaskKind::VisualSequenceClassification
                | TaskKind::Fusion
                    if !spec.labels.is_empty() =>
                {
                    let label = value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string());
                    let class_id = spec
                        .labels
                        .iter()
                        .position(|candidate| candidate == &label)
                        .or_else(|| {
                            value
                                .as_u64()
                                .and_then(|id| usize::try_from(id).ok())
                                .filter(|id| *id < spec.labels.len())
                        })
                        .ok_or_else(|| anyhow!("Unknown target label '{label}'"))?;
                    Annotation::Class {
                        class_id: class_id as u32,
                    }
                }
                TaskKind::SensorRegression | TaskKind::Fusion => Annotation::Scalar {
                    value: finite(value)?,
                },
                TaskKind::VisualAnomaly
                | TaskKind::SensorAnomaly
                | TaskKind::SequenceAutoencoder => Annotation::Anomaly {
                    is_anomaly: value
                        .as_bool()
                        .ok_or_else(|| anyhow!("Anomaly target must be a boolean"))?,
                },
                TaskKind::SequenceForecast => Annotation::Values {
                    values: decode(value)?,
                },
                _ => return Err(anyhow!("This task needs a typed annotation column")),
            })
        }
    }
}

fn validate_mapping(mapping: &TableMapping) -> Result<()> {
    if mapping.row_id.trim().is_empty() {
        return Err(anyhow!("An explicit stable row ID column is required"));
    }
    if let FeatureSource::Tabular { options } = &mapping.features {
        let mut protected: HashSet<&str> =
            [&mapping.row_id].into_iter().map(String::as_str).collect();
        for column in [
            &mapping.group_id,
            &mapping.timestamp_ms,
            &mapping.window_start_ms,
            &mapping.window_end_ms,
            &mapping.outcome,
        ]
        .into_iter()
        .flatten()
        {
            protected.insert(column);
        }
        match &mapping.target {
            TargetSource::Column { column } | TargetSource::Annotation { column } => {
                protected.insert(column);
            }
            TargetSource::Columns { columns } => {
                protected.extend(columns.iter().map(String::as_str));
            }
            TargetSource::Sample => {
                return Err(anyhow!("Tabular features need an explicit target mapping"));
            }
        }
        if options
            .numeric_columns
            .iter()
            .chain(&options.categorical_columns)
            .any(|name| protected.contains(name.as_str()))
        {
            return Err(anyhow!(
                "Feature columns must exclude row/group/time keys, outcomes and targets"
            ));
        }
    }
    Ok(())
}

/// Split identity uses row/group/time metadata before any fitted transformation.
pub(crate) fn raw_input_digest(
    mapping: &TableMapping,
    row: &Value,
    sample: &Sample,
) -> Result<String> {
    let raw_features = match &mapping.features {
        FeatureSource::Tabular { options } => Value::Object(
            options
                .numeric_columns
                .iter()
                .chain(&options.categorical_columns)
                .map(|name| (name.clone(), row.get(name).cloned().unwrap_or(Value::Null)))
                .collect(),
        ),
        FeatureSource::Tensor { column } => field(row, column)?.clone(),
        FeatureSource::Sample { .. } => json!(sample.input),
    };
    Ok(content_digest(&flow_like_types::json::to_vec(&json!({
        "features":raw_features,"group_id":sample.group_id,"timestamp_ms":sample.timestamp_ms,
        "window_start_ms":sample.window_start_ms,"window_end_ms":sample.window_end_ms,
    }))?))
}

pub(crate) struct TableIdentity {
    pub samples: Vec<Sample>,
    pub split: DatasetSplit,
    pub raw_content_digests: BTreeMap<String, String>,
}

pub(crate) fn serialized_json_size(value: &impl Serialize) -> Result<usize> {
    Ok(flow_like_ml_native::feature_engineering::serialized_bytes(
        value,
    )?)
}

pub(crate) fn prepare_identity_rows(
    source: &PinnedTableSource,
    request: &TableDatasetRequest,
    rows: &[Value],
) -> Result<TableIdentity> {
    request.budget.validate()?;
    request.spec.validate()?;
    validate_mapping(&request.mapping)?;
    if !source.reference.pinned
        || !source.reference.read_only
        || request.stream_id.trim().is_empty()
    {
        return Err(anyhow!(
            "Preparation requires a pinned source and an inspection stream"
        ));
    }
    if rows.len() > request.budget.maximum_rows {
        return Err(anyhow!("Table exceeds the configured row budget"));
    }
    let total_bytes = rows.iter().try_fold(0usize, |size, row| -> Result<usize> {
        size.checked_add(serialized_json_size(row)?)
            .ok_or_else(|| anyhow!("Table byte count overflow"))
    })?;
    if total_bytes > request.budget.maximum_bytes {
        return Err(anyhow!("Table exceeds the configured byte budget"));
    }
    let mut seen = HashSet::new();
    let mut samples = Vec::with_capacity(rows.len());
    let mut raw_content_digests = BTreeMap::new();
    for row in rows {
        let id = identifier(field(row, &request.mapping.row_id)?)?;
        if !seen.insert(id.clone()) {
            return Err(anyhow!("Duplicate row ID '{id}'"));
        }
        let mut sample = if let FeatureSource::Sample { column } = &request.mapping.features {
            if !matches!(request.mapping.target, TargetSource::Sample) {
                return Err(anyhow!(
                    "Serialized samples preserve their own target and provenance"
                ));
            }
            let sample: Sample = decode(field(row, column)?)?;
            if sample.id != id || sample.stream_id != request.stream_id {
                return Err(anyhow!(
                    "Serialized sample identity/stream differs from the table mapping"
                ));
            }
            sample
        } else {
            let time = timestamp(row, &request.mapping.timestamp_ms, 0)?;
            if matches!(request.split, TableSplitPolicy::Time { .. })
                && request.mapping.timestamp_ms.is_none()
            {
                return Err(anyhow!(
                    "Temporal splits require a timestamp column or serialized samples"
                ));
            }
            Sample {
                id: id.clone(),
                group_id: match &request.mapping.group_id {
                    Some(column) => identifier(field(row, column)?)?,
                    None => id.clone(),
                },
                stream_id: request.stream_id.clone(),
                timestamp_ms: time,
                window_start_ms: timestamp(row, &request.mapping.window_start_ms, time)?,
                window_end_ms: timestamp(row, &request.mapping.window_end_ms, time)?,
                input: match &request.mapping.features {
                    FeatureSource::Tensor { column } => decode(field(row, column)?)?,
                    _ => TensorData {
                        shape: vec![1],
                        values: vec![0.0],
                    },
                },
                annotation: request
                    .label_overrides
                    .get(&id)
                    .map(|review| Ok(review.annotation.clone()))
                    .unwrap_or_else(|| target(row, &request.spec, &request.mapping.target))?,
                provenance: request.mapping.provenance.clone(),
                outcome: request
                    .mapping
                    .outcome
                    .as_ref()
                    .map(|column| decode::<Outcome>(field(row, column)?))
                    .transpose()?,
            }
        };
        if let Some(review) = request.label_overrides.get(&sample.id) {
            if review.annotation_revision < 1 {
                return Err(anyhow!("Label override revisions must be positive"));
            }
            if let Some(available) = review.available_at_ms {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_millis();
                if available < sample.timestamp_ms || available as i128 > now as i128 {
                    return Err(anyhow!(
                        "Review availability must follow capture and cannot be in the future"
                    ));
                }
            }
            sample.annotation = review.annotation.clone();
            sample.provenance = review.provenance.clone();
            sample.outcome = review.outcome.clone();
        }
        sample.validate(
            request.spec.labels.len(),
            request.budget.maximum_tensor_elements,
        )?;
        raw_content_digests.insert(
            sample.id.clone(),
            raw_input_digest(&request.mapping, row, &sample)?,
        );
        samples.push(sample);
    }
    let mut split = match request.split {
        TableSplitPolicy::Group {
            train_fraction,
            validation_fraction,
            seed,
        } => split_grouped(&samples, train_fraction, validation_fraction, seed)?,
        TableSplitPolicy::Time {
            train_end_ms,
            validation_end_ms,
            embargo_ms,
        } => split_temporal(
            &samples,
            train_end_ms,
            validation_end_ms,
            embargo_ms,
            request.spec.prediction_horizon_ms.unwrap_or(0),
        )?,
    };
    if let TableSplitPolicy::Time {
        train_end_ms,
        validation_end_ms,
        ..
    } = request.split
    {
        for (indices, deadline) in [
            (&mut split.train, train_end_ms),
            (&mut split.validation, validation_end_ms),
        ] {
            indices.retain(|index| {
                let sample = &samples[*index];
                let reviewed = match sample.provenance {
                    LabelProvenance::Reviewed { reviewed_at_ms, .. } => reviewed_at_ms,
                    _ => sample.timestamp_ms,
                };
                let available = reviewed
                    .max(
                        sample
                            .outcome
                            .as_ref()
                            .map_or(sample.timestamp_ms, |outcome| outcome.available_at_ms),
                    )
                    .max(
                        request
                            .label_overrides
                            .get(&sample.id)
                            .and_then(|review| review.available_at_ms)
                            .unwrap_or(sample.timestamp_ms),
                    );
                if available >= deadline {
                    split.purged.push(*index);
                    false
                } else {
                    true
                }
            });
        }
    }
    let selection = &request.selection;
    let excluded_ids: HashSet<_> = selection.excluded_ids.iter().collect();
    let excluded_groups: HashSet<_> = selection.excluded_groups.iter().collect();
    let ineligible_ids: HashSet<_> = selection.ineligible_test_ids.iter().collect();
    let ineligible_groups: HashSet<_> = selection.ineligible_test_groups.iter().collect();
    for indices in [&mut split.train, &mut split.validation, &mut split.test] {
        indices.retain(|index| {
            let sample = &samples[*index];
            let retain =
                !excluded_ids.contains(&sample.id) && !excluded_groups.contains(&sample.group_id);
            if !retain {
                split.purged.push(*index);
            }
            retain
        });
    }
    split.test.retain(|index| {
        let sample = &samples[*index];
        let retain =
            !ineligible_ids.contains(&sample.id) && !ineligible_groups.contains(&sample.group_id);
        if !retain {
            split.purged.push(*index);
        }
        retain
    });
    if split.train.is_empty() || split.validation.is_empty() || split.test.is_empty() {
        return Err(anyhow!(
            "Preparation requires nonempty train, validation and test partitions after purging"
        ));
    }
    Ok(TableIdentity {
        samples,
        split,
        raw_content_digests,
    })
}

pub fn prepare_rows(
    source: PinnedTableSource,
    request: &TableDatasetRequest,
    rows: &[Value],
) -> Result<PreparedTableDataset> {
    let TableIdentity {
        mut samples,
        split,
        raw_content_digests,
    } = prepare_identity_rows(&source, request, rows)?;
    let preprocessing = match &request.mapping.features {
        FeatureSource::Tabular { options } => {
            let fitted = FittedTabularPreprocessor::fit(rows, &split.train, options)?;
            let inputs = fitted.transform_rows(rows, request.budget.maximum_tensor_elements)?;
            for (sample, input) in samples.iter_mut().zip(inputs) {
                sample.input = input;
            }
            TablePreprocessing::Tabular { fitted }
        }
        FeatureSource::Tensor { column } => TablePreprocessing::Tensor {
            column: column.clone(),
            input_shape: request.spec.input_shape.clone(),
        },
        FeatureSource::Sample { column } => TablePreprocessing::Sample {
            column: column.clone(),
            input_shape: request.spec.input_shape.clone(),
        },
    };
    let mut spec = request.spec.clone();
    if matches!(preprocessing, TablePreprocessing::Tabular { .. }) {
        spec.input_shape = samples[0].input.shape.clone();
    }
    let mut elements = 0usize;
    for sample in &samples {
        spec.validate_sample(sample)?;
        elements = elements
            .checked_add(sample.input.values.len())
            .ok_or_else(|| anyhow!("Tensor budget overflows"))?;
        if elements > request.budget.maximum_tensor_elements {
            return Err(anyhow!(
                "Prepared tensors exceed the aggregate element budget"
            ));
        }
    }
    let profile = profile_training_rows(rows, &split.train)?;
    let digest = content_digest(&flow_like_types::json::to_vec(&(
        &source.reference,
        &spec,
        &split,
        &samples,
        &preprocessing,
    ))?);
    Ok(PreparedTableDataset {
        source,
        spec,
        split,
        samples,
        preprocessing,
        profile,
        digest,
        rows: rows.to_vec(),
        raw_rows: rows.to_vec(),
        annotation_revisions: request
            .label_overrides
            .iter()
            .map(|(id, review)| (id.clone(), review.annotation_revision))
            .collect(),
        raw_content_digests,
        label_available_at_ms: request
            .label_overrides
            .iter()
            .filter_map(|(id, review)| {
                review
                    .available_at_ms
                    .map(|available| (id.clone(), available))
            })
            .collect(),
        feature_sources: Vec::new(),
    })
}

fn profile_training_rows(rows: &[Value], training: &[usize]) -> Result<TableProfile> {
    let mut profiles: BTreeMap<String, ColumnProfile> = BTreeMap::new();
    for index in training {
        let row = rows[*index]
            .as_object()
            .ok_or_else(|| anyhow!("Table rows must be objects"))?;
        for name in row.keys() {
            profiles
                .entry(name.clone())
                .or_insert_with(|| ColumnProfile {
                    name: name.clone(),
                    missing: 0,
                    numeric: 0,
                    boolean: 0,
                    text: 0,
                    other: 0,
                    minimum: None,
                    maximum: None,
                    examples: vec![],
                });
        }
    }
    if profiles.len() > 4096 {
        return Err(anyhow!("Table profile exceeds 4096 columns"));
    }
    for index in training {
        for (name, profile) in &mut profiles {
            let value = rows[*index].get(name).unwrap_or(&Value::Null);
            match value {
                Value::Null => profile.missing += 1,
                Value::Number(number) => {
                    profile.numeric += 1;
                    if let Some(value) = number.as_f64() {
                        profile.minimum = Some(
                            profile
                                .minimum
                                .map_or(value, |previous| previous.min(value)),
                        );
                        profile.maximum = Some(
                            profile
                                .maximum
                                .map_or(value, |previous| previous.max(value)),
                        );
                    }
                }
                Value::Bool(_) => profile.boolean += 1,
                Value::String(_) => profile.text += 1,
                _ => profile.other += 1,
            }
            if profile.examples.len() < 5
                && !value.is_null()
                && !profile.examples.contains(value)
                && flow_like_types::json::to_vec(value)?.len() <= 512
            {
                profile.examples.push(value.clone());
            }
        }
    }
    Ok(TableProfile {
        partition: "train".into(),
        rows: training.len(),
        columns: profiles.into_values().collect(),
    })
}

pub fn transform_table_rows(
    preprocessing: &TablePreprocessing,
    rows: &[Value],
    maximum_elements: usize,
) -> Result<Vec<TensorData>> {
    match preprocessing {
        TablePreprocessing::Engineered {
            pipeline,
            preprocessing,
            projection,
            ..
        } => {
            let inputs = transform_table_rows(
                preprocessing,
                &pipeline.transform_rows(rows)?,
                maximum_elements,
            )?;
            if let Some(projection) = projection {
                Ok(projection.transform_rows(&inputs, maximum_elements)?)
            } else {
                Ok(inputs)
            }
        }
        TablePreprocessing::Tabular { fitted } => {
            Ok(fitted.transform_rows(rows, maximum_elements)?)
        }
        TablePreprocessing::Tensor {
            column,
            input_shape,
        }
        | TablePreprocessing::Sample {
            column,
            input_shape,
        } => {
            let mut total = 0usize;
            rows.iter()
                .map(|row| {
                    let tensor = if matches!(preprocessing, TablePreprocessing::Sample { .. }) {
                        decode::<Sample>(field(row, column)?)?.input
                    } else {
                        decode::<TensorData>(field(row, column)?)?
                    };
                    tensor.validate(maximum_elements)?;
                    if &tensor.shape != input_shape {
                        return Err(anyhow!(
                            "Inference tensor shape differs from fitted preprocessing"
                        ));
                    }
                    total = total
                        .checked_add(tensor.values.len())
                        .ok_or_else(|| anyhow!("Tensor budget overflow"))?;
                    if total > maximum_elements {
                        return Err(anyhow!("Inference tensors exceed aggregate element budget"));
                    }
                    Ok(tensor)
                })
                .collect()
        }
    }
}

pub fn transform_table_rows_with_state(
    preprocessing: &TablePreprocessing,
    rows: &[Value],
    maximum_elements: usize,
    state: Option<flow_like_ml_native::feature_engineering::FeatureStreamState>,
    stateful: bool,
) -> Result<(
    Vec<TensorData>,
    Option<flow_like_ml_native::feature_engineering::FeatureStreamState>,
)> {
    if let TablePreprocessing::Engineered {
        pipeline,
        preprocessing,
        projection,
        ..
    } = preprocessing
    {
        if stateful || state.is_some() {
            if rows.len() > pipeline.limits.maximum_rows {
                return Err(anyhow!("Feature batch exceeds the row budget"));
            }
            let mut stream = if let Some(state) = state {
                pipeline.stream_with_state(state)?
            } else {
                pipeline.stream()?
            };
            let started = std::time::Instant::now();
            let mut bytes = 0usize;
            let mut output_bytes = 0usize;
            let transformed = rows
                .iter()
                .map(|row| -> Result<Value> {
                    bytes = bytes
                        .checked_add(serialized_json_size(row)?)
                        .ok_or_else(|| anyhow!("Feature byte budget overflow"))?;
                    if bytes > pipeline.limits.maximum_bytes
                        || started.elapsed().as_millis()
                            > pipeline.limits.maximum_duration_ms as u128
                    {
                        return Err(anyhow!("Feature batch exceeds its byte or time budget"));
                    }
                    let output = stream.transform_row(row)?;
                    output_bytes = output_bytes
                        .checked_add(serialized_json_size(&output)?)
                        .ok_or_else(|| anyhow!("Feature byte budget overflow"))?;
                    if output_bytes > pipeline.limits.maximum_bytes {
                        return Err(anyhow!("Transformed feature batch exceeds the byte budget"));
                    }
                    Ok(output)
                })
                .collect::<Result<Vec<_>>>()?;
            let inputs = transform_table_rows(preprocessing, &transformed, maximum_elements)?;
            let inputs = if let Some(projection) = projection {
                projection.transform_rows(&inputs, maximum_elements)?
            } else {
                inputs
            };
            return Ok((inputs, Some(stream.state())));
        }
    } else if state.is_some() || stateful {
        return Err(anyhow!(
            "Feature state requires an engineered model preprocessing pipeline"
        ));
    }
    Ok((
        transform_table_rows(preprocessing, rows, maximum_elements)?,
        None,
    ))
}

pub fn base_preprocessing(preprocessing: &TablePreprocessing) -> &TablePreprocessing {
    match preprocessing {
        TablePreprocessing::Engineered { preprocessing, .. } => base_preprocessing(preprocessing),
        preprocessing => preprocessing,
    }
}

#[cfg(feature = "execute")]
mod table_io {
    use super::*;
    use flow_like::flow::execution::context::ExecutionContext;
    use flow_like_catalog_core::{CachedDB, CachedDBRefreshHook, CachedDBRefresher};
    use flow_like_storage::{
        contracts::database::DatabaseSelector,
        databases::vector::{
            VectorStore, buffered::BufferedVectorStore, lancedb::LanceDBVectorStore,
        },
    };
    use flow_like_types::{async_trait, sync::RwLock};
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    struct ViewRefresher {
        source: NodeDBConnection,
        cache_key: String,
        source_generation: AtomicU64,
        generation: AtomicU64,
        lock: tokio::sync::Mutex<()>,
    }
    #[async_trait]
    impl CachedDBRefresher for ViewRefresher {
        async fn refresh(&self, context: &ExecutionContext) -> Result<()> {
            let _lock = self.lock.lock().await;
            let (source, generation) = self
                .source
                .load_with_generation(&mut context.clone())
                .await?;
            if self.source_generation.load(Ordering::Acquire) == generation {
                return Ok(());
            }
            let value = context
                .cache
                .read()
                .await
                .get(&self.cache_key)
                .cloned()
                .ok_or_else(|| anyhow!("Prepared table cache disappeared"))?;
            let cached = value
                .as_any()
                .downcast_ref::<CachedDB>()
                .ok_or_else(|| anyhow!("Unexpected prepared table cache type"))?;
            let mut target = cached.db.write().await;
            let source = source.db.read().await;
            let replacement = target
                .inner()
                .reopen(source.inner().connection()?.clone())
                .await?;
            *target.inner_mut() = replacement;
            self.source_generation.store(generation, Ordering::Release);
            self.generation.fetch_add(1, Ordering::Release);
            Ok(())
        }
        fn generation(&self) -> u64 {
            self.generation.load(Ordering::Acquire)
        }
    }
    async fn cache_view(
        context: &mut ExecutionContext,
        source: NodeDBConnection,
        store: LanceDBVectorStore,
    ) -> Result<NodeDBConnection> {
        let (_, generation) = source.load_with_generation(context).await?;
        let cache_key = format!("inspection::table::{}", uuid::Uuid::new_v4());
        let cached = CachedDB {
            db: Arc::new(RwLock::new(BufferedVectorStore::new(store, 0))),
        };
        let refresher = Arc::new(ViewRefresher {
            source,
            cache_key: cache_key.clone(),
            source_generation: AtomicU64::new(generation),
            generation: AtomicU64::new(0),
            lock: tokio::sync::Mutex::new(()),
        });
        let mut cache = context.cache.write().await;
        cache.insert(cache_key.clone(), Arc::new(cached));
        cache.insert(
            NodeDBConnection::refresh_cache_key(&cache_key),
            Arc::new(CachedDBRefreshHook::new(refresher)),
        );
        Ok(NodeDBConnection { cache_key })
    }
    pub async fn pin_table_source(
        context: &mut ExecutionContext,
        source: &NodeDBConnection,
    ) -> Result<PinnedTableSource> {
        let cached = source.load(context).await?;
        cached.ensure_flushed().await?;
        let store = cached.db.read().await.inner().clone();
        let scope = context
            .execution_cache
            .as_ref()
            .ok_or_else(|| anyhow!("Execution storage scope is unavailable"))?;
        let locator = TableLocator {
            app_id: scope.app_id.clone(),
            board_id: scope.board_id.clone(),
            connection_digest: content_digest(store.connection()?.uri().as_bytes()),
        };
        let reference = store.reference().await?;
        let snapshot = store
            .checkout(DatabaseSelector {
                branch: reference.branch,
                version: Some(reference.version),
                tag: None,
                read_only: true,
            })
            .await?;
        let reference = snapshot.reference().await?;
        let database = cache_view(context, source.clone(), snapshot).await?;
        Ok(PinnedTableSource {
            database,
            reference,
            locator: Some(locator),
        })
    }
    pub async fn inspect_table_schema(
        context: &mut ExecutionContext,
        source: &NodeDBConnection,
    ) -> Result<SourceTableSchema> {
        let source = pin_table_source(context, source).await?;
        let cached = source.database.load(context).await?;
        let schema = cached.db.read().await.schema().await?;
        let columns = schema
            .fields()
            .iter()
            .map(|field| TableColumnSchema {
                name: field.name().clone(),
                data_type: format!("{:?}", field.data_type()),
                nullable: field.is_nullable(),
            })
            .collect();
        Ok(SourceTableSchema { source, columns })
    }
    pub async fn read_rows(
        context: &mut ExecutionContext,
        source: &PinnedTableSource,
        budget: &TableBudget,
        page_size: usize,
    ) -> Result<Vec<Value>> {
        budget.validate()?;
        let mut rows = Vec::new();
        let mut bytes = 0usize;
        loop {
            let cached = source.database.load(context).await?;
            let page = cached
                .db
                .read()
                .await
                .filter(
                    "true",
                    None,
                    page_size.min(
                        budget
                            .maximum_rows
                            .saturating_add(1)
                            .saturating_sub(rows.len()),
                    ),
                    rows.len(),
                )
                .await?;
            if page.is_empty() {
                break;
            }
            for row in page {
                bytes = bytes
                    .checked_add(flow_like_types::json::to_vec(&row)?.len())
                    .ok_or_else(|| anyhow!("Table byte budget overflows"))?;
                if bytes > budget.maximum_bytes || rows.len() >= budget.maximum_rows {
                    return Err(anyhow!(
                        "Source table exceeds the row or byte budget; no partial dataset was prepared"
                    ));
                }
                rows.push(row);
            }
        }
        Ok(rows)
    }
    pub async fn prepare_table_dataset(
        context: &mut ExecutionContext,
        request: &TableDatasetRequest,
    ) -> Result<PreparedTableDataset> {
        let source = pin_table_source(context, &request.source).await?;
        let page_size = if matches!(request.mapping.features, FeatureSource::Tabular { .. }) {
            512
        } else {
            1
        };
        let rows = read_rows(context, &source, &request.budget, page_size).await?;
        prepare_rows(source, request, &rows)
    }
    pub async fn clone_experiment_table(
        context: &mut ExecutionContext,
        source: &PinnedTableSource,
        name: &str,
    ) -> Result<PinnedTableSource> {
        if context
            .execution_cache
            .as_ref()
            .is_some_and(|scope| scope.shadow)
        {
            return Err(anyhow!("Shadow execution cannot create experiment tables"));
        }
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || name == source.reference.table
        {
            return Err(anyhow!(
                "Clone requires a new bounded alphanumeric table name"
            ));
        }
        let rows = read_rows(context, source, &TableBudget::default(), 1).await?;
        let cached = source.database.load(context).await?;
        let connection = cached.db.read().await.inner().connection()?.clone();
        let mut store = LanceDBVectorStore::from_connection(connection, name.into()).await;
        if store.table_exists().await? {
            return Err(anyhow!("Clone destination already exists"));
        }
        store.insert(rows).await?;
        let reference = store.reference().await?;
        let database = cache_view(context, source.database.clone(), store).await?;
        Ok(PinnedTableSource {
            database,
            reference,
            locator: source.locator.clone(),
        })
    }
    pub async fn reopen_pinned_source(
        context: &mut ExecutionContext,
        source: &PinnedTableSource,
    ) -> Result<NodeDBConnection> {
        let locator = source.locator.as_ref().ok_or_else(|| {
            anyhow!(
                "Durable source locator is missing; supply the original scoped database connection"
            )
        })?;
        let scope = context
            .execution_cache
            .as_ref()
            .ok_or_else(|| anyhow!("Execution storage scope is unavailable"))?
            .clone();
        if locator.app_id != scope.app_id
            || locator.board_id != scope.board_id
            || !source.reference.pinned
            || !source.reference.read_only
        {
            return Err(anyhow!(
                "Dataset source belongs to a different execution scope or is not pinned"
            ));
        }
        let cache_present = context
            .cache
            .read()
            .await
            .contains_key(&source.database.cache_key);
        if cache_present {
            let cached = source.database.load(context).await?;
            let store = cached.db.read().await;
            if store.inner().reference().await? != source.reference
                || content_digest(store.inner().connection()?.uri().as_bytes())
                    != locator.connection_digest
            {
                return Err(anyhow!(
                    "Cached source differs from the pinned dataset reference"
                ));
            }
            return Ok(source.database.clone());
        }
        let callbacks = context.app_state.config.read().await.callbacks.clone();
        let mut matched = None;
        for user_scoped in [false, true] {
            let path = if user_scoped {
                scope.get_user_dir(false)?
            } else {
                scope.get_storage(false)?
            }
            .join("db");
            let builder = if let Some(credentials) = &context.credentials {
                if user_scoped {
                    credentials.to_db_scoped(&scope.sub, &scope.app_id).await
                } else {
                    credentials.to_db(&scope.app_id).await
                }
            } else {
                match if user_scoped {
                    callbacks.build_user_database.as_ref()
                } else {
                    callbacks.build_project_database.as_ref()
                } {
                    Some(build) => Ok(build(path)),
                    None => continue,
                }
            };
            let Ok(builder) = builder else {
                continue;
            };
            let Ok(connection) = context
                .app_state
                .with_lance_session(builder)
                .execute()
                .await
            else {
                continue;
            };
            if content_digest(connection.uri().as_bytes()) != locator.connection_digest {
                continue;
            }
            matched = Some(connection);
            break;
        }
        let connection=matched.ok_or_else(||anyhow!("The source connection is not available in the current authorized workspace; reconnect the original database"))?;
        let store = LanceDBVectorStore::from_connection_with_selector(
            connection,
            source.reference.table.clone(),
            DatabaseSelector {
                branch: source.reference.branch.clone(),
                version: Some(source.reference.version),
                tag: None,
                read_only: true,
            },
        )
        .await?;
        if store.reference().await? != source.reference {
            return Err(anyhow!(
                "Reopened source differs from the pinned dataset reference"
            ));
        }
        let cache_key = format!("inspection::reopened::{}", uuid::Uuid::new_v4());
        context.cache.write().await.insert(
            cache_key.clone(),
            Arc::new(CachedDB {
                db: Arc::new(RwLock::new(BufferedVectorStore::new(store, 0))),
            }),
        );
        Ok(NodeDBConnection { cache_key })
    }
    pub async fn reopen_latest_source(
        context: &mut ExecutionContext,
        source: &PinnedTableSource,
    ) -> Result<NodeDBConnection> {
        let pinned = reopen_pinned_source(context, source).await?;
        let cached = pinned.load(context).await?;
        let store = cached.db.read().await.inner().clone();
        let latest = store
            .checkout(DatabaseSelector {
                branch: source.reference.branch.clone(),
                version: None,
                tag: None,
                read_only: true,
            })
            .await?;
        cache_view(context, pinned, latest).await
    }
    fn table_name(
        experiment_id: &str,
        partition: &str,
        policy: &TableWritePolicy,
    ) -> Result<String> {
        let prefix = policy.table_prefix.as_deref().unwrap_or("ml_experiment");
        if prefix.is_empty()
            || prefix.len() > 48
            || experiment_id.trim().is_empty()
            || experiment_id.len() > 1024
            || partition.is_empty()
            || partition.len() > 32
            || !prefix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            || !partition
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        {
            return Err(anyhow!(
                "Experiment table names require a bounded alphanumeric prefix"
            ));
        }
        let identity = content_digest(experiment_id.as_bytes());
        Ok(format!("{prefix}_{}_{partition}", &identity[..16]))
    }
    pub async fn write_experiment_rows(
        context: &mut ExecutionContext,
        source: &PinnedTableSource,
        rows: Vec<Value>,
        experiment_id: &str,
        partition: &str,
        preprocessing_digest: &str,
        policy: &TableWritePolicy,
    ) -> Result<MaterializedTable> {
        if context
            .execution_cache
            .as_ref()
            .is_some_and(|scope| scope.shadow)
        {
            return Err(anyhow!("Shadow execution cannot write experiment tables"));
        }
        if rows.is_empty()
            || rows.len() > policy.maximum_rows
            || policy.maximum_rows > 1_000_000
            || policy.maximum_bytes > 1024 * 1024 * 1024
        {
            return Err(anyhow!("Invalid experiment table row/write budget"));
        }
        let mut bytes = 0usize;
        let mut largest_row = 1usize;
        let mut ids = HashSet::new();
        for row in &rows {
            if !ids.insert(identifier(field(row, "sample_id")?)?) {
                return Err(anyhow!("Experiment rows require unique sample_id values"));
            }
            let row_bytes = flow_like_types::json::to_vec(row)?.len();
            largest_row = largest_row.max(row_bytes);
            bytes = bytes
                .checked_add(row_bytes)
                .ok_or_else(|| anyhow!("Write byte count overflows"))?;
            if bytes > policy.maximum_bytes {
                return Err(anyhow!("Experiment table exceeds write byte budget"));
            }
        }
        let database = if let Some(destination) = &policy.allowed_destination {
            let expected = policy.expected_destination.as_ref().ok_or_else(|| {
                anyhow!("Existing table writes require its expected durable reference")
            })?;
            let cached = destination.load(context).await?;
            cached.ensure_flushed().await?;
            let store = cached.db.read().await;
            store.inner().ensure_writable()?;
            let actual = store.inner().reference().await?;
            if &actual != expected {
                return Err(anyhow!("Destination changed since write authorization"));
            }
            let escaped = "sample_id";
            let duplicate=store.inner().sql("destination",&format!("SELECT {escaped} FROM destination GROUP BY {escaped} HAVING {escaped} IS NULL OR COUNT(*) > 1 LIMIT 1")).await?.collect().await?;
            if duplicate.iter().any(|batch| batch.num_rows() > 0) {
                return Err(anyhow!(
                    "Destination sample_id values must be unique and non-null"
                ));
            }
            drop(store);
            cached
                .upsert_from(context, rows.clone(), "sample_id".into())
                .await?;
            cached.ensure_flushed().await?;
            destination.clone()
        } else {
            if !policy.create_tables {
                return Err(anyhow!("No experiment table writes are permitted"));
            }
            let cached = source.database.load(context).await?;
            let connection = cached.db.read().await.inner().connection()?.clone();
            let name = table_name(experiment_id, partition, policy)?;
            let mut store = LanceDBVectorStore::from_connection(connection, name).await;
            if store.table_exists().await? {
                let reference = store.reference().await?;
                store = store
                    .checkout(DatabaseSelector {
                        branch: reference.branch,
                        version: Some(reference.version),
                        tag: None,
                        read_only: true,
                    })
                    .await?;
                let expected: BTreeMap<_, _> = rows
                    .iter()
                    .map(|row| Ok((identifier(field(row, "sample_id")?)?, row)))
                    .collect::<Result<_>>()?;
                let mut found = HashSet::new();
                let mut read_bytes = 0usize;
                let page_size = (policy.maximum_bytes / 16 / largest_row).clamp(1, 512);
                loop {
                    let page = store
                        .filter(
                            "true",
                            None,
                            page_size.min(rows.len().saturating_add(1).saturating_sub(found.len())),
                            found.len(),
                        )
                        .await?;
                    if page.is_empty() {
                        break;
                    }
                    for row in page {
                        read_bytes = read_bytes
                            .checked_add(flow_like_types::json::to_vec(&row)?.len())
                            .ok_or_else(|| anyhow!("Existing output byte count overflows"))?;
                        let id = identifier(field(&row, "sample_id")?)?;
                        if read_bytes > policy.maximum_bytes
                            || !found.insert(id.clone())
                            || expected.get(&id).is_none_or(|value| **value != row)
                        {
                            return Err(anyhow!(
                                "Existing scoped experiment table differs from the requested rows; source data was not changed"
                            ));
                        }
                    }
                }
                if found.len() != rows.len() {
                    return Err(anyhow!("Existing scoped experiment table is incomplete"));
                }
            } else {
                store.insert(rows.clone()).await?;
            }
            cache_view(context, source.database.clone(), store).await?
        };
        let cached = database.load(context).await?;
        let store = cached.db.read().await.inner().clone();
        let reference = store.reference().await?;
        let pinned = store
            .checkout(DatabaseSelector {
                branch: reference.branch,
                version: Some(reference.version),
                tag: None,
                read_only: true,
            })
            .await?;
        let reference = pinned.reference().await?;
        let database = cache_view(context, database, pinned).await?;
        Ok(MaterializedTable {
            partition: partition.into(),
            database,
            reference,
            source: source.reference.clone(),
            rows: rows.len(),
            preprocessing_digest: preprocessing_digest.into(),
        })
    }
    pub async fn materialize_prepared_tables(
        context: &mut ExecutionContext,
        prepared: &PreparedTableDataset,
        experiment_id: &str,
        policy: &TableWritePolicy,
    ) -> Result<Vec<MaterializedTable>> {
        if !policy.create_tables && policy.allowed_destination.is_none() {
            return Ok(vec![]);
        }
        prepared.split.validate(&prepared.samples)?;
        let preprocessing = flow_like_types::json::to_string(&prepared.preprocessing)?;
        let preprocessing_digest = content_digest(preprocessing.as_bytes());
        let source = flow_like_types::json::to_string(&prepared.source.reference)?;
        let groups = [
            ("train", &prepared.split.train),
            ("validation", &prepared.split.validation),
            ("test", &prepared.split.test),
        ];
        let mut tables = Vec::new();
        let mut combined = Vec::new();
        for (partition, indices) in groups {
            let rows=indices.iter().map(|index|{
                let sample=&prepared.samples[*index];
                let mut row = prepared.rows.get(*index).and_then(Value::as_object).cloned().unwrap_or_default();
                if row.get("sample_id").is_some_and(|value|!identifier(value).is_ok_and(|id|id == sample.id)) {
                    return Err(anyhow!("Source sample_id differs from the mapped stable row ID"));
                }
                row.entry("sample_id".to_owned()).or_insert_with(||json!(sample.id));
                let metadata=json!({"group_id":sample.group_id,"timestamp_ms":sample.timestamp_ms,"partition":partition,"input":sample.input.values,"shape":sample.input.shape,"sample":flow_like_types::json::to_string(sample)?,"source_reference":source,"preprocessing_digest":preprocessing_digest,"experiment_id":experiment_id});
                for (name,value) in metadata.as_object().expect("metadata is an object") {
                    let name = if row.contains_key(name) {format!("__ml_{name}")}else{name.clone()};
                    if row.insert(name,value.clone()).is_some() {return Err(anyhow!("Source columns collide with reserved training metadata"));}
                }
                Ok(Value::Object(row))
            }).collect::<Result<Vec<_>>>()?;
            if policy.allowed_destination.is_some() {
                combined.extend(rows);
            } else {
                tables.push(
                    write_experiment_rows(
                        context,
                        &prepared.source,
                        rows,
                        experiment_id,
                        partition,
                        &preprocessing_digest,
                        policy,
                    )
                    .await?,
                );
            }
        }
        if policy.allowed_destination.is_some() {
            tables.push(
                write_experiment_rows(
                    context,
                    &prepared.source,
                    combined,
                    experiment_id,
                    "prepared",
                    &preprocessing_digest,
                    policy,
                )
                .await?,
            );
        }
        Ok(tables)
    }
}
#[cfg(feature = "execute")]
pub use table_io::{
    clone_experiment_table, inspect_table_schema, materialize_prepared_tables, pin_table_source,
    prepare_table_dataset, read_rows, reopen_latest_source, reopen_pinned_source,
    write_experiment_rows,
};

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_ml_native::preprocessing::Imputation;
    fn fixture() -> (PinnedTableSource, TableDatasetRequest, Vec<Value>) {
        let source = PinnedTableSource {
            database: NodeDBConnection {
                cache_key: "source".into(),
            },
            reference: DatabaseReference {
                table: "data".into(),
                branch: "main".into(),
                version: 1,
                read_only: true,
                pinned: true,
            },
            locator: None,
        };
        let request = TableDatasetRequest {
            source: source.database.clone(),
            stream_id: "inspection".into(),
            spec: InspectionSpec {
                id: "task".into(),
                description: "Classify measurements".into(),
                task: TaskKind::SensorClassification,
                labels: vec!["good".into(), "bad".into()],
                input_shape: vec![1],
                prediction_horizon_ms: None,
                minimum_examples: 3,
                minimum_examples_per_class: 1,
            },
            mapping: TableMapping {
                row_id: "id".into(),
                group_id: Some("group".into()),
                timestamp_ms: Some("time".into()),
                window_start_ms: None,
                window_end_ms: None,
                outcome: None,
                features: FeatureSource::Tabular {
                    options: TabularOptions {
                        numeric_columns: vec!["x".into()],
                        categorical_columns: vec!["category".into()],
                        imputation: Imputation::Median,
                        standardize: true,
                        maximum_categories_per_column: 10,
                        maximum_output_features: 20,
                    },
                },
                target: TargetSource::Column {
                    column: "target".into(),
                },
                provenance: LabelProvenance::Measured {
                    source: "test sensor".into(),
                },
            },
            split: TableSplitPolicy::Group {
                train_fraction: 0.6,
                validation_fraction: 0.2,
                seed: 42,
            },
            budget: TableBudget::default(),
            selection: DatasetSelection::default(),
            label_overrides: BTreeMap::new(),
        };
        let rows=(0..30).map(|index|json!({"id":index,"group":index/2,"time":index,"x":index,"z":index*2,"category":"shared","target":if index%2==0 {"good"}else{"bad"}})).collect();
        (source, request, rows)
    }
    #[test]
    fn feature_selection_preserves_split_and_held_out_rows_do_not_fit_or_profile() {
        let (source, mut request, mut rows) = fixture();
        let original = prepare_rows(source.clone(), &request, &rows).unwrap();
        for index in original.split.validation.iter().chain(&original.split.test) {
            rows[*index]["x"] = json!(1_000_000);
            rows[*index]["category"] = json!("test-only");
        }
        let changed = prepare_rows(source.clone(), &request, &rows).unwrap();
        assert_eq!(
            flow_like_types::json::to_value(&original.preprocessing).unwrap(),
            flow_like_types::json::to_value(&changed.preprocessing).unwrap()
        );
        assert_eq!(
            flow_like_types::json::to_value(&original.profile).unwrap(),
            flow_like_types::json::to_value(&changed.profile).unwrap()
        );
        let FeatureSource::Tabular { options } = &mut request.mapping.features else {
            unreachable!()
        };
        options.numeric_columns = vec!["z".into()];
        let subset = prepare_rows(source, &request, &rows).unwrap();
        assert_eq!(subset.split, original.split);
        subset.split.validate(&subset.samples).unwrap();
        assert_eq!(changed.profile.partition, "train");
    }
    #[test]
    fn target_leakage_duplicate_keys_and_partial_reads_are_rejected() {
        let (source, mut request, mut rows) = fixture();
        let FeatureSource::Tabular { options } = &mut request.mapping.features else {
            unreachable!()
        };
        options.numeric_columns = vec!["target".into()];
        assert!(prepare_rows(source.clone(), &request, &rows).is_err());
        let (_, request, _) = fixture();
        rows[1]["id"] = rows[0]["id"].clone();
        assert!(prepare_rows(source.clone(), &request, &rows).is_err());
        let (_, mut request, rows) = fixture();
        request.budget.maximum_rows = 3;
        assert!(prepare_rows(source, &request, &rows).is_err());
    }
    #[test]
    fn temporal_splits_purge_groups_and_future_outcomes() {
        let (source, mut request, mut rows) = fixture();
        request.split = TableSplitPolicy::Time {
            train_end_ms: 10,
            validation_end_ms: 20,
            embargo_ms: 1,
        };
        request.spec.task = TaskKind::SequenceForecast;
        request.spec.labels.clear();
        request.spec.prediction_horizon_ms = Some(2);
        request.mapping.target = TargetSource::Columns {
            columns: vec!["target".into()],
        };
        request.mapping.outcome = Some("outcome".into());
        for (index, row) in rows.iter_mut().enumerate() {
            row["target"] = json!(index + 2);
            row["outcome"] = json!({"target_start_ms":index+2,"target_end_ms":index+2,"available_at_ms":index+2});
        }
        let prepared = prepare_rows(source, &request, &rows).unwrap();
        assert!(prepared.split.train.iter().all(|index| {
            prepared.samples[*index]
                .outcome
                .as_ref()
                .unwrap()
                .available_at_ms
                < 10
        }));
        assert!(prepared.split.validation.iter().all(|index| {
            prepared.samples[*index].window_start_ms >= 11
                && prepared.samples[*index]
                    .outcome
                    .as_ref()
                    .unwrap()
                    .available_at_ms
                    < 20
        }));
        assert!(!prepared.split.purged.is_empty());
        prepared.split.validate(&prepared.samples).unwrap();
    }

    fn feature_plan() -> super::super::feature_engineering::FeaturePlan {
        flow_like_types::json::from_value(json!({
            "pipeline":{"steps":[
                {"kind":"derive_columns","columns":[{"name":"x_squared","expression":{"kind":"binary","op":"multiply","left":{"kind":"column","name":"x"},"right":{"kind":"column","name":"x"}}}]},
                {"kind":"window_features","group_by":["group"],"timestamp_column":"time","features":[{"name":"lag_x","column":"x","operation":{"kind":"lag","steps":1}}]},
                {"kind":"join_sources","alias":"calibration","keys":[{"left":"category","right":"category"}],"join":{"kind":"exact"},"columns":[{"source":"offset","name":"calibration_offset"}]}
            ]},
            "numeric_columns":["x","x_squared","lag_x","calibration_offset"],"categorical_columns":[],
            "standardize":false,"imputation":{"kind":"constant","value":0.0}
        })).unwrap()
    }

    #[test]
    fn engineered_features_preserve_names_splits_and_replay_window_state() {
        use super::super::feature_engineering::{PinnedFeatureSource, prepare_feature_rows};
        let (source, request, rows) = fixture();
        let side = vec![PinnedFeatureSource {
            alias: "calibration".into(),
            source: source.clone(),
            allowed_columns: vec!["category".into(), "offset".into()],
        }];
        let side_rows = BTreeMap::from([(
            "calibration".into(),
            vec![json!({"category":"shared","offset":2.5})],
        )]);
        let original = prepare_rows(source.clone(), &request, &rows).unwrap();
        let prepared =
            prepare_feature_rows(source, &request, &rows, &feature_plan(), side, &side_rows)
                .unwrap();
        assert_eq!(prepared.split, original.split);
        assert_eq!(prepared.raw_content_digests, original.raw_content_digests);
        assert_eq!(prepared.raw_rows, rows);
        for index in prepared
            .split
            .train
            .iter()
            .chain(&prepared.split.validation)
            .chain(&prepared.split.test)
        {
            assert_eq!(prepared.rows[*index]["x"], rows[*index]["x"]);
            assert_eq!(
                prepared.rows[*index]["x_squared"].as_f64().unwrap(),
                (*index * *index) as f64
            );
            assert_eq!(prepared.rows[*index]["calibration_offset"], json!(2.5));
            assert_eq!(
                prepared.samples[*index].annotation,
                original.samples[*index].annotation
            );
        }
        let batch = transform_table_rows(&prepared.preprocessing, &rows[..2], 100).unwrap();
        let (first, state) =
            transform_table_rows_with_state(&prepared.preprocessing, &rows[..1], 100, None, true)
                .unwrap();
        let (second, _) =
            transform_table_rows_with_state(&prepared.preprocessing, &rows[1..2], 100, state, true)
                .unwrap();
        assert_eq!(batch, first.into_iter().chain(second).collect::<Vec<_>>());
        let reloaded: TablePreprocessing =
            flow_like_types::json::from_value(json!(prepared.preprocessing)).unwrap();
        assert_eq!(
            batch,
            transform_table_rows(&reloaded, &rows[..2], 100).unwrap()
        );
    }

    #[test]
    fn engineered_fit_never_uses_held_out_values_and_blocks_target_expressions() {
        use super::super::feature_engineering::prepare_feature_rows;
        let (source, request, mut rows) = fixture();
        let mut plan = feature_plan();
        plan.pipeline.steps.truncate(1);
        plan.numeric_columns = vec!["x", "x_squared"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        plan.standardize = true;
        plan.pca_components = Some(1);
        let original = prepare_feature_rows(
            source.clone(),
            &request,
            &rows,
            &plan,
            vec![],
            &BTreeMap::new(),
        )
        .unwrap();
        for index in original.split.validation.iter().chain(&original.split.test) {
            rows[*index]["x"] = json!(1_000_000);
        }
        let changed = prepare_feature_rows(
            source.clone(),
            &request,
            &rows,
            &plan,
            vec![],
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(json!(original.preprocessing), json!(changed.preprocessing));
        assert_eq!(original.spec.input_shape, vec![1]);
        let index = original.split.train[0];
        assert!(original.rows[index].get("__ml_pca_1").is_some());
        let replay = transform_table_rows(
            &original.preprocessing,
            &original.raw_rows[index..index + 1],
            32,
        )
        .unwrap();
        assert_eq!(replay[0], original.samples[index].input);
        plan.limits.maximum_columns = original.rows[index].as_object().unwrap().len() - 1;
        assert!(
            prepare_feature_rows(
                source.clone(),
                &request,
                &rows,
                &plan,
                vec![],
                &BTreeMap::new(),
            )
            .unwrap_err()
            .to_string()
            .contains("column budget")
        );
        plan.limits = Default::default();
        let bad=flow_like_types::json::from_value(json!({"steps":[{"kind":"derive_columns","columns":[{"name":"leak","expression":{"kind":"column","name":"target"}}]}]})).unwrap();
        plan.pipeline = bad;
        plan.numeric_columns = vec!["leak".into()];
        assert!(
            prepare_feature_rows(source, &request, &rows, &plan, vec![], &BTreeMap::new()).is_err()
        );
    }

    #[test]
    fn learning_selection_and_review_overlays_preserve_raw_identity() {
        let (source, mut request, rows) = fixture();
        let original = prepare_rows(source.clone(), &request, &rows).unwrap();
        let audit = original.split.test[0];
        request
            .selection
            .excluded_groups
            .push(original.samples[audit].group_id.clone());
        let trained = original.split.test[2];
        request
            .selection
            .ineligible_test_ids
            .push(original.samples[trained].id.clone());
        let reviewed = original.split.train[0];
        request.label_overrides.insert(
            original.samples[reviewed].id.clone(),
            TableLabelOverride {
                annotation: Annotation::Class { class_id: 1 },
                provenance: LabelProvenance::Reviewed {
                    reviewer: "quality".into(),
                    reviewed_at_ms: 100,
                },
                annotation_revision: 2,
                outcome: None,
                available_at_ms: Some(100),
            },
        );
        let changed = prepare_rows(source, &request, &rows).unwrap();
        assert!(
            !changed
                .split
                .train
                .iter()
                .chain(&changed.split.validation)
                .chain(&changed.split.test)
                .any(|index| changed.samples[*index].group_id == original.samples[audit].group_id)
        );
        assert!(!changed.split.test.contains(&trained));
        assert_eq!(
            changed.samples[reviewed].annotation,
            Annotation::Class { class_id: 1 }
        );
        assert_eq!(changed.raw_content_digests, original.raw_content_digests);
        assert_eq!(changed.rows, rows);
        changed.split.validate(&changed.samples).unwrap();
    }

    #[test]
    fn engineered_datasets_keep_excluded_rows_out_of_fitting_and_materialization() {
        let (source, mut request, rows) = fixture();
        let original = prepare_rows(source.clone(), &request, &rows).unwrap();
        let excluded = original.split.test[0];
        request.selection.excluded_groups = vec![original.samples[excluded].group_id.clone()];
        let mut plan = feature_plan();
        plan.pipeline.steps.truncate(1);
        plan.numeric_columns = vec!["x_squared".into()];
        let prepared = super::super::feature_engineering::prepare_feature_rows(
            source.clone(),
            &request,
            &rows,
            &plan,
            vec![],
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(prepared.split.purged.contains(&excluded));
        assert!(prepared.rows[excluded]["x_squared"].is_null());
        assert_eq!(prepared.raw_rows[excluded], rows[excluded]);
        prepared.split.validate(&prepared.samples).unwrap();
        plan.limits.maximum_rows = rows.len() - 1;
        assert!(
            super::super::feature_engineering::prepare_feature_rows(
                source,
                &request,
                &rows,
                &plan,
                vec![],
                &BTreeMap::new(),
            )
            .is_err()
        );
    }

    #[test]
    fn stateful_replay_bounds_the_whole_transformed_batch() {
        let (source, request, rows) = fixture();
        let mut plan = feature_plan();
        plan.pipeline.steps.truncate(1);
        plan.numeric_columns = vec!["x_squared".into()];
        let mut prepared = super::super::feature_engineering::prepare_feature_rows(
            source,
            &request,
            &rows,
            &plan,
            vec![],
            &BTreeMap::new(),
        )
        .unwrap();
        let TablePreprocessing::Engineered { pipeline, .. } = &mut prepared.preprocessing else {
            unreachable!()
        };
        pipeline.limits.maximum_bytes = rows
            .iter()
            .map(|row| serialized_json_size(row).unwrap())
            .sum();
        let error =
            transform_table_rows_with_state(&prepared.preprocessing, &rows, 1024, None, true)
                .unwrap_err();
        assert!(error.to_string().contains("Transformed feature batch"));
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn local_table_pin_materialization_retry_and_reopen_preserve_source_versions() {
        use ahash::AHashMap;
        use flow_like::{
            flow::{
                board::ExecutionStage,
                execution::{
                    LogLevel, Run,
                    context::{ExecutionContext, ExecutionContextCache},
                    internal_node::InternalNode,
                },
                node::NodeLogic,
            },
            profile::Profile,
            state::{FlowLikeConfig, FlowLikeState, FlowLikeStores},
            utils::http::HTTPClient,
        };
        use flow_like_catalog_core::CachedDB;
        use flow_like_storage::{
            databases::vector::{
                VectorStore, buffered::BufferedVectorStore, lancedb::LanceDBVectorStore,
            },
            object_store::path::Path,
        };
        use flow_like_types::sync::{Mutex, RwLock};
        use std::sync::{Arc, Weak};
        struct Directory(std::path::PathBuf);
        impl Drop for Directory {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let directory = Directory(std::env::temp_dir().join(format!(
            "flow-like-training-tables-{}",
            uuid::Uuid::new_v4()
        )));
        std::fs::create_dir_all(&directory.0).unwrap();
        let location = directory.0.to_string_lossy().into_owned();
        let mut config = FlowLikeConfig::new();
        let reopen_location = location.clone();
        config.register_build_project_database(Arc::new(move |_| {
            flow_like_storage::lancedb::connect(&reopen_location)
        }));
        let state = Arc::new(FlowLikeState::new(
            config,
            HTTPClient::new_without_refetch(),
        ));
        let logic: Arc<dyn NodeLogic> = Arc::new(super::super::data::MakeInspectionSampleNode);
        let node = Arc::new(InternalNode::new(
            logic.get_node(),
            AHashMap::new(),
            logic,
            AHashMap::new(),
        ));
        let mut nodes = AHashMap::new();
        nodes.insert(node.node_id().to_string(), node.clone());
        let run: Weak<Mutex<Run>> = Weak::new();
        let mut context = ExecutionContext::new(
            Arc::new(nodes),
            &run,
            &state,
            &node,
            &Arc::new(Mutex::new(AHashMap::new())),
            &Arc::new(RwLock::new(AHashMap::new())),
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
            None,
            Arc::new(AHashMap::new()),
            None,
        )
        .await;
        context.execution_cache = Some(ExecutionContextCache {
            stores: FlowLikeStores::default(),
            app_id: "table-test".into(),
            model_usage_app_id: None,
            board_dir: Path::from("board"),
            board_id: "board".into(),
            node_id: node.shared_node_id(),
            sub: "tester".into(),
            shadow: false,
        });
        let connection = flow_like_storage::lancedb::connect(&location)
            .execute()
            .await
            .unwrap();
        let mut store = LanceDBVectorStore::from_connection(connection, "original".into()).await;
        let (_, mut request, rows) = fixture();
        store.insert(rows.clone()).await.unwrap();
        let initial = store.reference().await.unwrap();
        let database = NodeDBConnection {
            cache_key: "original".into(),
        };
        context.cache.write().await.insert(
            database.cache_key.clone(),
            Arc::new(CachedDB {
                db: Arc::new(RwLock::new(BufferedVectorStore::new(store, 0))),
            }),
        );
        request.source = database.clone();
        let prepared = prepare_table_dataset(&mut context, &request).await.unwrap();
        let callback = context
            .app_state
            .config
            .write()
            .await
            .callbacks
            .build_project_database
            .take();
        let cached_pin = reopen_pinned_source(&mut context, &prepared.source)
            .await
            .unwrap();
        assert_eq!(cached_pin.cache_key, prepared.source.database.cache_key);
        let mut mismatched = prepared.source.clone();
        mismatched.reference.version += 1;
        assert!(
            reopen_pinned_source(&mut context, &mismatched)
                .await
                .is_err()
        );
        context
            .app_state
            .config
            .write()
            .await
            .callbacks
            .build_project_database = callback;
        let cloned = clone_experiment_table(&mut context, &prepared.source, "independent_clone")
            .await
            .unwrap();
        assert!(!cloned.reference.read_only);
        assert_eq!(
            cloned
                .database
                .load(&mut context)
                .await
                .unwrap()
                .db
                .read()
                .await
                .filter("true", None, 100, 0)
                .await
                .unwrap()
                .len(),
            rows.len()
        );
        let policy = TableWritePolicy::default();
        let materialized =
            materialize_prepared_tables(&mut context, &prepared, "experiment-1", &policy)
                .await
                .unwrap();
        assert_eq!(materialized.len(), 3);
        for table in &materialized {
            assert!(table.reference.pinned && table.reference.read_only);
            assert_ne!(table.reference.table, initial.table);
        }
        let retried = materialize_prepared_tables(&mut context, &prepared, "experiment-1", &policy)
            .await
            .unwrap();
        assert_eq!(
            materialized
                .iter()
                .map(|table| &table.reference)
                .collect::<Vec<_>>(),
            retried
                .iter()
                .map(|table| &table.reference)
                .collect::<Vec<_>>()
        );
        assert!(write_experiment_rows(&mut context,&prepared.source,vec![json!({"sample_id":prepared.samples[prepared.split.train[0]].id,"unexpected":"change"})],"experiment-1","train","different",&policy).await.is_err());
        let mut plan = feature_plan();
        plan.pipeline.steps.truncate(1);
        plan.numeric_columns = vec!["x".into(), "x_squared".into()];
        let engineered = super::super::feature_engineering::prepare_feature_rows(
            prepared.source.clone(),
            &request,
            &rows,
            &plan,
            vec![],
            &BTreeMap::new(),
        )
        .unwrap();
        let engineered_tables =
            materialize_prepared_tables(&mut context, &engineered, "experiment-features", &policy)
                .await
                .unwrap();
        for table in engineered_tables {
            let stored = table.database.load(&mut context).await.unwrap();
            let stored_rows = stored
                .db
                .read()
                .await
                .filter("true", None, 100, 0)
                .await
                .unwrap();
            for row in stored_rows {
                let x = row["x"].as_f64().unwrap();
                assert_eq!(row["x_squared"].as_f64().unwrap(), x * x);
                assert!(row.get("target").is_some());
                assert!(row.get("category").is_some());
            }
        }
        let cached = database.load(&mut context).await.unwrap();
        assert_eq!(
            cached.db.read().await.inner().reference().await.unwrap(),
            initial
        );
        let mut changed = rows[0].clone();
        changed["x"] = json!(9999);
        cached
            .upsert_from(&context, vec![changed], "id".into())
            .await
            .unwrap();
        cached.ensure_flushed().await.unwrap();
        assert!(
            cached
                .db
                .read()
                .await
                .inner()
                .reference()
                .await
                .unwrap()
                .version
                > initial.version
        );
        context.cache.write().await.clear();
        let reopened = reopen_pinned_source(&mut context, &prepared.source)
            .await
            .unwrap();
        request.source = reopened;
        let again = prepare_table_dataset(&mut context, &request).await.unwrap();
        assert_eq!(again.samples, prepared.samples);
        assert_eq!(again.digest, prepared.digest);
        request.source = reopen_latest_source(&mut context, &prepared.source)
            .await
            .unwrap();
        let latest = prepare_table_dataset(&mut context, &request).await.unwrap();
        assert!(latest.source.reference.version > prepared.source.reference.version);
        assert!(
            latest
                .raw_rows
                .iter()
                .any(|row| row["id"] == 0 && row["x"] == 9999)
        );
        context.execution_cache.as_mut().unwrap().shadow = true;
        assert!(
            materialize_prepared_tables(&mut context, &again, "shadow", &policy)
                .await
                .is_err()
        );
    }
}

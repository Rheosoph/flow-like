use super::auto_training_tables::{
    FeatureSource, MaterializedTable, PinnedTableSource, PreparedTableDataset, TableDatasetRequest,
    TablePreprocessing, TableWritePolicy,
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_ml_native::feature_engineering::FeatureStep;
use flow_like_ml_native::{
    feature_engineering::{FeatureLimits, FeaturePipeline, FeaturePolicy},
    preprocessing::Imputation,
};
use flow_like_types::{Result, Value, anyhow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FeaturePlan {
    #[serde(default)]
    pub pipeline: FeaturePipeline,
    pub numeric_columns: Vec<String>,
    pub categorical_columns: Vec<String>,
    #[serde(default)]
    pub imputation: Imputation,
    pub standardize: bool,
    #[serde(default)]
    pub pca_components: Option<usize>,
    #[serde(default)]
    pub limits: FeatureLimits,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FeatureSourceInput {
    pub alias: String,
    pub source: NodeDBConnection,
    pub allowed_columns: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PinnedFeatureSource {
    pub alias: String,
    pub source: PinnedTableSource,
    pub allowed_columns: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct FeatureTransformRequest {
    pub dataset: TableDatasetRequest,
    pub plan: FeaturePlan,
    #[serde(default)]
    pub sources: Vec<FeatureSourceInput>,
    pub output_id: String,
    #[serde(default)]
    pub tables: TableWritePolicy,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct FeatureTransformResult {
    pub prepared: PreparedTableDataset,
    pub tables: Vec<MaterializedTable>,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct FitPreprocessingRequest {
    pub dataset: TableDatasetRequest,
    pub plan: FeaturePlan,
    #[serde(default)]
    pub sources: Vec<FeatureSourceInput>,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct MaterializeDatasetRequest {
    pub prepared: PreparedTableDataset,
    pub output_id: String,
    #[serde(default)]
    pub tables: TableWritePolicy,
}

fn policy(request: &TableDatasetRequest, sources: &[PinnedFeatureSource]) -> Result<FeaturePolicy> {
    let FeatureSource::Tabular { options } = &request.mapping.features else {
        return Err(anyhow!(
            "Feature engineering requires a tabular source mapping"
        ));
    };
    let mut forbidden = vec![request.mapping.row_id.clone()];
    if let Some(value) = &request.mapping.outcome {
        forbidden.push(value.clone());
    }
    match &request.mapping.target {
        super::auto_training_tables::TargetSource::Column { column }
        | super::auto_training_tables::TargetSource::Annotation { column } => {
            forbidden.push(column.clone())
        }
        super::auto_training_tables::TargetSource::Columns { columns } => {
            forbidden.extend(columns.clone())
        }
        super::auto_training_tables::TargetSource::Sample => {
            return Err(anyhow!(
                "Feature engineering needs an explicit target mapping"
            ));
        }
    }
    let mut metadata = vec![request.mapping.row_id.clone()];
    metadata.extend(
        [
            &request.mapping.group_id,
            &request.mapping.timestamp_ms,
            &request.mapping.window_start_ms,
            &request.mapping.window_end_ms,
        ]
        .into_iter()
        .flatten()
        .cloned(),
    );
    let mut aliases = BTreeMap::new();
    for source in sources {
        if source.alias.trim().is_empty()
            || source.alias.len() > 128
            || aliases
                .insert(source.alias.clone(), source.allowed_columns.clone())
                .is_some()
        {
            return Err(anyhow!("Feature sources need unique bounded aliases"));
        }
    }
    Ok(FeaturePolicy {
        allowed_input_columns: options
            .numeric_columns
            .iter()
            .chain(&options.categorical_columns)
            .cloned()
            .collect(),
        forbidden_columns: forbidden,
        metadata_columns: metadata,
        allowed_sources: aliases,
    })
}

pub fn prepare_feature_rows(
    source: PinnedTableSource,
    request: &TableDatasetRequest,
    rows: &[Value],
    plan: &FeaturePlan,
    sources: Vec<PinnedFeatureSource>,
    side_rows: &BTreeMap<String, Vec<Value>>,
) -> Result<PreparedTableDataset> {
    let policy = policy(request, &sources)?;
    let mut limits = plan.limits.clone();
    limits.maximum_rows = limits.maximum_rows.min(request.budget.maximum_rows);
    limits.maximum_bytes = limits.maximum_bytes.min(request.budget.maximum_bytes);
    let fitted = plan.pipeline.fit(side_rows, &policy, &limits)?;
    let generated = plan.pipeline.generated_columns();
    let allowed: HashSet<_> = policy
        .allowed_input_columns
        .iter()
        .chain(&generated)
        .collect();
    if plan
        .numeric_columns
        .iter()
        .chain(&plan.categorical_columns)
        .any(|name| !allowed.contains(name))
    {
        return Err(anyhow!(
            "Selected features must be authorized inputs or generated columns"
        ));
    }
    let mut bounded_request = request.clone();
    bounded_request.budget.maximum_rows = limits.maximum_rows;
    bounded_request.budget.maximum_bytes = limits.maximum_bytes;
    let original =
        super::auto_training_tables::prepare_identity_rows(&source, &bounded_request, rows)?;
    let mut transformed = rows.to_vec();
    // Reset causal history at partition boundaries; held-out values cannot enter training.
    for indices in [
        &original.split.train,
        &original.split.validation,
        &original.split.test,
    ] {
        let input: Vec<_> = indices.iter().map(|index| rows[*index].clone()).collect();
        let output = fitted.transform_rows(&input)?;
        if output.len() != indices.len() {
            return Err(anyhow!("Feature pipeline changed row membership"));
        }
        for (index, row) in indices.iter().zip(output) {
            transformed[*index] = row;
        }
    }
    for index in &original.split.purged {
        let row = transformed[*index]
            .as_object_mut()
            .ok_or_else(|| anyhow!("Feature rows must be objects"))?;
        for column in &generated {
            row.entry(column.clone()).or_insert(Value::Null);
        }
    }
    let mut prepared_request = bounded_request;
    let FeatureSource::Tabular { options } = &mut prepared_request.mapping.features else {
        unreachable!()
    };
    options.numeric_columns = plan.numeric_columns.clone();
    options.categorical_columns = plan.categorical_columns.clone();
    options.standardize = plan.standardize;
    options.imputation = plan.imputation.clone();
    let mut prepared =
        super::auto_training_tables::prepare_rows(source, &prepared_request, &transformed)?;
    prepared.raw_content_digests = original.raw_content_digests;
    prepared.raw_rows = rows.to_vec();
    let projection = if let Some(components) = plan.pca_components {
        let inputs: Vec<_> = prepared
            .samples
            .iter()
            .map(|sample| sample.input.clone())
            .collect();
        let projection = flow_like_ml_native::reduction::FittedPca::fit(
            &inputs,
            &prepared.split.train,
            components,
        )?;
        let projected =
            projection.transform_rows(&inputs, request.budget.maximum_tensor_elements)?;
        for ((sample, row), input) in prepared
            .samples
            .iter_mut()
            .zip(&mut prepared.rows)
            .zip(projected)
        {
            for (component, value) in input.values.iter().enumerate() {
                let name = format!("__ml_pca_{}", component + 1);
                if row
                    .as_object_mut()
                    .ok_or_else(|| anyhow!("Feature row must be an object"))?
                    .insert(name, flow_like_types::json::json!(value))
                    .is_some()
                {
                    return Err(anyhow!("Source column collides with generated PCA output"));
                }
            }
            sample.input = input;
        }
        prepared.spec.input_shape = vec![components];
        Some(projection)
    } else {
        None
    };
    let mut output_bytes = 0usize;
    for row in &prepared.rows {
        if row.as_object().map_or(0, |row| row.len()) > limits.maximum_columns {
            return Err(anyhow!("Prepared features exceed the column budget"));
        }
        output_bytes = output_bytes
            .checked_add(super::auto_training_tables::serialized_json_size(row)?)
            .ok_or_else(|| anyhow!("Feature byte budget overflow"))?;
        if output_bytes > limits.maximum_bytes {
            return Err(anyhow!("Prepared features exceed the byte budget"));
        }
    }
    prepared.preprocessing = TablePreprocessing::Engineered {
        pipeline: fitted,
        preprocessing: Box::new(prepared.preprocessing),
        projection,
        plan: Some(Box::new(plan.clone())),
    };
    prepared.feature_sources = sources;
    prepared.digest = flow_like_ml_core::content_digest(&flow_like_types::json::to_vec(&(
        &prepared.source.reference,
        &prepared.feature_sources,
        &prepared.samples,
        &prepared.preprocessing,
        &prepared.split,
    ))?);
    Ok(prepared)
}

#[cfg(feature = "execute")]
pub async fn prepare_feature_dataset(
    context: &mut ExecutionContext,
    request: &TableDatasetRequest,
    plan: &FeaturePlan,
    sources: &[FeatureSourceInput],
) -> Result<PreparedTableDataset> {
    let source = super::auto_training_tables::pin_table_source(context, &request.source).await?;
    let pinned = pin_feature_sources(context, sources).await?;
    prepare_pinned_feature_dataset(context, source, request, plan, pinned).await
}

#[cfg(feature = "execute")]
pub async fn pin_feature_sources(
    context: &mut ExecutionContext,
    sources: &[FeatureSourceInput],
) -> Result<Vec<PinnedFeatureSource>> {
    if sources.len() > 32 {
        return Err(anyhow!("At most 32 authorized side sources are supported"));
    }
    let mut pinned = Vec::new();
    let mut aliases = HashSet::new();
    for input in sources {
        if input.alias.is_empty() || input.alias.len() > 128 || !aliases.insert(&input.alias) {
            return Err(anyhow!("Feature source aliases must be unique and bounded"));
        }
        pinned.push(PinnedFeatureSource {
            alias: input.alias.clone(),
            source: super::auto_training_tables::pin_table_source(context, &input.source).await?,
            allowed_columns: input.allowed_columns.clone(),
        });
    }
    Ok(pinned)
}

#[cfg(feature = "execute")]
pub async fn prepare_pinned_feature_dataset(
    context: &mut ExecutionContext,
    mut source: PinnedTableSource,
    request: &TableDatasetRequest,
    plan: &FeaturePlan,
    mut sources: Vec<PinnedFeatureSource>,
) -> Result<PreparedTableDataset> {
    source.database = super::auto_training_tables::reopen_pinned_source(context, &source).await?;
    let rows =
        super::auto_training_tables::read_rows(context, &source, &request.budget, 512).await?;
    let mut side_rows = BTreeMap::new();
    let mut aggregate_bytes = 0usize;
    for side in &mut sources {
        side.source.database =
            super::auto_training_tables::reopen_pinned_source(context, &side.source).await?;
        let mut budget = request.budget.clone();
        budget.maximum_rows = budget.maximum_rows.min(plan.limits.maximum_join_rows);
        budget.maximum_bytes = budget.maximum_bytes.min(plan.limits.maximum_bytes);
        let rows =
            super::auto_training_tables::read_rows(context, &side.source, &budget, 512).await?;
        aggregate_bytes = aggregate_bytes
            .checked_add(flow_like_types::json::to_vec(&rows)?.len())
            .ok_or_else(|| anyhow!("Side source byte count overflow"))?;
        if aggregate_bytes > plan.limits.maximum_bytes {
            return Err(anyhow!("Combined feature sources exceed the byte budget"));
        }
        if side_rows.insert(side.alias.clone(), rows).is_some() {
            return Err(anyhow!("Duplicate feature source alias"));
        }
    }
    prepare_feature_rows(source, request, &rows, plan, sources, &side_rows)
}

#[cfg(feature = "execute")]
pub async fn transform_and_materialize(
    context: &mut ExecutionContext,
    request: FeatureTransformRequest,
) -> Result<FeatureTransformResult> {
    let prepared =
        prepare_feature_dataset(context, &request.dataset, &request.plan, &request.sources).await?;
    let tables = super::auto_training_tables::materialize_prepared_tables(
        context,
        &prepared,
        &request.output_id,
        &request.tables,
    )
    .await?;
    Ok(FeatureTransformResult { prepared, tables })
}

#[cfg(feature = "training")]
pub async fn derive_pipeline_candidate(
    context: &mut ExecutionContext,
    id: &str,
    plan: FeaturePlan,
) -> Result<()> {
    let repo = super::auto_training::experiment_repository(context, id)?;
    let experiment = repo.get_experiment(id)?;
    let original_limits: FeatureLimits = experiment
        .request
        .context
        .pointer("/feature_plan/limits")
        .filter(|value| !value.is_null())
        .map(|value| flow_like_types::json::from_value(value.clone()))
        .transpose()?
        .unwrap_or_default();
    let limits = &plan.limits;
    if limits.maximum_rows > original_limits.maximum_rows
        || limits.maximum_bytes > original_limits.maximum_bytes
        || limits.maximum_columns > original_limits.maximum_columns
        || limits.maximum_expression_nodes > original_limits.maximum_expression_nodes
        || limits.maximum_history_rows > original_limits.maximum_history_rows
        || limits.maximum_groups > original_limits.maximum_groups
        || limits.maximum_join_rows > original_limits.maximum_join_rows
        || limits.maximum_duration_ms > original_limits.maximum_duration_ms
    {
        return Err(anyhow!(
            "A feature proposal cannot increase the operator's transformation limits"
        ));
    }
    let request: TableDatasetRequest =
        flow_like_types::json::from_value(experiment.request.context["dataset_request"].clone())?;
    let source: PinnedTableSource =
        flow_like_types::json::from_value(experiment.request.context["pinned_source"].clone())?;
    let sources: Vec<PinnedFeatureSource> = flow_like_types::json::from_value(
        experiment
            .request
            .context
            .get("feature_sources")
            .cloned()
            .unwrap_or_else(|| flow_like_types::json::json!([])),
    )?;
    let prepared =
        prepare_pinned_feature_dataset(context, source, &request, &plan, sources).await?;
    super::auto_training::register_prepared_candidate(context, id, prepared).await
}

#[crate::register_node]
#[derive(Default)]
pub struct DeriveColumnsNode;

#[crate::register_node]
#[derive(Default)]
pub struct WindowFeaturesNode;

#[crate::register_node]
#[derive(Default)]
pub struct JoinSourcesNode;

macro_rules! feature_node {
    ($node:ident,$id:literal,$name:literal,$function:literal,$variant:ident) => {
        #[flow_like_types::async_trait]
        impl NodeLogic for $node {
            fn get_node(&self) -> Node {
                super::operation_node::<FeatureTransformRequest, FeatureTransformResult>(
                    $id,
                    $name,
                    "Create named feature columns in pinned derived tables",
                    $function,
                    "AI/ML/Feature Engineering",
                )
            }
            async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
                #[cfg(feature = "execute")]
                {
                    context.deactivate_exec_pin("exec_out").await?;
                    let request: FeatureTransformRequest = context.evaluate_pin("request").await?;
                    if !request
                        .plan
                        .pipeline
                        .steps
                        .iter()
                        .any(|step| matches!(step, FeatureStep::$variant { .. }))
                    {
                        return Err(anyhow!(
                            "The feature plan does not contain this node's operation"
                        ));
                    }
                    let result = transform_and_materialize(context, request).await?;
                    context
                        .set_pin_value("result", flow_like_types::json::json!(result))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                    Ok(())
                }
                #[cfg(not(feature = "execute"))]
                {
                    let _ = context;
                    Err(anyhow!("Feature table execution requires a native build"))
                }
            }
        }
    };
}
feature_node!(
    DeriveColumnsNode,
    "ml_derive_columns",
    "Derive Columns",
    "deriveColumns",
    DeriveColumns
);
feature_node!(
    WindowFeaturesNode,
    "ml_window_features",
    "Window Features",
    "windowFeatures",
    WindowFeatures
);
feature_node!(
    JoinSourcesNode,
    "ml_join_sources",
    "Join Sources",
    "joinSources",
    JoinSources
);

#[crate::register_node]
#[derive(Default)]
pub struct FitPreprocessingNode;
#[flow_like_types::async_trait]
impl NodeLogic for FitPreprocessingNode {
    fn get_node(&self) -> Node {
        super::operation_node::<FitPreprocessingRequest, PreparedTableDataset>(
            "ml_fit_preprocessing",
            "Fit Preprocessing",
            "Fit imputation, encoding and scaling on training rows",
            "fitPreprocessing",
            "AI/ML/Feature Engineering",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let request: FitPreprocessingRequest = context.evaluate_pin("request").await?;
            let result =
                prepare_feature_dataset(context, &request.dataset, &request.plan, &request.sources)
                    .await?;
            context
                .set_pin_value("result", flow_like_types::json::json!(result))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "execute"))]
        {
            let _ = context;
            Err(anyhow!("Feature preprocessing requires a native build"))
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MaterializeDatasetNode;
#[flow_like_types::async_trait]
impl NodeLogic for MaterializeDatasetNode {
    fn get_node(&self) -> Node {
        super::operation_node::<MaterializeDatasetRequest, Vec<MaterializedTable>>(
            "ml_materialize_dataset",
            "Materialize Dataset",
            "Write prepared partitions with original and generated named columns",
            "materializeDataset",
            "AI/ML/Feature Engineering",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let request: MaterializeDatasetRequest = context.evaluate_pin("request").await?;
            let result = super::auto_training_tables::materialize_prepared_tables(
                context,
                &request.prepared,
                &request.output_id,
                &request.tables,
            )
            .await?;
            context
                .set_pin_value("result", flow_like_types::json::json!(result))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "execute"))]
        {
            let _ = context;
            Err(anyhow!("Feature materialization requires a native build"))
        }
    }
}

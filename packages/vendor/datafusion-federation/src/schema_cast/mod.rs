use async_stream::stream;
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::common::Statistics;
use datafusion::config::ConfigOptions;
use datafusion::error::{DataFusionError, Result};
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::physical_plan::filter_pushdown::{FilterDescription, FilterPushdownPhase};
use datafusion::physical_plan::metrics::{BaselineMetrics, ExecutionPlanMetricsSet, MetricsSet};
use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
use datafusion::physical_plan::{
    DisplayAs, DisplayFormatType, ExecutionPlan, ExecutionPlanProperties, PhysicalExpr,
    PlanProperties,
};
use futures::StreamExt;
use std::any::Any;
use std::clone::Clone;
use std::fmt;
use std::sync::Arc;

mod intervals_cast;
mod lists_cast;
pub mod record_convert;
mod struct_cast;

#[derive(Debug)]
#[allow(clippy::module_name_repetitions)]
pub struct SchemaCastScanExec {
    input: Arc<dyn ExecutionPlan>,
    schema: SchemaRef,
    properties: Arc<PlanProperties>,
    metrics_set: ExecutionPlanMetricsSet,
}

impl SchemaCastScanExec {
    pub fn new(input: Arc<dyn ExecutionPlan>, schema: SchemaRef) -> Self {
        let eq_properties = input.equivalence_properties().clone();
        let emission_type = input.pipeline_behavior();
        let boundedness = input.boundedness();
        let properties = Arc::new(PlanProperties::new(
            eq_properties,
            input.output_partitioning().clone(),
            emission_type,
            boundedness,
        ));
        Self {
            input,
            schema,
            properties,
            metrics_set: ExecutionPlanMetricsSet::new(),
        }
    }
}

impl DisplayAs for SchemaCastScanExec {
    fn fmt_as(&self, _t: DisplayFormatType, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "SchemaCastScanExec")
    }
}

impl ExecutionPlan for SchemaCastScanExec {
    fn name(&self) -> &str {
        "SchemaCastScanExec"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }

    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![&self.input]
    }

    /// Prevents the introduction of additional `RepartitionExec` and processing input in parallel.
    /// This guarantees that the input is processed as a single stream, preserving the order of the data.
    fn benefits_from_input_partitioning(&self) -> Vec<bool> {
        vec![false]
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if children.len() == 1 {
            Ok(Arc::new(Self::new(
                Arc::clone(&children[0]),
                Arc::clone(&self.schema),
            )))
        } else {
            Err(DataFusionError::Execution(
                "SchemaCastScanExec expects exactly one input".to_string(),
            ))
        }
    }

    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        let mut stream = self.input.execute(partition, context)?;
        let schema = Arc::clone(&self.schema);
        let baseline_metrics = BaselineMetrics::new(&self.metrics_set, partition);

        Ok(Box::pin(RecordBatchStreamAdapter::new(
            Arc::clone(&schema),
            {
                stream! {
                    while let Some(batch) = stream.next().await {
                        let _timer = baseline_metrics.elapsed_compute().timer();
                        let batch = record_convert::try_cast_to(batch?, Arc::clone(&schema));
                        let batch = batch.map_err(|e| { DataFusionError::External(Box::new(e)) });
                        if let Ok(ref b) = batch {
                            baseline_metrics.output_rows().add(b.num_rows());
                        }
                        yield batch;
                    }
                }
            },
        )))
    }

    fn partition_statistics(&self, partition: Option<usize>) -> Result<Statistics> {
        self.input.partition_statistics(partition)
    }

    fn metrics(&self) -> Option<MetricsSet> {
        Some(self.metrics_set.clone_inner())
    }

    fn gather_filters_for_pushdown(
        &self,
        _phase: FilterPushdownPhase,
        parent_filters: Vec<Arc<dyn PhysicalExpr>>,
        _config: &ConfigOptions,
    ) -> Result<FilterDescription> {
        FilterDescription::from_children(parent_filters, &self.children())
    }
}

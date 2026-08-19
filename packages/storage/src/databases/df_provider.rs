//! Table-provider wrapper that keeps zero-column scans alive.
//!
//! LanceDB's DataFusion adapter pipes every batch through an operator that
//! rebuilds it with `RecordBatch::try_new`, which cannot represent a batch
//! without columns — the row count has nowhere to live, so the call fails and
//! the operator unwraps that failure into a panic. DataFusion asks for exactly
//! that shape whenever a query needs row counts but no values (`COUNT(*)`,
//! `SELECT 1`, `EXISTS` subqueries), so those queries take the whole process
//! down.
//!
//! [`zero_column_safe`] keeps one cheap column in the projection pushed into
//! LanceDB and strips it again above the scan, carrying the row count across.

use std::any::Any;
use std::sync::Arc;

use arrow_array::{RecordBatch, RecordBatchOptions};
use arrow_schema::{DataType, Schema as ArrowSchema, SchemaRef};
use datafusion::catalog::{Session, TableProvider};
use datafusion::common::stats::Precision;
use datafusion::common::{DataFusionError, Result as DataFusionResult, Statistics};
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::logical_expr::dml::InsertOp;
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown, TableType};
use datafusion::physical_expr::EquivalenceProperties;
use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
use datafusion::physical_plan::{
    DisplayAs, DisplayFormatType, ExecutionPlan, ExecutionPlanProperties, Partitioning,
    PlanProperties,
};
use flow_like_types::async_trait;
use futures::StreamExt;

/// Wraps a table provider so scans that project no columns still work.
pub fn zero_column_safe(inner: Arc<dyn TableProvider>) -> Arc<dyn TableProvider> {
    let placeholder_column = cheapest_column(&inner.schema());
    Arc::new(ZeroColumnSafeProvider {
        inner,
        placeholder_column,
    })
}

/// Picks the column a row-count-only scan should read. Reading an embedding or
/// a nested column just to count rows would move orders of magnitude more data
/// than a scalar column, so the widest types are chosen last.
fn cheapest_column(schema: &SchemaRef) -> Option<usize> {
    schema
        .fields()
        .iter()
        .enumerate()
        .min_by_key(|(index, field)| (column_scan_cost(field.data_type()), *index))
        .map(|(index, _)| index)
}

fn column_scan_cost(data_type: &DataType) -> u8 {
    if data_type.is_primitive() || matches!(data_type, DataType::Boolean | DataType::Null) {
        return 0;
    }

    match data_type {
        DataType::Utf8
        | DataType::LargeUtf8
        | DataType::Utf8View
        | DataType::Binary
        | DataType::LargeBinary
        | DataType::BinaryView => 1,
        _ => 2,
    }
}

#[derive(Debug)]
struct ZeroColumnSafeProvider {
    inner: Arc<dyn TableProvider>,
    placeholder_column: Option<usize>,
}

#[async_trait]
impl TableProvider for ZeroColumnSafeProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> SchemaRef {
        self.inner.schema()
    }

    fn table_type(&self) -> TableType {
        self.inner.table_type()
    }

    fn get_table_definition(&self) -> Option<&str> {
        self.inner.get_table_definition()
    }

    fn get_column_default(&self, column: &str) -> Option<&Expr> {
        self.inner.get_column_default(column)
    }

    async fn scan(
        &self,
        state: &dyn Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        let placeholder = self
            .placeholder_column
            .filter(|_| projection.is_some_and(|projection| projection.is_empty()));

        let Some(placeholder) = placeholder else {
            return self.inner.scan(state, projection, filters, limit).await;
        };

        let projection = vec![placeholder];
        let plan = self
            .inner
            .scan(state, Some(&projection), filters, limit)
            .await?;
        Ok(Arc::new(RowCountOnlyExec::new(plan)))
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&Expr],
    ) -> DataFusionResult<Vec<TableProviderFilterPushDown>> {
        self.inner.supports_filters_pushdown(filters)
    }

    fn statistics(&self) -> Option<Statistics> {
        self.inner.statistics()
    }

    async fn insert_into(
        &self,
        state: &dyn Session,
        input: Arc<dyn ExecutionPlan>,
        insert_op: InsertOp,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        self.inner.insert_into(state, input, insert_op).await
    }

    async fn delete_from(
        &self,
        state: &dyn Session,
        filters: Vec<Expr>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        self.inner.delete_from(state, filters).await
    }

    async fn update(
        &self,
        state: &dyn Session,
        assignments: Vec<(String, Expr)>,
        filters: Vec<Expr>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        self.inner.update(state, assignments, filters).await
    }
}

/// Replaces its input's batches with column-less batches of the same row count,
/// which is what DataFusion expects back from a scan that projects nothing.
#[derive(Debug)]
struct RowCountOnlyExec {
    input: Arc<dyn ExecutionPlan>,
    schema: SchemaRef,
    properties: PlanProperties,
}

impl RowCountOnlyExec {
    fn new(input: Arc<dyn ExecutionPlan>) -> Self {
        let schema = Arc::new(ArrowSchema::empty());
        let properties = PlanProperties::new(
            EquivalenceProperties::new(schema.clone()),
            Partitioning::UnknownPartitioning(input.output_partitioning().partition_count()),
            input.pipeline_behavior(),
            input.boundedness(),
        );
        Self {
            input,
            schema,
            properties,
        }
    }
}

impl DisplayAs for RowCountOnlyExec {
    fn fmt_as(&self, _: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RowCountOnlyExec")
    }
}

impl ExecutionPlan for RowCountOnlyExec {
    fn name(&self) -> &str {
        "RowCountOnlyExec"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn properties(&self) -> &PlanProperties {
        &self.properties
    }

    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![&self.input]
    }

    fn maintains_input_order(&self) -> Vec<bool> {
        vec![true]
    }

    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        let input = children.into_iter().next().ok_or_else(|| {
            DataFusionError::Internal("RowCountOnlyExec expects exactly one child".to_string())
        })?;
        Ok(Arc::new(Self::new(input)))
    }

    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> DataFusionResult<SendableRecordBatchStream> {
        let schema = self.schema.clone();
        let stream = self.input.execute(partition, context)?.map(move |batch| {
            let rows = batch?.num_rows();
            let options = RecordBatchOptions::new().with_row_count(Some(rows));
            RecordBatch::try_new_with_options(schema.clone(), vec![], &options)
                .map_err(DataFusionError::from)
        });
        Ok(Box::pin(RecordBatchStreamAdapter::new(
            self.schema.clone(),
            stream,
        )))
    }

    fn partition_statistics(&self, partition: Option<usize>) -> DataFusionResult<Statistics> {
        let statistics = self.input.partition_statistics(partition)?;
        Ok(Statistics {
            num_rows: statistics.num_rows,
            total_byte_size: Precision::Absent,
            column_statistics: vec![],
        })
    }

    fn supports_limit_pushdown(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::Field;

    fn schema(fields: Vec<(&str, DataType)>) -> SchemaRef {
        Arc::new(ArrowSchema::new(
            fields
                .into_iter()
                .map(|(name, data_type)| Field::new(name, data_type, true))
                .collect::<Vec<_>>(),
        ))
    }

    #[test]
    fn row_count_scans_avoid_wide_columns() {
        let vector =
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 1536);

        assert_eq!(
            cheapest_column(&schema(vec![
                ("vector", vector.clone()),
                ("text", DataType::Utf8),
                ("id", DataType::Int64),
            ])),
            Some(2)
        );
        assert_eq!(
            cheapest_column(&schema(vec![("vector", vector), ("text", DataType::Utf8)])),
            Some(1)
        );
        assert_eq!(
            cheapest_column(&schema(vec![
                ("first", DataType::Int32),
                ("second", DataType::Float64),
            ])),
            Some(0)
        );
        assert_eq!(cheapest_column(&schema(vec![])), None);
    }
}

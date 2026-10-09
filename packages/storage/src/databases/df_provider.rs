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
//! [`lance_table_provider`] uses a native empty projection when the table exposes
//! a Lance dataset, avoiding value-column reads for row-count-only scans.
//!
//! [`zero_column_safe_writable`] additionally carries the [`lancedb::Table`]
//! handle so SQL `UPDATE`/`DELETE` route into Lance's own mutation API (see
//! [`crate::databases::lance_dml`]); without it those statements fail with
//! DataFusion's NotImplemented error while `INSERT INTO` still forwards to
//! the adapter.

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

use crate::databases::vector::schema::primary_key_columns;

use crate::databases::lance_dml::{
    LanceDmlExec, LanceDmlOp, assignments_to_lance_updates, filters_to_lance_predicate,
};

#[path = "lance_order.rs"]
mod lance_order;
pub use lance_order::with_lance_order_pushdown;

/// Wraps a table provider so scans that project no columns still work.
pub fn zero_column_safe(inner: Arc<dyn TableProvider>) -> Arc<dyn TableProvider> {
    let placeholder_column = cheapest_column(&inner.schema());
    Arc::new(ZeroColumnSafeProvider {
        inner,
        placeholder_column,
        dml_table: None,
        scan_table: None,
    })
}

/// Like [`zero_column_safe`], but also enables SQL `UPDATE`/`DELETE` by
/// translating them onto the given Lance table handle. Only register a
/// provider built here on surfaces where writing is intended — read-only
/// surfaces must keep validating SQL before execution.
pub fn zero_column_safe_writable(
    inner: Arc<dyn TableProvider>,
    table: lancedb::Table,
) -> Arc<dyn TableProvider> {
    let placeholder_column = cheapest_column(&inner.schema());
    Arc::new(ZeroColumnSafeProvider {
        inner,
        placeholder_column,
        dml_table: Some(table),
        scan_table: None,
    })
}

/// A plain Lance table can defer payload columns until after an ordered limit.
/// Construct its adapter here so hidden FTS or custom-provider filters cannot be
/// lost when the ordered scan is rebuilt from its captured dataset.
/// `enable_dml` enables UPDATE/DELETE. INSERT still forwards to LanceDB, so
/// read-only entrypoints must keep their existing SQL validation or provider gate.
pub async fn lance_table_provider(
    table: lancedb::Table,
    enable_dml: bool,
) -> lancedb::Result<Arc<dyn TableProvider>> {
    let inner = Arc::new(
        lancedb::table::datafusion::BaseTableAdapter::try_new(table.base_table().clone()).await?,
    );
    Ok(Arc::new(ZeroColumnSafeProvider {
        placeholder_column: cheapest_column(&inner.schema()),
        inner,
        dml_table: enable_dml.then(|| table.clone()),
        scan_table: Some(table),
    }))
}

/// Derive FTS only from an explicitly mounted, plain LanceDB adapter. The caller
/// must retain its authorization checks and expose the result through a read-only
/// provider. Generic wrapping keeps native scan rewrites from dropping the query.
pub(crate) fn with_full_text_query(
    provider: &Arc<dyn TableProvider>,
    query: lancedb::index::scalar::FullTextSearchQuery,
) -> Option<Arc<dyn TableProvider>> {
    let provider = provider.as_any().downcast_ref::<ZeroColumnSafeProvider>()?;
    provider.scan_table.as_ref()?;
    let adapter = provider
        .inner
        .as_any()
        .downcast_ref::<lancedb::table::datafusion::BaseTableAdapter>()?;
    Some(zero_column_safe(Arc::new(adapter.with_fts_query(query))))
}

/// Spatial expressions stay above the Lance scan until exact pushdown is verified.
fn contains_spatial_function(expression: &Expr) -> bool {
    use datafusion::common::tree_node::{TreeNode, TreeNodeRecursion};
    let mut spatial = false;
    let _ = expression.apply(|expression| {
        if let Expr::ScalarFunction(function) = expression {
            let name = function.func.name().to_ascii_lowercase();
            if name.starts_with("st_") || name.starts_with("flow_geom") {
                spatial = true;
            }
        }
        Ok(TreeNodeRecursion::Continue)
    });
    spatial
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
    /// When present, UPDATE/DELETE are translated onto this handle instead of
    /// forwarding to the adapter (which cannot execute them).
    dml_table: Option<lancedb::Table>,
    /// Only the constructor for plain LanceDB tables opts into native scan paths.
    scan_table: Option<lancedb::Table>,
}

impl ZeroColumnSafeProvider {
    /// An overwrite commits the query's schema, which carries no key marker. The key is
    /// read from the live table because an upsert may have set it after this mount.
    async fn ensure_overwrite_keeps_key(&self) -> DataFusionResult<()> {
        let keys = match &self.dml_table {
            Some(table) => {
                table
                    .checkout_latest()
                    .await
                    .map_err(|error| DataFusionError::External(error.into()))?;
                let schema = table
                    .schema()
                    .await
                    .map_err(|error| DataFusionError::External(error.into()))?;
                primary_key_columns(&schema)
            }
            None => primary_key_columns(&self.schema()),
        };
        if keys.is_empty() {
            return Ok(());
        }
        Err(DataFusionError::Plan(format!(
            "INSERT OVERWRITE would replace the table schema and drop its key column '{}'; delete the rows and use INSERT INTO instead",
            keys.join("', '")
        )))
    }
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
        let safe_filters: Vec<Expr> = filters
            .iter()
            .filter(|expr| !contains_spatial_function(expr))
            .cloned()
            .collect();
        let limit = if safe_filters.len() != filters.len() {
            None
        } else {
            limit
        };
        let filters = safe_filters.as_slice();
        if projection.is_some_and(|projection| projection.is_empty())
            && let Some(dataset) = self.scan_table.as_ref().and_then(|table| table.dataset())
        {
            if limit == Some(0) {
                // Lance can omit a zero limit when filters or stable row IDs
                // prevent scan-range pushdown. Keep the provider limit exact.
                return Ok(Arc::new(datafusion::physical_plan::empty::EmptyExec::new(
                    Arc::new(ArrowSchema::empty()),
                )));
            }
            // Resolve through LanceDB's consistency wrapper once, preserving its
            // freshness policy and pinning this plan to the selected snapshot.
            let dataset = dataset
                .get()
                .await
                .map_err(|error| DataFusionError::External(error.into()))?;
            let mut scanner = dataset.scan();
            scanner.empty_project()?;
            if let Some(filter) = filters.iter().cloned().reduce(Expr::and) {
                scanner.filter_expr(filter);
            }
            let limit = limit
                .map(i64::try_from)
                .transpose()
                .map_err(|_| DataFusionError::Plan("Lance scan limit exceeds i64::MAX".into()))?;
            scanner.limit(limit, None)?;
            scanner.batch_size(state.config().batch_size());
            let plan = scanner.create_plan().await?;
            return Ok(Arc::new(RowCountOnlyExec::new(plan)));
        }
        let placeholder = self
            .placeholder_column
            .filter(|_| projection.is_some_and(|projection| projection.is_empty()));

        let Some(placeholder) = placeholder else {
            let plan = self.inner.scan(state, projection, filters, limit).await?;
            if self.scan_table.is_none() {
                return Ok(plan);
            }
            return Ok(lance_order::mark_scan(
                plan,
                filters.to_vec(),
                limit,
                state.config().batch_size(),
            ));
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
        // DataFusion 53 also extracts UPDATE/DELETE predicates from TableScan.filters,
        // so exact Lance filters can remove redundant filtering and push down limits.
        let mut supported = self.inner.supports_filters_pushdown(filters)?;
        for (expression, support) in filters.iter().zip(&mut supported) {
            if contains_spatial_function(expression) {
                *support = TableProviderFilterPushDown::Inexact;
            }
        }
        Ok(supported)
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
        if self
            .schema()
            .fields()
            .iter()
            .any(|field| crate::geometry::is_geometry_field(field))
        {
            return Err(DataFusionError::Plan("SQL INSERT into geometry tables is unsupported; use validated JSON or Arrow insert/upsert".into()));
        }
        if matches!(insert_op, InsertOp::Overwrite) {
            self.ensure_overwrite_keeps_key().await?;
        }
        self.inner.insert_into(state, input, insert_op).await
    }

    /// The mutation itself runs when the returned plan executes — EXPLAIN
    /// builds this plan without running it.
    async fn delete_from(
        &self,
        state: &dyn Session,
        filters: Vec<Expr>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        let Some(table) = &self.dml_table else {
            return self.inner.delete_from(state, filters).await;
        };
        let predicate = filters_to_lance_predicate(&filters, &self.schema())?;
        Ok(Arc::new(LanceDmlExec::new(
            table.clone(),
            LanceDmlOp::Delete { predicate },
        )))
    }

    async fn update(
        &self,
        state: &dyn Session,
        assignments: Vec<(String, Expr)>,
        filters: Vec<Expr>,
    ) -> DataFusionResult<Arc<dyn ExecutionPlan>> {
        let Some(table) = &self.dml_table else {
            return self.inner.update(state, assignments, filters).await;
        };
        let schema = self.schema();
        let predicate = filters_to_lance_predicate(&filters, &schema)?;
        let assignments = assignments_to_lance_updates(&assignments, &schema)?;
        Ok(Arc::new(LanceDmlExec::new(
            table.clone(),
            LanceDmlOp::Update {
                predicate,
                assignments,
            },
        )))
    }
}

/// Replaces its input's batches with column-less batches of the same row count,
/// which is what DataFusion expects back from a scan that projects nothing.
#[derive(Debug)]
struct RowCountOnlyExec {
    input: Arc<dyn ExecutionPlan>,
    schema: SchemaRef,
    properties: Arc<PlanProperties>,
}

impl RowCountOnlyExec {
    fn new(input: Arc<dyn ExecutionPlan>) -> Self {
        let schema = Arc::new(ArrowSchema::empty());
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new(schema.clone()),
            Partitioning::UnknownPartitioning(input.output_partitioning().partition_count()),
            input.pipeline_behavior(),
            input.boundedness(),
        ));
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

    fn properties(&self) -> &Arc<PlanProperties> {
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
    use arrow_array::{Int64Array, StringArray};
    use arrow_schema::Field;
    use datafusion::common::tree_node::{TreeNode, TreeNodeRecursion};
    use datafusion::physical_plan::{collect, displayable};
    use datafusion::prelude::SessionContext;

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

    struct EmptyProjectionFixture {
        directory: std::path::PathBuf,
        table: lancedb::Table,
        provider: Arc<dyn TableProvider>,
        native: SessionContext,
        fallback: SessionContext,
    }

    impl Drop for EmptyProjectionFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    impl EmptyProjectionFixture {
        async fn new() -> flow_like_types::Result<Self> {
            let directory = std::env::temp_dir().join(format!(
                "lance-empty-projection-{}",
                flow_like_types::create_id()
            ));
            std::fs::create_dir_all(&directory)?;
            let connection =
                crate::databases::vector::lancedb::connect_lance(directory.to_str().unwrap())
                    .execute()
                    .await?;
            let batch = RecordBatch::try_new(
                Arc::new(ArrowSchema::new(vec![
                    Field::new("id", DataType::Int64, false),
                    Field::new("tag", DataType::Utf8, true),
                    Field::new("payload", DataType::Utf8, false),
                ])),
                vec![
                    Arc::new(Int64Array::from_iter_values(0..12)),
                    Arc::new(StringArray::from_iter((0..12).map(|id| match id % 3 {
                        0 => Some("new"),
                        1 => Some("old"),
                        _ => None,
                    }))),
                    Arc::new(StringArray::from_iter_values(
                        (0..12).map(|_| "payload ".repeat(1024)),
                    )),
                ],
            )?;
            let table = connection
                .create_table("items", vec![batch])
                .execute()
                .await?;
            let provider = lance_table_provider(table.clone(), false).await?;
            let native = SessionContext::new();
            native.register_table("items", provider.clone())?;
            let fallback = SessionContext::new();
            fallback.register_table(
                "items",
                zero_column_safe(Arc::new(
                    lancedb::table::datafusion::BaseTableAdapter::try_new(
                        table.base_table().clone(),
                    )
                    .await?,
                )),
            )?;
            Ok(Self {
                directory,
                table,
                provider,
                native,
                fallback,
            })
        }
    }

    async fn query_rows(
        context: &SessionContext,
        query: &str,
    ) -> flow_like_types::Result<Vec<flow_like_types::Value>> {
        Ok(context
            .sql(query)
            .await?
            .collect()
            .await?
            .iter()
            .map(crate::arrow_utils::record_batch_to_value)
            .collect::<flow_like_types::Result<Vec<_>>>()?
            .concat())
    }

    #[tokio::test]
    async fn native_empty_projection_avoids_value_columns_and_preserves_limits()
    -> flow_like_types::Result<()> {
        let fixture = EmptyProjectionFixture::new().await?;
        for (limit, expected) in [(None, 12), (Some(0), 0), (Some(1), 1), (Some(7), 7)] {
            let plan = fixture
                .provider
                .scan(&fixture.native.state(), Some(&vec![]), &[], limit)
                .await?;
            plan.apply(|node| {
                assert!(
                    node.schema()
                        .fields()
                        .iter()
                        .all(|field| !matches!(field.name().as_str(), "id" | "tag" | "payload")),
                    "empty projection must not read a placeholder value column: {}",
                    displayable(plan.as_ref()).indent(true)
                );
                Ok(TreeNodeRecursion::Continue)
            })?;
            let batches = collect(plan, fixture.native.task_ctx()).await?;
            assert!(batches.iter().all(|batch| batch.num_columns() == 0));
            assert_eq!(
                batches.iter().map(RecordBatch::num_rows).sum::<usize>(),
                expected
            );
        }
        for query in [
            "SELECT COUNT(*) AS n FROM items",
            "SELECT 1 AS value FROM items LIMIT 7 OFFSET 3",
            "SELECT 1 AS value FROM items LIMIT 0",
            "SELECT 1 AS present WHERE EXISTS(SELECT 1 FROM items)",
            "SELECT 1 AS present WHERE EXISTS(SELECT 1 FROM items WHERE id = 5)",
            "SELECT 1 AS present WHERE EXISTS(SELECT 1 FROM items WHERE id > 100)",
        ] {
            assert_eq!(
                query_rows(&fixture.native, query).await?,
                query_rows(&fixture.fallback, query).await?,
                "{query}"
            );
        }
        assert_eq!(
            query_rows(
                &fixture.native,
                "SELECT 1 AS present WHERE EXISTS(SELECT 1 FROM items)"
            )
            .await?
            .len(),
            1
        );
        let plan = fixture
            .provider
            .scan(
                &fixture.native.state(),
                Some(&vec![]),
                &[datafusion::prelude::col("id").gt(datafusion::prelude::lit(5))],
                Some(0),
            )
            .await?;
        let batches = collect(plan, fixture.native.task_ctx()).await?;
        assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn native_empty_projection_keeps_indexed_filters_and_null_counts()
    -> flow_like_types::Result<()> {
        let fixture = EmptyProjectionFixture::new().await?;
        fixture
            .table
            .create_index(&["tag"], lancedb::index::Index::Bitmap(Default::default()))
            .execute()
            .await?;
        for query in [
            "SELECT COUNT(*) AS n FROM items WHERE tag = 'new'",
            "SELECT COUNT(*) AS n FROM items WHERE tag IS NULL",
            "SELECT COUNT(*) AS n FROM items WHERE tag <> 'old' AND id > 3",
            "SELECT COUNT(*) AS n FROM (SELECT 1 FROM items WHERE tag = 'new' LIMIT 2)",
            "SELECT 1 AS present WHERE EXISTS(SELECT 1 FROM items WHERE tag = 'new' AND id > 10)",
        ] {
            assert_eq!(
                query_rows(&fixture.native, query).await?,
                query_rows(&fixture.fallback, query).await?,
                "{query}"
            );
        }
        assert_eq!(
            query_rows(
                &fixture.native,
                "SELECT COUNT(*) AS n FROM items WHERE tag IS NULL"
            )
            .await?[0]["n"],
            4
        );
        Ok(())
    }

    #[tokio::test]
    async fn native_empty_projection_keeps_snapshot_deletions_and_empty_rows()
    -> flow_like_types::Result<()> {
        let fixture = EmptyProjectionFixture::new().await?;
        let retained = fixture
            .provider
            .scan(&fixture.native.state(), Some(&vec![]), &[], None)
            .await?;
        fixture.table.delete("id >= 8").await?;
        let batches = collect(retained, fixture.native.task_ctx()).await?;
        assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 12);
        assert_eq!(
            query_rows(&fixture.native, "SELECT COUNT(*) AS n FROM items").await?[0]["n"],
            8
        );
        for query in [
            "SELECT COUNT(*) AS n FROM items",
            "SELECT COUNT(*) AS n FROM items WHERE id % 2 = 0",
            "SELECT 1 AS value FROM items LIMIT 10",
        ] {
            assert_eq!(
                query_rows(&fixture.native, query).await?,
                query_rows(&fixture.fallback, query).await?
            );
        }
        fixture.table.delete("true").await?;
        assert_eq!(
            query_rows(&fixture.native, "SELECT COUNT(*) AS n FROM items").await?[0]["n"],
            0
        );
        assert!(
            query_rows(
                &fixture.native,
                "SELECT 1 AS present WHERE EXISTS(SELECT 1 FROM items)"
            )
            .await?
            .is_empty()
        );
        assert!(
            query_rows(&fixture.native, "SELECT 1 FROM items LIMIT 5")
                .await?
                .is_empty()
        );
        Ok(())
    }
}

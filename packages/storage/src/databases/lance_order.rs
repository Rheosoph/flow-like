use std::{any::Any, sync::Arc};

use arrow_array::RecordBatch;
use arrow_schema::DataType;
use datafusion::common::config::ConfigOptions;
use datafusion::common::tree_node::{Transformed, TreeNode};
use datafusion::common::{DataFusionError, Result, Statistics};
use datafusion::execution::session_state::{SessionState, SessionStateBuilder};
use datafusion::execution::{SendableRecordBatchStream, TaskContext};
use datafusion::logical_expr::Expr;
use datafusion::physical_expr::expressions::Column;
use datafusion::physical_expr::{EquivalenceProperties, PhysicalExpr, PhysicalSortExpr};
use datafusion::physical_optimizer::PhysicalOptimizerRule;
use datafusion::physical_plan::execution_plan::{Boundedness, EmissionType};
use datafusion::physical_plan::projection::ProjectionExec;
use datafusion::physical_plan::sorts::sort::SortExec;
use datafusion::physical_plan::stream::RecordBatchStreamAdapter;
use datafusion::physical_plan::{
    DisplayAs, DisplayFormatType, ExecutionPlan, ExecutionPlanProperties, Partitioning,
    PlanProperties, execute_stream,
};
use futures::{StreamExt, TryStreamExt};
use lance::Dataset;
use lance::dataset::scanner::{ColumnOrdering, MaterializationStyle};
use lance::io::exec::filtered_read::FilteredReadExec;
use lance::io::exec::scalar_index::ScalarIndexExec;
use lance::io::exec::{LanceScanExec, TakeExec};

/// Install before repartitioning and sort rewrites. Preserve the session's
/// planner, catalogs, security settings, and existing optimizer rules.
pub fn with_lance_order_pushdown(state: SessionState) -> SessionState {
    if state
        .physical_optimizers()
        .iter()
        .any(|rule| rule.name() == "LanceOrderLimitPushdown")
    {
        return state;
    }
    let mut rules: Vec<Arc<dyn PhysicalOptimizerRule + Send + Sync>> =
        vec![Arc::new(LanceOrderLimitPushdown)];
    rules.extend_from_slice(state.physical_optimizers());
    SessionStateBuilder::new_from_existing(state)
        .with_physical_optimizer_rules(rules)
        .build()
}

#[derive(Debug)]
struct LanceOrderLimitPushdown;

impl PhysicalOptimizerRule for LanceOrderLimitPushdown {
    fn optimize(
        &self,
        plan: Arc<dyn ExecutionPlan>,
        config: &ConfigOptions,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if !config.optimizer.enable_sort_pushdown {
            return Ok(plan);
        }
        Ok(plan
            .transform_down(|plan| {
                let Some(sort) = plan.as_any().downcast_ref::<SortExec>() else {
                    return Ok(Transformed::no(plan));
                };
                let Some(fetch) = sort
                    .fetch()
                    .filter(|fetch| *fetch > 0 && i64::try_from(*fetch).is_ok())
                else {
                    return Ok(Transformed::no(plan));
                };
                // Only a global sort is replaced. Local sorts introduced for joins
                // or parallel merges keep their original distribution semantics.
                if sort.preserve_partitioning() {
                    return Ok(Transformed::no(plan));
                }
                match ordered_input(sort.input(), sort.expr(), fetch)? {
                    Some(input) => Ok(Transformed::yes(input)),
                    None => Ok(Transformed::no(plan)),
                }
            })?
            .data)
    }

    fn name(&self) -> &str {
        "LanceOrderLimitPushdown"
    }
    fn schema_check(&self) -> bool {
        true
    }
}

fn volatile(expression: &Arc<dyn PhysicalExpr>) -> bool {
    expression.is_volatile_node() || expression.children().iter().any(|child| volatile(child))
}

fn ordered_input(
    input: &Arc<dyn ExecutionPlan>,
    ordering: &[PhysicalSortExpr],
    fetch: usize,
) -> Result<Option<Arc<dyn ExecutionPlan>>> {
    if let Some(scan) = input.as_any().downcast_ref::<LanceScanSourceExec>() {
        if scan.source_limit.is_some() || ordering.is_empty() {
            return Ok(None);
        }
        let mut columns = Vec::with_capacity(ordering.len());
        let schema = input.schema();
        for order in ordering {
            let Some(column) = order.expr.as_any().downcast_ref::<Column>() else {
                return Ok(None);
            };
            let Some(field) = schema.fields().get(column.index()) else {
                return Ok(None);
            };
            if field.name() != column.name()
                || field.name().contains(['.', '`'])
                || !(field.data_type().is_primitive()
                    || matches!(
                        field.data_type(),
                        DataType::Boolean
                            | DataType::Utf8
                            | DataType::LargeUtf8
                            | DataType::Utf8View
                            | DataType::Binary
                            | DataType::LargeBinary
                            | DataType::BinaryView
                    ))
            {
                return Ok(None);
            }
            columns.push(ColumnOrdering {
                column_name: field.name().clone(),
                ascending: !order.options.descending,
                nulls_first: order.options.nulls_first,
            });
        }
        let properties = Arc::new(PlanProperties::new(
            EquivalenceProperties::new_with_orderings(schema, [ordering.to_vec()]),
            Partitioning::UnknownPartitioning(1),
            EmissionType::Final,
            Boundedness::Bounded,
        ));
        return Ok(Some(Arc::new(NativeLanceTopKExec {
            source: scan.clone(),
            ordering: columns,
            fetch,
            properties,
        })));
    }
    if let Some(projection) = input.as_any().downcast_ref::<ProjectionExec>() {
        // Payload expressions stay above the limited native scan. A computed
        // sort key or a volatile payload keeps the ordinary DataFusion plan.
        if projection.expr().iter().any(|expr| volatile(&expr.expr)) {
            return Ok(None);
        }
        let mut child_order = Vec::with_capacity(ordering.len());
        for order in ordering {
            let Some(column) = order.expr.as_any().downcast_ref::<Column>() else {
                return Ok(None);
            };
            let Some(projected) = projection.expr().get(column.index()) else {
                return Ok(None);
            };
            let Some(child_column) = projected.expr.as_any().downcast_ref::<Column>() else {
                return Ok(None);
            };
            child_order.push(PhysicalSortExpr {
                expr: Arc::new(child_column.clone()),
                options: order.options,
            });
        }
        if let Some(child) = ordered_input(projection.input(), &child_order, fetch)? {
            return Ok(Some(input.clone().with_new_children(vec![child])?));
        }
    }
    // Filters, aggregates, joins and inner limits cannot move below the top-K.
    Ok(None)
}

/// Discover the exact snapshot retained by the original Lance plan. Pinned
/// references and a write between planning and execution therefore keep their
/// original meaning. Unknown source shapes use the original plan.
fn snapshot(plan: &Arc<dyn ExecutionPlan>) -> Option<Arc<Dataset>> {
    fn visit(plan: &Arc<dyn ExecutionPlan>, found: &mut Option<Arc<Dataset>>) -> bool {
        let dataset = if let Some(scan) = plan.as_any().downcast_ref::<LanceScanExec>() {
            Some(scan.dataset())
        } else if let Some(scan) = plan.as_any().downcast_ref::<FilteredReadExec>() {
            Some(scan.dataset())
        } else if let Some(take) = plan.as_any().downcast_ref::<TakeExec>() {
            Some(take.dataset())
        } else {
            plan.as_any()
                .downcast_ref::<ScalarIndexExec>()
                .map(|scan| scan.dataset())
        };
        if let Some(dataset) = dataset {
            if let Some(previous) = found {
                if !Arc::ptr_eq(previous, dataset) {
                    return false;
                }
            } else {
                *found = Some(dataset.clone());
            }
        }
        plan.children().iter().all(|child| visit(child, found))
    }
    let mut found = None;
    visit(plan, &mut found).then_some(found).flatten()
}

pub(super) fn mark_scan(
    input: Arc<dyn ExecutionPlan>,
    filters: Vec<Expr>,
    source_limit: Option<usize>,
    batch_size: usize,
) -> Arc<dyn ExecutionPlan> {
    // Blob, geometry and nested-column adapters can have additional projection
    // semantics. Keep those on the existing provider path until verified.
    if input.schema().fields().iter().any(|field| {
        !(field.data_type().is_primitive()
            || matches!(
                field.data_type(),
                DataType::Boolean
                    | DataType::Utf8
                    | DataType::LargeUtf8
                    | DataType::Utf8View
                    | DataType::Binary
                    | DataType::LargeBinary
                    | DataType::BinaryView
            ))
            || crate::geometry::is_geometry_field(field)
            || field.name().contains(['.', '`'])
    }) {
        return input;
    }
    let Some(dataset) = snapshot(&input) else {
        return input;
    };
    Arc::new(LanceScanSourceExec {
        input,
        dataset,
        filters,
        source_limit,
        batch_size,
    })
}

#[derive(Clone, Debug)]
struct LanceScanSourceExec {
    input: Arc<dyn ExecutionPlan>,
    dataset: Arc<Dataset>,
    filters: Vec<Expr>,
    source_limit: Option<usize>,
    batch_size: usize,
}

impl DisplayAs for LanceScanSourceExec {
    fn fmt_as(&self, _: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LanceScanSourceExec: version={}",
            self.dataset.version_id()
        )
    }
}

impl ExecutionPlan for LanceScanSourceExec {
    fn name(&self) -> &str {
        "LanceScanSourceExec"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn properties(&self) -> &Arc<PlanProperties> {
        self.input.properties()
    }
    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![&self.input]
    }
    fn maintains_input_order(&self) -> Vec<bool> {
        vec![true]
    }
    fn benefits_from_input_partitioning(&self) -> Vec<bool> {
        vec![false]
    }
    fn supports_limit_pushdown(&self) -> bool {
        true
    }
    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let [input]: [Arc<dyn ExecutionPlan>; 1] = children.try_into().map_err(|_| {
            DataFusionError::Internal("Lance scan wrapper expects one child".into())
        })?;
        Ok(Arc::new(Self {
            input,
            ..self.as_ref().clone()
        }))
    }
    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        self.input.execute(partition, context)
    }
    fn partition_statistics(&self, partition: Option<usize>) -> Result<Statistics> {
        self.input.partition_statistics(partition)
    }
}

#[derive(Debug)]
struct NativeLanceTopKExec {
    source: LanceScanSourceExec,
    ordering: Vec<ColumnOrdering>,
    fetch: usize,
    properties: Arc<PlanProperties>,
}

impl NativeLanceTopKExec {
    async fn native_plan(&self) -> Result<Arc<dyn ExecutionPlan>> {
        let mut scanner = self.source.dataset.scan();
        let schema = self.schema();
        let columns: Vec<_> = schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect();
        scanner.project(&columns).map_err(lance_error)?;
        if let Some(filter) = self.source.filters.iter().cloned().reduce(Expr::and) {
            scanner.filter_expr(filter);
        }
        scanner.batch_size(self.source.batch_size);
        scanner.materialization_style(MaterializationStyle::AllLate);
        scanner
            .order_by(Some(self.ordering.clone()))
            .map_err(lance_error)?;
        scanner
            .limit(Some(self.fetch as i64), None)
            .map_err(lance_error)?;
        let plan = scanner.create_plan().await.map_err(lance_error)?;
        if plan.schema().fields() != schema.fields() {
            return Err(DataFusionError::Execution(
                "Ordered Lance scan changed the projected schema".into(),
            ));
        }
        if plan.output_partitioning().partition_count() != 1 {
            return Err(DataFusionError::Execution(
                "Ordered Lance scan must return one globally sorted partition".into(),
            ));
        }
        Ok(plan)
    }
}

fn lance_error(error: lance::Error) -> DataFusionError {
    DataFusionError::External(Box::new(error))
}

impl DisplayAs for NativeLanceTopKExec {
    fn fmt_as(&self, _: DisplayFormatType, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NativeLanceTopKExec: version={}, fetch={}, order={:?}, late_materialization=true",
            self.source.dataset.version_id(),
            self.fetch,
            self.ordering
        )
    }
}

impl ExecutionPlan for NativeLanceTopKExec {
    fn name(&self) -> &str {
        "NativeLanceTopKExec"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn properties(&self) -> &Arc<PlanProperties> {
        &self.properties
    }
    fn children(&self) -> Vec<&Arc<dyn ExecutionPlan>> {
        vec![]
    }
    fn with_new_children(
        self: Arc<Self>,
        children: Vec<Arc<dyn ExecutionPlan>>,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        if !children.is_empty() {
            return Err(DataFusionError::Internal(
                "Native Lance top-K has no children".into(),
            ));
        }
        Ok(self)
    }
    fn execute(
        &self,
        partition: usize,
        context: Arc<TaskContext>,
    ) -> Result<SendableRecordBatchStream> {
        if partition != 0 {
            return Err(DataFusionError::Execution(
                "Native Lance top-K has only partition zero".into(),
            ));
        }
        let source = Self {
            source: self.source.clone(),
            ordering: self.ordering.clone(),
            fetch: self.fetch,
            properties: self.properties.clone(),
        };
        let schema = self.schema();
        let output_schema = schema.clone();
        let stream = futures::stream::once(async move {
            let plan = source.native_plan().await?;
            execute_stream(plan, context)
        })
        .try_flatten()
        .map(move |batch| {
            RecordBatch::try_new(output_schema.clone(), batch?.columns().to_vec())
                .map_err(DataFusionError::from)
        });
        Ok(Box::pin(RecordBatchStreamAdapter::new(schema, stream)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrow_utils::record_batch_to_value;
    use crate::databases::df_provider::{lance_table_provider, zero_column_safe};
    use crate::databases::vector::lancedb::connect_lance;
    use arrow_array::{Int64Array, StringArray, TimestampMillisecondArray};
    use arrow_schema::{Field, Schema, TimeUnit};
    use datafusion::physical_plan::displayable;
    use datafusion::prelude::SessionContext;

    struct Fixture {
        directory: std::path::PathBuf,
        table: lancedb::Table,
        normal: SessionContext,
        optimized: SessionContext,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    impl Fixture {
        async fn new() -> flow_like_types::Result<Self> {
            let directory =
                std::env::temp_dir().join(format!("lance-top-k-{}", flow_like_types::create_id()));
            std::fs::create_dir_all(&directory)?;
            let connection = connect_lance(directory.to_str().unwrap()).execute().await?;
            let schema = Arc::new(Schema::new(vec![
                Field::new("id", DataType::Int64, false),
                Field::new(
                    "last_updated_at",
                    DataType::Timestamp(TimeUnit::Millisecond, None),
                    true,
                ),
                Field::new("status", DataType::Utf8, false),
                Field::new("content", DataType::Utf8, false),
            ]));
            let batch = RecordBatch::try_new(
                schema,
                vec![
                    Arc::new(Int64Array::from_iter_values(0..128)),
                    Arc::new(TimestampMillisecondArray::from_iter((0..128).map(|id| {
                        (id % 11 != 0).then_some(1_700_000_000_000 + (id % 17) * 1000)
                    }))),
                    Arc::new(StringArray::from_iter_values((0..128).map(|id| {
                        match id % 3 {
                            0 => "new",
                            1 => "updated",
                            _ => "old",
                        }
                    }))),
                    Arc::new(StringArray::from_iter_values(
                        (0..128).map(|id| format!("article {id} {}", "payload ".repeat(1024))),
                    )),
                ],
            )?;
            let table = connection
                .create_table("articles", vec![batch])
                .execute()
                .await?;
            let normal = SessionContext::new();
            normal.register_table(
                "articles",
                zero_column_safe(Arc::new(
                    lancedb::table::datafusion::BaseTableAdapter::try_new(
                        table.base_table().clone(),
                    )
                    .await?,
                )),
            )?;
            // new_from_existing preserves catalogs, so use a distinct catalog for
            // the candidate to keep the reference provider independent.
            let optimized = SessionContext::new_with_state(with_lance_order_pushdown(
                SessionContext::new().state(),
            ));
            optimized
                .register_table("articles", lance_table_provider(table.clone(), true).await?)?;
            Ok(Self {
                directory,
                table,
                normal,
                optimized,
            })
        }
    }

    fn find_native(plan: &Arc<dyn ExecutionPlan>) -> Option<&NativeLanceTopKExec> {
        if let Some(native) = plan.as_any().downcast_ref::<NativeLanceTopKExec>() {
            return Some(native);
        }
        plan.children().into_iter().find_map(find_native)
    }

    async fn rows(
        context: &SessionContext,
        query: &str,
    ) -> flow_like_types::Result<Vec<flow_like_types::Value>> {
        Ok(context
            .sql(query)
            .await?
            .collect()
            .await?
            .iter()
            .map(record_batch_to_value)
            .collect::<flow_like_types::Result<Vec<_>>>()?
            .concat())
    }

    #[tokio::test]
    async fn native_top_k_keeps_filters_aliases_case_and_offset_and_defers_content()
    -> flow_like_types::Result<()> {
        let fixture = Fixture::new().await?;
        let query = "SELECT id, last_updated_at AS updated, status, content,
            CASE WHEN status = 'new' THEN true ELSE false END AS is_new,
            CASE WHEN status = 'updated' THEN true ELSE false END AS is_updated
            FROM articles WHERE status IN ('new', 'updated')
            ORDER BY updated DESC NULLS LAST, id ASC LIMIT 7 OFFSET 3";
        let plan = fixture
            .optimized
            .sql(query)
            .await?
            .create_physical_plan()
            .await?;
        let native = find_native(&plan).unwrap_or_else(|| {
            panic!(
                "missing native top-K: {}",
                displayable(plan.as_ref()).indent(true)
            )
        });
        assert_eq!(native.fetch, 10);
        let native_plan = native.native_plan().await?;
        let mut deferred_content = false;
        let mut top_k = false;
        native_plan.apply(|node| {
            if node.as_any().is::<TakeExec>()
                && node.schema().field_with_name("content").is_ok()
                && node
                    .children()
                    .iter()
                    .all(|child| child.schema().field_with_name("content").is_err())
            {
                node.apply(|child| {
                    if let Some(sort) = child.as_any().downcast_ref::<SortExec>() {
                        top_k |= sort.fetch() == Some(10);
                    }
                    Ok(datafusion::common::tree_node::TreeNodeRecursion::Continue)
                })?;
                deferred_content = true;
            }
            Ok(datafusion::common::tree_node::TreeNodeRecursion::Continue)
        })?;
        assert!(
            deferred_content && top_k,
            "content must be fetched above the native top-K: {}",
            displayable(native_plan.as_ref()).indent(true)
        );
        let expected = rows(&fixture.normal, query).await?;
        let actual = rows(&fixture.optimized, query).await?;
        assert_eq!(actual.len(), 7);
        assert_eq!(actual, expected);

        let query = "SELECT id, last_updated_at, status, content,
            CASE WHEN last_updated_at >= date_trunc('second', CAST(now() AS TIMESTAMP)) - INTERVAL '24 hours'
                THEN true ELSE false END AS is_updated
            FROM articles WHERE status <> 'deleted'
            ORDER BY last_updated_at DESC NULLS LAST, id ASC LIMIT 100";
        let plan = fixture
            .optimized
            .sql(query)
            .await?
            .create_physical_plan()
            .await?;
        assert!(
            find_native(&plan).is_some(),
            "the monitor's timestamp CASE must keep native ordering: {}",
            displayable(plan.as_ref()).indent(true)
        );
        assert_eq!(
            rows(&fixture.optimized, query).await?,
            rows(&fixture.normal, query).await?
        );

        let query = "WITH renamed AS (
                SELECT id AS key, last_updated_at AS updated, content AS body FROM articles
            )
            SELECT key AS article_id, key AS repeated_id, body,
                CASE WHEN key % 2 = 0 THEN true ELSE false END AS selected
            FROM renamed ORDER BY updated DESC NULLS LAST, key ASC LIMIT 9 OFFSET 2";
        let plan = fixture
            .optimized
            .sql(query)
            .await?
            .create_physical_plan()
            .await?;
        assert!(find_native(&plan).is_some());
        assert_eq!(
            rows(&fixture.optimized, query).await?,
            rows(&fixture.normal, query).await?
        );

        fixture
            .table
            .create_index(
                &["status"],
                lancedb::index::Index::Bitmap(Default::default()),
            )
            .execute()
            .await?;
        let query = "SELECT id, last_updated_at, status, content FROM articles
            WHERE status = 'new'
            ORDER BY last_updated_at DESC NULLS LAST, id ASC LIMIT 7 OFFSET 3";
        let plan = fixture
            .optimized
            .sql(query)
            .await?
            .create_physical_plan()
            .await?;
        let native = find_native(&plan).expect("indexed native top-K");
        let mut indexed = false;
        native.source.input.apply(|node| {
            indexed |= node.as_any().is::<ScalarIndexExec>()
                || node
                    .as_any()
                    .is::<lance::io::exec::scalar_index::MaterializeIndexExec>();
            Ok(datafusion::common::tree_node::TreeNodeRecursion::Continue)
        })?;
        assert!(
            indexed,
            "fixture must exercise snapshot discovery from an indexed scan: {}",
            displayable(native.source.input.as_ref()).indent(true)
        );
        assert_eq!(
            rows(&fixture.optimized, query).await?,
            rows(&fixture.normal, query).await?
        );
        Ok(())
    }

    #[tokio::test]
    async fn native_top_k_preserves_null_ordering_and_limit_boundaries()
    -> flow_like_types::Result<()> {
        let fixture = Fixture::new().await?;
        for direction in ["ASC", "DESC"] {
            for nulls in ["FIRST", "LAST"] {
                for (limit, offset) in [(1, 0), (13, 5), (200, 125), (3, 200)] {
                    let query = format!(
                        "SELECT id, last_updated_at, content FROM articles ORDER BY last_updated_at {direction} NULLS {nulls}, id ASC LIMIT {limit} OFFSET {offset}"
                    );
                    assert_eq!(
                        rows(&fixture.optimized, &query).await?,
                        rows(&fixture.normal, &query).await?,
                        "{query}"
                    );
                }
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn unsupported_ordered_queries_keep_the_datafusion_plan() -> flow_like_types::Result<()> {
        let fixture = Fixture::new().await?;
        crate::geometry::register_geo_functions(&fixture.normal);
        crate::geometry::register_geo_functions(&fixture.optimized);
        for query in [
            "SELECT id, content FROM articles ORDER BY id + 1 DESC LIMIT 4",
            "SELECT id, content FROM (SELECT * FROM articles LIMIT 12) ORDER BY id DESC LIMIT 4",
            "SELECT a.id, a.content FROM articles a JOIN articles b ON a.id = b.id ORDER BY a.last_updated_at DESC LIMIT 4",
            "SELECT status, COUNT(*) FROM articles GROUP BY status ORDER BY COUNT(*) DESC LIMIT 2",
            "SELECT id, content FROM articles ORDER BY last_updated_at DESC",
            "SELECT id, content FROM articles ORDER BY last_updated_at DESC LIMIT 0",
            "SELECT id, content FROM articles WHERE ST_X(ST_Point(CAST(id AS DOUBLE), 0.0)) >= 120 ORDER BY id ASC LIMIT 4",
        ] {
            let plan = fixture
                .optimized
                .sql(query)
                .await?
                .create_physical_plan()
                .await?;
            assert!(
                find_native(&plan).is_none(),
                "unexpected native top-K for {query}"
            );
        }
        let query =
            "SELECT id, content FROM (SELECT * FROM articles LIMIT 12) ORDER BY id DESC LIMIT 4";
        assert_eq!(
            rows(&fixture.optimized, query).await?,
            rows(&fixture.normal, query).await?
        );
        let query = "SELECT id, content FROM articles
            WHERE ST_X(ST_Point(CAST(id AS DOUBLE), 0.0)) >= 120 ORDER BY id ASC LIMIT 4";
        let actual = rows(&fixture.optimized, query).await?;
        assert_eq!(actual.len(), 4);
        assert_eq!(actual[0]["id"], 120);
        assert_eq!(actual, rows(&fixture.normal, query).await?);
        let mut config = datafusion::prelude::SessionConfig::new();
        config.options_mut().optimizer.enable_sort_pushdown = false;
        let disabled = SessionContext::new_with_state(with_lance_order_pushdown(
            SessionContext::new_with_config(config).state(),
        ));
        disabled.register_table(
            "articles",
            lance_table_provider(fixture.table.clone(), false).await?,
        )?;
        let plan = disabled
            .sql("SELECT id, content FROM articles ORDER BY id DESC LIMIT 4")
            .await?
            .create_physical_plan()
            .await?;
        assert!(find_native(&plan).is_none());
        Ok(())
    }

    #[tokio::test]
    async fn planned_native_top_k_retains_its_dataset_snapshot() -> flow_like_types::Result<()> {
        let fixture = Fixture::new().await?;
        let query = "SELECT id, content FROM articles ORDER BY id DESC LIMIT 2";
        let expected = rows(&fixture.normal, query).await?;
        let plan = fixture
            .optimized
            .sql(query)
            .await?
            .create_physical_plan()
            .await?;
        let native = find_native(&plan).expect("native top-K");
        let version = native.source.dataset.version_id();
        fixture.table.delete("id >= 126").await?;
        assert!(fixture.table.version().await? > version);
        let batches =
            datafusion::physical_plan::collect(plan, fixture.optimized.task_ctx()).await?;
        let actual = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<flow_like_types::Result<Vec<_>>>()?
            .concat();
        assert_eq!(actual, expected);
        assert_ne!(rows(&fixture.optimized, query).await?, expected);
        Ok(())
    }

    #[tokio::test]
    async fn volatile_payload_below_sort_keeps_its_evaluation_position()
    -> flow_like_types::Result<()> {
        use datafusion::physical_expr::ScalarFunctionExpr;

        let fixture = Fixture::new().await?;
        let plan = fixture
            .optimized
            .sql("SELECT id, content FROM articles ORDER BY id DESC LIMIT 2")
            .await?
            .create_physical_plan()
            .await?;
        let source = Arc::new(find_native(&plan).expect("native top-K").source.clone());
        let random: Arc<dyn PhysicalExpr> = Arc::new(ScalarFunctionExpr::try_new(
            datafusion::functions::math::random(),
            vec![],
            &source.schema(),
            Arc::new(ConfigOptions::new()),
        )?);
        let id: Arc<dyn PhysicalExpr> = Arc::new(Column::new("id", 0));
        let projection: Arc<dyn ExecutionPlan> = Arc::new(ProjectionExec::try_new(
            vec![(id.clone(), "id".to_owned()), (random, "random".to_owned())],
            source,
        )?);
        let ordering = [PhysicalSortExpr {
            expr: id,
            options: Default::default(),
        }];
        assert!(ordered_input(&projection, &ordering, 2)?.is_none());
        Ok(())
    }
}

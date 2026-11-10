use super::{WorkbenchSurface, WorkbenchView, bind_params, build_context, validate_workbench_sql};
use datafusion::{common::tree_node::TreeNodeRecursion, logical_expr::LogicalPlan};
use flow_like_storage_contracts::column_metadata::{ColumnReader, read_attributes};
use flow_like_types::{Result, Value, anyhow};
use lancedb::Connection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MAXIMUM_REQUIREMENTS: usize = 4096;

/// A source column and its policy when a query was planned. A missing column
/// records a table dependency such as COUNT(*) without requiring unrelated fields.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceColumnRequirement {
    pub table: String,
    pub column: Option<String>,
    pub read_attributes: Vec<String>,
}

fn collect(
    plan: &LogicalPlan,
    depth: usize,
    result: &mut BTreeSet<SourceColumnRequirement>,
) -> datafusion::error::Result<()> {
    if depth > 64 {
        return Err(datafusion::error::DataFusionError::Plan(
            "Saved query source dependencies are too deeply nested".into(),
        ));
    }
    plan.apply_with_subqueries(|node| {
        if let LogicalPlan::TableScan(scan) = node {
            if let Some(view) = scan.source.get_logical_plan() {
                collect(&view, depth + 1, result)?;
            } else {
                let table = scan.table_name.table().to_string();
                if super::is_reserved_table(&table) {
                    return Err(datafusion::error::DataFusionError::Plan(
                        "Saved forecasts cannot read reserved tables".into(),
                    ));
                }
                let schema = scan.source.schema();
                let mut indices: BTreeSet<usize> = scan
                    .projection
                    .as_ref()
                    .map(|indices| indices.iter().copied().collect())
                    .unwrap_or_else(|| (0..schema.fields().len()).collect());
                // Pushed filters can read a column that the scan does not return.
                for expression in &scan.filters {
                    for column in expression.column_refs() {
                        indices.insert(schema.index_of(&column.name)?);
                    }
                }
                if indices.is_empty() {
                    result.insert(SourceColumnRequirement {
                        table: table.clone(),
                        column: None,
                        read_attributes: vec![],
                    });
                }
                for index in indices {
                    let field = schema.fields().get(index).ok_or_else(|| {
                        datafusion::error::DataFusionError::Plan(
                            "Query source projection is invalid".into(),
                        )
                    })?;
                    let mut attributes = read_attributes(field.metadata());
                    attributes.sort();
                    result.insert(SourceColumnRequirement {
                        table: table.clone(),
                        column: Some(field.name().clone()),
                        read_attributes: attributes,
                    });
                }
                if result.len() > MAXIMUM_REQUIREMENTS {
                    return Err(datafusion::error::DataFusionError::Plan(
                        "Saved forecast has too many source columns".into(),
                    ));
                }
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(())
}

/// Resolves SQL and views without executing data scans. Planning uses the same
/// restricted providers as workbench execution, and captures filters and joins
/// as well as projected fields.
pub async fn capture(
    connection: &Connection,
    surface: &WorkbenchSurface,
    views: &[WorkbenchView],
    reader: &ColumnReader,
    sql: &str,
    params: &Value,
) -> Result<Vec<SourceColumnRequirement>> {
    validate_workbench_sql(sql)?;
    let future = async {
        let semaphore = crate::databases::graph::lancegraph::global_query_semaphore(
            crate::databases::graph::lancegraph::CypherSafetyConfig::default().max_concurrent,
        );
        let _permit = semaphore.acquire().await?;
        let context = build_context(connection, surface, views, sql, reader).await?;
        let plan = context
            .sql(sql)
            .await?
            .with_param_values(bind_params(params)?)?
            .into_unoptimized_plan();
        let state = context.state();
        let plan = state
            .analyzer()
            .execute_and_check(plan, state.config_options(), |_, _| {})?;
        // Only prune unused projections. General optimization can replace a
        // sensitive aggregate with a literal from table statistics and erase
        // the source dependency we need to preserve for later readers.
        let optimizer = datafusion::optimizer::Optimizer::with_rules(vec![std::sync::Arc::new(
            datafusion::optimizer::optimize_projections::OptimizeProjections::new(),
        )]);
        let options = datafusion::optimizer::OptimizerContext::new_with_config_options(
            state.config_options().clone(),
        );
        let plan = optimizer.optimize(plan, &options, |_, _| {})?;
        let mut requirements = BTreeSet::new();
        collect(&plan, 0, &mut requirements)?;
        Ok(requirements.into_iter().collect())
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), future)
        .await
        .map_err(|_| anyhow!("Planning saved forecast source access exceeded 30 seconds"))?
}

/// Requires both the captured policy and the latest source policy. Removed
/// tables or columns deny access because their current policy cannot be checked.
pub async fn allows(
    connection: &Connection,
    reader: &ColumnReader,
    requirements: &[SourceColumnRequirement],
) -> Result<bool> {
    AccessChecker::new(connection, reader)
        .allows(requirements)
        .await
}

/// Reuses fresh source schemas within a single list/read operation.
pub struct AccessChecker<'a> {
    connection: &'a Connection,
    reader: &'a ColumnReader,
    schemas: std::collections::HashMap<String, Option<arrow_schema::SchemaRef>>,
}

impl<'a> AccessChecker<'a> {
    pub fn new(connection: &'a Connection, reader: &'a ColumnReader) -> Self {
        Self {
            connection,
            reader,
            schemas: std::collections::HashMap::new(),
        }
    }

    pub async fn allows(&mut self, requirements: &[SourceColumnRequirement]) -> Result<bool> {
        if requirements.len() > MAXIMUM_REQUIREMENTS {
            return Ok(false);
        }
        for requirement in requirements {
            if super::is_reserved_table(&requirement.table)
                || !self.reader.can_read(&requirement.read_attributes)
            {
                return Ok(false);
            }
            if !self.schemas.contains_key(&requirement.table) {
                let table = match self
                    .connection
                    .open_table(&requirement.table)
                    .execute()
                    .await
                {
                    Ok(table) => table,
                    Err(lancedb::Error::TableNotFound { .. }) => {
                        self.schemas.insert(requirement.table.clone(), None);
                        return Ok(false);
                    }
                    Err(error) => return Err(error.into()),
                };
                table.checkout_latest().await?;
                self.schemas
                    .insert(requirement.table.clone(), Some(table.schema().await?));
            }
            let Some(schema) = &self.schemas[&requirement.table] else {
                return Ok(false);
            };
            if let Some(column) = &requirement.column {
                let Ok(field) = schema.field_with_name(column) else {
                    return Ok(false);
                };
                if !self.reader.can_read_field(field.metadata()) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}

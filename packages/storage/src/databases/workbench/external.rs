//! Tables served by something other than an app's LanceDB, such as an external database. The
//! storage layer only turns a table name into a provider; connecting, credentials and network
//! policy belong to whoever implements the resolver.

use datafusion::catalog::{MemorySchemaProvider, TableProvider};
use datafusion::common::TableReference;
use datafusion::prelude::SessionContext;
use flow_like_types::{Result, anyhow, async_trait};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[async_trait]
pub trait ExternalTableResolver: Send + Sync {
    /// A provider for `table` of this source, or an error that names why it cannot be read.
    async fn table(&self, table: &str) -> Result<Arc<dyn TableProvider>>;
}

/// One external source in a session, addressed as `<name>.<table>`. External tables carry no
/// version, so results that read them can only expire by time.
#[derive(Clone)]
pub struct ExternalSchema {
    /// A lowercase SQL identifier, unique among every schema of the session.
    pub name: String,
    pub resolver: Arc<dyn ExternalTableResolver>,
}

/// `schema.table` references of `statements` that point at one of `externals`, grouped by
/// schema. Bare and catalog-qualified references are left to the caller.
pub(crate) fn external_references(
    ctx: &SessionContext,
    statements: &[&str],
    externals: &[ExternalSchema],
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut wanted: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if externals.is_empty() {
        return Ok(wanted);
    }
    let state = ctx.state();
    let options = state.config().options();
    let default_catalog = options.catalog.default_catalog.clone();
    for sql in statements {
        let statement = state.sql_to_statement(sql, &options.sql_parser.dialect)?;
        for reference in state.resolve_table_references(&statement)? {
            let (schema, table) = match &reference {
                TableReference::Partial { schema, table } => (schema.as_ref(), table.as_ref()),
                TableReference::Full {
                    catalog,
                    schema,
                    table,
                } if catalog.as_ref() == default_catalog => (schema.as_ref(), table.as_ref()),
                _ => continue,
            };
            if let Some(external) = externals.iter().find(|external| external.name == schema) {
                wanted
                    .entry(external.name.clone())
                    .or_default()
                    .insert(table.to_string());
            }
        }
    }
    Ok(wanted)
}

/// Registers every referenced table of `externals` in `ctx` and returns their `schema.table`
/// names. Only named tables are opened, so an unused external source costs nothing.
pub(crate) async fn register_external_tables(
    ctx: &SessionContext,
    statements: &[&str],
    externals: &[ExternalSchema],
) -> Result<Vec<String>> {
    let wanted = external_references(ctx, statements, externals)?;
    let default_catalog = ctx
        .state()
        .config()
        .options()
        .catalog
        .default_catalog
        .clone();
    let catalog = ctx
        .catalog(&default_catalog)
        .ok_or_else(|| anyhow!("The session has no default catalog '{default_catalog}'"))?;
    let mut registered = Vec::new();
    for (schema_name, tables) in wanted {
        let Some(external) = externals.iter().find(|e| e.name == schema_name) else {
            continue;
        };
        if catalog.schema(&schema_name).is_none() {
            catalog.register_schema(&schema_name, Arc::new(MemorySchemaProvider::new()))?;
        }
        for table in tables {
            let provider = external
                .resolver
                .table(&table)
                .await
                .map_err(|error| anyhow!("Failed to open '{schema_name}.{table}': {error}"))?;
            ctx.register_table(
                TableReference::partial(schema_name.as_str(), table.as_str()),
                provider,
            )?;
            registered.push(format!("{schema_name}.{table}"));
        }
    }
    Ok(registered)
}

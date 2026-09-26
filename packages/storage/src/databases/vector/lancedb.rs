use arrow_array::RecordBatch;
use arrow_schema::{DataType, Schema};
use datafusion::catalog::TableProvider;
use datafusion::prelude::*;
pub use flow_like_storage_contracts::database::{
    DatabaseBranch, DatabaseCleanupStats, DatabaseReference, DatabaseSelector, DatabaseTag,
    DatabaseVersion,
};
use flow_like_types::Cacheable;
use flow_like_types::async_trait;
use flow_like_types::{Result, Value, anyhow};
use futures::TryStreamExt;
use lance::index::{DatasetIndexExt, DatasetIndexInternalExt};
use lance_index::metrics::NoOpMetricsCollector;
use lance_index::scalar::{BuiltinIndexType, ScalarIndexParams};
use lancedb::index::IndexConfig;
use lancedb::index::scalar::BTreeIndexBuilder;
use lancedb::index::scalar::BitmapIndexBuilder;
use lancedb::index::scalar::FmIndexBuilder;
use lancedb::index::scalar::LabelListIndexBuilder;
use lancedb::index::vector::{
    IvfFlatIndexBuilder, IvfHnswFlatIndexBuilder, IvfHnswPqIndexBuilder, IvfHnswSqIndexBuilder,
    IvfPqIndexBuilder, IvfRqIndexBuilder, IvfSqIndexBuilder,
};
use lancedb::query::QueryExecutionOptions;
use lancedb::table::AddColumnsResult;
use lancedb::table::AlterColumnsResult;
use lancedb::table::ColumnAlteration;
use lancedb::table::NewColumnTransform;
use lancedb::table::WriteOptions;
use lancedb::{
    Connection, Table, connect,
    index::{
        Index,
        scalar::{FtsIndexBuilder, FullTextSearchQuery},
    },
    query::{ExecutableQuery, QueryBase},
    table::{CompactionOptions, Duration, OptimizeOptions},
};

use std::{any::Any, collections::HashMap, path::PathBuf, sync::Arc};

use crate::arrow_utils::record_batch_to_value;
use crate::arrow_utils::{
    ValueBatchReader, value_to_batch_reader_with_fields,
    value_to_batch_reader_with_utc_timestamp_inference,
};
use crate::databases::df_provider::{zero_column_safe, zero_column_safe_writable};
use crate::databases::lance_filter_params::orient_spatial_relations;

use super::VectorStore;
use super::schema::{
    PrimaryKeyRejected, TableInputRejected, primary_key_columns, primary_key_ineligibility,
    primary_key_marker_update, without_primary_key_marker,
};

#[cfg(test)]
#[path = "reference_tests.rs"]
mod reference_tests;

#[cfg(test)]
#[path = "mutation_tests.rs"]
mod mutation_tests;

/// Tables live at the database root, so LanceDB's namespace manifest only
/// costs a LIST + GET per connect and creates `__manifest` in fresh roots.
pub fn connect_lance(uri: &str) -> lancedb::connection::ConnectBuilder {
    connect(uri).namespace_client_property("manifest_enabled", "false")
}

#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema, Clone, Debug)]
pub struct IndexConfigDto {
    pub name: String,
    pub index_type: String, // render enum via Display
    pub columns: Vec<String>,
}

impl From<IndexConfig> for IndexConfigDto {
    fn from(idx: IndexConfig) -> Self {
        Self {
            name: idx.name,
            index_type: idx.index_type.to_string(),
            columns: idx.columns,
        }
    }
}

/// Include native Lance indexes that LanceDB's index enum cannot represent.
pub async fn list_table_indices(table: &Table) -> Result<Vec<IndexConfigDto>> {
    if let Some(wrapper) = table.dataset() {
        let dataset = wrapper.get().await?;
        let metadata = dataset.load_indices().await?;
        let mut indices = std::collections::BTreeMap::new();
        for index in metadata.iter() {
            if lance_index::infer_system_index_type(index).is_some()
                || indices.contains_key(&index.name)
            {
                continue;
            }
            let Ok(columns) = index
                .fields
                .iter()
                .map(|id| dataset.schema().field_path(*id))
                .collect::<std::result::Result<Vec<_>, _>>()
            else {
                continue;
            };
            let Some(column) = columns.first() else {
                continue;
            };
            let declared_type = index.index_details.as_ref().and_then(|details| {
                lance::index::scalar::IndexDetails(details.clone())
                    .get_plugin()
                    .ok()
                    .and_then(|plugin| exposed_index_type(plugin.name()))
            });
            let index_type = if let Some(kind) = declared_type {
                kind
            } else {
                // Read the physical type for vectors and legacy metadata. Avoid
                // describe_indices(): it requires coverage metadata absent in older
                // indexes. index_statistics() can also migrate a manifest.
                let Ok(opened) = dataset
                    .open_generic_index(column, &index.uuid, &NoOpMetricsCollector)
                    .await
                else {
                    continue;
                };
                let statistics = if opened.index_type().is_scalar() {
                    None
                } else {
                    opened.statistics().ok()
                };
                let kind = statistics
                    .as_ref()
                    .and_then(|s| s.get("index_type"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| opened.index_type().to_string());
                let Some(kind) = exposed_index_type(&kind) else {
                    continue;
                };
                kind
            };
            indices.insert(
                index.name.clone(),
                IndexConfigDto {
                    name: index.name.clone(),
                    index_type,
                    columns,
                },
            );
        }
        return Ok(indices.into_values().collect());
    }
    Ok(table
        .list_indices()
        .await?
        .into_iter()
        .map(Into::into)
        .collect())
}

fn exposed_index_type(kind: &str) -> Option<String> {
    kind.parse::<lancedb::index::IndexType>()
        .map(|kind| kind.to_string())
        .ok()
        .or_else(|| native_scalar_index(Some(kind)).map(|kind| kind.as_str().to_ascii_uppercase()))
}

fn validate_new_columns(transform: &NewColumnTransform) -> Result<()> {
    use datafusion::sql::parser::DFParser;
    use datafusion::sql::sqlparser::ast::{Expr, Value as SqlValue};

    if let NewColumnTransform::SqlExpressions(expressions) = transform {
        for (name, sql) in expressions {
            // Preserve the node's typed-column contract now that Lance accepts
            // Null fields. Leave other expression validation to Lance's planner.
            if let Ok(parsed) = DFParser::parse_sql_into_expr(sql) {
                let mut expression = &parsed.expr;
                while let Expr::Nested(inner) = expression {
                    expression = inner;
                }
                if matches!(expression, Expr::Value(value) if value.value == SqlValue::Null) {
                    return Err(anyhow!(
                        "Column '{name}' requires a typed expression; use CAST(NULL AS <type>) instead of bare NULL"
                    ));
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogicalTableMutation {
    Insert {
        items: Vec<Value>,
    },
    Upsert {
        items: Vec<Value>,
        id_field: String,
    },
    Update {
        filter: String,
        updates: Vec<(String, String)>,
    },
    Delete {
        filter: String,
    },
}

/// A local durable acknowledgement. Cloud acknowledgement is tracked separately
/// by the adapter's replay service.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct LocalWriteReceipt {
    pub operation_id: String,
    pub sequence: u64,
    pub state: String,
}

#[async_trait]
pub trait LogicalTableMutationAdapter: Send + Sync {
    /// Return only after the operation and its local materialization can survive
    /// a process crash. Retries and remote acknowledgement belong to the adapter.
    async fn apply(&self, mutation: LogicalTableMutation) -> Result<LocalWriteReceipt>;

    /// Recover pending local materialization before returning a complete view.
    /// None means confirmed absence, never a partial or unavailable snapshot.
    async fn read_table(&self) -> Result<Option<Table>>;

    /// Advance when a retained query provider needs to be rebuilt.
    fn generation(&self) -> u64 {
        0
    }
}

#[derive(Clone)]
pub struct LanceDBVectorStore {
    connection: Connection,
    table: Option<Table>,
    table_name: String,
    write_options: Option<WriteOptions>,
    selector: DatabaseSelector,
    mutation_adapter: Option<Arc<dyn LogicalTableMutationAdapter>>,
    last_write_receipt: Arc<std::sync::RwLock<Option<LocalWriteReceipt>>>,
    /// ID columns upserts ruled out as the table key, with the column's type and
    /// nullability at that time; a changed column is checked again.
    unmarkable_keys: HashMap<String, Option<(DataType, bool)>>,
}

impl Cacheable for LanceDBVectorStore {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
impl LanceDBVectorStore {
    pub fn validate_table_name(table_name: &str) -> Result<()> {
        lancedb::utils::validate_table_name(table_name)?;
        Ok(())
    }

    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    pub fn connection(&self) -> Result<&Connection> {
        self.ensure_unmanaged("raw database connections")?;
        Ok(&self.connection)
    }

    pub fn with_mutation_adapter(mut self, adapter: Arc<dyn LogicalTableMutationAdapter>) -> Self {
        self.mutation_adapter = Some(adapter);
        self
    }

    pub fn is_durably_managed(&self) -> bool {
        self.mutation_adapter.is_some()
    }

    pub fn last_write_receipt(&self) -> Option<LocalWriteReceipt> {
        self.last_write_receipt
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn mutation_generation(&self) -> u64 {
        self.mutation_adapter
            .as_ref()
            .map_or(0, |adapter| adapter.generation())
    }

    fn ensure_unmanaged(&self, operation: &str) -> Result<()> {
        if self.is_durably_managed() {
            return Err(anyhow!(
                "Offline-buffered tables do not support {operation}; use logical row operations"
            ));
        }
        Ok(())
    }

    async fn readable_table(&self) -> Result<Option<Table>> {
        match &self.mutation_adapter {
            Some(adapter) => adapter.read_table().await,
            None => Ok(self.table.clone()),
        }
    }

    async fn require_readable_table(&self) -> Result<Table> {
        self.readable_table()
            .await?
            .ok_or_else(|| anyhow!("Table not initialized"))
    }

    pub async fn table_exists(&self) -> Result<bool> {
        Ok(self.readable_table().await?.is_some())
    }

    async fn apply_logical_mutation(&self, mutation: LogicalTableMutation) -> Result<()> {
        self.ensure_writable()?;
        let adapter = self
            .mutation_adapter
            .as_ref()
            .ok_or_else(|| anyhow!("Missing logical mutation adapter"))?;
        let receipt = adapter.apply(mutation).await?;
        *self
            .last_write_receipt
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Some(receipt);
        Ok(())
    }

    pub async fn new(path: PathBuf, table_name: String) -> Result<Self> {
        Self::validate_table_name(&table_name)?;
        let connection = connect_lance(path.to_str().unwrap()).execute().await.ok();
        let connection: Connection = connection.ok_or(anyhow!("Error connecting to LanceDB"))?;

        let table = connection.open_table(&table_name).execute().await.ok();

        Ok(LanceDBVectorStore {
            connection,
            table,
            table_name,
            write_options: None,
            selector: DatabaseSelector::default(),
            mutation_adapter: None,
            last_write_receipt: Default::default(),
            unmarkable_keys: Default::default(),
        })
    }

    pub async fn from_connection(connection: Connection, table_name: String) -> Self {
        // LanceDB 0.27's listing backend unwraps table-name validation while
        // deriving the table URI. Never call it with invalid input; callers
        // that need a table will receive the existing "Table not initialized"
        // error instead of taking down the runtime with a dependency panic.
        let table = if Self::validate_table_name(&table_name).is_ok() {
            connection.open_table(&table_name).execute().await.ok()
        } else {
            None
        };

        LanceDBVectorStore {
            connection,
            table,
            table_name,
            write_options: None,
            selector: DatabaseSelector::default(),
            mutation_adapter: None,
            last_write_receipt: Default::default(),
            unmarkable_keys: Default::default(),
        }
    }

    /// Construct the table handle before installing a local mutation adapter.
    /// Opening cloud metadata here would delay every new run during an outage.
    pub fn from_connection_for_overlay(
        connection: Connection,
        table_name: String,
        selector: DatabaseSelector,
    ) -> Result<Self> {
        Self::validate_table_name(&table_name)?;
        Self::validate_overlay_selector(&selector)?;
        Ok(Self {
            connection,
            table: None,
            table_name,
            write_options: None,
            selector,
            mutation_adapter: None,
            last_write_receipt: Default::default(),
            unmarkable_keys: Default::default(),
        })
    }

    pub fn validate_overlay_selector(selector: &DatabaseSelector) -> Result<()> {
        selector.validate()?;
        if selector.branch != "main" || selector.version.is_some() || selector.tag.is_some() {
            return Err(anyhow!(
                "Offline-buffered tables expose only their current version; open branches, tags or versions from a run without offline buffering"
            ));
        }
        Ok(())
    }

    /// Open an existing reference strictly. A missing branch, tag, or version is an error.
    pub async fn from_connection_with_selector(
        connection: Connection,
        table_name: String,
        mut selector: DatabaseSelector,
    ) -> Result<Self> {
        Self::validate_table_name(&table_name)?;
        selector.validate()?;
        if let Some(tag) = selector.tag.take() {
            let root = connection.open_table(&table_name).execute().await?;
            let tags = root.tags().await?.list().await?;
            let contents = tags
                .get(&tag)
                .ok_or_else(|| anyhow!("Tag '{tag}' does not exist"))?;
            selector.branch = contents.branch.clone().unwrap_or_else(|| "main".into());
            selector.version = Some(contents.version);
        }

        let mut open = connection.open_table(&table_name).branch(&selector.branch);
        if let Some(version) = selector.version {
            open = open.version(version);
        }
        let table = open.execute().await?;
        Ok(Self {
            connection,
            table: Some(table),
            table_name,
            write_options: None,
            selector,
            mutation_adapter: None,
            last_write_receipt: Default::default(),
            unmarkable_keys: Default::default(),
        })
    }

    /// Tags are resolved to immutable branch/version pins before this value is returned.
    pub fn selector(&self) -> DatabaseSelector {
        self.selector.clone()
    }

    pub async fn reference(&self) -> Result<DatabaseReference> {
        let table = self.require_readable_table().await?;
        Ok(DatabaseReference {
            table: self.table_name.clone(),
            branch: table.current_branch().unwrap_or_else(|| "main".into()),
            version: table.version().await?,
            read_only: self.selector.is_read_only(),
            pinned: self.selector.is_pinned(),
        })
    }

    /// Open another handle. Never change a shared LanceDB table handle in place.
    pub async fn checkout(&self, mut selector: DatabaseSelector) -> Result<Self> {
        self.ensure_unmanaged("reference checkout")?;
        // A read-only capability cannot be promoted through checkout.
        selector.read_only |= self.selector.read_only;
        let mut store = Self::from_connection_with_selector(
            self.connection.clone(),
            self.table_name.clone(),
            selector,
        )
        .await?;
        store.write_options = self.write_options.clone();
        Ok(store)
    }

    /// Replace credentials without moving a branch or a resolved snapshot.
    pub async fn reopen(&self, connection: Connection) -> Result<Self> {
        self.ensure_unmanaged("raw database reconnection")?;
        let mut store = if self.table.is_none() && self.selector == DatabaseSelector::default() {
            Self::from_connection(connection, self.table_name.clone()).await
        } else {
            Self::from_connection_with_selector(
                connection,
                self.table_name.clone(),
                self.selector(),
            )
            .await?
        };
        store.write_options = self.write_options.clone();
        Ok(store)
    }

    pub fn ensure_writable(&self) -> Result<()> {
        if self.selector.is_read_only() {
            return Err(anyhow!(
                "Database reference is read-only; open the latest branch to write"
            ));
        }
        Ok(())
    }

    fn ensure_reference_management(&self) -> Result<()> {
        self.ensure_unmanaged("reference management")?;
        if self.selector.read_only {
            return Err(anyhow!(
                "Reference management is disabled for a read-only database"
            ));
        }
        Ok(())
    }

    pub async fn list_versions(&self) -> Result<Vec<DatabaseVersion>> {
        let mut versions = self
            .raw()
            .await?
            .list_versions()
            .await?
            .into_iter()
            .map(|version| DatabaseVersion {
                version: version.version,
                timestamp: version.timestamp.to_rfc3339(),
                metadata: version.metadata,
            })
            .collect::<Vec<_>>();
        versions.sort_by_key(|version| std::cmp::Reverse(version.version));
        Ok(versions)
    }

    pub async fn list_branches(&self) -> Result<Vec<DatabaseBranch>> {
        let mut branches = self
            .raw()
            .await?
            .list_branches()
            .await?
            .into_iter()
            .map(|(name, branch)| DatabaseBranch {
                name,
                parent_branch: Some(branch.parent_branch.unwrap_or_else(|| "main".into())),
                parent_version: Some(branch.parent_version),
                created_at: Some(branch.create_at),
            })
            .collect::<Vec<_>>();
        if !branches.iter().any(|branch| branch.name == "main") {
            branches.push(DatabaseBranch {
                name: "main".into(),
                parent_branch: None,
                parent_version: None,
                created_at: None,
            });
        }
        branches.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(branches)
    }

    pub async fn list_tags(&self) -> Result<Vec<DatabaseTag>> {
        let table = self.raw().await?;
        let mut tags = table
            .tags()
            .await?
            .list()
            .await?
            .into_iter()
            .map(|(name, tag)| DatabaseTag {
                name,
                branch: tag.branch.unwrap_or_else(|| "main".into()),
                version: tag.version,
                created_at: tag.created_at.map(|date| date.to_rfc3339()),
                updated_at: tag.updated_at.map(|date| date.to_rfc3339()),
            })
            .collect::<Vec<_>>();
        tags.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tags)
    }

    pub async fn create_branch(&self, name: &str) -> Result<Self> {
        self.ensure_reference_management()?;
        if name == "main" {
            return Err(anyhow!("The main branch already exists"));
        }
        let reference = self.reference().await?;
        let table = self
            .raw()
            .await?
            .create_branch(name, (reference.branch.as_str(), reference.version))
            .await?;
        Ok(Self {
            connection: self.connection.clone(),
            table: Some(table),
            table_name: self.table_name.clone(),
            write_options: self.write_options.clone(),
            selector: DatabaseSelector {
                branch: name.into(),
                ..Default::default()
            },
            mutation_adapter: None,
            last_write_receipt: Default::default(),
            unmarkable_keys: Default::default(),
        })
    }

    pub async fn delete_branch(&self, name: &str) -> Result<()> {
        self.ensure_reference_management()?;
        if name == "main" {
            return Err(anyhow!(
                "The main branch cannot be deleted; drop the table instead"
            ));
        }
        if name == self.selector.branch {
            return Err(anyhow!(
                "Open a different branch before deleting the selected branch"
            ));
        }
        if let Some(tag) = self
            .list_tags()
            .await?
            .into_iter()
            .find(|tag| tag.branch == name)
        {
            return Err(anyhow!(
                "Branch '{name}' is protected by tag '{}'; remove the tag first",
                tag.name
            ));
        }
        self.raw().await?.delete_branch(name).await?;
        Ok(())
    }

    pub async fn create_tag(&self, name: &str) -> Result<()> {
        self.ensure_reference_management()?;
        let table = self.raw().await?;
        let version = table.version().await?;
        table.tags().await?.create(name, version).await?;
        Ok(())
    }

    pub async fn update_tag(&self, name: &str) -> Result<()> {
        self.ensure_reference_management()?;
        self.ensure_clone_tag_unused(name).await?;
        let table = self.raw().await?;
        let version = table.version().await?;
        table.tags().await?.update(name, version).await?;
        Ok(())
    }

    pub async fn delete_tag(&self, name: &str) -> Result<()> {
        self.ensure_reference_management()?;
        self.ensure_clone_tag_unused(name).await?;
        self.raw().await?.tags().await?.delete(name).await?;
        Ok(())
    }

    /// Restore through a fresh handle, leaving all existing snapshot readers pinned.
    pub async fn restore(&self) -> Result<DatabaseReference> {
        self.ensure_reference_management()?;
        if !self.selector.is_pinned() {
            return Err(anyhow!(
                "Select a historical version or tag before restoring"
            ));
        }
        let mut restored = self.checkout(self.selector()).await?;
        restored
            .table
            .as_ref()
            .ok_or_else(|| anyhow!("Table not initialized"))?
            .restore()
            .await?;
        restored.selector.version = None;
        restored.selector.tag = None;
        restored.reference().await
    }

    /// Remove eligible history while preserving tagged versions and shared branch files.
    pub async fn cleanup_versions(&self, older_than_days: u64) -> Result<DatabaseCleanupStats> {
        self.ensure_unmanaged("version cleanup")?;
        self.ensure_writable()?;
        if older_than_days == 0 {
            return Err(anyhow!("Version retention must be at least one day"));
        }
        let days =
            i64::try_from(older_than_days).map_err(|_| anyhow!("Retention period is too large"))?;
        let retention =
            Duration::try_days(days).ok_or_else(|| anyhow!("Retention period is too large"))?;
        let stats = self
            .raw()
            .await?
            .optimize(lancedb::table::OptimizeAction::Prune {
                older_than: Some(retention),
                delete_unverified: Some(false),
                error_if_tagged_old_versions: Some(false),
            })
            .await?;
        let stats = stats.prune.unwrap_or_default();
        Ok(DatabaseCleanupStats {
            bytes_removed: stats.bytes_removed,
            old_versions: stats.old_versions,
            data_files_removed: stats.data_files_removed,
            transaction_files_removed: stats.transaction_files_removed,
            index_files_removed: stats.index_files_removed,
            deletion_files_removed: stats.deletion_files_removed,
        })
    }

    /// A shallow clone shares source files. Its retained source tag protects cleanup.
    pub async fn clone_table(&self, target: &str) -> Result<Self> {
        self.ensure_reference_management()?;
        Self::validate_table_name(target)?;
        if target == self.table_name {
            return Err(anyhow!("Choose a different table name for the clone"));
        }
        match self.connection.open_table(target).execute().await {
            Ok(_) => return Err(anyhow!("Table '{target}' already exists")),
            Err(lancedb::Error::TableNotFound { .. }) => {}
            Err(error) => return Err(error.into()),
        }
        let source = self.raw().await?;
        let source_uri = source.uri().await?;
        let source_version = source.version().await?;
        let retention_tag = format!("flow_clone_source_{target}");
        source
            .tags()
            .await?
            .create(&retention_tag, source_version)
            .await?;
        let result = self
            .connection
            .clone_table(target, source_uri)
            .source_tag(&retention_tag)
            .execute()
            .await;
        let table = match result {
            Ok(table) => table,
            Err(error) => {
                // The tag is safe to remove when no target was committed.
                if matches!(
                    self.connection.open_table(target).execute().await,
                    Err(lancedb::Error::TableNotFound { .. })
                ) {
                    source.tags().await?.delete(&retention_tag).await?;
                }
                return Err(error.into());
            }
        };
        Ok(Self {
            connection: self.connection.clone(),
            table: Some(table),
            table_name: target.into(),
            write_options: self.write_options.clone(),
            selector: DatabaseSelector::default(),
            mutation_adapter: None,
            last_write_receipt: Default::default(),
            unmarkable_keys: Default::default(),
        })
    }

    async fn ensure_clone_tag_unused(&self, name: &str) -> Result<()> {
        if let Some(target) = name.strip_prefix("flow_clone_source_") {
            if Self::validate_table_name(target).is_err() {
                return Ok(());
            }
            match self.connection.open_table(target).execute().await {
                Ok(_) => {
                    return Err(anyhow!(
                        "Table '{target}' shares source files protected by tag '{name}'; drop that clone first"
                    ));
                }
                Err(lancedb::Error::TableNotFound { .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    pub fn set_write_options(&mut self, options: WriteOptions) {
        self.write_options = Some(options);
    }

    fn creation_write_options(&self) -> WriteOptions {
        let mut options = self.write_options.clone().unwrap_or_default();
        // Creation must elect one writer even when later writes use Append.
        // Preserve credentials, store wrappers and all other write settings.
        options
            .lance_write_params
            .get_or_insert_with(Default::default)
            .mode = lance::dataset::WriteMode::Create;
        options
    }

    /// Create an empty table from an explicit schema, without inserting a seed row.
    ///
    /// Returns `true` when this call created the table and `false` when the table already
    /// existed and `if_not_exists` was enabled.
    pub async fn create_empty_table(
        &mut self,
        schema: Schema,
        if_not_exists: bool,
    ) -> Result<bool> {
        self.ensure_unmanaged("explicit table creation")?;
        self.ensure_writable()?;
        for field in schema.fields() {
            crate::geometry::validate_geometry_field(field)?;
        }
        let existed = self.table.is_some();
        if existed && if_not_exists {
            let existing_schema = self
                .table
                .as_ref()
                .expect("table existence was checked")
                .schema()
                .await?;
            if schemas_compatible_for_creation(existing_schema.as_ref(), &schema) {
                return Ok(false);
            }
            return Err(anyhow!(
                "Table '{}' already exists with a different schema",
                self.table_name
            ));
        }

        let requested_schema = Arc::new(schema);
        let builder = self
            .connection
            .create_empty_table(&self.table_name, requested_schema.clone())
            .write_options(self.creation_write_options());

        let (table, created) = match builder.execute().await {
            Ok(table) => (table, true),
            Err(lancedb::Error::TableAlreadyExists { .. }) if if_not_exists => {
                let table = self
                    .connection
                    .open_table(&self.table_name)
                    .execute()
                    .await?;
                (table, false)
            }
            Err(error) => return Err(error.into()),
        };
        if !created
            && !schemas_compatible_for_creation(
                table.schema().await?.as_ref(),
                requested_schema.as_ref(),
            )
        {
            return Err(anyhow!(
                "Table '{}' already exists with a different schema",
                self.table_name
            ));
        }
        self.table = Some(table);
        Ok(created)
    }

    /// Check table-wide drop restrictions before callers discard pending writes.
    /// Returns whether the table currently exists.
    pub async fn ensure_can_drop_table(&self) -> Result<bool> {
        self.ensure_unmanaged("table deletion")?;
        self.ensure_writable()?;
        if self.selector.branch != "main" {
            return Err(anyhow!(
                "Dropping a table removes every branch; open the latest main branch first"
            ));
        }
        let exists = self
            .connection
            .table_names()
            .execute()
            .await?
            .iter()
            .any(|name| name == &self.table_name);
        if exists {
            // The legacy constructor permits an unopened table for lazy creation.
            // Reopen strictly here so transient read errors cannot bypass dependencies.
            let table = self
                .connection
                .open_table(&self.table_name)
                .execute()
                .await?;
            for tag in table.tags().await?.list().await?.keys() {
                self.ensure_clone_tag_unused(tag).await?;
            }
        }
        Ok(exists)
    }

    /// Drop the whole table and every branch. Purge removes rows only.
    pub async fn drop_table(&mut self) -> Result<()> {
        let exists = self.ensure_can_drop_table().await?;
        if exists {
            self.connection.drop_table(&self.table_name, &[]).await?;
        }
        self.table = None;
        self.unmarkable_keys.clear();
        Ok(())
    }

    pub async fn list_tables(&self) -> Result<Vec<String>> {
        self.ensure_unmanaged("raw database listing")?;
        let tables = self.connection.table_names().execute().await?;
        Ok(tables)
    }

    /// Compact fragments, rebuild indices, and prune unprotected history.
    /// Tagged versions and branch dependencies remain available.
    pub async fn prune_history(&self) -> Result<()> {
        self.run_optimize_actions(prune_history_actions()).await
    }

    async fn run_optimize_actions(
        &self,
        actions: Vec<lancedb::table::OptimizeAction>,
    ) -> Result<()> {
        self.ensure_unmanaged("table optimization")?;
        self.ensure_writable()?;
        let table = self.table.clone().ok_or(anyhow!("Table not initialized"))?;
        let scalar_indices = scalar_indices_for_compaction(&table).await?;

        for action in actions {
            let compacting = matches!(&action, lancedb::table::OptimizeAction::Compact { .. });
            table.optimize(action).await?;
            if compacting {
                restore_compacted_scalar_indices(&table, &scalar_indices).await?;
            }
        }

        Ok(())
    }

    pub async fn add_columns(
        &self,
        transform: NewColumnTransform,
        read_columns: Option<Vec<String>>,
    ) -> Result<AddColumnsResult> {
        self.ensure_unmanaged("schema changes")?;
        self.ensure_writable()?;
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;
        // The merged schema replaces the manifest's; a stale one would drop a newer key.
        table.checkout_latest().await?;

        validate_new_columns(&transform)?;
        if let NewColumnTransform::SqlExpressions(expressions) = &transform {
            let schema = table.schema().await?;
            for (_, expression) in expressions {
                use datafusion::sql::sqlparser::{
                    dialect::GenericDialect,
                    tokenizer::{Token, Tokenizer},
                };
                let tokens = Tokenizer::new(&GenericDialect {}, expression)
                    .tokenize()
                    .map_err(|error| anyhow!("Invalid column expression: {error}"))?;
                for token in tokens {
                    if let Token::Word(word) = token {
                        let name = word.value.to_ascii_lowercase();
                        if name.starts_with("st_")
                            || name.starts_with("flow_geom")
                            || schema.fields().iter().any(|field| {
                                crate::geometry::is_geometry_field(field)
                                    && field.name().eq_ignore_ascii_case(&word.value)
                            })
                        {
                            return Err(anyhow!(
                                "Geometry expressions cannot add columns without preserving metadata; add a typed column with type 'geometry' instead, then write GeoJSON values"
                            ));
                        }
                    }
                }
            }
        }
        let result = table.add_columns(transform, read_columns).await?;
        Ok(result)
    }

    pub async fn drop_columns(&self, column_names: &[&str]) -> Result<()> {
        self.ensure_unmanaged("schema changes")?;
        self.ensure_writable()?;
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;

        table.checkout_latest().await?;
        let keys = table_primary_key(&table).await?;
        if let Some(key) = column_names
            .iter()
            .find_map(|name| targeted_key(&keys, name))
        {
            return Err(PrimaryKeyRejected(format!(
                "Column '{key}' is the key of table '{}'; the key column cannot be removed",
                self.table_name
            ))
            .into());
        }
        table.drop_columns(column_names).await?;
        Ok(())
    }

    pub async fn alter_column(
        &self,
        alteration: &[ColumnAlteration],
    ) -> Result<AlterColumnsResult> {
        self.ensure_unmanaged("schema changes")?;
        self.ensure_writable()?;
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;

        // A change built on a stale schema would commit over, and drop, a newer key.
        table.checkout_latest().await?;
        let schema = table.schema().await?;
        let keys = primary_key_columns(&schema);
        for change in alteration {
            let root = change.path.split('.').next().unwrap_or(&change.path);
            if change.data_type.is_some()
                && schema.fields().iter().any(|field| {
                    field.name() == root && crate::geometry::contains_geometry_field(field)
                })
            {
                return Err(anyhow!(
                    "Geometry column types cannot be altered; create a declared geometry column and insert validated values"
                ));
            }
            self.ensure_key_alteration_allowed(&keys, change)?;
        }
        let result = table.alter_columns(alteration).await?;
        Ok(result)
    }

    /// Lance accepts both changes, but a nullable key fails every later write.
    fn ensure_key_alteration_allowed(
        &self,
        keys: &[String],
        change: &ColumnAlteration,
    ) -> Result<()> {
        let Some(key) = targeted_key(keys, &change.path) else {
            return Ok(());
        };
        if change.nullable != Some(true) && change.data_type.is_none() {
            return Ok(());
        }
        Err(PrimaryKeyRejected(format!(
            "Column '{key}' is the key of table '{}'; the key column must stay required and keep its type",
            self.table_name
        ))
        .into())
    }

    pub async fn list_indices(&self) -> Result<Vec<IndexConfigDto>> {
        let indices = self.require_readable_table().await?;
        list_table_indices(&indices).await
    }

    pub async fn drop_index(&self, name: &str) -> Result<()> {
        self.ensure_unmanaged("index changes")?;
        self.ensure_writable()?;
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;
        table.drop_index(name).await?;
        Ok(())
    }

    pub async fn update(
        &self,
        filter: &str,
        updates: std::collections::HashMap<String, Value>,
    ) -> Result<()> {
        self.ensure_writable()?;
        let filter = orient_spatial_relations(filter)?;
        let filter = filter.as_str();
        let table = self.require_readable_table().await?;
        let schema = table.schema().await?;
        let mut unknown: Vec<&str> = updates
            .keys()
            .map(String::as_str)
            .filter(|column| schema.field_with_name(column).is_err())
            .collect();
        if !unknown.is_empty() {
            unknown.sort_unstable();
            return Err(
                crate::arrow_utils::unknown_columns(&self.table_name, &unknown, &schema).into(),
            );
        }
        let expressions = updates
            .into_iter()
            .map(|(column, value)| {
                let literal =
                    update_sql_literal(&self.table_name, schema.field_with_name(&column)?, &value)?;
                Ok((column, literal))
            })
            .collect::<Result<Vec<_>>>()?;
        if self.is_durably_managed() {
            return self
                .apply_logical_mutation(LogicalTableMutation::Update {
                    filter: filter.to_string(),
                    updates: expressions,
                })
                .await;
        }
        let mut op = table.update().only_if(filter);
        for (column, value) in expressions {
            op = op.column(&column, &value);
        }
        op.execute().await?;
        Ok(())
    }

    pub async fn add_column(&self, name: &str, sql_expression: &str) -> Result<()> {
        let transform = NewColumnTransform::SqlExpressions(vec![(
            name.to_string(),
            sql_expression.to_string(),
        )]);
        self.add_columns(transform, None).await?;
        Ok(())
    }

    /// Adds a nullable column of a `create_table` type, null in every existing row. Unlike an
    /// SQL expression, the declared field keeps geometry's WKB/WGS84 metadata.
    pub async fn add_typed_column(
        &self,
        name: &str,
        data_type: &str,
        vector_size: Option<u32>,
    ) -> Result<()> {
        let schema = super::schema::database_fields_to_arrow_schema(&[
            super::schema::DatabaseSchemaField {
                name: name.to_string(),
                data_type: data_type.to_string(),
                nullable: true,
                vector_size,
                primary_key: false,
            },
        ])?;
        self.add_columns(NewColumnTransform::AllNulls(Arc::new(schema)), None)
            .await?;
        Ok(())
    }

    /// A column definition from the API or FlowPilot: exactly one of an SQL expression computed
    /// from existing columns, or a type for an initially empty column.
    pub async fn add_column_definition(
        &self,
        name: &str,
        sql_expression: Option<&str>,
        data_type: Option<&str>,
        vector_size: Option<u32>,
    ) -> Result<()> {
        match (sql_expression, data_type) {
            (Some(expression), None) => self.add_column(name, expression).await,
            (None, Some(data_type)) => self.add_typed_column(name, data_type, vector_size).await,
            _ => Err(anyhow!(
                "Column '{name}' needs exactly one of sql_expression or type"
            )),
        }
    }

    pub async fn make_column_nullable(&self, column: &str, nullable: bool) -> Result<()> {
        let alteration = ColumnAlteration::new(column.to_string()).set_nullable(nullable);
        self.alter_column(&[alteration]).await?;
        Ok(())
    }

    /// The table key (Lance unenforced primary key), when exactly one column carries it.
    pub async fn primary_key(&self) -> Result<Option<String>> {
        let table = self.require_readable_table().await?;
        let keys = table_primary_key(&table).await?;
        Ok(<[String; 1]>::try_from(keys).ok().map(|[key]| key))
    }

    /// Mark `column` as the table key. Repeating the current key succeeds; the key never changes.
    pub async fn set_primary_key(&self, column: &str) -> Result<()> {
        self.ensure_unmanaged("schema changes")?;
        self.ensure_writable()?;
        let table = self.table.clone().ok_or_else(|| {
            anyhow!(
                "Table '{}' does not exist; create it before setting its key",
                self.table_name
            )
        })?;
        table.checkout_latest().await?;
        let keys = table_primary_key(&table).await?;
        if !keys.is_empty() {
            return self.expect_primary_key(&keys, column);
        }
        self.ensure_key_candidate(&table, column).await?;

        let Err(error) = table
            .update_field_metadata(&[primary_key_marker_update(column)])
            .await
        else {
            return Ok(());
        };
        table.checkout_latest().await?;
        let keys = table_primary_key(&table).await?;
        if keys.is_empty() {
            return Err(anyhow!(
                "Setting column '{column}' as the key of table '{}' failed: {error}",
                self.table_name
            ));
        }
        self.expect_primary_key(&keys, column)
    }

    async fn ensure_key_candidate(&self, table: &Table, column: &str) -> Result<()> {
        let schema = table.schema().await?;
        let field = schema.field_with_name(column).map_err(|_| {
            PrimaryKeyRejected(format!(
                "Table '{}' has no column '{column}' to use as its key",
                self.table_name
            ))
        })?;
        if let Some(reason) = primary_key_ineligibility(field) {
            return Err(PrimaryKeyRejected(format!(
                "Column '{column}' cannot be the key of table '{}': {reason}",
                self.table_name
            ))
            .into());
        }
        let duplicates = self.duplicate_key_values(column).await?;
        if duplicates > 0 {
            return Err(PrimaryKeyRejected(format!(
                "Column '{column}' cannot be the key of table '{}': it holds {duplicates} duplicate values; remove the duplicate rows first",
                self.table_name
            ))
            .into());
        }
        Ok(())
    }

    fn expect_primary_key(&self, keys: &[String], column: &str) -> Result<()> {
        if keys == [column] {
            return Ok(());
        }
        Err(PrimaryKeyRejected(format!(
            "Table '{}' is already keyed on '{}'; the key cannot change to '{column}'",
            self.table_name,
            keys.join("', '")
        ))
        .into())
    }

    /// Rows beyond the first for each value of `column`.
    async fn duplicate_key_values(&self, column: &str) -> Result<i64> {
        use datafusion::functions_aggregate::expr_fn::{count, count_distinct};

        let batches = SessionContext::new()
            .read_table(self.to_datafusion().await?)?
            .aggregate(
                vec![],
                vec![
                    count(lit(1)).alias("rows"),
                    count_distinct(ident(column)).alias("values"),
                ],
            )?
            .select(vec![col("rows") - col("values")])?
            .collect()
            .await?;
        batches
            .first()
            .filter(|batch| batch.num_rows() == 1)
            .and_then(|batch| {
                batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<arrow_array::Int64Array>()
            })
            .map(|duplicates| duplicates.value(0))
            .ok_or_else(|| {
                anyhow!(
                    "Counting duplicate values of column '{column}' in table '{}' returned no result",
                    self.table_name
                )
            })
    }

    /// Mark the upsert's ID column as the key of a table without one, so concurrent
    /// upserts of the same new ID retry as updates. Returns whether `id_field` is the key.
    async fn ensure_upsert_key(&mut self, table: &Table, id_field: &str) -> Result<bool> {
        let keys = table_primary_key(table).await?;
        if !keys.is_empty() {
            return Ok(keys == [id_field]);
        }
        // The handle may predate another writer's key or a change to the column.
        table.checkout_latest().await?;
        let schema = table.schema().await?;
        let keys = primary_key_columns(&schema);
        if !keys.is_empty() {
            return Ok(keys == [id_field]);
        }
        let field = schema.field_with_name(id_field).ok();
        let signature = field.map(|field| (field.data_type().clone(), field.is_nullable()));
        if self.unmarkable_keys.get(id_field) == Some(&signature) {
            return Ok(false);
        }
        if field.is_none_or(|field| primary_key_ineligibility(field).is_some()) {
            self.unmarkable_keys.insert(id_field.to_string(), signature);
            return Ok(false);
        }
        let duplicates = self.duplicate_key_values(id_field).await?;
        if duplicates > 0 {
            eprintln!(
                "[LanceDB] Not marking column '{id_field}' as the key of table '{}': it holds {duplicates} duplicate values. Remove them to stop concurrent upserts from adding more.",
                self.table_name
            );
            self.unmarkable_keys.insert(id_field.to_string(), signature);
            return Ok(false);
        }
        let Err(error) = table
            .update_field_metadata(&[primary_key_marker_update(id_field)])
            .await
        else {
            return Ok(true);
        };
        table.checkout_latest().await?;
        let keys = table_primary_key(table).await?;
        if keys.is_empty() {
            eprintln!(
                "[LanceDB] Could not mark column '{id_field}' as the key of table '{}'; concurrent upserts may duplicate new IDs: {error:#}",
                self.table_name
            );
        }
        Ok(keys == [id_field])
    }

    /// The returned provider supports SELECT, INSERT INTO and (via
    /// [`crate::databases::lance_dml`]) UPDATE/DELETE with a WHERE clause.
    /// Read-only surfaces registering it must validate their SQL first
    /// ([`crate::databases::sql_guard::validate_readonly_sql`]).
    pub async fn to_datafusion(&self) -> Result<Arc<dyn TableProvider>> {
        let table = self.require_readable_table().await?;
        let df_table = table.base_table();
        let adapter =
            lancedb::table::datafusion::BaseTableAdapter::try_new(df_table.clone()).await?;
        if self.selector.is_read_only() || self.is_durably_managed() {
            return Ok(Arc::new(ReadOnlyDatabaseProvider {
                inner: zero_column_safe(Arc::new(adapter)),
                mutation_adapter: self.mutation_adapter.clone(),
            }));
        }
        Ok(zero_column_safe_writable(Arc::new(adapter), table))
    }

    pub async fn raw(&self) -> Result<Table> {
        self.ensure_unmanaged("raw table access")?;
        let table = self
            .table
            .clone()
            .ok_or_else(|| anyhow!("Table not initialized"))?;
        Ok(table)
    }

    pub async fn sql(
        &self,
        table_name: &str,
        sql: &str,
    ) -> Result<datafusion::dataframe::DataFrame> {
        crate::databases::sql_guard::validate_lance_dml_sql(sql)?;
        let table = self.to_datafusion().await?;
        let ctx = SessionContext::new();
        crate::geometry::register_geo_functions(&ctx);
        ctx.register_table(table_name, table)?;
        let results = ctx.sql(sql).await?;

        Ok(results)
    }

    pub async fn insert_record_batch(&mut self, batch: RecordBatch) -> Result<()> {
        if self.is_durably_managed() {
            crate::geometry::validate_batch(&batch)?;
            return self.insert(record_batch_to_value(&batch)?).await;
        }
        self.ensure_writable()?;
        crate::geometry::validate_batch(&batch)?;
        let items = vec![batch];

        if self.table.is_none() {
            let builder = self
                .connection
                .create_table(&self.table_name, items.clone())
                .write_options(self.creation_write_options());
            match builder.execute().await {
                Ok(table) => {
                    self.table = Some(table);
                    return Ok(());
                }
                Err(lancedb::Error::TableAlreadyExists { .. }) => {
                    self.table = Some(
                        self.connection
                            .open_table(&self.table_name)
                            .execute()
                            .await?,
                    );
                }
                Err(err) => {
                    eprintln!(
                        "[LanceDB] Error creating table '{}' from record batch: {err:#}",
                        self.table_name
                    );
                    return Err(anyhow!("Error creating table '{}': {err}", self.table_name));
                }
            }
        }

        let table = self.table.clone().unwrap();
        let batch = crate::geometry::normalize_batch(&items[0], &table.schema().await?)?;
        let mut add = table.add(vec![batch]);
        if let Some(opts) = &self.write_options {
            add = add.write_options(opts.clone());
        }
        match add.execute().await {
            Ok(_) => Ok(()),
            Err(err) => Err(anyhow!(err.to_string())),
        }
    }

    async fn write_batch_reader(&self, items: Vec<Value>) -> Result<ValueBatchReader> {
        if let Some(table) = &self.table {
            let schema = table.schema().await?;
            let fields = schema.fields().iter().cloned().collect();
            return value_to_batch_reader_with_fields(items, Some(fields));
        }

        value_to_batch_reader_with_utc_timestamp_inference(items)
    }
}

/// Delegate reads only. Omitting mutation methods makes DataFusion reject SQL writes
/// even when the underlying latest table is writable.
struct ReadOnlyDatabaseProvider {
    inner: Arc<dyn TableProvider>,
    mutation_adapter: Option<Arc<dyn LogicalTableMutationAdapter>>,
}

impl std::fmt::Debug for ReadOnlyDatabaseProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadOnlyDatabaseProvider")
            .field("inner", &self.inner)
            .field("managed", &self.mutation_adapter.is_some())
            .finish()
    }
}

#[async_trait]
impl TableProvider for ReadOnlyDatabaseProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> arrow_schema::SchemaRef {
        self.inner.schema()
    }

    fn table_type(&self) -> datafusion::logical_expr::TableType {
        self.inner.table_type()
    }

    async fn scan(
        &self,
        state: &dyn datafusion::catalog::Session,
        projection: Option<&Vec<usize>>,
        filters: &[datafusion::logical_expr::Expr],
        limit: Option<usize>,
    ) -> datafusion::common::Result<Arc<dyn datafusion::physical_plan::ExecutionPlan>> {
        if let Some(adapter) = &self.mutation_adapter {
            // Mounted SQL providers can outlive a grant. Check the adapter on
            // every new scan even when its data generation has not changed.
            let table = adapter
                .read_table()
                .await
                .map_err(|error| datafusion::common::DataFusionError::External(error.into()))?;
            if table.is_none() {
                return Err(datafusion::common::DataFusionError::Execution(
                    "The managed table is no longer available".into(),
                ));
            }
        }
        self.inner.scan(state, projection, filters, limit).await
    }

    fn supports_filters_pushdown(
        &self,
        filters: &[&datafusion::logical_expr::Expr],
    ) -> datafusion::common::Result<Vec<datafusion::logical_expr::TableProviderFilterPushDown>>
    {
        self.inner.supports_filters_pushdown(filters)
    }
}

async fn table_primary_key(table: &Table) -> Result<Vec<String>> {
    let schema = table.schema().await?;
    Ok(primary_key_columns(&schema))
}

/// The key a column path names, resolved the way Lance resolves paths, so a
/// backtick-quoted spelling of the key column is still recognised.
fn targeted_key<'a>(keys: &'a [String], path: &str) -> Option<&'a String> {
    let segments = lance::datatypes::parse_field_path(path).ok()?;
    let [name] = segments.as_slice() else {
        return None;
    };
    keys.iter().find(|key| *key == name)
}

/// Whether `error` means concurrent commits kept winning, as opposed to bad input.
pub fn is_write_contention(error: &flow_like_types::Error) -> bool {
    error.chain().any(|cause| {
        let lance = match cause.downcast_ref::<lancedb::Error>() {
            Some(lancedb::Error::Lance { source }) => source,
            _ => match cause.downcast_ref::<lance::Error>() {
                Some(source) => source,
                None => return false,
            },
        };
        matches!(
            lance,
            lance::Error::TooMuchWriteContention { .. }
                | lance::Error::RetryableCommitConflict { .. }
                | lance::Error::CommitConflict { .. }
        )
    })
}

fn update_sql_literal(table: &str, field: &arrow_schema::Field, value: &Value) -> Result<String> {
    let column = field.name();
    if crate::geometry::is_geometry_field(field) {
        return geometry_sql_literal(column, value);
    }
    let binary = matches!(
        field.data_type(),
        DataType::Binary
            | DataType::LargeBinary
            | DataType::BinaryView
            | DataType::FixedSizeBinary(_)
    );
    ensure_update_value_casts(table, field, value)?;
    Ok(match value {
        Value::Array(bytes) if binary => binary_sql_literal(column, bytes)?,
        Value::Object(_) if binary => return Err(binary_value_rejected(column, "an object").into()),
        Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "NULL".to_string(),
        _ => format!("'{}'", value.to_string().replace('\'', "''")),
    })
}

/// Lance casts each SET literal to the column type while it writes; the same cast
/// up front rejects a value the column cannot hold before anything is written.
fn ensure_update_value_casts(
    table: &str,
    field: &arrow_schema::Field,
    value: &Value,
) -> Result<()> {
    use arrow_array::{ArrayRef, BooleanArray, Float64Array, Int64Array, StringArray};
    let target = field.data_type();
    if !(target.is_primitive() || target == &DataType::Boolean) {
        return Ok(());
    }
    let literal: ArrayRef = match value {
        Value::Null => return Ok(()),
        Value::Bool(flag) => Arc::new(BooleanArray::from(vec![*flag])),
        Value::Number(number) => match (number.as_i64(), number.as_f64()) {
            (Some(integer), _) => Arc::new(Int64Array::from(vec![integer])),
            (None, Some(float)) => Arc::new(Float64Array::from(vec![float])),
            (None, None) => return Ok(()),
        },
        Value::String(text) => Arc::new(StringArray::from(vec![text.as_str()])),
        structured => Arc::new(StringArray::from(vec![structured.to_string()])),
    };
    let options = arrow::compute::CastOptions {
        safe: false,
        ..Default::default()
    };
    arrow::compute::cast_with_options(&literal, target, &options)
        .map(|_| ())
        .map_err(|error| {
            TableInputRejected(format!(
                "Column '{}' of table '{table}' is {target}; cannot store {value}: {error}",
                field.name()
            ))
            .into()
        })
}

/// Lance casts a quoted string to Binary as its UTF-8 text, so bytes read back as a
/// JSON array are written as a hex literal.
fn binary_sql_literal(column: &str, bytes: &[Value]) -> Result<String> {
    bytes
        .iter()
        .map(|byte| {
            byte.as_u64()
                .and_then(|byte| u8::try_from(byte).ok())
        })
        .collect::<Option<Vec<u8>>>()
        .map(|bytes| hex_sql_literal(&bytes))
        .ok_or_else(|| {
            binary_value_rejected(column, "an array that is not all integers 0-255").into()
        })
}

/// A plain Binary column would otherwise store structured values as their JSON text.
fn binary_value_rejected(column: &str, received: &str) -> TableInputRejected {
    TableInputRejected(format!(
        "Binary column '{column}' takes an array of integers 0-255, but received {received}. To store GeoJSON geometry, add a geometry column with add_column {{\"name\": \"<new column>\", \"type\": \"geometry\"}} and write the geometry there"
    ))
}

/// An update replaces values only, so the column keeps its geometry metadata.
fn geometry_sql_literal(column: &str, value: &Value) -> Result<String> {
    let bytes = crate::geometry::geometry_input_wkb(value).map_err(|error| {
        TableInputRejected(format!(
            "Geometry column '{column}' takes a GeoJSON geometry object, a GeoJSON Feature, GeoJSON or WKT text, or null: {error}"
        ))
    })?;
    Ok(bytes.map_or_else(|| "NULL".to_string(), |bytes| hex_sql_literal(&bytes)))
}

fn hex_sql_literal(bytes: &[u8]) -> String {
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    format!("X'{hex}'")
}

/// Treat the historical timezone-less millisecond timestamp as compatible
/// with the UTC-aware schema now emitted for new timestamp columns. The stored
/// schema remains authoritative; writes to that legacy shape are normalized at
/// the serialization boundary.
/// Upserts key a table after it is created, so an existing key the request does not
/// declare is compatible; a declared key must match the existing one.
fn schemas_compatible_for_creation(existing: &Schema, requested: &Schema) -> bool {
    if existing == requested {
        return true;
    }
    let requested_keys = primary_key_columns(requested);
    if !requested_keys.is_empty() && requested_keys != primary_key_columns(existing) {
        return false;
    }

    existing.metadata() == requested.metadata()
        && existing.fields().len() == requested.fields().len()
        && existing
            .fields()
            .iter()
            .zip(requested.fields())
            .all(|(existing, requested)| {
                existing.name() == requested.name()
                    && existing.is_nullable() == requested.is_nullable()
                    && without_primary_key_marker(existing.metadata())
                        == without_primary_key_marker(requested.metadata())
                    && (existing.data_type() == requested.data_type()
                        || matches!(
                            (existing.data_type(), requested.data_type()),
                            (
                                DataType::Timestamp(existing_unit, None),
                                DataType::Timestamp(requested_unit, Some(timezone)),
                            ) if existing_unit == requested_unit && timezone.eq_ignore_ascii_case("UTC")
                        ))
            })
}

pub fn record_batches_to_vec(batches: Option<Vec<RecordBatch>>) -> Result<Vec<Value>> {
    batches
        .as_ref()
        .ok_or(anyhow!("Error converting record batches to vec"))?;

    let batches = batches.unwrap();
    let mut items = vec![];

    for (index, batch) in batches.iter().enumerate() {
        let mut values = record_batch_to_value(batch)
            .map_err(|error| anyhow!("Unable to decode result batch {index}: {error}"))?;
        items.append(&mut values);
    }

    Ok(items)
}

fn is_vector_data_type(data_type: &DataType) -> bool {
    match data_type {
        DataType::FixedSizeList(field, _) | DataType::List(field) | DataType::LargeList(field) => {
            match field.data_type() {
                DataType::Float16 | DataType::Float32 | DataType::Float64 => true,
                nested => is_vector_data_type(nested),
            }
        }
        _ => false,
    }
}

fn cosine_vector_index() -> Index {
    Index::IvfPq(IvfPqIndexBuilder::default().distance_type(lancedb::DistanceType::Cosine))
}

fn normalized_index_selection(selection: Option<&str>) -> String {
    // Saved nodes use uppercase labels, while HTTP and desktop requests use
    // enum names. Both spellings must select the same builder and metric.
    selection
        .unwrap_or("AUTO")
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '_' && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase()
}

fn native_scalar_index(selection: Option<&str>) -> Option<BuiltinIndexType> {
    match normalized_index_selection(selection).as_str() {
        "NGRAM" => Some(BuiltinIndexType::NGram),
        "ZONEMAP" => Some(BuiltinIndexType::ZoneMap),
        "BLOOMFILTER" => Some(BuiltinIndexType::BloomFilter),
        "RTREE" => Some(BuiltinIndexType::RTree),
        _ => None,
    }
}

fn validate_rtree_column(field: &arrow_schema::Field) -> Result<()> {
    if !field
        .metadata()
        .get("ARROW:extension:name")
        .is_some_and(|name| name.starts_with("geoarrow."))
    {
        return Err(anyhow!(
            "R-Tree requires a column with GeoArrow extension metadata"
        ));
    }
    let empty = arrow_array::new_empty_array(field.data_type());
    lance_index::scalar::rtree::extract_bounding_boxes(empty.as_ref(), field).map_err(|error| {
        anyhow!(
            "Unsupported GeoArrow layout for R-Tree. Use separated Float64 coordinates or GeoArrow WKB/WKT. Lance does not preserve interleaved coordinate field names: {error}"
        )
    })?;
    Ok(())
}

fn index_for_column(selection: Option<&str>, data_type: &DataType) -> Index {
    let selection = normalized_index_selection(selection);
    let cosine = lancedb::DistanceType::Cosine;
    match selection.as_str() {
        "FULLTEXT" | "FTS" | "INVERTED" => Index::FTS(FtsIndexBuilder::default()),
        "BTREE" => Index::BTree(BTreeIndexBuilder::default()),
        "BITMAP" => Index::Bitmap(BitmapIndexBuilder::default()),
        "LABELLIST" => Index::LabelList(LabelListIndexBuilder::default()),
        "FM" => Index::Fm(FmIndexBuilder::default()),
        "VECTOR" | "IVFPQ" => cosine_vector_index(),
        "IVFFLAT" => Index::IvfFlat(IvfFlatIndexBuilder::default().distance_type(cosine)),
        "IVFSQ" => Index::IvfSq(IvfSqIndexBuilder::default().distance_type(cosine)),
        "IVFRQ" => Index::IvfRq(IvfRqIndexBuilder::default().distance_type(cosine)),
        "IVFHNSWFLAT" => {
            Index::IvfHnswFlat(IvfHnswFlatIndexBuilder::default().distance_type(cosine))
        }
        "IVFHNSWPQ" => Index::IvfHnswPq(IvfHnswPqIndexBuilder::default().distance_type(cosine)),
        "IVFHNSWSQ" => Index::IvfHnswSq(IvfHnswSqIndexBuilder::default().distance_type(cosine)),
        "AUTO" if lancedb::utils::supported_vector_data_type(data_type) => cosine_vector_index(),
        // Preserve the historical fallback for unrecognized saved selections.
        _ => Index::Auto,
    }
}

fn optimize_actions(keep_versions: bool) -> Vec<lancedb::table::OptimizeAction> {
    let mut actions = vec![
        lancedb::table::OptimizeAction::Compact {
            options: CompactionOptions::default(),
            remap_options: None,
        },
        lancedb::table::OptimizeAction::Index(OptimizeOptions::new()),
    ];

    if !keep_versions {
        actions.push(lancedb::table::OptimizeAction::Prune {
            older_than: Some(Duration::try_days(7).expect("seven days is a valid duration")),
            delete_unverified: Some(false),
            error_if_tagged_old_versions: Some(true),
        });
    }

    actions
}

/// Compact, rebuild indices, and prune unprotected history before an archive export.
fn prune_history_actions() -> Vec<lancedb::table::OptimizeAction> {
    vec![
        lancedb::table::OptimizeAction::Compact {
            options: CompactionOptions::default(),
            remap_options: None,
        },
        lancedb::table::OptimizeAction::Index(OptimizeOptions::new()),
        lancedb::table::OptimizeAction::Prune {
            older_than: Some(Duration::zero()),
            delete_unverified: Some(false),
            error_if_tagged_old_versions: Some(false),
        },
    ]
}

#[derive(Debug, PartialEq)]
struct CompactionScalarIndex {
    name: String,
    column: String,
    params: ScalarIndexParams,
}

async fn scalar_indices_for_compaction(table: &Table) -> Result<Vec<CompactionScalarIndex>> {
    let Some(wrapper) = table.dataset() else {
        return Ok(Vec::new());
    };
    wrapper.ensure_mutable()?;
    let dataset = wrapper.get().await?;
    let mut indices = std::collections::BTreeMap::new();
    for metadata in dataset.load_indices().await?.iter() {
        if indices.contains_key(&metadata.name) || metadata.fields.len() != 1 {
            continue;
        }
        let column = dataset.schema().field_path(metadata.fields[0])?;
        let needs_preservation = if let Some(details) = metadata.index_details.clone() {
            let details = lance::index::scalar::IndexDetails(details);
            if details.is_vector() {
                false
            } else {
                details.get_plugin().is_ok_and(|plugin| {
                    matches!(
                        normalized_index_selection(Some(plugin.name())).as_str(),
                        "FM" | "ZONEMAP" | "BLOOMFILTER" | "RTREE"
                    )
                })
            }
        } else {
            // Imported manifests can omit details. Identify their physical index
            // without the statistics API, which can migrate legacy manifests.
            let index = dataset
                .open_generic_index(&column, &metadata.uuid, &NoOpMetricsCollector)
                .await?;
            matches!(
                index.index_type(),
                lance_index::IndexType::Fm
                    | lance_index::IndexType::ZoneMap
                    | lance_index::IndexType::BloomFilter
                    | lance_index::IndexType::RTree
            )
        };
        if !needs_preservation {
            continue;
        }
        let index = dataset
            .open_scalar_index(&column, &metadata.uuid, &NoOpMetricsCollector)
            .await?;
        if !index.can_remap() {
            // Lance drops these indexes when compaction changes row addresses.
            // Read their saved configuration first, including imported tuning.
            indices.insert(
                metadata.name.clone(),
                CompactionScalarIndex {
                    name: metadata.name.clone(),
                    column,
                    params: index.derive_index_params()?,
                },
            );
        }
    }
    Ok(indices.into_values().collect())
}

async fn restore_compacted_scalar_indices(
    table: &Table,
    indices: &[CompactionScalarIndex],
) -> Result<()> {
    if indices.is_empty() {
        return Ok(());
    }
    let wrapper = table
        .dataset()
        .ok_or_else(|| anyhow!("Native table required to restore compacted indexes"))?;
    wrapper.ensure_mutable()?;
    let mut dataset = wrapper.get().await?.as_ref().clone();
    for index in indices {
        if dataset
            .load_indices()
            .await?
            .iter()
            .any(|metadata| metadata.name == index.name)
        {
            continue;
        }
        dataset
            .create_index_builder(
                &[&index.column],
                lance_index::IndexType::Scalar,
                &index.params,
            )
            .name(index.name.clone())
            .replace(false)
            .await?;
        // Publish each successful commit even if a later rebuild fails.
        wrapper.update(dataset.clone());
    }
    Ok(())
}

fn split_hybrid_fields(
    schema: &Schema,
    fields: Option<Vec<String>>,
) -> (Option<String>, Option<Vec<String>>) {
    let Some(fields) = fields else {
        return (None, None);
    };

    let mut vector_column = None;
    let mut fts_fields = Vec::new();

    for field in fields {
        let is_vector_field = schema
            .field_with_name(&field)
            .map(|schema_field| is_vector_data_type(schema_field.data_type()))
            .unwrap_or(false);

        if vector_column.is_none() && is_vector_field {
            vector_column = Some(field);
        } else {
            fts_fields.push(field);
        }
    }

    let fts_fields = if fts_fields.is_empty() {
        None
    } else {
        Some(fts_fields)
    };

    (vector_column, fts_fields)
}

#[async_trait]
impl VectorStore for LanceDBVectorStore {
    fn is_durably_managed(&self) -> bool {
        self.mutation_adapter.is_some()
    }

    fn ensure_writable(&self) -> Result<()> {
        LanceDBVectorStore::ensure_writable(self)
    }

    async fn vector_search(
        &self,
        vector: Vec<f64>,
        filter: Option<&str>,
        select: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>> {
        let table = self.require_readable_table().await?;

        let mut query = table
            .query()
            .nearest_to(vector)?
            .distance_type(lancedb::DistanceType::Cosine)
            .limit(limit)
            .offset(offset);

        if let Some(filter) = filter {
            query = query.only_if(orient_spatial_relations(filter)?);
        }

        if let Some(select) = select {
            query = query.select(lancedb::query::Select::Columns(select));
        }

        let result = query.execute().await?;
        let result = result.try_collect::<Vec<_>>().await?;
        let result = record_batches_to_vec(Some(result))?;
        Ok(result)
    }

    async fn fts_search(
        &self,
        text: &str,
        filter: Option<&str>,
        select: Option<Vec<String>>,
        fields: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>> {
        let table = self.require_readable_table().await?;

        let mut fts_query = FullTextSearchQuery::new(text.to_string());
        if let Some(fields) = fields {
            match fields.len() {
                1 => fts_query = fts_query.with_column(fields[0].clone())?,
                n if n > 1 => fts_query = fts_query.with_columns(&fields)?,
                _ => {}
            }
        }

        let mut query = table
            .query()
            .full_text_search(fts_query)
            .limit(limit)
            .offset(offset);

        if let Some(filter) = filter {
            query = query.only_if(orient_spatial_relations(filter)?);
        }

        if let Some(select) = select {
            query = query.select(lancedb::query::Select::Columns(select));
        }

        let result = query.execute().await?;
        let result = result.try_collect::<Vec<_>>().await?;
        let result = record_batches_to_vec(Some(result))?;
        Ok(result)
    }

    async fn hybrid_search(
        &self,
        vector: Vec<f64>,
        text: &str,
        filter: Option<&str>,
        select: Option<Vec<String>>,
        fields: Option<Vec<String>>,
        limit: usize,
        offset: usize,
        rerank: bool,
    ) -> Result<Vec<Value>> {
        let table = self.require_readable_table().await?;
        let schema = table.schema().await?;
        let (vector_column, fields) = split_hybrid_fields(&schema, fields);

        let mut fts_query = FullTextSearchQuery::new(text.to_string());
        if let Some(ref fields) = fields {
            match fields.len() {
                1 => fts_query = fts_query.with_column(fields[0].clone())?,
                n if n > 1 => fts_query = fts_query.with_columns(fields)?,
                _ => {}
            }
        }

        let mut query = table
            .query()
            .nearest_to(vector)?
            .distance_type(lancedb::DistanceType::Cosine)
            .full_text_search(fts_query)
            .limit(limit)
            .offset(offset);

        if let Some(vector_column) = vector_column {
            query = query.column(&vector_column);
        }

        if rerank {
            let reranker = Arc::new(lancedb::rerankers::rrf::RRFReranker::new(60.0));
            query = query.rerank(reranker);
        }

        if let Some(filter) = filter {
            query = query.only_if(orient_spatial_relations(filter)?);
        }

        if let Some(select) = select {
            query = query.select(lancedb::query::Select::Columns(select));
        }

        let result = query
            .execute_hybrid(QueryExecutionOptions::default())
            .await?;
        let result = result.try_collect::<Vec<_>>().await?;
        let result = record_batches_to_vec(Some(result))?;
        Ok(result)
    }

    async fn filter(
        &self,
        filter: &str,
        select: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>> {
        let table = self.require_readable_table().await?;

        let mut query = table
            .query()
            .limit(limit)
            .only_if(orient_spatial_relations(filter)?)
            .offset(offset);

        if let Some(select) = select {
            query = query.select(lancedb::query::Select::Columns(select));
        }

        let result = query.execute().await?;
        let result = result.try_collect::<Vec<_>>().await?;
        let result = record_batches_to_vec(Some(result))?;
        Ok(result)
    }

    async fn upsert(&mut self, items: Vec<Value>, id_field: String) -> Result<()> {
        if self.is_durably_managed() {
            return self
                .apply_logical_mutation(LogicalTableMutation::Upsert { items, id_field })
                .await;
        }
        self.ensure_writable()?;
        if self.table.is_none() {
            let reader = self.write_batch_reader(items.clone()).await?;
            let builder = self
                .connection
                .create_table(&self.table_name, reader)
                .write_options(self.creation_write_options());
            match builder.execute().await {
                Ok(table) => {
                    self.table = Some(table.clone());
                    if let Err(error) = self.ensure_upsert_key(&table, &id_field).await {
                        eprintln!(
                            "[LanceDB] Created table '{}' but could not mark '{id_field}' as its key: {error:#}",
                            self.table_name
                        );
                    }
                    return Ok(());
                }
                Err(lancedb::Error::TableAlreadyExists { .. }) => {
                    self.table = Some(
                        self.connection
                            .open_table(&self.table_name)
                            .execute()
                            .await?,
                    );
                }
                Err(err) => {
                    eprintln!(
                        "[LanceDB] Error creating table '{}' for upsert: {err:#}",
                        self.table_name
                    );
                    return Err(anyhow!("Error creating table '{}': {err}", self.table_name));
                }
            }
        }

        let table = self.table.clone().unwrap();
        // Convert first: a batch that fails must not leave an irreversible key behind.
        let items = self.write_batch_reader(items).await?;
        // Lance only detects a concurrent insert of the same key on the unindexed path.
        let keyed = self.ensure_upsert_key(&table, &id_field).await?;
        table
            .merge_insert(&[&id_field])
            .when_matched_update_all(None)
            .when_not_matched_insert_all()
            .use_index(!keyed)
            .to_owned()
            .execute(items)
            .await?;
        Ok(())
    }

    async fn insert(&mut self, items: Vec<Value>) -> Result<()> {
        if self.is_durably_managed() {
            return self
                .apply_logical_mutation(LogicalTableMutation::Insert { items })
                .await;
        }
        self.ensure_writable()?;
        if self.table.is_none() {
            let reader = self.write_batch_reader(items.clone()).await?;
            let builder = self
                .connection
                .create_table(&self.table_name, reader)
                .write_options(self.creation_write_options());
            match builder.execute().await {
                Ok(table) => {
                    self.table = Some(table);
                    return Ok(());
                }
                Err(lancedb::Error::TableAlreadyExists { .. }) => {
                    self.table = Some(
                        self.connection
                            .open_table(&self.table_name)
                            .execute()
                            .await?,
                    );
                }
                Err(err) => {
                    eprintln!(
                        "[LanceDB] Error creating table '{}' for insert: {err:#}",
                        self.table_name
                    );
                    return Err(anyhow!("Error creating table '{}': {err}", self.table_name));
                }
            }
        }

        let items = self.write_batch_reader(items).await?;
        let table = self.table.clone().unwrap();
        let mut add = table.add(items);
        if let Some(opts) = &self.write_options {
            add = add.write_options(opts.clone());
        }
        match add.execute().await {
            Ok(_) => return Ok(()),
            Err(err) => {
                return Err(anyhow!(err.to_string()));
            }
        }
    }

    async fn delete(&self, filter: &str) -> Result<()> {
        let filter = orient_spatial_relations(filter)?;
        let filter = filter.as_str();
        if self.is_durably_managed() {
            return self
                .apply_logical_mutation(LogicalTableMutation::Delete {
                    filter: filter.to_string(),
                })
                .await;
        }
        self.ensure_writable()?;
        let table = self.table.clone().ok_or(anyhow!("Table not initialized"))?;
        table.delete(filter).await?;
        return Ok(());
    }

    async fn optimize(&self, keep_versions: bool) -> Result<()> {
        self.run_optimize_actions(optimize_actions(keep_versions))
            .await
    }

    async fn list(
        &self,
        select: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Value>> {
        let table = self.require_readable_table().await?;

        let mut query = table.query().limit(limit).offset(offset);

        if let Some(select) = select {
            query = query.select(lancedb::query::Select::Columns(select));
        }

        let result = query.execute().await?;
        let result = result.try_collect::<Vec<_>>().await?;
        record_batches_to_vec(Some(result))
    }

    async fn index(&self, column: &str, index_type: Option<&str>) -> Result<()> {
        self.ensure_unmanaged("index changes")?;
        self.ensure_writable()?;
        let table = self.table.clone().ok_or(anyhow!("Table not initialized"))?;
        if let Some(kind) = native_scalar_index(index_type) {
            let wrapper = table.dataset().ok_or_else(|| {
                anyhow!(
                    "{} indexes require a native Lance table",
                    kind.as_str().to_uppercase()
                )
            })?;
            // Use the table's dataset and consistency wrapper so credentials,
            // object-store overrides and pinned-version protection are retained.
            wrapper.ensure_mutable()?;
            let mut dataset = wrapper.get().await?.as_ref().clone();
            if kind == BuiltinIndexType::RTree {
                let field = dataset
                    .schema()
                    .field_case_insensitive(column)
                    .ok_or_else(|| anyhow!("Column '{column}' does not exist"))?;
                validate_rtree_column(&arrow_schema::Field::from(field))?;
            }
            let params = ScalarIndexParams::for_builtin(kind);
            dataset
                .create_index_builder(&[column], lance_index::IndexType::Scalar, &params)
                .replace(true)
                .await?;
            wrapper.update(dataset);
            return Ok(());
        }
        let index_type = if normalized_index_selection(index_type) == "AUTO" {
            let schema = table.schema().await?;
            let field = schema.field_with_name(column)?;
            index_for_column(index_type, field.data_type())
        } else {
            // Explicit builders resolve nested and quoted paths in Lance.
            // Only AUTO needs the field type to choose a vector algorithm.
            index_for_column(index_type, &DataType::Null)
        };

        table.create_index(&[column], index_type).execute().await?;
        Ok(())
    }

    async fn purge(&self) -> Result<()> {
        if self.is_durably_managed() {
            return self.delete("true").await;
        }
        self.ensure_writable()?;
        let table = self.table.clone().ok_or(anyhow!("Table not initialized"))?;
        table.delete("1=1").await?;
        Ok(())
    }

    async fn count(&self, filter: Option<String>) -> Result<usize> {
        let table = self.require_readable_table().await?;
        let filter = filter
            .map(|filter| orient_spatial_relations(&filter))
            .transpose()?;
        Ok(table.count_rows(filter).await?)
    }

    async fn schema(&self) -> Result<arrow_schema::Schema> {
        let table = self.require_readable_table().await?;
        let schema = table.schema().await?;
        let schema = schema.as_ref().clone();
        Ok(schema)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use super::*;
    use crate::databases::vector::buffered::{
        BufferedVectorStore, BufferedWriteError, BufferedWriteKind, BufferedWriteOrigin,
    };
    use arrow_array::{FixedSizeListArray, Float32Array, Int64Array};
    use arrow_schema::{Field, TimeUnit};
    use flow_like_types::{
        create_id,
        json::{from_value, json, to_value},
        tokio,
    };
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
    struct TestStruct {
        id: i32,
        name: String,
        vector: Vec<f32>,
    }

    #[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
    struct TestStruct2 {
        id: i32,
        name: String,
    }

    #[derive(Serialize, Deserialize, PartialEq, Clone, Debug)]
    struct NullableFieldRow {
        id: i32,
        name: String,
        #[serde(default)]
        tag: Option<String>,
    }

    #[test]
    fn regression_index_names_preserve_defaults_and_transport_spellings() {
        let vector =
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 16);
        for selection in [
            None,
            Some("AUTO"),
            Some("Auto"),
            Some("VECTOR"),
            Some("IvfPq"),
        ] {
            assert!(matches!(
                index_for_column(selection, &vector),
                Index::IvfPq(_)
            ));
        }
        assert!(matches!(
            index_for_column(None, &DataType::Int64),
            Index::Auto
        ));
        assert!(matches!(
            index_for_column(Some("old_unknown"), &vector),
            Index::Auto
        ));
        for name in ["FULL TEXT", "FullText", "full_text", "FTS"] {
            assert!(matches!(
                index_for_column(Some(name), &DataType::Utf8),
                Index::FTS(_)
            ));
        }
        for name in ["LABEL LIST", "LabelList", "label_list"] {
            assert!(matches!(
                index_for_column(Some(name), &DataType::Utf8),
                Index::LabelList(_)
            ));
        }
        for (name, kind) in [
            ("NGram", BuiltinIndexType::NGram),
            ("ZONE MAP", BuiltinIndexType::ZoneMap),
            ("bloom_filter", BuiltinIndexType::BloomFilter),
            ("RTREE", BuiltinIndexType::RTree),
        ] {
            assert_eq!(native_scalar_index(Some(name)), Some(kind));
        }
    }

    #[test]
    fn regression_lance_optimize_plan_retains_versions_unless_cleanup_is_explicit() {
        let retain_actions = optimize_actions(true);
        assert_eq!(retain_actions.len(), 2);
        assert!(matches!(
            &retain_actions[0],
            lancedb::table::OptimizeAction::Compact { .. }
        ));
        assert!(matches!(
            &retain_actions[1],
            lancedb::table::OptimizeAction::Index(_)
        ));

        let cleanup_actions = optimize_actions(false);
        assert_eq!(cleanup_actions.len(), 3);
        assert!(matches!(
            &cleanup_actions[0],
            lancedb::table::OptimizeAction::Compact { .. }
        ));
        assert!(matches!(
            &cleanup_actions[1],
            lancedb::table::OptimizeAction::Index(_)
        ));
        match &cleanup_actions[2] {
            lancedb::table::OptimizeAction::Prune {
                older_than,
                delete_unverified,
                error_if_tagged_old_versions,
            } => {
                assert_eq!(
                    older_than.as_ref(),
                    Some(&Duration::try_days(7).expect("seven days is a valid duration"))
                );
                assert_eq!(*delete_unverified, Some(false));
                assert_eq!(*error_if_tagged_old_versions, Some(true));
            }
            _ => panic!("cleanup must prune only after compaction and index maintenance"),
        }
    }

    /// Deterministic values that are independent across vector components.
    /// A linear sequence places every row on one curve, where HNSW builds a
    /// chain whose parallel construction can strand whole segments.
    fn scattered_unit_value(index: usize) -> f32 {
        let mut state = (index as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
        state = (state ^ (state >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        state = (state ^ (state >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        state ^= state >> 31;
        (state >> 40) as f32 / (1_u64 << 24) as f32
    }

    #[tokio::test]
    async fn regression_lance_vector_and_auto_indices_match_cosine_searches() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "cosine_indices".to_string())
                .await?;

        let dimension = 16;
        let item = Arc::new(Field::new("item", DataType::Float32, true));
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(item.clone(), dimension),
                false,
            ),
        ]));
        let ids = Arc::new(Int64Array::from_iter_values(0..512));
        let values = (0..512 * dimension as usize)
            .map(scattered_unit_value)
            .collect::<Vec<_>>();
        let query_vector = values
            .iter()
            .take(dimension as usize)
            .map(|v| *v as f64)
            .collect::<Vec<_>>();
        let vectors = Arc::new(FixedSizeListArray::try_new(
            item,
            dimension,
            Arc::new(Float32Array::from(values)),
            None,
        )?);
        db.insert_record_batch(RecordBatch::try_new(schema, vec![ids, vectors])?)
            .await?;

        for (selection, expected) in [
            ("VECTOR", lancedb::index::IndexType::IvfPq),
            ("AUTO", lancedb::index::IndexType::IvfPq),
            ("IvfPq", lancedb::index::IndexType::IvfPq),
            ("IVF_FLAT", lancedb::index::IndexType::IvfFlat),
            ("IVF_SQ", lancedb::index::IndexType::IvfSq),
            ("IVF_RQ", lancedb::index::IndexType::IvfRq),
            ("IVF_HNSW_FLAT", lancedb::index::IndexType::IvfHnswFlat),
            ("IVF_HNSW_PQ", lancedb::index::IndexType::IvfHnswPq),
            ("IVF_HNSW_SQ", lancedb::index::IndexType::IvfHnswSq),
        ] {
            db.index("vector", Some(selection)).await?;
            let table = db.raw().await?;
            let config = table
                .list_indices()
                .await?
                .into_iter()
                .find(|index| index.columns.len() == 1 && index.columns[0] == "vector")
                .expect("vector index should exist");
            assert_eq!(config.index_type, expected, "{selection}");
            let stats = table
                .index_stats(&config.name)
                .await?
                .expect("vector index statistics should exist");
            assert_eq!(stats.index_type, expected, "{selection}");
            assert_eq!(stats.distance_type, Some(lancedb::DistanceType::Cosine));
            let rows = db
                .vector_search(
                    query_vector.clone(),
                    Some("id < 128"),
                    Some(vec!["id".into()]),
                    5,
                    0,
                )
                .await?;
            assert_eq!(rows.len(), 5, "{selection} filtered vector search");
            assert!(
                rows.iter()
                    .all(|row| row["id"].as_i64().is_some_and(|id| id < 128))
            );
        }

        db.index("id", Some("AUTO")).await?;
        let table = db.raw().await?;
        let scalar_config = table
            .list_indices()
            .await?
            .into_iter()
            .find(|index| index.columns.len() == 1 && index.columns[0] == "id")
            .expect("scalar index should exist");
        assert_eq!(scalar_config.index_type, lancedb::index::IndexType::BTree);
        let scalar_stats = table
            .index_stats(&scalar_config.name)
            .await?
            .expect("scalar index statistics should exist");
        assert_eq!(scalar_stats.distance_type, None);

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn regression_native_scalar_indices_survive_queries_reopen_and_maintenance() -> Result<()>
    {
        use arrow_array::{StringArray, TimestampMillisecondArray};

        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "scalar_indices".into()).await?;
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("text", DataType::Utf8, false),
            Field::new(
                "at",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                false,
            ),
        ]));
        db.insert_record_batch(RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from_iter_values(0..512)),
                Arc::new(StringArray::from_iter_values(
                    (0..512).map(|i| format!("event-{i:04}-record")),
                )),
                Arc::new(TimestampMillisecondArray::from_iter_values(
                    (0..512).map(|i| 1_700_000_000_000 + i * 60_000),
                )),
            ],
        )?)
        .await?;

        for (column, selection, filter, expected) in [
            ("text", "NGRAM", "contains(text, '0012')", 1),
            ("text", "FM", "contains(text, '0012')", 1),
            ("id", "BLOOMFILTER", "id IN (7, 33, 499)", 3),
            (
                "at",
                "ZONEMAP",
                "at >= TIMESTAMP '2023-11-14 22:23:20' AND at < TIMESTAMP '2023-11-14 22:33:20'",
                10,
            ),
        ] {
            let before = db.count(Some(filter.into())).await?;
            assert_eq!(before, expected, "unindexed {selection}");
            db.index(column, Some(selection)).await?;
            assert_eq!(
                db.count(Some(filter.into())).await?,
                before,
                "indexed {selection}"
            );
            let ctx = SessionContext::new();
            crate::geometry::register_geo_functions(&ctx);
            ctx.register_table("scalar_indices", db.to_datafusion().await?)?;
            assert_eq!(
                ctx.sql(&format!("SELECT id FROM scalar_indices WHERE {filter}"))
                    .await?
                    .count()
                    .await?,
                before,
                "DataFusion must preserve {selection} filter results"
            );
            let index = db
                .list_indices()
                .await?
                .into_iter()
                .find(|i| i.columns == [column])
                .expect("native indexes must be visible to the node and interface");
            assert_eq!(
                normalized_index_selection(Some(&index.index_type)),
                selection
            );
            db.optimize(true).await?;
            let reopened =
                LanceDBVectorStore::new(PathBuf::from(&test_path), "scalar_indices".into()).await?;
            assert_eq!(reopened.count(Some(filter.into())).await?, before);
            assert!(
                reopened
                    .list_indices()
                    .await?
                    .iter()
                    .any(|i| i.name == index.name)
            );
            reopened.drop_index(&index.name).await?;
            assert!(
                !reopened
                    .list_indices()
                    .await?
                    .iter()
                    .any(|i| i.name == index.name)
            );
            db =
                LanceDBVectorStore::new(PathBuf::from(&test_path), "scalar_indices".into()).await?;
        }

        let table = db.raw().await?;
        let version = table.version().await?;
        table.checkout(version).await?;
        assert!(
            db.index("at", Some("ZONEMAP")).await.is_err(),
            "time travel must stay read-only"
        );
        table.checkout_latest().await?;
        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn geometry_wkb_roundtrip_index_and_spatial_fallback() -> Result<()> {
        use crate::geometry::geometry_field;
        use datafusion::common::ScalarValue;
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "places".into()).await?;
        let schema = Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            geometry_field("geom", true),
        ]);
        db.create_empty_table(schema, false).await?;
        let points = [(2., 2.), (9., 9.), (0.5, 0.5), (3.5, 3.5)];
        let mut rows: Vec<Value> = points
            .iter()
            .enumerate()
            .map(|(id, (x, y))| {
                json!({
                    "id": id, "geom": {"type": "Point", "coordinates": [x, y]}
                })
            })
            .collect();
        rows.push(json!({"id": 4, "geom": null}));
        db.insert(rows.clone()).await?;
        let polygon = "POLYGON ((0 0,4 0,4 4,0 4,0 0),(1 1,1 3,3 3,3 1,1 1))";
        let predicate = format!("ST_Intersects(geom, ST_GeomFromText('{polygon}'))");
        assert_eq!(db.count(Some(predicate.clone())).await?, 2);
        db.index("geom", Some("RTREE")).await?;
        assert_eq!(db.count(Some(predicate.clone())).await?, 2);
        let plan = db
            .raw()
            .await?
            .query()
            .only_if(&predicate)
            .explain_plan(false)
            .await?;
        assert!(plan.contains("ScalarIndexQuery"), "{plan}");
        let reopened = LanceDBVectorStore::new(PathBuf::from(&test_path), "places".into()).await?;
        assert_eq!(
            reopened.schema().await?.field(1),
            &geometry_field("geom", true)
        );
        let stored = reopened
            .sql("places", "SELECT * FROM places ORDER BY id")
            .await?
            .collect()
            .await?;
        assert_eq!(record_batches_to_vec(Some(stored))?, rows);
        let ctx = SessionContext::new();
        crate::geometry::register_geo_functions(&ctx);
        ctx.register_table("places", reopened.to_datafusion().await?)?;
        let result = ctx.sql("SELECT id FROM places WHERE ST_Intersects(geom, flow_geomfromtext($1)) ORDER BY id LIMIT 1")
            .await?.with_param_values(vec![ScalarValue::Utf8(Some(polygon.into()))])?
            .collect().await?;
        assert_eq!(record_batches_to_vec(Some(result))?, vec![json!({"id": 2})]);
        let computed = ctx
            .sql("SELECT ST_Centroid(geom) AS center FROM places WHERE id = 2")
            .await?
            .collect()
            .await?;
        assert_eq!(
            record_batches_to_vec(Some(computed))?[0]["center"],
            rows[2]["geom"]
        );
        assert!(
            ctx.sql("UPDATE places SET geom = flow_geomfromtext('POINT(0 0)') WHERE id = 2")
                .await?
                .collect()
                .await
                .is_err()
        );
        assert!(
            db.insert(vec![
                json!({"id": 5, "geom": {"type":"Point", "coordinates":[181, 0]}})
            ])
            .await
            .is_err()
        );
        assert_eq!(db.count(None).await?, 5);
        db.drop_index(&db.list_indices().await?[0].name).await?;
        assert_eq!(db.count(Some(predicate)).await?, 2);
        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[test]
    fn geometry_batch_decode_errors_do_not_drop_rows() -> Result<()> {
        use arrow_array::BinaryArray;
        let ordinary = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "bytes",
                DataType::Binary,
                true,
            )])),
            vec![Arc::new(BinaryArray::from(vec![
                Some(&[1_u8, 2][..]),
                None,
            ]))],
        )?;
        assert_eq!(
            record_batches_to_vec(Some(vec![ordinary.clone()]))?,
            vec![json!({"bytes":[1,2]}), json!({"bytes":null})]
        );
        let invalid = RecordBatch::try_new(
            Arc::new(Schema::new(vec![crate::geometry::geometry_field(
                "geom", true,
            )])),
            vec![Arc::new(BinaryArray::from(vec![Some(&[1_u8, 2][..])]))],
        )?;
        assert!(record_batches_to_vec(Some(vec![ordinary, invalid])).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn regression_rtree_index_retains_geoarrow_metadata() -> Result<()> {
        use arrow_array::{Float64Array, StructArray};

        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "geometry".into()).await?;
        let coords = arrow_schema::Fields::from(vec![
            Field::new("x", DataType::Float64, false),
            Field::new("y", DataType::Float64, false),
        ]);
        let geometry_type = DataType::Struct(coords.clone());
        let metadata = HashMap::from([
            ("ARROW:extension:name".into(), "geoarrow.point".into()),
            (
                "ARROW:extension:metadata".into(),
                crate::geometry::WGS84_METADATA.into(),
            ),
        ]);
        let schema = Arc::new(Schema::new(vec![
            Field::new("point", geometry_type, false).with_metadata(metadata.clone()),
        ]));
        let points = StructArray::try_new(
            coords,
            vec![
                Arc::new(Float64Array::from(vec![1.0, 3.0, 5.0])),
                Arc::new(Float64Array::from(vec![2.0, 4.0, 6.0])),
            ],
            None,
        )?;
        db.insert_record_batch(RecordBatch::try_new(schema, vec![Arc::new(points)])?)
            .await?;
        let spatial_filter =
            "ST_Intersects(point, ST_GeomFromText('POLYGON ((0 0, 4 0, 4 5, 0 5, 0 0))'))";
        assert_eq!(db.count(Some(spatial_filter.into())).await?, 2);
        db.index("point", Some("RTREE")).await?;
        assert_eq!(db.count(Some(spatial_filter.into())).await?, 2);
        let plan = db
            .raw()
            .await?
            .query()
            .only_if(spatial_filter)
            .explain_plan(false)
            .await?;
        assert!(plan.contains("ScalarIndexQuery"), "{plan}");
        let indices = db.list_indices().await?;
        assert_eq!(indices.len(), 1);
        assert_eq!(
            normalized_index_selection(Some(&indices[0].index_type)),
            "RTREE"
        );
        let reopened =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "geometry".into()).await?;
        assert_eq!(reopened.schema().await?.field(0).metadata(), &metadata);
        assert_eq!(reopened.count(None).await?, 3);
        assert_eq!(reopened.count(Some(spatial_filter.into())).await?, 2);
        reopened.drop_index(&indices[0].name).await?;
        assert!(reopened.list_indices().await?.is_empty());
        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn regression_rtree_rejects_persisted_interleaved_coordinates() -> Result<()> {
        use arrow_array::Float64Array;

        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "interleaved".into()).await?;
        let coords = Arc::new(Field::new("xy", DataType::Float64, false));
        let schema = Arc::new(Schema::new(vec![
            Field::new("point", DataType::FixedSizeList(coords.clone(), 2), false).with_metadata(
                HashMap::from([
                    ("ARROW:extension:name".into(), "geoarrow.point".into()),
                    (
                        "ARROW:extension:metadata".into(),
                        crate::geometry::WGS84_METADATA.into(),
                    ),
                ]),
            ),
        ]));
        let points = FixedSizeListArray::try_new(
            coords,
            2,
            Arc::new(Float64Array::from(vec![1.0, 2.0])),
            None,
        )?;
        db.insert_record_batch(RecordBatch::try_new(schema, vec![Arc::new(points)])?)
            .await?;
        let schema = db.schema().await?;
        let DataType::FixedSizeList(child, _) = schema.field(0).data_type() else {
            panic!("point must keep its physical list shape");
        };
        assert_eq!(child.name(), "item");
        let error = db.index("point", Some("RTREE")).await.unwrap_err();
        assert!(error.to_string().contains("separated Float64 coordinates"));
        assert!(db.list_indices().await?.is_empty());
        assert_eq!(db.count(None).await?, 1);
        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn regression_lists_legacy_index_metadata_without_writing() -> Result<()> {
        use lance::dataset::transaction::{Operation, Transaction};
        use lance::dataset::write::CommitBuilder;

        let path = PathBuf::from(format!("./tmp/{}", create_id()));
        let mut db = LanceDBVectorStore::new(path.clone(), "legacy_metadata".into()).await?;
        db.insert(vec![
            json!({"a": 1, "b": 2, "c": 3}),
            json!({"a": 4, "b": 5, "c": 6}),
        ])
        .await?;
        db.index("a", Some("BTREE")).await?;
        db.index("b", Some("BITMAP")).await?;
        db.index("c", Some("BTREE")).await?;
        let expected = serde_json::to_value(db.list_indices().await?)?;
        let table = db.raw().await?;
        let wrapper = table.dataset().expect("native table");
        let dataset = wrapper.get().await?.clone();
        let original = dataset.load_indices().await?;
        let mut legacy = original.as_ref().clone();
        // Older scalar index manifests do not always record type details.
        for index in &mut legacy {
            index.index_details = None;
            index.files = None;
            index.created_at = None;
        }
        let transaction = Transaction::new(
            dataset.version().version,
            Operation::CreateIndex {
                new_indices: legacy,
                removed_indices: original.as_ref().clone(),
            },
            None,
        );
        let dataset = CommitBuilder::new(dataset).execute(transaction).await?;
        wrapper.update(dataset);
        let version = table.version().await?;
        table.checkout(version).await?;
        let persisted = wrapper.get().await?.load_indices().await?;
        assert_eq!(
            persisted
                .iter()
                .filter(|i| i.index_details.is_none())
                .count(),
            3
        );

        assert_eq!(serde_json::to_value(db.list_indices().await?)?, expected);
        assert_eq!(table.version().await?, version);
        let reopened = LanceDBVectorStore::new(path.clone(), "legacy_metadata".into()).await?;
        assert_eq!(reopened.raw().await?.version().await?, version);
        assert_eq!(
            serde_json::to_value(reopened.list_indices().await?)?,
            expected
        );
        assert_eq!(reopened.count(None).await?, 2);
        std::fs::remove_dir_all(path)?;
        Ok(())
    }

    #[tokio::test]
    async fn regression_opens_and_appends_lancedb_0_27_2_tables() -> Result<()> {
        use arrow_array::{Date32Array, StringArray, TimestampMillisecondArray};

        fn copy_fixture(
            source: &std::path::Path,
            destination: &std::path::Path,
        ) -> std::io::Result<()> {
            std::fs::create_dir_all(destination)?;
            for entry in std::fs::read_dir(source)? {
                let entry = entry?;
                let target = destination.join(entry.file_name());
                if entry.file_type()?.is_dir() {
                    copy_fixture(&entry.path(), &target)?;
                } else {
                    std::fs::copy(entry.path(), target)?;
                }
            }
            Ok(())
        }

        for fixture in ["lancedb-0.27.2", "lancedb-0.27.2-v2.2"] {
            let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(fixture)
                .join("legacy.lance");
            let path = PathBuf::from(format!("./tmp/{}", create_id()));
            copy_fixture(&source, &path.join("legacy.lance"))?;
            let mut db = LanceDBVectorStore::new(path.clone(), "legacy".into()).await?;
            let schema = Arc::new(db.schema().await?);
            assert_eq!(db.count(None).await?, 4);
            assert_eq!(db.count(Some("id >= 2".into())).await?, 3);
            assert_eq!(db.list_indices().await?.len(), 3);
            assert_eq!(
                db.count(Some("event_date >= DATE '2025-01-02'".into()))
                    .await?,
                2
            );
            let matches = db
                .vector_search(vec![1.0, 0.0, 0.0, 0.0], None, None, 1, 0)
                .await?;
            assert_eq!(matches[0]["id"], json!(1));
            assert_eq!(
                db.sql(
                    "legacy",
                    "SELECT id FROM legacy WHERE occurred_at >= TIMESTAMP '2025-01-02T00:00:00Z'"
                )
                .await?
                .count()
                .await?,
                2
            );

            let item = match schema.field_with_name("vector")?.data_type() {
                DataType::FixedSizeList(item, 4) => item.clone(),
                other => panic!("legacy vector schema changed: {other:?}"),
            };
            db.insert_record_batch(RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(Int64Array::from(vec![5])),
                    Arc::new(StringArray::from(vec!["epsilon"])),
                    Arc::new(Date32Array::from(vec![Some(20_092)])),
                    Arc::new(
                        TimestampMillisecondArray::from(vec![Some(1_735_948_800_000)])
                            .with_timezone("UTC"),
                    ),
                    Arc::new(FixedSizeListArray::try_new(
                        item,
                        4,
                        Arc::new(Float32Array::from(vec![0.5; 4])),
                        None,
                    )?),
                ],
            )?)
            .await?;
            db.index("event_date", Some("ZONEMAP")).await?;
            db.optimize(true).await?;
            let reopened = LanceDBVectorStore::new(path.clone(), "legacy".into()).await?;
            assert_eq!(reopened.schema().await?, *schema);
            assert_eq!(reopened.count(None).await?, 5);
            assert_eq!(
                reopened
                    .count(Some("event_date >= DATE '2025-01-02'".into()))
                    .await?,
                3
            );
            assert_eq!(reopened.list_indices().await?.len(), 3);
            std::fs::remove_dir_all(path)?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn regression_lance_explicit_cleanup_retains_recent_versions() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "recent_versions".to_string())
                .await?;
        db.insert(vec![json!({ "id": 1 })]).await?;
        db.insert(vec![json!({ "id": 2 })]).await?;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let table = db.raw().await?;
        let versions_before = table
            .list_versions()
            .await?
            .into_iter()
            .map(|version| version.version)
            .collect::<Vec<_>>();
        db.optimize(false).await?;
        let versions_after = table
            .list_versions()
            .await?
            .into_iter()
            .map(|version| version.version)
            .collect::<Vec<_>>();

        assert!(
            versions_before
                .iter()
                .all(|version| versions_after.contains(version)),
            "cleanup removed a version newer than the seven-day retention window"
        );
        assert_eq!(db.count(None).await?, 2);

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn metadata_only_connection_lists_empty_database_without_opening_table() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let connection = connect(&test_path).execute().await?;
        let db = LanceDBVectorStore::from_connection(connection, String::new()).await;

        assert!(db.list_tables().await?.is_empty());

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn connect_lance_does_not_create_namespace_manifest() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let connection = connect_lance(&test_path).execute().await?;

        assert!(connection.table_names().execute().await?.is_empty());
        assert!(!std::path::Path::new(&test_path).join("__manifest").exists());

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn create_empty_table_is_strictly_idempotent() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "schema_test".to_string()).await?;
        let schema = Schema::new(vec![Field::new("id", DataType::Int64, false)]);

        assert!(db.create_empty_table(schema.clone(), true).await?);
        assert_eq!(db.count(None).await?, 0);
        assert!(!db.create_empty_table(schema, true).await?);

        let mismatch = Schema::new(vec![Field::new("id", DataType::Utf8, false)]);
        let error = db.create_empty_table(mismatch, true).await.unwrap_err();
        assert!(error.to_string().contains("different schema"));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn new_tables_type_geometry_and_byte_columns_from_their_values() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let point = json!({"type": "Point", "coordinates": [13.405, 52.52]});
        let line = json!({"type": "LineString", "coordinates": [[10.0, 20.0], [20.0, 30.0]]});

        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "entities".to_string()).await?;
        db.insert(vec![
            json!({"id": 1, "geometry": point, "properties": [0, 255, 17]}),
            json!({"id": 2, "geometry": line, "properties": [1]}),
        ])
        .await?;
        db.upsert(
            vec![json!({"id": 3, "geometry": point, "properties": [2, 3]})],
            "id".into(),
        )
        .await?;

        let reopened =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "entities".to_string()).await?;
        let schema = reopened.schema().await?;
        assert!(crate::geometry::is_geometry_field(
            schema.field_with_name("geometry")?
        ));
        assert_eq!(
            schema.field_with_name("properties")?.data_type(),
            &DataType::Binary
        );
        let stored = reopened
            .sql("entities", "SELECT * FROM entities ORDER BY id")
            .await?
            .collect()
            .await?;
        assert_eq!(
            record_batches_to_vec(Some(stored))?,
            vec![
                json!({"id": 1, "geometry": point, "properties": [0, 255, 17]}),
                json!({"id": 2, "geometry": line, "properties": [1]}),
                json!({"id": 3, "geometry": point, "properties": [2, 3]}),
            ]
        );

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn binary_updates_store_the_bytes_not_their_json_text() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "blobs".to_string()).await?;
        db.insert(vec![
            json!({"id": 1, "payload": [1, 2]}),
            json!({"id": 2, "payload": [3]}),
        ])
        .await?;
        let set = |value: Value| HashMap::from([("payload".to_string(), value)]);

        db.update("id = 1", set(json!([0, 255, 17]))).await?;
        db.update("id = 2", set(json!([]))).await?;
        let mut rows = db.list(None, 10, 0).await?;
        rows.sort_by_key(|row| row["id"].as_i64());
        assert_eq!(
            rows,
            vec![
                json!({"id": 1, "payload": [0, 255, 17]}),
                json!({"id": 2, "payload": []}),
            ]
        );

        let error = db
            .update("id = 1", set(json!([256])))
            .await
            .expect_err("256 is not a byte");
        assert!(error.to_string().contains("'payload'"), "{error}");

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    async fn sql_rows(db: &LanceDBVectorStore, sql: &str) -> Result<Vec<Value>> {
        record_batches_to_vec(Some(db.sql("entities", sql).await?.collect().await?))
    }

    async fn sql_ids(db: &LanceDBVectorStore, predicate: &str) -> Result<Vec<Value>> {
        let sql = format!("SELECT id FROM entities WHERE {predicate} ORDER BY id");
        Ok(sql_rows(db, &sql)
            .await?
            .into_iter()
            .map(|row| row["id"].clone())
            .collect())
    }

    #[tokio::test]
    async fn inferred_geometry_columns_answer_spatial_queries() -> Result<()> {
        use datafusion::common::ScalarValue;
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let rows = vec![
            json!({"id": 1, "geometry": {"type": "Point", "coordinates": [2.0, 2.0]}}),
            json!({"id": 2, "geometry": {"type": "Point", "coordinates": [9.0, 9.0]}}),
            json!({"id": 3, "geometry": {"type": "LineString", "coordinates": [[-1.0, 1.0], [5.0, 1.0]]}}),
            json!({"id": 4, "geometry": {"type": "Polygon", "coordinates": [[[3.0, 3.0], [6.0, 3.0], [6.0, 6.0], [3.0, 6.0], [3.0, 3.0]]]}}),
            json!({"id": 5, "geometry": null}),
        ];
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "entities".to_string()).await?;
        db.insert(rows.clone()).await?;
        assert!(crate::geometry::is_geometry_field(
            db.schema().await?.field_with_name("geometry")?
        ));

        let area = "POLYGON ((0 0,4 0,4 4,0 4,0 0))";
        let lance_filter = format!("ST_Intersects(geometry, ST_GeomFromText('{area}'))");
        let lance_ordered = [
            format!("ST_Contains(ST_GeomFromText('{area}'), geometry)"),
            format!("ST_Within(geometry, ST_GeomFromText('{area}'))"),
        ];
        assert_eq!(db.count(Some(lance_filter.clone())).await?, 3);
        for predicate in &lance_ordered {
            assert_eq!(db.count(Some(predicate.clone())).await?, 1, "{predicate}");
        }
        db.index("geometry", Some("RTREE")).await?;
        for predicate in &lance_ordered {
            assert_eq!(db.count(Some(predicate.clone())).await?, 1, "{predicate}");
        }
        let plan = db
            .raw()
            .await?
            .query()
            .only_if(&lance_filter)
            .explain_plan(false)
            .await?;
        assert!(plan.contains("ScalarIndexQuery"), "{plan}");
        let mut matched = db.filter(&lance_filter, None, 10, 0).await?;
        matched.sort_by_key(|row| row["id"].as_i64());
        assert_eq!(matched, [&rows[0], &rows[2], &rows[3]].map(Clone::clone));

        let region = format!("flow_geomfromtext('{area}')");
        for (predicate, expected) in [
            (format!("ST_Intersects(geometry, {region})"), vec![1, 3, 4]),
            (format!("ST_Within(geometry, {region})"), vec![1]),
            (format!("ST_Contains({region}, geometry)"), vec![1]),
            (format!("ST_Crosses(geometry, {region})"), vec![3]),
            (format!("ST_Overlaps(geometry, {region})"), vec![4]),
            (format!("ST_Disjoint(geometry, {region})"), vec![2]),
        ] {
            let expected: Vec<Value> = expected.into_iter().map(Value::from).collect();
            assert_eq!(sql_ids(&db, &predicate).await?, expected, "{predicate}");
        }

        let measured = sql_rows(
            &db,
            "SELECT id, ST_Area(geometry) AS area, ST_Length(geometry) AS length, \
             ST_Distance(geometry, flow_geomfromtext('POINT(2 5)')) AS distance, \
             ST_Centroid(geometry) AS center \
             FROM entities WHERE id IN (1, 3, 4) ORDER BY id",
        )
        .await?;
        assert_eq!(measured[0]["distance"], json!(3.0));
        assert_eq!(measured[1]["length"], json!(6.0));
        assert_eq!(measured[2]["area"], json!(9.0));
        assert_eq!(
            measured[2]["center"],
            json!({"type": "Point", "coordinates": [4.5, 4.5]})
        );

        let ctx = SessionContext::new();
        crate::geometry::register_geo_functions(&ctx);
        ctx.register_table("entities", db.to_datafusion().await?)?;
        let bound = |sql: &'static str| {
            let ctx = ctx.clone();
            async move {
                let batches = ctx
                    .sql(sql)
                    .await?
                    .with_param_values(vec![ScalarValue::Utf8(Some(area.into()))])?
                    .collect()
                    .await?;
                Ok::<_, flow_like_types::Error>(
                    record_batches_to_vec(Some(batches))?
                        .into_iter()
                        .map(|row| row["id"].as_i64().unwrap_or_default())
                        .collect::<Vec<_>>(),
                )
            }
        };
        assert_eq!(
            bound("SELECT id FROM entities WHERE ST_Intersects(geometry, flow_geomfromtext($1)) ORDER BY id").await?,
            [1, 3, 4]
        );
        let limited = bound(
            "SELECT id FROM entities WHERE ST_Intersects(geometry, flow_geomfromtext($1)) LIMIT 2",
        )
        .await?;
        assert_eq!(limited.len(), 2, "{limited:?}");
        assert!(
            limited.iter().all(|id| [1, 3, 4].contains(id)),
            "{limited:?}"
        );

        let geometry_param = vec![(
            "area".to_string(),
            json!({"type": "Polygon", "coordinates": [[[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [0.0, 0.0]]]}),
        )];
        for (predicate, expected) in [
            ("ST_Intersects(geometry, $area)", vec![1, 3, 4]),
            ("ST_Contains($area, geometry)", vec![1]),
            ("ST_Within(geometry, flow_geomfromtext($area))", vec![1]),
        ] {
            let sql = format!("SELECT id FROM entities WHERE {predicate} ORDER BY id");
            let batches = ctx
                .sql(&sql)
                .await?
                .with_param_values(crate::databases::sql_params::to_param_values(
                    &geometry_param,
                )?)?
                .collect()
                .await?;
            let ids: Vec<i64> = record_batches_to_vec(Some(batches))?
                .iter()
                .filter_map(|row| row["id"].as_i64())
                .collect();
            assert_eq!(ids, expected, "{predicate}");
        }
        for (filter, expected) in [
            ("ST_Intersects(geometry, $area)", 3),
            ("ST_Contains($area, geometry)", 1),
            ("ST_Within(geometry, ST_GeomFromText($area))", 1),
        ] {
            let bound =
                crate::databases::lance_filter_params::bind_filter_params(filter, &geometry_param)?;
            assert_eq!(db.count(Some(bound)).await?, expected, "{filter}");
        }

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn typed_geometry_column_on_an_existing_table_takes_geojson_updates() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "entities".to_string()).await?;
        db.insert(vec![
            json!({"id": 1, "name": "inside"}),
            json!({"id": 2, "name": "outline"}),
            json!({"id": 3, "name": "unplaced"}),
        ])
        .await?;

        assert!(
            db.add_column("location", "flow_geomfromtext(CAST(NULL AS VARCHAR))")
                .await
                .unwrap_err()
                .to_string()
                .contains("type 'geometry'")
        );
        db.add_typed_column("location", "geometry", None).await?;
        let point = json!({"type": "Point", "coordinates": [2.0, 2.0]});
        let polygon = json!({"type": "Polygon", "coordinates": [[[3.0, 3.0], [6.0, 3.0], [6.0, 6.0], [3.0, 6.0], [3.0, 3.0]]]});
        db.update("id = 1", HashMap::from([("location".to_string(), point.clone())]))
            .await?;
        db.update(
            "id = 2",
            HashMap::from([("location".to_string(), polygon.clone())]),
        )
        .await?;
        assert!(
            db.update(
                "id = 3",
                HashMap::from([("location".to_string(), json!({"type": "Point"}))])
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("GeoJSON geometry")
        );

        let reopened =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "entities".to_string()).await?;
        assert!(crate::geometry::is_geometry_field(
            reopened.schema().await?.field_with_name("location")?
        ));
        let area = "flow_geomfromtext('POLYGON ((0 0,4 0,4 4,0 4,0 0))')";
        assert_eq!(
            sql_ids(&reopened, &format!("ST_Intersects(location, {area})")).await?,
            vec![json!(1), json!(2)]
        );
        let mut rows = reopened.filter("id IN (1, 2, 3)", None, 10, 0).await?;
        rows.sort_by_key(|row| row["id"].as_i64());
        assert_eq!(rows[0]["location"], point);
        assert_eq!(rows[1]["location"], polygon);
        assert_eq!(rows[2]["location"], Value::Null);

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn new_tables_infer_utc_dates_without_changing_legacy_string_columns() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let timestamp = "2026-08-09T12:34:56.789Z";

        let mut inferred =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "inferred_dates".to_string())
                .await?;
        inferred
            .insert(vec![json!({ "created_at": timestamp })])
            .await?;
        assert_eq!(
            inferred
                .schema()
                .await?
                .field_with_name("created_at")?
                .data_type(),
            &DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
        );

        let mut legacy =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "legacy_strings".to_string())
                .await?;
        legacy
            .create_empty_table(
                Schema::new(vec![Field::new("created_at", DataType::LargeUtf8, false)]),
                false,
            )
            .await?;
        let mut legacy =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "legacy_strings".to_string())
                .await?;
        legacy
            .insert(vec![json!({ "created_at": timestamp })])
            .await?;
        assert_eq!(
            legacy
                .schema()
                .await?
                .field_with_name("created_at")?
                .data_type(),
            &DataType::LargeUtf8
        );
        assert_eq!(legacy.list(None, 1, 0).await?[0]["created_at"], timestamp);

        let mut legacy_timestamp =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "legacy_timestamp".to_string())
                .await?;
        legacy_timestamp
            .create_empty_table(
                Schema::new(vec![
                    Field::new("id", DataType::Int64, false),
                    Field::new(
                        "created_at",
                        DataType::Timestamp(TimeUnit::Millisecond, None),
                        false,
                    ),
                ]),
                false,
            )
            .await?;
        assert!(
            !legacy_timestamp
                .create_empty_table(
                    Schema::new(vec![
                        Field::new("id", DataType::Int64, false),
                        Field::new(
                            "created_at",
                            DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
                            false,
                        ),
                    ]),
                    true,
                )
                .await?
        );
        legacy_timestamp
            .insert(vec![json!({ "id": 1, "created_at": timestamp })])
            .await?;
        assert_eq!(legacy_timestamp.count(None).await?, 1);
        assert_eq!(
            legacy_timestamp
                .schema()
                .await?
                .field_with_name("created_at")?
                .data_type(),
            &DataType::Timestamp(TimeUnit::Millisecond, None)
        );

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn update_filters_match_serialized_row_values_by_column_type() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "row_identity".to_string()).await?;
        db.insert(vec![
            json!({ "created_at": "2026-08-16T12:00:00.000Z", "label": "it's a", "score": 1.5, "flag": true, "note": null }),
            json!({ "created_at": "2026-08-17T12:00:00.000Z", "label": "b", "score": 2.5, "flag": false, "note": "x" }),
        ])
        .await?;
        assert_eq!(
            db.schema()
                .await?
                .field_with_name("created_at")?
                .data_type(),
            &DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
        );

        let rows = db.list(None, 10, 0).await?;
        let first = rows
            .iter()
            .find(|row| row["label"] == "it's a")
            .expect("row a should be listed");
        let created_at = first["created_at"]
            .as_i64()
            .expect("timestamps are serialized as native-unit integers");
        assert_eq!(created_at, 1_786_881_600_000);

        let bare_literal = db
            .update(
                &format!("created_at = {created_at}"),
                HashMap::from([("label".to_string(), json!("bare"))]),
            )
            .await;
        assert!(
            bare_literal.is_err(),
            "bare integer literals do not coerce to timestamps"
        );

        let filter = format!(
            "created_at = CAST({created_at} AS TIMESTAMP(3)) AND label = 'it''s a' AND score = 1.5 AND flag = true AND note IS NULL"
        );
        db.update(
            &filter,
            HashMap::from([("label".to_string(), json!("updated"))]),
        )
        .await?;

        let rows = db.list(None, 10, 0).await?;
        let labels: Vec<&str> = rows
            .iter()
            .filter_map(|row| row["label"].as_str())
            .collect();
        assert!(labels.contains(&"updated"), "labels: {labels:?}");
        assert!(labels.contains(&"b"), "labels: {labels:?}");
        assert!(!labels.contains(&"it's a"), "labels: {labels:?}");

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn upsert_round_trips_rows_read_back_with_integer_timestamps() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "round_trip".to_string()).await?;
        db.insert(vec![json!({
            "id": "a",
            "first_seen_at": "2026-08-16T12:00:00.000Z",
            "hits": 1
        })])
        .await?;

        let mut row = db.list(None, 10, 0).await?.remove(0);
        let first_seen_at = row["first_seen_at"]
            .as_i64()
            .expect("timestamps are read back as native-unit integers");
        row["hits"] = json!(2);

        db.upsert(vec![row], "id".to_string()).await?;

        let rows = db.list(None, 10, 0).await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["hits"], json!(2));
        assert_eq!(rows[0]["first_seen_at"].as_i64(), Some(first_seen_at));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn buffered_upsert_persists_rows_carrying_integer_timestamps() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;

        let inner =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "buffered_round_trip".to_string())
                .await?;
        let mut db = BufferedVectorStore::new(inner, 2);
        db.upsert(
            vec![json!({ "id": "a", "first_seen_at": "2026-08-16T12:00:00.000Z", "hits": 1 })],
            "id".to_string(),
        )
        .await?;
        db.flush().await?;

        let mut row = db.list(None, 10, 0).await?.remove(0);
        row["hits"] = json!(2);

        let origin = BufferedWriteOrigin::new(Arc::from("writer"), Some("operation".to_string()));
        db.upsert_with_origin(vec![row], "id".to_string(), origin)
            .await?;
        db.flush().await?;

        assert!(!db.has_write_failures());
        let rows = db.list(None, 10, 0).await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["hits"], json!(2));
        assert_eq!(rows[0]["first_seen_at"].as_i64(), Some(1_786_881_600_000));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn sql_supports_queries_that_project_no_columns() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "count_star".to_string()).await?;
        db.insert(vec![
            json!({ "id": 1, "name": "a" }),
            json!({ "id": 2, "name": "b" }),
            json!({ "id": 3, "name": "c" }),
        ])
        .await?;

        let batches = db
            .sql("count_star", "SELECT COUNT(*) AS cnt FROM count_star")
            .await?
            .collect()
            .await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["cnt"], json!(3));

        let batches = db
            .sql(
                "count_star",
                "SELECT COUNT(*) AS cnt FROM count_star WHERE id > 1",
            )
            .await?
            .collect()
            .await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows[0]["cnt"], json!(2));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn dml_statements_flow_through_a_registered_datafusion_table() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "people".to_string()).await?;
        db.insert(vec![
            json!({ "id": 1, "name": "a" }),
            json!({ "id": 2, "name": "b" }),
            json!({ "id": 3, "name": "c" }),
        ])
        .await?;

        let ctx = SessionContext::new();
        crate::geometry::register_geo_functions(&ctx);
        ctx.register_table("people", db.to_datafusion().await?)?;

        let count = |ctx: SessionContext| async move {
            let batches = ctx
                .sql("SELECT COUNT(*) AS cnt FROM people")
                .await?
                .collect()
                .await?;
            let rows: Vec<Value> = batches
                .iter()
                .map(record_batch_to_value)
                .collect::<Result<Vec<_>>>()?
                .concat();
            Ok::<Value, flow_like_types::Error>(rows[0]["cnt"].clone())
        };

        // EXPLAIN builds the DML plan without executing the mutation.
        ctx.sql("EXPLAIN DELETE FROM people WHERE id = 1")
            .await?
            .collect()
            .await?;
        assert_eq!(count(ctx.clone()).await?, json!(3));

        let batches = ctx
            .sql("UPDATE people SET name = 'z' WHERE id = 1")
            .await?
            .collect()
            .await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows[0]["count"], json!(1));

        // The mutation is visible through the provider registered before it ran.
        let batches = ctx
            .sql("SELECT name FROM people WHERE id = 1")
            .await?
            .collect()
            .await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows[0]["name"], json!("z"));

        let batches = ctx
            .sql("DELETE FROM people WHERE id = 3")
            .await?
            .collect()
            .await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows[0]["count"], json!(1));
        assert_eq!(count(ctx.clone()).await?, json!(2));

        // No effective WHERE clause (missing, constant-true or constant-false —
        // indistinguishable after optimization) must refuse, not write the table.
        assert!(
            ctx.sql("DELETE FROM people")
                .await?
                .collect()
                .await
                .is_err()
        );
        assert!(
            ctx.sql("DELETE FROM people WHERE false")
                .await?
                .collect()
                .await
                .is_err()
        );
        assert!(
            ctx.sql("UPDATE people SET name = 'q'")
                .await?
                .collect()
                .await
                .is_err()
        );
        assert_eq!(count(ctx.clone()).await?, json!(2));

        // Subquery DML shapes are refused before planning — DataFusion would
        // only forward the subquery's inner filters to the table, silently
        // mutating the wrong rows.
        assert!(
            db.sql(
                "people",
                "DELETE FROM people WHERE id IN (SELECT id FROM people WHERE name = 'z')",
            )
            .await
            .is_err()
        );
        assert_eq!(count(ctx.clone()).await?, json!(2));

        // INSERT keeps working through the same provider.
        ctx.sql("INSERT INTO people (id, name) VALUES (4, 'd')")
            .await?
            .collect()
            .await?;
        assert_eq!(count(ctx.clone()).await?, json!(3));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn dml_translates_temporal_predicates() -> Result<()> {
        use arrow_array::{Int64Array, RecordBatch, TimestampMicrosecondArray};
        use arrow_schema::{DataType, Schema as ArrowSchema};

        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "events".to_string()).await?;

        let schema = Arc::new(ArrowSchema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("ts", DataType::Timestamp(TimeUnit::Microsecond, None), true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(TimestampMicrosecondArray::from(vec![
                    1_609_459_200_000_000, // 2021-01-01
                    1_640_995_200_000_000, // 2022-01-01
                ])),
            ],
        )?;
        db.insert_record_batch(batch).await?;

        let ctx = SessionContext::new();
        crate::geometry::register_geo_functions(&ctx);
        ctx.register_table("events", db.to_datafusion().await?)?;

        let batches = ctx
            .sql("DELETE FROM events WHERE ts < '2021-06-01T00:00:00'")
            .await?
            .collect()
            .await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows[0]["count"], json!(1));

        let batches = ctx.sql("SELECT id FROM events").await?.collect().await?;
        let rows: Vec<Value> = batches
            .iter()
            .map(record_batch_to_value)
            .collect::<Result<Vec<_>>>()?
            .concat();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["id"], json!(2));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn test_lance_ingest() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_search_first() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;

        let search_results: Vec<Value> = db
            .vector_search(vec![1.0, 2.0, 3.0], None, None, 10, 0)
            .await?;

        assert!(!search_results.is_empty());

        let first_item: TestStruct = from_value(search_results[0].clone())?;

        assert_eq!(first_item, records[0]);

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_search_fts() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;
        db.index("name", Some("FULL TEXT")).await?;

        let search_results: Vec<Value> = db.fts_search("Alice", None, None, None, 10, 0).await?;

        assert!(!search_results.is_empty());

        let first_item: TestStruct = from_value(search_results[0].clone())?;
        assert_eq!(first_item, records[0]);

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_hybrid_search_without_vector_index() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;
        db.index("name", Some("FULL TEXT")).await?;

        let search_results: Vec<Value> = db
            .hybrid_search(
                vec![1.0, 2.0, 3.0],
                "Alice",
                None,
                None,
                Some(vec!["name".to_string()]),
                10,
                0,
                true,
            )
            .await?;

        assert!(!search_results.is_empty());
        let items: Vec<TestStruct> = search_results
            .into_iter()
            .map(from_value)
            .collect::<Result<_, _>>()?;
        assert!(items.iter().any(|item| item.id == 1));

        let search_results_with_vector_field: Vec<Value> = db
            .hybrid_search(
                vec![1.0, 2.0, 3.0],
                "Alice",
                None,
                None,
                Some(vec!["vector".to_string(), "name".to_string()]),
                10,
                0,
                true,
            )
            .await?;

        assert!(!search_results_with_vector_field.is_empty());

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_search_second() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;

        let search_results: Vec<Value> = db
            .vector_search(vec![2.0, 3.0, 4.0], None, None, 10, 0)
            .await?;

        assert!(!search_results.is_empty());

        let first_item: TestStruct = from_value(search_results[0].clone())?;

        assert_eq!(first_item, records[1]);

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_search_filter() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;

        let search_results: Vec<Value> = db
            .vector_search(vec![1.0, 2.0, 3.0], Some("id = 2"), None, 10, 0)
            .await?;

        assert!(!search_results.is_empty());

        let first_item: TestStruct = from_value(search_results[0].clone())?;

        assert_eq!(first_item, records[1]);

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_no_vec() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct2 {
                id: 1,
                name: "Alice".to_string(),
            },
            TestStruct2 {
                id: 2,
                name: "Bob".to_string(),
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;

        let count = db.count(None).await?;

        assert_eq!(count, 2);

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_casting() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string())
            .await
            .unwrap();
        let cacheable: Arc<dyn Cacheable> = Arc::new(db.clone());
        let resolved = cacheable
            .as_any()
            .downcast_ref::<LanceDBVectorStore>()
            .unwrap();
        let resolved = resolved.clone();
        assert_eq!(resolved.connection.uri(), db.connection.uri());

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_select() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let records = vec![
            TestStruct {
                id: 1,
                name: "Alice".to_string(),
                vector: vec![1.0, 2.0, 3.0],
            },
            TestStruct {
                id: 2,
                name: "Bob".to_string(),
                vector: vec![2.0, 3.0, 4.0],
            },
        ];

        let json_records: Vec<Value> = records
            .clone()
            .into_iter()
            .map(to_value)
            .collect::<Result<_, _>>()?;

        db.upsert(json_records, "id".to_string()).await?;

        let select = Some(vec!["id".to_string(), "name".to_string()]);
        let results: Vec<Value> = db.list(select, 10, 0).await?;

        assert!(!results.is_empty());

        let first_item: TestStruct2 = from_value(results[0].clone())?;

        assert_eq!(
            first_item,
            TestStruct2 {
                id: records[0].id,
                name: records[0].name.clone()
            }
        );

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_upsert_rejects_missing_non_nullable_fields() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();

        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        db.upsert(
            vec![json!({"id": 1, "name": "Alice", "tag": "alpha"})],
            "id".to_string(),
        )
        .await?;

        let result = db
            .upsert(vec![json!({"id": 2, "name": "Bob"})], "id".to_string())
            .await;

        assert!(result.is_err());

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_lance_upsert_nullable_option_field() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();

        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;

        let rows_in: Vec<Value> = vec![
            to_value(&NullableFieldRow {
                id: 1,
                name: "Alice".to_string(),
                tag: Some("alpha".to_string()),
            })?,
            to_value(&NullableFieldRow {
                id: 2,
                name: "Bob".to_string(),
                tag: None,
            })?,
        ];

        db.upsert(rows_in, "id".to_string()).await?;

        let rows: Vec<NullableFieldRow> = db
            .list(
                Some(vec![
                    "id".to_string(),
                    "name".to_string(),
                    "tag".to_string(),
                ]),
                10,
                0,
            )
            .await?
            .into_iter()
            .map(from_value)
            .collect::<Result<_, _>>()?;

        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row
            == &NullableFieldRow {
                id: 1,
                name: "Alice".to_string(),
                tag: Some("alpha".to_string()),
            }));
        assert!(rows.iter().any(|row| row
            == &NullableFieldRow {
                id: 2,
                name: "Bob".to_string(),
                tag: None,
            }));

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_buffered_upsert_deduplicates_same_id_before_flush() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();

        let inner = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let mut db = BufferedVectorStore::new(inner, 10);

        db.upsert(
            vec![json!({"id": 1, "name": "Alice", "tag": "alpha"})],
            "id".to_string(),
        )
        .await?;
        db.upsert(
            vec![json!({"id": 1, "name": "Alice Updated", "tag": "beta"})],
            "id".to_string(),
        )
        .await?;
        db.upsert(
            vec![json!({"id": 2, "name": "Bob", "tag": "gamma"})],
            "id".to_string(),
        )
        .await?;

        db.flush().await?;

        let mut rows: Vec<NullableFieldRow> = db
            .list(
                Some(vec![
                    "id".to_string(),
                    "name".to_string(),
                    "tag".to_string(),
                ]),
                10,
                0,
            )
            .await?
            .into_iter()
            .map(from_value)
            .collect::<Result<_, _>>()?;

        rows.sort_by_key(|row| row.id);

        assert_eq!(
            rows,
            vec![
                NullableFieldRow {
                    id: 1,
                    name: "Alice Updated".to_string(),
                    tag: Some("beta".to_string()),
                },
                NullableFieldRow {
                    id: 2,
                    name: "Bob".to_string(),
                    tag: Some("gamma".to_string()),
                },
            ]
        );

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn test_buffered_upsert_rejects_missing_fields_against_existing_table() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();

        let inner = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let mut db = BufferedVectorStore::new(inner, 10);

        // First: establish the table schema by writing a record with "tag"
        db.upsert(
            vec![json!({"id": 1, "name": "Alice", "tag": "alpha"})],
            "id".to_string(),
        )
        .await?;
        db.flush().await?;

        // Now upsert a record that is MISSING the "tag" field
        db.upsert(vec![json!({"id": 2, "name": "Bob"})], "id".to_string())
            .await?;

        let result = db.flush().await;
        assert!(
            result.is_err(),
            "flush should fail when records are missing fields from the established schema"
        );

        std::fs::remove_dir_all(&test_path).unwrap();

        Ok(())
    }

    #[tokio::test]
    async fn buffered_write_failures_keep_the_exact_writer_origin() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;

        let inner = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let mut db = BufferedVectorStore::new(inner, 2);

        // Establish a non-nullable three-column schema first.
        db.upsert(
            vec![json!({"id": 1, "name": "seed", "tag": "seed"})],
            "id".to_string(),
        )
        .await?;
        db.flush().await?;

        let good_origin =
            BufferedWriteOrigin::new(Arc::from("writer-good"), Some("operation-good".to_string()));
        let bad_origin =
            BufferedWriteOrigin::new(Arc::from("writer-bad"), Some("operation-bad".to_string()));

        db.upsert_with_origin(
            vec![json!({"id": 2, "name": "persisted", "tag": "valid"})],
            "id".to_string(),
            good_origin,
        )
        .await?;
        let error = db
            .upsert_with_origin(
                vec![json!({"id": 3, "name": "rejected"})],
                "id".to_string(),
                bad_origin.clone(),
            )
            .await
            .expect_err("the second row should trigger a threshold flush failure");

        let report = error
            .downcast_ref::<BufferedWriteError>()
            .expect("flush error should retain structured failures");
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].origin.as_ref(), Some(&bad_origin));
        assert_eq!(report.failures[0].operation, BufferedWriteKind::Upsert);
        assert!(!report.failures[0].error.is_empty());
        assert!(!error.to_string().contains("rejected"));

        // The report survives the failed flush so a completion callback can
        // still create a node-attributed error after the buffer was drained.
        assert!(!db.is_dirty());
        assert!(db.has_write_failures());
        assert_eq!(
            db.write_failure_report()
                .expect("ensure-flush callers should still see the failure")
                .failures,
            report.failures
        );
        let retained = db.take_write_failures();
        assert_eq!(retained, report.failures);
        assert!(!db.has_write_failures());

        let insert_origin = BufferedWriteOrigin::new(
            Arc::from("insert-writer"),
            Some("insert-operation".to_string()),
        );
        db.insert_with_origin(
            vec![json!({"id": 4, "name": "invalid-insert"})],
            insert_origin.clone(),
        )
        .await?;
        let insert_error = db.flush().await.expect_err("insert row should fail");
        let insert_report = insert_error
            .downcast_ref::<BufferedWriteError>()
            .expect("insert flush should retain structured failures");
        assert_eq!(insert_report.failures.len(), 1);
        assert_eq!(
            insert_report.failures[0].origin.as_ref(),
            Some(&insert_origin)
        );
        assert_eq!(
            insert_report.failures[0].operation,
            BufferedWriteKind::Insert
        );
        db.take_write_failures();

        let rows = db.list(None, 10, 0).await?;
        assert!(rows.iter().any(|row| row["id"] == json!(2)));
        assert!(!rows.iter().any(|row| row["id"] == json!(3)));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn buffered_upsert_deduplication_keeps_last_writer_origin() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;

        let inner = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        let mut db = BufferedVectorStore::new(inner, 10);
        db.upsert(
            vec![json!({"id": 1, "name": "seed", "tag": "seed"})],
            "id".to_string(),
        )
        .await?;
        db.flush().await?;

        let first_origin =
            BufferedWriteOrigin::new(Arc::from("writer-first"), Some("first".to_string()));
        let last_origin =
            BufferedWriteOrigin::new(Arc::from("writer-last"), Some("last".to_string()));
        db.upsert_with_origin(
            vec![json!({"id": 2, "name": "first-invalid"})],
            "id".to_string(),
            first_origin,
        )
        .await?;
        db.upsert_with_origin(
            vec![json!({"id": 2, "name": "last-invalid"})],
            "id".to_string(),
            last_origin.clone(),
        )
        .await?;

        let error = db.flush().await.expect_err("deduplicated row should fail");
        let report = error
            .downcast_ref::<BufferedWriteError>()
            .expect("flush error should retain structured failures");
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].origin.as_ref(), Some(&last_origin));

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn test_lance_add_and_drop_columns() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();

        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        db.upsert(
            vec![to_value(&TestStruct2 {
                id: 1,
                name: "Alice".to_string(),
            })?],
            "id".to_string(),
        )
        .await?;

        db.add_column("counter", "CAST(0 AS INT)").await?;
        db.add_column("note", "CAST('' AS STRING)").await?;
        db.add_column("flag", "CAST(NULL AS STRING)").await?;

        let names: Vec<String> = db
            .schema()
            .await?
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        assert!(names.contains(&"counter".to_string()));
        assert!(names.contains(&"note".to_string()));
        assert!(names.contains(&"flag".to_string()));

        db.drop_columns(&["counter", "note"]).await?;

        let names: Vec<String> = db
            .schema()
            .await?
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        assert!(!names.contains(&"counter".to_string()));
        assert!(!names.contains(&"note".to_string()));
        assert!(names.contains(&"flag".to_string()));

        std::fs::remove_dir_all(&test_path).unwrap();
        Ok(())
    }

    #[tokio::test]
    async fn test_lance_add_column_bare_null_fails() -> Result<()> {
        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path).unwrap();

        let mut db = LanceDBVectorStore::new(PathBuf::from(&test_path), "t".to_string()).await?;
        db.upsert(
            vec![to_value(&TestStruct2 {
                id: 1,
                name: "Alice".to_string(),
            })?],
            "id".to_string(),
        )
        .await?;

        let version = db.raw().await?.version().await?;
        // Existing nodes require CAST(NULL AS <type>) for a nullable column.
        for expression in ["NULL", " null ", "((NULL))", "/* default */ NULL"] {
            let bare_null = db.add_column("flag", expression).await;
            assert!(
                bare_null.is_err(),
                "bare NULL should require an explicit type"
            );
        }
        let bare_null = db
            .add_columns(
                NewColumnTransform::SqlExpressions(vec![
                    ("valid".into(), "0".into()),
                    ("flag".into(), "NULL".into()),
                ]),
                None,
            )
            .await;
        assert!(
            bare_null.is_err(),
            "bulk additions must reject bare NULL before adding any columns"
        );
        assert_eq!(db.raw().await?.version().await?, version);
        assert_eq!(db.schema().await?.fields().len(), 2);

        std::fs::remove_dir_all(&test_path).unwrap();
        Ok(())
    }

    /// The end of the parameter path: a value bound into an `only_if` predicate reaches the
    /// row it names, and a value that tries to close its own literal reaches none — proved
    /// against a real table rather than against the string this module produces.
    #[tokio::test]
    async fn bound_filter_values_stay_inside_their_literal() -> Result<()> {
        use crate::databases::lance_filter_params::{bind_filter_params, resolve_filter_params};

        fn bind(filter: &str, supplied: Value) -> Result<String> {
            bind_filter_params(filter, &resolve_filter_params(filter, &supplied)?)
        }

        let test_path = format!("./tmp/{}", create_id());
        std::fs::create_dir_all(&test_path)?;
        let mut db =
            LanceDBVectorStore::new(PathBuf::from(&test_path), "bound_filter".to_string()).await?;
        db.insert(vec![
            json!({ "id": "a", "name": "first" }),
            json!({ "id": "o'brien", "name": "second" }),
            json!({ "id": "c", "name": "third" }),
        ])
        .await?;

        let quoted = bind("id = $id", json!({ "id": "o'brien" }))?;
        let rows = db.filter(&quoted, None, 10, 0).await?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["name"], "second");

        for attempt in [
            "' OR id != '",
            "a' OR 'a' = 'a",
            // The backslash case: this dialect does not read `\'` as an escape, so the
            // doubled quote is what keeps the tail inside the literal.
            "x\\' OR true --",
        ] {
            let filter = bind("id = $id", json!({ "id": attempt }))?;
            assert!(
                db.filter(&filter, None, 10, 0).await?.is_empty(),
                "matched rows for {attempt}: {filter}"
            );
        }

        let in_list = bind("id IN ($ids)", json!({ "ids": ["a", "c"] }))?;
        assert_eq!(db.filter(&in_list, None, 10, 0).await?.len(), 2);
        let empty_list = bind("id IN ($ids)", json!({ "ids": [] }))?;
        assert!(db.filter(&empty_list, None, 10, 0).await?.is_empty());

        // The delete predicate goes through the same parser as the query one.
        db.delete(&quoted).await?;
        assert_eq!(db.count(None).await?, 2);

        std::fs::remove_dir_all(&test_path)?;
        Ok(())
    }
}

// impl VectorStoreIndex for LanceDBVectorStore {
//     fn top_n<T: for<'a> serde::Deserialize<'a> + rig::wasm_compat::WasmCompatSend>(
//             &self,
//             req: rig::vector_store::VectorSearchRequest<Self::Filter>,
//         ) -> impl std::future::Future<Output = std::result::Result<Vec<(f64, String, T)>, rig::vector_store::VectorStoreError>>
//         + rig::wasm_compat::WasmCompatSend {
//         todo!("Implement top_n_ids")
//     }

//     fn top_n_ids(
//             &self,
//             req: rig::vector_store::VectorSearchRequest<Self::Filter>,
//         ) -> impl std::future::Future<Output = std::result::Result<Vec<(f64, String)>, rig::vector_store::VectorStoreError>> + rig::wasm_compat::WasmCompatSend {
//         todo!("Implement top_n_ids")
//     }

//     type Filter;
// }

#[cfg(test)]
mod scalar_maintenance_tests {
    use super::*;
    use arrow_array::{Float64Array, Int64Array, StringArray, StructArray};
    use arrow_schema::Field;
    use flow_like_types::{create_id, json::json};

    #[tokio::test]
    async fn explicit_scalar_index_preserves_nested_column_paths() -> Result<()> {
        let test_path = PathBuf::from(format!("./tmp/{}", create_id()));
        std::fs::create_dir_all(&test_path)?;
        let mut db = LanceDBVectorStore::new(test_path.clone(), "nested".into()).await?;
        db.insert(vec![
            json!({"id": 1, "metadata": {"category": 3}}),
            json!({"id": 2, "metadata": {"category": 7}}),
            json!({"id": 3, "metadata": {"category": 7}}),
        ])
        .await?;

        db.index("metadata.category", Some("BTREE")).await?;
        let mut rows = db
            .filter("metadata.category = 7", Some(vec!["id".into()]), 10, 0)
            .await?;
        rows.sort_by_key(|row| row["id"].as_i64());
        assert_eq!(rows, vec![json!({"id": 2}), json!({"id": 3})]);
        let indices = db.list_indices().await?;
        assert_eq!(indices.len(), 1);
        assert_eq!(indices[0].columns, vec!["metadata.category"]);
        assert_eq!(indices[0].index_type, "BTREE");
        db.drop_index(&indices[0].name).await?;
        assert!(db.list_indices().await?.is_empty());
        std::fs::remove_dir_all(test_path)?;
        Ok(())
    }

    #[tokio::test]
    async fn compaction_preserves_scalar_index_names_tuning_and_results() -> Result<()> {
        let test_path = PathBuf::from(format!("./tmp/{}", create_id()));
        std::fs::create_dir_all(&test_path)?;
        let mut db = LanceDBVectorStore::new(test_path.clone(), "maintenance".into()).await?;
        let coordinates = vec![
            Arc::new(Field::new("x", DataType::Float64, true)),
            Arc::new(Field::new("y", DataType::Float64, true)),
        ];
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("text", DataType::Utf8, false),
            Field::new("zoned", DataType::Int64, false),
            Field::new("member", DataType::Int64, false),
            Field::new("point", DataType::Struct(coordinates.clone().into()), true).with_metadata(
                std::collections::HashMap::from([
                    ("ARROW:extension:name".into(), "geoarrow.point".into()),
                    (
                        "ARROW:extension:metadata".into(),
                        crate::geometry::WGS84_METADATA.into(),
                    ),
                ]),
            ),
        ]));
        for fragment in 0..4 {
            let ids = (fragment * 32..(fragment + 1) * 32).collect::<Vec<i64>>();
            let points = StructArray::new(
                coordinates.clone().into(),
                vec![
                    Arc::new(Float64Array::from_iter_values(
                        ids.iter().map(|id| (*id % 80) as f64),
                    )),
                    Arc::new(Float64Array::from_iter_values(
                        ids.iter().map(|id| (*id % 80) as f64),
                    )),
                ],
                Some(ids.iter().map(|id| id % 4 != 0).collect::<Vec<_>>().into()),
            );
            db.insert_record_batch(RecordBatch::try_new(
                schema.clone(),
                vec![
                    Arc::new(Int64Array::from(ids.clone())),
                    Arc::new(StringArray::from_iter_values(
                        ids.iter()
                            .map(|id| if id % 2 == 0 { "needle" } else { "haystack" }),
                    )),
                    Arc::new(Int64Array::from(ids.clone())),
                    Arc::new(Int64Array::from_iter_values(ids.iter().map(|id| id % 11))),
                    Arc::new(points),
                ],
            )?)
            .await?;
        }

        let table = db.table.as_ref().unwrap().clone();
        let wrapper = table.dataset().unwrap();
        let mut dataset = wrapper.get().await?.as_ref().clone();
        assert!(!dataset.manifest().uses_stable_row_ids());
        assert_eq!(dataset.get_fragments().len(), 4);
        for (name, column, params) in [
            (
                "imported_fm",
                "text",
                ScalarIndexParams::for_builtin(BuiltinIndexType::Fm),
            ),
            (
                "imported_zone",
                "zoned",
                ScalarIndexParams::for_builtin(BuiltinIndexType::ZoneMap)
                    .with_params(&json!({"rows_per_zone": 16})),
            ),
            (
                "imported_bloom",
                "member",
                ScalarIndexParams::for_builtin(BuiltinIndexType::BloomFilter)
                    .with_params(&json!({"number_of_items": 16, "probability": 0.01})),
            ),
            (
                "imported_rtree",
                "point",
                ScalarIndexParams::for_builtin(BuiltinIndexType::RTree)
                    .with_params(&json!({"page_size": 16})),
            ),
        ] {
            dataset
                .create_index_builder(&[column], lance_index::IndexType::Scalar, &params)
                .name(name.into())
                .await?;
        }
        wrapper.update(dataset);
        let expected_indices = scalar_indices_for_compaction(&table).await?;
        assert_eq!(expected_indices.len(), 4);
        let filters = [
            "text LIKE '%needle%'",
            "zoned >= 32 AND zoned < 96",
            "member = 5",
            "point IS NULL",
        ];
        let mut expected = Vec::new();
        for filter in filters {
            let mut rows = db.filter(filter, Some(vec!["id".into()]), 128, 0).await?;
            rows.sort_by_key(|row| row["id"].as_i64());
            assert!(!rows.is_empty(), "filter should select rows: {filter}");
            expected.push(rows);
        }

        db.optimize(true).await?;
        assert_eq!(wrapper.get().await?.get_fragments().len(), 1);
        assert_eq!(
            scalar_indices_for_compaction(&table).await?,
            expected_indices
        );
        let reopened = LanceDBVectorStore::new(test_path.clone(), "maintenance".into()).await?;
        assert_eq!(reopened.list_indices().await?.len(), 4);
        for (filter, expected) in filters.into_iter().zip(expected) {
            let mut rows = reopened
                .filter(filter, Some(vec!["id".into()]), 128, 0)
                .await?;
            rows.sort_by_key(|row| row["id"].as_i64());
            assert_eq!(rows, expected, "filter changed after compaction: {filter}");
        }
        std::fs::remove_dir_all(test_path)?;
        Ok(())
    }
}

#[cfg(test)]
mod creation_concurrency_tests {
    use super::*;
    use arrow_array::{Int64Array, StringArray};
    use arrow_schema::Field;
    use flow_like_types::{create_id, json::json};
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Notify;

    #[derive(Debug)]
    struct PauseFirstFragment {
        paused: AtomicBool,
        ready: Arc<Notify>,
        resume: Arc<Notify>,
    }

    #[async_trait]
    impl lance::dataset::progress::WriteFragmentProgress for PauseFirstFragment {
        async fn begin(&self, _: &lance::table::format::Fragment) -> lance::Result<()> {
            if !self.paused.swap(true, Ordering::SeqCst) {
                self.ready.notify_one();
                self.resume.notified().await;
            }
            Ok(())
        }

        async fn complete(&self, _: &lance::table::format::Fragment) -> lance::Result<()> {
            Ok(())
        }
    }

    fn batch(ids: Vec<i64>, values: Vec<&str>) -> Result<RecordBatch> {
        Ok(RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("id", DataType::Int64, false),
                Field::new("value", DataType::Utf8, false),
            ])),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(values)),
            ],
        )?)
    }

    #[tokio::test]
    async fn initial_writes_replay_after_another_creator_wins() -> Result<()> {
        for operation in ["insert", "record_batch", "upsert"] {
            let path = PathBuf::from(format!("./tmp/{}", create_id()));
            std::fs::create_dir_all(&path)?;
            let mut delayed = LanceDBVectorStore::new(path.clone(), "concurrent".into()).await?;
            let mut winner = LanceDBVectorStore::new(path.clone(), "concurrent".into()).await?;
            let ready = Arc::new(Notify::new());
            let resume = Arc::new(Notify::new());
            let mut options = crate::lancedb_write_options::default_write_options();
            options.lance_write_params.as_mut().unwrap().progress = Arc::new(PauseFirstFragment {
                paused: AtomicBool::new(false),
                ready: ready.clone(),
                resume: resume.clone(),
            });
            delayed.set_write_options(options);
            winner.set_write_options(crate::lancedb_write_options::default_write_options());
            let writing = tokio::spawn(async move {
                match operation {
                    "insert" => {
                        delayed
                            .insert(vec![json!({"id": 3, "value": "delayed"})])
                            .await?
                    }
                    "record_batch" => {
                        delayed
                            .insert_record_batch(batch(vec![3], vec!["delayed"])?)
                            .await?
                    }
                    _ => {
                        delayed
                            .upsert(
                                vec![
                                    json!({"id": 1, "value": "updated"}),
                                    json!({"id": 3, "value": "delayed"}),
                                ],
                                "id".into(),
                            )
                            .await?
                    }
                }
                Ok::<_, flow_like_types::Error>(delayed)
            });
            tokio::time::timeout(std::time::Duration::from_secs(10), ready.notified()).await?;
            winner
                .insert_record_batch(batch(vec![1, 2], vec!["winner", "winner"])?)
                .await?;
            resume.notify_one();
            let mut delayed = writing.await??;
            assert!(matches!(
                delayed
                    .write_options
                    .as_ref()
                    .unwrap()
                    .lance_write_params
                    .as_ref()
                    .unwrap()
                    .mode,
                lance::dataset::WriteMode::Append
            ));
            delayed
                .insert(vec![json!({"id": 4, "value": "later"})])
                .await?;
            let reopened = LanceDBVectorStore::new(path.clone(), "concurrent".into()).await?;
            let mut rows = reopened.list(None, 10, 0).await?;
            rows.sort_by_key(|row| row["id"].as_i64());
            assert_eq!(
                rows,
                vec![
                    json!({"id": 1, "value": if operation == "upsert" { "updated" } else { "winner" }}),
                    json!({"id": 2, "value": "winner"}),
                    json!({"id": 3, "value": "delayed"}),
                    json!({"id": 4, "value": "later"}),
                ],
                "{operation}"
            );
            std::fs::remove_dir_all(path)?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn empty_table_creation_with_append_options_preserves_existing_table() -> Result<()> {
        let path = PathBuf::from(format!("./tmp/{}", create_id()));
        std::fs::create_dir_all(&path)?;
        let mut stale = LanceDBVectorStore::new(path.clone(), "concurrent".into()).await?;
        let mut winner = LanceDBVectorStore::new(path.clone(), "concurrent".into()).await?;
        stale.set_write_options(crate::lancedb_write_options::default_write_options());
        winner
            .insert_record_batch(batch(vec![1], vec!["winner"])?)
            .await?;
        let schema = winner.schema().await?;
        assert!(
            stale
                .create_empty_table(schema.clone(), false)
                .await
                .is_err()
        );
        assert!(!stale.create_empty_table(schema, true).await?);
        assert_eq!(
            stale.list(None, 10, 0).await?,
            vec![json!({"id": 1, "value": "winner"})]
        );
        std::fs::remove_dir_all(path)?;
        Ok(())
    }
}

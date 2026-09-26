use super::NodeDBConnection;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_storage_contracts::database::DatabaseSelector;
use flow_like_storage_contracts::database::{
    DatabaseBranch, DatabaseCleanupStats, DatabaseDiff, DatabaseReference, DatabaseTag,
    DatabaseVersion,
};
use flow_like_types::{Result, async_trait, json::json};

#[cfg(feature = "execute")]
use flow_like_catalog_core::{CachedDB, CachedDBRefreshHook, CachedDBRefresher};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::{
    VectorStore, buffered::BufferedVectorStore, lancedb::LanceDBVectorStore,
};
#[cfg(feature = "execute")]
use flow_like_types::sync::RwLock;
#[cfg(feature = "execute")]
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub(super) fn add_selector_pins(node: &mut Node) {
    node.add_input_pin(
        "branch",
        "Branch",
        "Branch to open. A tag resolves its own branch.",
        VariableType::String,
    )
    .set_default_value(Some(json!("main")));
    node.add_input_pin(
        "revision",
        "Revision",
        "Latest stays writable; Version and Tag select a read-only snapshot.",
        VariableType::String,
    )
    .set_options(
        PinOptions::new()
            .set_valid_values(vec!["Latest".into(), "Version".into(), "Tag".into()])
            .build(),
    )
    .set_default_value(Some(json!("Latest")));
    node.add_input_pin(
        "version",
        "Version",
        "Version within the selected branch, used when Revision is Version.",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(1)));
    node.add_input_pin(
        "tag",
        "Tag",
        "Named snapshot, used when Revision is Tag.",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "read_only",
        "Read Only",
        "Disallow writes even when opening the latest revision.",
        VariableType::Boolean,
    )
    .set_default_value(Some(json!(false)));
}

pub(super) fn add_reference_output(node: &mut Node) {
    node.add_output_pin("reference", "Reference", "Resolved table, branch, version and access state. An unopened new table has no reference yet.", VariableType::Struct)
        .set_schema::<DatabaseReference>()
        .set_options(PinOptions::new().set_optional(true).build());
}

#[cfg(feature = "execute")]
pub(super) async fn read_selector(context: &mut ExecutionContext) -> Result<DatabaseSelector> {
    // Older saved nodes have no selector pins. Supplied pins must still validate.
    let branch: String = if context.get_pin_by_name("branch").await.is_ok() {
        context.evaluate_pin("branch").await?
    } else {
        "main".into()
    };
    let revision: String = if context.get_pin_by_name("revision").await.is_ok() {
        context.evaluate_pin("revision").await?
    } else {
        "Latest".into()
    };
    let read_only = if context.get_pin_by_name("read_only").await.is_ok() {
        context.evaluate_pin("read_only").await?
    } else {
        false
    };
    let mut selector = DatabaseSelector {
        branch: branch.trim().into(),
        version: None,
        tag: None,
        read_only,
    };
    match revision.as_str() {
        "Latest" => {}
        "Version" => {
            let version: i64 = context.evaluate_pin("version").await?;
            if version < 1 {
                return Err(flow_like_types::anyhow!("Version must be positive"));
            }
            selector.version = Some(version as u64);
        }
        "Tag" => {
            let tag: String = context.evaluate_pin("tag").await?;
            if tag.trim().is_empty() {
                return Err(flow_like_types::anyhow!("Tag must not be empty"));
            }
            selector.tag = Some(tag.trim().into());
            selector.branch = "main".into();
        }
        _ => return Err(flow_like_types::anyhow!("Unknown revision: {revision}")),
    }
    selector.validate()?;
    Ok(selector)
}

#[cfg(feature = "execute")]
pub(super) fn selection_cache_key(base: &str, selector: &DatabaseSelector) -> Result<String> {
    if selector.branch == "main"
        && selector.version.is_none()
        && selector.tag.is_none()
        && !selector.read_only
    {
        return Ok(base.to_owned());
    }
    let serialized = flow_like_types::json::to_vec(selector)?;
    Ok(format!(
        "{base}::ref::{}",
        blake3::hash(&serialized).to_hex()
    ))
}

#[cfg(feature = "execute")]
pub(super) async fn optional_reference(
    store: &LanceDBVectorStore,
) -> Result<Option<DatabaseReference>> {
    // Local mirror versions and offline branches are never valid cloud references.
    if store.is_durably_managed() {
        return Ok(None);
    }
    if !store.table_exists().await? {
        // Legacy Open Database creates the physical table on the first write.
        return Ok(None);
    }
    Ok(Some(store.reference().await?))
}

#[cfg(feature = "execute")]
struct ViewRefresher {
    source: NodeDBConnection,
    cache_key: String,
    source_generation: AtomicU64,
    generation: AtomicU64,
    lock: flow_like_types::tokio::sync::Mutex<()>,
}

#[cfg(feature = "execute")]
#[async_trait]
impl CachedDBRefresher for ViewRefresher {
    async fn refresh(&self, context: &ExecutionContext) -> Result<()> {
        let _guard = self.lock.lock().await;
        let (source, generation) = self
            .source
            .load_with_generation(&mut context.clone())
            .await?;
        if self.source_generation.load(Ordering::Acquire) == generation {
            return Ok(());
        }
        let cached = context
            .cache
            .read()
            .await
            .get(&self.cache_key)
            .cloned()
            .ok_or_else(|| flow_like_types::anyhow!("Database view cache disappeared"))?;
        let cached = cached
            .as_any()
            .downcast_ref::<CachedDB>()
            .ok_or_else(|| flow_like_types::anyhow!("Unexpected database view cache type"))?;
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

#[cfg(feature = "execute")]
pub(super) async fn cache_view(
    context: &mut ExecutionContext,
    source: NodeDBConnection,
    store: LanceDBVectorStore,
) -> Result<NodeDBConnection> {
    let (_, source_generation) = source.load_with_generation(context).await?;
    let cache_key = format!("db::view::{}", uuid::Uuid::new_v4());
    let cached = CachedDB {
        db: Arc::new(RwLock::new(BufferedVectorStore::new(store, 0))),
    };
    let refresher = Arc::new(ViewRefresher {
        source,
        cache_key: cache_key.clone(),
        source_generation: AtomicU64::new(source_generation),
        generation: AtomicU64::new(0),
        lock: flow_like_types::tokio::sync::Mutex::new(()),
    });
    let mut refresh_context = context.clone();
    refresh_context.completion_callbacks = Arc::new(RwLock::new(Vec::new()));
    let completion_db = cached.clone();
    let completion_refresher = refresher.clone();
    let origin = CachedDB::write_origin(context);
    context
        .hook_completion_event(Arc::new(move |run| {
            let db = completion_db.clone();
            let refresher = completion_refresher.clone();
            let context = refresh_context.clone();
            let origin = origin.clone();
            Box::pin(async move {
                if db.has_buffered_writes().await
                    && let Err(error) = refresher.refresh(&context).await
                {
                    db.log_pending_write_error(
                        run,
                        &origin,
                        &format!("Database view writes could not be persisted because its connection could not be refreshed: {error:#}"),
                    ).await;
                    return Err(error);
                }
                db.flush_on_completion(run, &origin).await
            })
        }))
        .await;
    let mut cache = context.cache.write().await;
    cache.insert(cache_key.clone(), Arc::new(cached));
    cache.insert(
        NodeDBConnection::refresh_cache_key(&cache_key),
        Arc::new(CachedDBRefreshHook::new(refresher)),
    );
    Ok(NodeDBConnection { cache_key })
}

#[derive(Clone, Copy)]
enum Operation {
    Reference,
    Versions,
    Branches,
    Tags,
    Checkout,
    Snapshot,
    CreateBranch,
    DeleteBranch,
    CreateTag,
    UpdateTag,
    DeleteTag,
    Restore,
    Cleanup,
    Compare,
    Clone,
}

impl Operation {
    fn description(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Self::Reference => (
                "database_reference",
                "Get Database Reference",
                "reference",
                "Read the selected branch, version and access state without flushing pending writes.",
            ),
            Self::Versions => (
                "database_versions",
                "List Database Versions",
                "versions",
                "List committed versions of the selected branch.",
            ),
            Self::Branches => (
                "database_branches",
                "List Database Branches",
                "branches",
                "List branches and their parent references.",
            ),
            Self::Tags => (
                "database_tags",
                "List Database Tags",
                "tags",
                "List tags and the exact branch versions they reference.",
            ),
            Self::Checkout => (
                "database_checkout",
                "Checkout Database",
                "checkout",
                "Open an independent branch or snapshot handle without changing the source connection.",
            ),
            Self::Snapshot => (
                "database_snapshot",
                "Snapshot Database",
                "snapshot",
                "Flush pending writes and pin an independent read-only snapshot. Optionally create a retention tag.",
            ),
            Self::CreateBranch => (
                "database_create_branch",
                "Create Database Branch",
                "createBranch",
                "Create a writable branch from the current committed snapshot and return its connection.",
            ),
            Self::DeleteBranch => (
                "database_delete_branch",
                "Delete Database Branch",
                "deleteBranch",
                "Delete a named branch. Use Drop Table only to delete the entire table.",
            ),
            Self::CreateTag => (
                "database_create_tag",
                "Create Database Tag",
                "createTag",
                "Flush pending writes and create a tag for the selected branch version.",
            ),
            Self::UpdateTag => (
                "database_update_tag",
                "Move Database Tag",
                "moveTag",
                "Explicitly move an existing tag to the selected branch version.",
            ),
            Self::DeleteTag => (
                "database_delete_tag",
                "Delete Database Tag",
                "deleteTag",
                "Remove a tag. Its old snapshot may become eligible for version cleanup.",
            ),
            Self::Restore => (
                "database_restore",
                "Restore Database Version",
                "restore",
                "Restore the selected historical snapshot as a new committed version of its branch.",
            ),
            Self::Cleanup => (
                "database_cleanup_versions",
                "Cleanup Database Versions",
                "cleanupVersions",
                "Remove old unprotected versions according to an explicit age threshold.",
            ),
            Self::Compare => (
                "database_compare",
                "Compare Database Views",
                "compare",
                "Compare two snapshots by a unique, non-null string or integer key. Return counts, schema changes and bounded row previews.",
            ),
            Self::Clone => (
                "database_clone",
                "Clone Database",
                "clone",
                "Create a shallow clone of this snapshot as another table. Shared source data remains protected while the clone exists.",
            ),
        }
    }
}

fn definition(operation: Operation) -> Node {
    let (id, name, script, description) = operation.description();
    let mut node = Node::new(id, name, description, "Data/Database/Versioning");
    node.set_flowscript_name("db", script);
    node.set_receiver("database");
    node.set_version(1);
    node.add_icon("/flow/icons/database.svg");
    node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
    node.add_input_pin(
        "database",
        "Database",
        "Database connection",
        VariableType::Struct,
    )
    .set_schema::<NodeDBConnection>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_output_pin(
        "exec_out",
        "Done",
        "Operation completed",
        VariableType::Execution,
    );
    add_reference_output(&mut node);
    match operation {
        Operation::Checkout => add_selector_pins(&mut node),
        Operation::Snapshot => {
            node.add_input_pin(
                "name",
                "Retention Tag",
                "Optional unique tag protecting this snapshot from cleanup.",
                VariableType::String,
            )
            .set_default_value(Some(json!("")));
        }
        Operation::CreateBranch
        | Operation::DeleteBranch
        | Operation::CreateTag
        | Operation::UpdateTag
        | Operation::DeleteTag => {
            node.add_input_pin("name", "Name", "Branch or tag name", VariableType::String);
        }
        Operation::Clone => {
            node.add_input_pin(
                "name",
                "Table Name",
                "Name of the new table",
                VariableType::String,
            );
        }
        Operation::Cleanup => {
            node.set_long_running(true);
            node.add_input_pin(
                "older_than_days",
                "Older Than Days",
                "Minimum age of versions eligible for cleanup; must be at least one day.",
                VariableType::Integer,
            )
            .set_default_value(Some(json!(7)));
            node.add_output_pin(
                "stats",
                "Cleanup Statistics",
                "Removed versions and storage",
                VariableType::Struct,
            )
            .set_schema::<DatabaseCleanupStats>();
        }
        Operation::Versions => {
            node.add_output_pin(
                "versions",
                "Versions",
                "Committed branch history",
                VariableType::Struct,
            )
            .set_schema::<DatabaseVersion>()
            .set_value_type(ValueType::Array);
        }
        Operation::Branches => {
            node.add_output_pin(
                "branches",
                "Branches",
                "Table branches",
                VariableType::Struct,
            )
            .set_schema::<DatabaseBranch>()
            .set_value_type(ValueType::Array);
        }
        Operation::Tags => {
            node.add_output_pin("tags", "Tags", "Named snapshots", VariableType::Struct)
                .set_schema::<DatabaseTag>()
                .set_value_type(ValueType::Array);
        }
        Operation::Reference => {
            node.add_output_pin(
                "pending_writes",
                "Pending Writes",
                "Whether writes are queued but not yet committed",
                VariableType::Boolean,
            );
        }
        Operation::Compare => {
            node.set_long_running(true);
            node.add_input_pin(
                "other",
                "Compare With",
                "Target database view",
                VariableType::Struct,
            )
            .set_schema::<NodeDBConnection>();
            node.add_input_pin(
                "key",
                "Key Column",
                "Unique, non-null string or integer application key present in both views",
                VariableType::String,
            )
            .set_default_value(Some(json!("id")));
            node.add_input_pin(
                "limit",
                "Preview Limit",
                "Maximum changed rows returned, from 0 to 1000. Counts cover the full comparison.",
                VariableType::Integer,
            )
            .set_default_value(Some(json!(100)));
            node.add_output_pin(
                "diff",
                "Difference",
                "Added, removed, changed and unchanged counts with row previews",
                VariableType::Struct,
            )
            .set_schema::<DatabaseDiff>();
        }
        _ => {}
    }
    if matches!(
        operation,
        Operation::Checkout
            | Operation::Snapshot
            | Operation::CreateBranch
            | Operation::Restore
            | Operation::Clone
    ) {
        node.add_output_pin(
            "database_out",
            "Database",
            "Independent connection for the resulting reference",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>();
    }
    node
}

#[cfg(feature = "execute")]
async fn execute(operation: Operation, context: &mut ExecutionContext) -> Result<()> {
    context.deactivate_exec_pin("exec_out").await?;
    let source: NodeDBConnection = context.evaluate_pin("database").await?;
    let cached = source.load(context).await?;
    if matches!(
        operation,
        Operation::Snapshot
            | Operation::CreateBranch
            | Operation::CreateTag
            | Operation::UpdateTag
            | Operation::Checkout
            | Operation::Restore
            | Operation::Cleanup
            | Operation::Clone
            | Operation::Compare
    ) {
        cached.ensure_flushed().await?;
    }
    // Flush both handles before locking either store; the same handle is a valid comparison.
    let comparison = if matches!(operation, Operation::Compare) {
        let other: NodeDBConnection = context.evaluate_pin("other").await?;
        let other = other.load(context).await?;
        other.ensure_flushed().await?;
        let store = other.db.read().await.inner().clone();
        Some(store)
    } else {
        None
    };
    let mut output_store = None;
    let reference;
    {
        let guard = cached.db.read().await;
        let store = guard.inner();
        match operation {
            Operation::Reference => {
                context
                    .set_pin_value("pending_writes", json!(guard.is_dirty()))
                    .await?;
            }
            Operation::Versions => {
                context
                    .set_pin_value("versions", json!(store.list_versions().await?))
                    .await?;
            }
            Operation::Branches => {
                context
                    .set_pin_value("branches", json!(store.list_branches().await?))
                    .await?;
            }
            Operation::Tags => {
                context
                    .set_pin_value("tags", json!(store.list_tags().await?))
                    .await?;
            }
            Operation::Checkout => {
                output_store = Some(store.checkout(read_selector(context).await?).await?);
            }
            Operation::Snapshot => {
                let name: String = context.evaluate_pin("name").await?;
                let current = store.reference().await?;
                let snapshot = store
                    .checkout(DatabaseSelector {
                        branch: current.branch,
                        version: Some(current.version),
                        tag: None,
                        read_only: store.selector().read_only,
                    })
                    .await?;
                if !name.trim().is_empty() {
                    snapshot.create_tag(name.trim()).await?;
                }
                output_store = Some(snapshot);
            }
            Operation::CreateBranch => {
                let name: String = context.evaluate_pin("name").await?;
                output_store = Some(store.create_branch(name.trim()).await?);
            }
            Operation::Clone => {
                let name: String = context.evaluate_pin("name").await?;
                output_store = Some(store.clone_table(name.trim()).await?);
            }
            Operation::Compare => {
                let key: String = context.evaluate_pin("key").await?;
                let limit: i64 = context.evaluate_pin("limit").await?;
                if !(0..=1000).contains(&limit) {
                    return Err(flow_like_types::anyhow!(
                        "Preview limit must be from 0 to 1000"
                    ));
                }
                let other = comparison.as_ref().expect("comparison handle loaded");
                context
                    .set_pin_value(
                        "diff",
                        json!(store.compare(other, &key, limit as usize).await?),
                    )
                    .await?;
            }
            Operation::DeleteBranch => {
                let name: String = context.evaluate_pin("name").await?;
                store.delete_branch(name.trim()).await?;
            }
            Operation::CreateTag | Operation::UpdateTag | Operation::DeleteTag => {
                let name: String = context.evaluate_pin("name").await?;
                match operation {
                    Operation::CreateTag => {
                        store.create_tag(name.trim()).await?;
                    }
                    Operation::UpdateTag => {
                        store.update_tag(name.trim()).await?;
                    }
                    _ => {
                        store.delete_tag(name.trim()).await?;
                    }
                }
            }
            Operation::Restore => {
                let restored = store.restore().await?;
                output_store = Some(
                    store
                        .checkout(DatabaseSelector {
                            branch: restored.branch,
                            version: None,
                            tag: None,
                            read_only: false,
                        })
                        .await?,
                );
            }
            Operation::Cleanup => {
                let days: i64 = context.evaluate_pin("older_than_days").await?;
                if days < 1 {
                    return Err(flow_like_types::anyhow!(
                        "Retention must be at least one day"
                    ));
                }
                context
                    .set_pin_value("stats", json!(store.cleanup_versions(days as u64).await?))
                    .await?;
            }
        }
        reference = if let Some(view) = &output_store {
            Some(view.reference().await?)
        } else if matches!(operation, Operation::Reference) {
            optional_reference(store).await?
        } else {
            Some(store.reference().await?)
        };
    }
    if let Some(view) = output_store {
        let handle = cache_view(context, source, view).await?;
        context.set_pin_value("database_out", json!(handle)).await?;
    }
    context.set_pin_value("reference", json!(reference)).await?;
    context.activate_exec_pin("exec_out").await?;
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct CompareDatabaseViewsNode {}

#[async_trait]
impl NodeLogic for CompareDatabaseViewsNode {
    fn get_node(&self) -> Node {
        definition(Operation::Compare)
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::Compare, context).await
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CloneDatabaseNode {}

#[async_trait]
impl NodeLogic for CloneDatabaseNode {
    fn get_node(&self) -> Node {
        definition(Operation::Clone)
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::Clone, context).await
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetDatabaseReferenceNode {}

#[async_trait]
impl NodeLogic for GetDatabaseReferenceNode {
    fn get_node(&self) -> Node {
        definition(Operation::Reference)
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::Reference, context).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ListDatabaseVersionsNode {}

#[async_trait]
impl NodeLogic for ListDatabaseVersionsNode {
    fn get_node(&self) -> Node {
        definition(Operation::Versions)
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::Versions, context).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ListDatabaseBranchesNode {}

#[async_trait]
impl NodeLogic for ListDatabaseBranchesNode {
    fn get_node(&self) -> Node {
        definition(Operation::Branches)
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::Branches, context).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ListDatabaseTagsNode {}

#[async_trait]
impl NodeLogic for ListDatabaseTagsNode {
    fn get_node(&self) -> Node {
        definition(Operation::Tags)
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::Tags, context).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CheckoutDatabaseNode {}

#[async_trait]
impl NodeLogic for CheckoutDatabaseNode {
    fn get_node(&self) -> Node {
        definition(Operation::Checkout)
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::Checkout, context).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct SnapshotDatabaseNode {}

#[async_trait]
impl NodeLogic for SnapshotDatabaseNode {
    fn get_node(&self) -> Node {
        definition(Operation::Snapshot)
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::Snapshot, context).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateDatabaseBranchNode {}

#[async_trait]
impl NodeLogic for CreateDatabaseBranchNode {
    fn get_node(&self) -> Node {
        definition(Operation::CreateBranch)
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::CreateBranch, context).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DeleteDatabaseBranchNode {}

#[async_trait]
impl NodeLogic for DeleteDatabaseBranchNode {
    fn get_node(&self) -> Node {
        definition(Operation::DeleteBranch)
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::DeleteBranch, context).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateDatabaseTagNode {}

#[async_trait]
impl NodeLogic for CreateDatabaseTagNode {
    fn get_node(&self) -> Node {
        definition(Operation::CreateTag)
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::CreateTag, context).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MoveDatabaseTagNode {}

#[async_trait]
impl NodeLogic for MoveDatabaseTagNode {
    fn get_node(&self) -> Node {
        definition(Operation::UpdateTag)
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::UpdateTag, context).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DeleteDatabaseTagNode {}

#[async_trait]
impl NodeLogic for DeleteDatabaseTagNode {
    fn get_node(&self) -> Node {
        definition(Operation::DeleteTag)
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::DeleteTag, context).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RestoreDatabaseVersionNode {}

#[async_trait]
impl NodeLogic for RestoreDatabaseVersionNode {
    fn get_node(&self) -> Node {
        definition(Operation::Restore)
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::Restore, context).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CleanupDatabaseVersionsNode {}

#[async_trait]
impl NodeLogic for CleanupDatabaseVersionsNode {
    fn get_node(&self) -> Node {
        definition(Operation::Cleanup)
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        execute(Operation::Cleanup, context).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "Database execution requires the execute feature"
        ))
    }
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use flow_like_storage::databases::vector::lancedb::{
        LocalWriteReceipt, LogicalTableMutation, LogicalTableMutationAdapter,
    };

    struct UnreachableAdapter;

    #[async_trait]
    impl LogicalTableMutationAdapter for UnreachableAdapter {
        async fn apply(&self, _mutation: LogicalTableMutation) -> Result<LocalWriteReceipt> {
            Err(flow_like_types::anyhow!("the adapter must not be written"))
        }

        async fn read_table(&self) -> Result<Option<flow_like_storage::lancedb::Table>> {
            Err(flow_like_types::anyhow!("the adapter must not be read"))
        }
    }

    #[tokio::test]
    async fn managed_store_reference_is_none() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store =
            LanceDBVectorStore::new(directory.path().to_path_buf(), "items".into()).await?;
        store.insert(vec![json!({ "id": 1 })]).await?;
        assert!(optional_reference(&store).await?.is_some());

        let managed = store.with_mutation_adapter(Arc::new(UnreachableAdapter));
        assert!(optional_reference(&managed).await?.is_none());
        Ok(())
    }
}

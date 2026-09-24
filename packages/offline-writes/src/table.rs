use crate::{
    BufferedTable,
    fs::unix_time,
    limits::validate_request,
    manager::{RefreshOutcome, TableActivation, TableSetup, WriteManager},
};
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::{
    OfflineExpected, OfflineMutation, OfflineReplayRequest, OfflineResource, format_limit,
};
use flow_like_storage::{
    arrow_array::{Array, Int64Array},
    arrow_utils::record_batch_to_value,
    databases::{
        df_provider::zero_column_safe,
        vector::{
            lancedb::{LocalWriteReceipt, LogicalTableMutation, LogicalTableMutationAdapter},
            offline_replay::{self, ReplayMarker, ReplayMutation, ReplayOutcome},
        },
    },
    datafusion::{
        execution::{
            disk_manager::{DiskManagerBuilder, DiskManagerMode},
            memory_pool::FairSpillPool,
            runtime_env::RuntimeEnvBuilder,
        },
        prelude::{SessionConfig, SessionContext},
    },
    lancedb::{
        self, Connection, Table,
        query::{ExecutableQuery, QueryBase, Select},
        table::datafusion::BaseTableAdapter,
    },
};
use futures_util::StreamExt;
use serde_json::Value;
use std::{
    collections::HashSet,
    path::Path,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
    },
};

pub(crate) struct TableOverlay {
    pub(crate) manager: Weak<WriteManager>,
    pub(crate) key: String,
    pub(crate) resource: OfflineResource,
    pub(crate) selection: BufferedTable,
    /// "apps/p/storage/db"
    pub(crate) database_path: String,
    pub(crate) local_name: String,
    pub(crate) generation: AtomicU64,
    pub(crate) last_refresh: AtomicI64,
    last_probe: AtomicI64,
    active: AtomicBool,
    removed: AtomicBool,
    remote_missing: AtomicBool,
}

pub async fn optional_table(connection: &Connection, name: &str) -> Result<Option<Table>> {
    match connection.open_table(name).execute().await {
        Ok(table) => Ok(Some(table)),
        Err(lancedb::Error::TableNotFound { .. }) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn sql_identifier(name: &str) -> String {
    // Lance treats double quotes as string literals, including projections.
    format!("`{}`", name.replace('`', "``"))
}
fn key_literal(value: &Value) -> Result<String> {
    match value {
        Value::String(value) if value.len() <= 4096 => {
            Ok(format!("'{}'", value.replace('\'', "''")))
        }
        Value::Number(value) if value.is_i64() || value.is_u64() => Ok(value.to_string()),
        _ => anyhow::bail!("Offline primary keys must be non-null strings or integers"),
    }
}
fn key_filter(column: &str, value: &Value) -> Result<String> {
    Ok(format!(
        "{} = {}",
        sql_identifier(column),
        key_literal(value)?
    ))
}

struct MutationSize {
    written: usize,
    limit: usize,
}
impl std::io::Write for MutationSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.written = self.written.saturating_add(bytes.len());
        if self.written > self.limit {
            return Err(std::io::Error::other(format!(
                "Offline mutation exceeds {}; split the logical batch",
                format_limit(self.limit)
            )));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(crate) async fn rows(
    table: &Table,
    filter: &str,
    select: Select,
    limit: usize,
) -> Result<Vec<Value>> {
    let mut stream = table
        .query()
        .only_if(filter)
        .select(select)
        .execute()
        .await?;
    let mut result = Vec::new();
    let mut bytes = 0usize;
    while let Some(batch) = stream.next().await {
        let batch = batch?;
        ensure!(
            batch.get_array_memory_size() <= limit,
            "Offline mutation batch exceeds {}; use a narrower predicate",
            format_limit(limit)
        );
        for row in record_batch_to_value(&batch)? {
            bytes = bytes.saturating_add(serde_json::to_vec(&row)?.len());
            ensure!(
                bytes <= limit,
                "Offline mutation exceeds {}; split the logical operation",
                format_limit(limit)
            );
            result.push(row);
        }
    }
    Ok(result)
}

impl TableOverlay {
    pub(crate) fn new(
        manager: Weak<WriteManager>,
        key: String,
        resource: OfflineResource,
        selection: BufferedTable,
        database_path: String,
        activation: TableActivation,
    ) -> Self {
        let local_name = format!("table_{}", blake3::hash(key.as_bytes()).to_hex());
        Self {
            manager,
            key,
            resource,
            selection,
            database_path,
            local_name,
            generation: AtomicU64::new(0),
            last_refresh: AtomicI64::new(0),
            last_probe: AtomicI64::new(0),
            active: AtomicBool::new(activation == TableActivation::Active),
            removed: AtomicBool::new(false),
            remote_missing: AtomicBool::new(false),
        }
    }
    pub(crate) fn activation(&self) -> TableActivation {
        if self.active.load(Ordering::Acquire) {
            TableActivation::Active
        } else {
            TableActivation::Held
        }
    }
    pub(crate) fn activate(&self) {
        self.active.store(true, Ordering::Release);
    }
    pub(crate) fn remove(&self) {
        self.removed.store(true, Ordering::Release);
    }
    pub(crate) fn remote_missing(&self) -> bool {
        self.remote_missing.load(Ordering::Acquire)
    }
    /// Fails for removed, cloud-deleted and held tables.
    pub(crate) fn usable(&self) -> Result<()> {
        let table = &self.selection.table;
        ensure!(
            !self.removed.load(Ordering::Acquire),
            "Offline access for table '{table}' was turned off while this run was using it. Start the run again."
        );
        ensure!(
            !self.remote_missing(),
            "Table '{table}' was deleted in the cloud. Turn off offline access for it on this device; its queued changes cannot be applied."
        );
        ensure!(
            self.activation() == TableActivation::Active,
            "Table '{table}' is being prepared for offline use on this device. Try again when the flows of this project that were already running have finished and the hub is reachable."
        );
        Ok(())
    }
    fn upgrade(&self) -> Result<Arc<WriteManager>> {
        self.manager
            .upgrade()
            .context("Offline write manager stopped")
    }
    /// The idle refresh of the drain loop: throttled by `refresh_interval`, skipped while
    /// the lane has queued changes.
    pub(crate) async fn refresh_idle(&self, manager: &WriteManager) -> Result<()> {
        manager.authorize()?;
        let now = unix_time()?;
        if now.saturating_sub(self.last_refresh.load(Ordering::Acquire))
            < manager.refresh_interval.as_secs() as i64
            || manager.queue.has_pending(&self.key)?
        {
            return Ok(());
        }
        self.refresh(manager).await.map(|_| ())
    }
    /// Switches the local snapshot to the cloud's current version when it changed.
    pub(crate) async fn refresh(&self, manager: &WriteManager) -> Result<RefreshOutcome> {
        manager.authorize()?;
        if manager.queue.has_pending(&self.key)? {
            return Ok(RefreshOutcome::Deferred);
        }
        let now = unix_time()?;
        self.last_refresh.store(now, Ordering::Release);
        let previous = manager
            .queue
            .resource_revision(&self.key)?
            .context("Missing offline table revision")?;
        let remote = manager.host.remote_table(&self.selection).await?;
        let Some(remote) = remote else {
            return self.remote_absent(manager, &previous).await;
        };
        let (version, fingerprint) = offline_replay::revision(&remote).await?;
        let expected = OfflineExpected::TableVersion {
            version,
            fingerprint: Some(fingerprint),
        };
        if manager.queue.resource_revision(&self.key)? == Some(serde_json::to_value(&expected)?) {
            self.confirm_remote(manager, now)?;
            return Ok(RefreshOutcome::Unchanged);
        }
        let new_name = format!("snapshot_{}", uuid::Uuid::new_v4().simple());
        let local = manager.local()?;
        let snapshot = offline_replay::materialize(
            &remote,
            &local,
            &new_name,
            manager.max_mirror_bytes.load(Ordering::Acquire),
        )
        .await?;
        let _guard = manager.gate.lock().await;
        manager.authorize()?;
        let (old_name, _, _) = self.local_view(manager)?;
        if manager.queue.has_pending(&self.key)?
            || !manager.queue.checkpoint(
                &self.key,
                &new_name,
                Some(snapshot.table.version().await?),
                &serde_json::to_value(OfflineExpected::TableVersion {
                    version: snapshot.source_version,
                    fingerprint: Some(snapshot.source_fingerprint),
                })?,
                &previous,
            )?
        {
            local.drop_table(&new_name, &[]).await?;
            return Ok(RefreshOutcome::Deferred);
        }
        manager.retire(&old_name)?;
        self.generation.fetch_add(1, Ordering::Release);
        self.confirm_remote(manager, now)?;
        Ok(RefreshOutcome::Refreshed {
            version: snapshot.source_version,
            downloaded_bytes: 0,
        })
    }
    fn confirm_remote(&self, manager: &WriteManager, now: i64) -> Result<()> {
        manager.queue.mirror_error(&self.key, None)?;
        manager.queue.record_refresh(&self.key, now)?;
        if self.remote_missing.swap(false, Ordering::AcqRel) {
            manager.queue.set_remote_missing(&self.key, false)?;
        }
        Ok(())
    }
    async fn remote_absent(
        &self,
        manager: &WriteManager,
        previous: &Value,
    ) -> Result<RefreshOutcome> {
        if !manager.recreate_dropped_tables {
            manager.queue.set_remote_missing(&self.key, true)?;
            self.remote_missing.store(true, Ordering::Release);
            manager.queue.mirror_error(
                &self.key,
                Some(&format!(
                    "Table '{}' was deleted in the cloud. Turn off offline access for it on this device; its queued changes cannot be applied.",
                    self.selection.table
                )),
            )?;
            return Ok(RefreshOutcome::RemoteMissing);
        }
        let _guard = manager.gate.lock().await;
        manager.authorize()?;
        let absent = serde_json::to_value(OfflineExpected::TableVersion {
            version: 0,
            fingerprint: None,
        })?;
        let (old_name, _, _) = self.local_view(manager)?;
        if *previous != absent
            && manager.queue.checkpoint(
                &self.key,
                &format!("absent_{}", uuid::Uuid::new_v4().simple()),
                None,
                &absent,
                previous,
            )?
        {
            manager.retire(&old_name)?;
            self.generation.fetch_add(1, Ordering::Release);
        }
        manager.queue.mirror_error(&self.key, None)?;
        manager.queue.record_refresh(&self.key, unix_time()?)?;
        Ok(RefreshOutcome::RemoteMissing)
    }
    /// Probes the cloud revision of an idle table before a write and refreshes when it
    /// changed, so the write is frozen against the current cloud version. Failures and
    /// timeouts keep the current base.
    async fn fast_forward(&self, manager: &WriteManager) {
        let Some(fast_forward) = manager.fast_forward else {
            return;
        };
        if manager
            .connectivity
            .as_ref()
            .is_some_and(|connectivity| connectivity.is_offline())
            || !matches!(manager.queue.has_pending(&self.key), Ok(false))
        {
            return;
        }
        let Ok(now) = unix_time() else { return };
        let last = self.last_probe.load(Ordering::Acquire);
        if now.saturating_sub(last) < fast_forward.min_interval.as_secs() as i64
            || self
                .last_probe
                .compare_exchange(last, now, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return;
        }
        let probe = async {
            let remote = manager.host.remote_table(&self.selection).await?;
            let Some(remote) = remote else {
                return Ok::<_, anyhow::Error>(None);
            };
            let (version, fingerprint) = offline_replay::revision(&remote).await?;
            Ok(Some(serde_json::to_value(OfflineExpected::TableVersion {
                version,
                fingerprint: Some(fingerprint),
            })?))
        };
        let changed = match tokio::time::timeout(fast_forward.probe_timeout, probe).await {
            Ok(Ok(Some(revision))) => {
                manager.queue.resource_revision(&self.key).ok().flatten() != Some(revision)
            }
            _ => false,
        };
        if changed && let Err(error) = self.refresh(manager).await {
            tracing::debug!(%error, "Fast-forward refresh failed; freezing against the current base");
        }
    }
    fn local_view(&self, manager: &WriteManager) -> Result<(String, String, Option<u64>)> {
        let (name, branch, base) = manager.queue.local_view(&self.key)?;
        Ok((
            name.unwrap_or_else(|| self.local_name.clone()),
            branch,
            base,
        ))
    }
    pub(crate) async fn local_table(&self, manager: &WriteManager) -> Result<Option<Table>> {
        let (name, branch, _) = self.local_view(manager)?;
        match manager
            .local()?
            .open_table(&name)
            .branch(&branch)
            .execute()
            .await
        {
            Ok(table) => Ok(Some(table)),
            Err(lancedb::Error::TableNotFound { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    pub(crate) async fn skip(&self, manager: &WriteManager, id: &str, reason: &str) -> Result<()> {
        let (name, branch, base) = self.local_view(manager)?;
        if let Some(base) = base {
            let table = self
                .local_table(manager)
                .await?
                .context("Local baseline is missing")?;
            let new_branch = format!("skip_{}", uuid::Uuid::new_v4().simple());
            let baseline = table
                .create_branch(&new_branch, (branch.as_str(), base))
                .await?;
            manager.queue.complete_skip(
                id,
                reason,
                Some(&name),
                &new_branch,
                Some(baseline.version().await?),
            )?;
        } else {
            let name = format!("table_{}", uuid::Uuid::new_v4().simple());
            manager
                .queue
                .complete_skip(id, reason, Some(&name), "main", None)?;
        }
        self.generation.fetch_add(1, Ordering::Release);
        self.recover_local(manager).await
    }
    /// An existing baseline needs no network. An absent one is materialized from the cloud
    /// and checked before anything is recorded; a failure drops the snapshot.
    pub(crate) async fn initialize(&self, manager: &WriteManager, setup: TableSetup) -> Result<()> {
        if manager.queue.resource_revision(&self.key)?.is_some() {
            self.remote_missing
                .store(manager.queue.remote_missing(&self.key)?, Ordering::Release);
            if let Some(head) = manager.queue.local_head(&self.key)? {
                let table = self.local_table(manager).await?.context(
                    "Accepted offline table snapshot is missing; restore its private data before running this placement",
                )?;
                // Startup holds the exclusive scope lock and has not exported readers.
                // Pending work may still need the older acknowledged rollback version.
                if !manager.queue.has_pending(&self.key)?
                    && self.local_view(manager)?.2 == Some(head)
                    && table.version().await? == head
                {
                    if let Err(error) = offline_replay::compact_idle_local(&table).await {
                        tracing::warn!(%error, "Could not prune acknowledged local table history");
                    }
                }
            }
            return Ok(());
        }
        let remote = manager.host.remote_table(&self.selection).await?;
        match remote {
            Some(remote) => {
                let name = format!("snapshot_{}", uuid::Uuid::new_v4().simple());
                let local = manager.local()?;
                let snapshot = offline_replay::materialize(
                    &remote,
                    &local,
                    &name,
                    manager.max_mirror_bytes.load(Ordering::Acquire),
                )
                .await?;
                let recorded = async {
                    if setup.validate_key {
                        validate_key(
                            remote,
                            snapshot.source_version,
                            &self.selection.table,
                            &self.selection.primary_key,
                            &manager.spill_directory(),
                            manager.key_validation_memory(),
                        )
                        .await?;
                    }
                    let expected = OfflineExpected::TableVersion {
                        version: snapshot.source_version,
                        fingerprint: Some(snapshot.source_fingerprint.clone()),
                    };
                    manager.queue.initialize_table(
                        &self.key,
                        &serde_json::to_value(expected)?,
                        snapshot.table.version().await?,
                        &name,
                    )?;
                    manager.queue.record_refresh(&self.key, unix_time()?)
                }
                .await;
                if let Err(error) = recorded {
                    if let Err(drop_error) = local.drop_table(&name, &[]).await {
                        tracing::warn!(%drop_error, "Could not drop a rejected offline snapshot");
                    }
                    return Err(error);
                }
            }
            None if manager.recreate_dropped_tables => manager.queue.initialize_resource(
                &self.key,
                &serde_json::to_value(OfflineExpected::TableVersion {
                    version: 0,
                    fingerprint: None,
                })?,
                None,
            )?,
            None => anyhow::bail!(
                "Table '{}' does not exist in the cloud",
                self.selection.table
            ),
        }
        Ok(())
    }
    pub(crate) async fn recover_local(&self, manager: &WriteManager) -> Result<()> {
        let result = self.recover_local_inner(manager).await;
        if result.is_err()
            && let Ok(Some(operation)) = manager.queue.next_unmaterialized(&self.key)
        {
            // Keep detailed provider/schema diagnostics local. Management
            // needs the blocked operation and a safe recovery instruction.
            if manager
                .queue
                .block(
                    &operation.operation_id,
                    "blocked",
                    "The local table view could not be updated. Check the device disk budget and table schema, then retry or explicitly skip this retained operation.",
                )
                .is_ok()
            {
                manager.host.queue_changed();
            }
        }
        result
    }
    async fn recover_local_inner(&self, manager: &WriteManager) -> Result<()> {
        while let Some(operation) = manager.queue.next_unmaterialized(&self.key)? {
            let request: OfflineReplayRequest = serde_json::from_value(operation.payload.clone())?;
            let table = self.local_table(manager).await?;
            let expected = match manager.queue.local_expected(&operation.operation_id)? {
                Some(expected) => serde_json::from_value::<OfflineExpected>(expected)?,
                None => {
                    let expected = match &table {
                        Some(table) => {
                            let (version, fingerprint) = offline_replay::revision(table).await?;
                            OfflineExpected::TableVersion {
                                version,
                                fingerprint: Some(fingerprint),
                            }
                        }
                        None => OfflineExpected::TableVersion {
                            version: 0,
                            fingerprint: None,
                        },
                    };
                    manager.queue.prepare_local(
                        &operation.operation_id,
                        &serde_json::to_value(&expected)?,
                    )?;
                    expected
                }
            };
            let OfflineExpected::TableVersion {
                version,
                fingerprint,
            } = expected
            else {
                anyhow::bail!("Invalid local table precondition")
            };
            let marker = ReplayMarker {
                operation_id: operation.operation_id.clone(),
                digest: request.digest()?,
                expected_version: version,
                expected_fingerprint: fingerprint,
            };
            let mutation = replay_mutation(request.mutation)?;
            let outcome = match table {
                Some(table) => offline_replay::replay(&table, &marker, mutation).await?,
                None => {
                    let items = match mutation {
                        ReplayMutation::Insert { items } | ReplayMutation::Upsert { items, .. } => {
                            items
                        }
                        _ => anyhow::bail!("Cannot update an absent local table"),
                    };
                    let (name, _, _) = self.local_view(manager)?;
                    offline_replay::create(&manager.local()?, &name, &marker, items).await?
                }
            };
            match outcome {
                ReplayOutcome::Applied { version, .. } => {
                    manager.queue.mark_local(&operation.operation_id, version)?;
                    self.generation.fetch_add(1, Ordering::Release);
                }
                other => {
                    anyhow::bail!("Local offline table could not be materialized: {other:?}");
                }
            }
        }
        Ok(())
    }
    async fn freeze(
        &self,
        manager: &WriteManager,
        table: Option<&Table>,
        mutation: LogicalTableMutation,
    ) -> Result<(OfflineMutation, Option<String>)> {
        let key = &self.selection.primary_key;
        let limit = manager.replay_limits.get().max_operation_bytes;
        let (mutation, values) = match mutation {
            LogicalTableMutation::Insert { items } => (
                OfflineMutation::TableInsert {
                    rows: items.clone(),
                },
                items,
            ),
            LogicalTableMutation::Upsert { items, id_field } => {
                ensure!(
                    &id_field == key,
                    "Offline upsert must use the configured primary key"
                );
                (
                    OfflineMutation::TableUpsert {
                        id_field,
                        rows: items.clone(),
                    },
                    items,
                )
            }
            LogicalTableMutation::Update { filter, updates } => {
                let table = table.context("Cannot update an absent offline table")?;
                ensure!(
                    !updates.iter().any(|(column, _)| column == key),
                    "Offline updates cannot change a primary key; insert and delete explicitly"
                );
                let schema = table.schema().await?;
                for (column, _) in &updates {
                    ensure!(
                        schema.field_with_name(column).is_ok(),
                        "Update column is absent from the local table"
                    );
                }
                let columns = schema
                    .fields()
                    .iter()
                    .map(|field| {
                        (
                            field.name().clone(),
                            updates
                                .iter()
                                .find(|(column, _)| column == field.name())
                                .map(|(_, expression)| expression.clone())
                                .unwrap_or_else(|| sql_identifier(field.name())),
                        )
                    })
                    .collect();
                let values = rows(table, &filter, Select::Dynamic(columns), limit).await?;
                (
                    OfflineMutation::TableUpsert {
                        id_field: key.clone(),
                        rows: values.clone(),
                    },
                    values,
                )
            }
            LogicalTableMutation::Delete { filter } => {
                let table = table.context("Cannot delete from an absent offline table")?;
                let values =
                    rows(table, &filter, Select::Columns(vec![key.clone()]), limit).await?;
                let predicates = values
                    .iter()
                    .map(|row| {
                        key_filter(key, row.get(key).context("Offline primary key is missing")?)
                    })
                    .collect::<Result<Vec<_>>>()?;
                let filter = if predicates.is_empty() {
                    "false".into()
                } else {
                    predicates.join(" OR ")
                };
                (OfflineMutation::TableDelete { filter }, values)
            }
        };
        let mut unique = HashSet::new();
        for row in &values {
            let value = row
                .get(key)
                .context("Offline mutation requires a complete primary key")?;
            key_literal(value)?;
            ensure!(
                unique.insert(serde_json::to_string(value)?),
                "Offline mutation contains duplicate primary keys"
            );
        }
        if let Some(table) = table.filter(|_| !values.is_empty()) {
            let literals = values
                .iter()
                .map(|row| key_literal(&row[key]))
                .collect::<Result<Vec<_>>>()?;
            let filter = format!("{} IN ({})", sql_identifier(key), literals.join(","));
            let existing = rows(table, &filter, Select::Columns(vec![key.clone()]), limit).await?;
            if matches!(mutation, OfflineMutation::TableInsert { .. }) {
                ensure!(
                    existing.is_empty(),
                    "Insert conflicts with an existing local primary key"
                );
            }
            let mut existing_keys = HashSet::new();
            for row in existing {
                ensure!(
                    existing_keys.insert(serde_json::to_string(&row[key])?),
                    "Offline table contains duplicate primary keys"
                );
            }
        }
        let coalesce =
            if values.len() == 1 && !matches!(mutation, OfflineMutation::TableInsert { .. }) {
                Some(format!("{key}:{}", serde_json::to_string(&values[0][key])?))
            } else {
                None
            };
        if let OfflineMutation::TableInsert { rows } | OfflineMutation::TableUpsert { rows, .. } =
            &mutation
        {
            if !rows.is_empty() {
                let fields = match table {
                    Some(table) => Some(table.schema().await?.fields().to_vec()),
                    None => None,
                };
                flow_like_storage::arrow_utils::value_to_batch_reader_with_fields(
                    rows.clone(),
                    fields,
                )
                .context("Offline rows do not match the table schema")?;
            }
        }
        Ok((mutation, coalesce))
    }
}

/// No nulls and no duplicates in `key` at `version` of the cloud table. DataFusion reads
/// only the key column with a bounded pool that spills to `spill`.
pub(crate) async fn validate_key(
    remote: Table,
    version: u64,
    table: &str,
    key: &str,
    spill: &Path,
    memory: usize,
) -> Result<()> {
    let column = format!("\"{}\"", key.replace('"', "\"\""));
    let checked = async {
        remote.checkout(version).await?;
        let provider = zero_column_safe(Arc::new(
            BaseTableAdapter::try_new(remote.base_table().clone()).await?,
        ));
        let runtime = RuntimeEnvBuilder::new()
            .with_memory_pool(Arc::new(FairSpillPool::new(memory)))
            .with_disk_manager_builder(
                DiskManagerBuilder::default()
                    .with_mode(DiskManagerMode::Directories(vec![spill.to_path_buf()])),
            )
            .build_arc()?;
        let context = SessionContext::new_with_config_rt(
            SessionConfig::new().with_target_partitions(2),
            runtime,
        );
        context.register_table("offline_key_check", provider)?;
        let mut invalid = 0i64;
        for query in [
            format!("SELECT COUNT(*) FROM offline_key_check WHERE {column} IS NULL"),
            format!(
                "SELECT COALESCE(SUM(c), 0) FROM (SELECT COUNT(*) AS c FROM offline_key_check WHERE {column} IS NOT NULL GROUP BY {column} HAVING COUNT(*) > 1) AS duplicated"
            ),
        ] {
            for batch in context.sql(&query).await?.collect().await? {
                let values = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .context("Key check returned an unexpected type")?;
                if !values.is_empty() && !values.is_null(0) {
                    invalid = invalid.saturating_add(values.value(0));
                }
            }
        }
        Ok::<_, anyhow::Error>(invalid)
    };
    match checked.await {
        Ok(0) => Ok(()),
        Ok(invalid) => anyhow::bail!(
            "Column '{key}' is not a unique, non-empty key in '{table}': {invalid} rows are empty or duplicated."
        ),
        Err(error) => anyhow::bail!(
            "Could not check the key column '{key}' of '{table}' on this device: {error}"
        ),
    }
}

pub(crate) fn replay_mutation(mutation: OfflineMutation) -> Result<ReplayMutation> {
    Ok(match mutation {
        OfflineMutation::TableInsert { rows } => ReplayMutation::Insert { items: rows },
        OfflineMutation::TableUpsert { id_field, rows } => ReplayMutation::Upsert {
            id_field,
            items: rows,
        },
        OfflineMutation::TableUpdate { filter, updates } => ReplayMutation::Update {
            filter,
            updates: updates.into_iter().collect(),
        },
        OfflineMutation::TableDelete { filter } => ReplayMutation::Delete { filter },
        _ => anyhow::bail!("File mutation in offline table queue"),
    })
}

#[async_trait::async_trait]
impl LogicalTableMutationAdapter for TableOverlay {
    async fn apply(&self, mutation: LogicalTableMutation) -> Result<LocalWriteReceipt> {
        let manager = self.upgrade()?;
        self.usable()?;
        serde_json::to_writer(
            &mut MutationSize {
                written: 0,
                limit: manager.replay_limits.get().max_operation_bytes,
            },
            &mutation,
        )
        .context("Offline mutation is too large")?;
        self.fast_forward(&manager).await;
        let _guard = manager.gate.lock().await;
        self.usable()?;
        manager.authorize()?;
        self.recover_local(&manager).await?;
        let table = self.local_table(&manager).await?;
        let (mutation, key) = self.freeze(&manager, table.as_ref(), mutation).await?;
        if matches!(&mutation,OfflineMutation::TableInsert {rows} | OfflineMutation::TableUpsert {rows,..} if rows.is_empty())
            || matches!(&mutation,OfflineMutation::TableDelete {filter} if filter == "false")
        {
            return Ok(LocalWriteReceipt {
                operation_id: String::new(),
                sequence: 0,
                state: "unchanged".into(),
            });
        }
        let expected: OfflineExpected = serde_json::from_value(
            manager
                .queue
                .resource_revision(&self.key)?
                .context("Offline table was not initialized")?,
        )?;
        // Retain the schema-bearing creation before later replacements or
        // deletes. A delete cannot create the cloud table at version zero.
        let key = if matches!(&expected, OfflineExpected::TableVersion { version: 0, .. }) {
            None
        } else {
            key
        };
        let request = OfflineReplayRequest {
            operation_id: uuid::Uuid::new_v4().to_string(),
            resource: self.resource.clone(),
            expected,
            mutation,
        };
        validate_request(&request, &manager.replay_limits.get())?;
        let operation = manager.queue.enqueue(
            &self.key,
            serde_json::to_value(request)?,
            key.as_deref(),
            unix_time()?,
        )?;
        self.recover_local(&manager).await.with_context(||format!("Offline write {} was retained in the queue but its local table view could not be updated; inspect the queue before retrying",operation.operation_id))?;
        manager.wake.notify_one();
        manager.host.queue_changed();
        Ok(LocalWriteReceipt {
            operation_id: operation.operation_id,
            sequence: operation.sequence,
            state: "pending".into(),
        })
    }
    async fn read_table(&self) -> Result<Option<Table>> {
        let manager = self.upgrade()?;
        self.usable()?;
        let _guard = manager.gate.lock().await;
        manager.authorize()?;
        self.recover_local(&manager).await?;
        self.local_table(&manager).await
    }
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
}

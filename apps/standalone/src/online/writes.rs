use super::{ProjectCredentials, WorkloadIdentity};
use crate::{
    config::PlacementConfig,
    outbox::{BufferedTable, BufferingConfig, Outbox},
};
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::{
    OfflineExpected, OfflineMutation, OfflineReplayRequest, OfflineReplayResponse,
    OfflineReplayStatus, OfflineResource, OnlineProjectAccess,
};
use flow_like_runtime::state::FlowLikeConfig;
use flow_like_storage::{
    arrow_utils::record_batch_to_value,
    databases::vector::{
        lancedb::{LocalWriteReceipt, LogicalTableMutation, LogicalTableMutationAdapter},
        offline_replay::{self, ReplayMarker, ReplayMutation, ReplayOutcome},
    },
    lance_io::object_store::ObjectStoreRegistry,
    lancedb::{
        self, Connection, Table,
        query::{ExecutableQuery, QueryBase, Select},
    },
    object_store::path::Path as ObjectPath,
};
use futures_util::StreamExt;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashSet},
    fs::File,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, Notify};

pub(super) mod files;

pub(super) struct WriteManager {
    queue: Arc<Outbox>,
    credentials: Arc<ProjectCredentials>,
    local: Connection,
    tables: Mutex<BTreeMap<String, Arc<TableOverlay>>>,
    gate: Arc<Mutex<()>>,
    wake: Arc<Notify>,
    _lock: File,
    max_mirror_bytes: u64,
}

pub(super) async fn configure(
    config: &PlacementConfig,
    _identity: &WorkloadIdentity,
    buffering: &BufferingConfig,
    credentials: Arc<ProjectCredentials>,
    runtime: &mut FlowLikeConfig,
    local_data: &Path,
    registry: Arc<ObjectStoreRegistry>,
) -> Result<Arc<WriteManager>> {
    buffering.validate()?;
    ensure!(
        credentials.access == OnlineProjectAccess::ReadWrite,
        "Offline buffering requires a writable project grant"
    );
    let scope = credentials.scope.clone();
    let queue = Arc::new(Outbox::open(
        local_data,
        &config.id,
        &scope,
        buffering.clone(),
    )?);
    let lock = queue.acquire_writer()?;
    queue.quarantine_other_scopes()?;
    let tables_root = queue.root().join("tables");
    crate::outbox::private_directory(&tables_root)?;
    clean_orphaned_tables(&tables_root, &queue.retained_table_names()?)?;
    let local =
        offline_replay::budgeted_local_connection(&tables_root, buffering.max_mirror_bytes).await?;
    let manager = Arc::new(WriteManager {
        queue,
        credentials,
        local,
        tables: Mutex::new(BTreeMap::new()),
        gate: Arc::new(Mutex::new(())),
        wake: Arc::new(Notify::new()),
        _lock: lock,
        max_mirror_bytes: buffering.max_mirror_bytes,
    });
    let session = Arc::new(flow_like_storage::lance::session::Session::new(
        flow_like_storage::lance::dataset::DEFAULT_INDEX_CACHE_SIZE,
        flow_like_storage::lance::dataset::DEFAULT_METADATA_CACHE_SIZE,
        registry,
    ));
    let mut configured = BTreeMap::new();
    for selected in &buffering.tables {
        let location = manager
            .credentials
            .locations
            .get(&selected.purpose)
            .context("Missing buffered database scope")?;
        let path = ObjectPath::parse(format!("{}{}", location.prefix, selected.database))?;
        let connection = lancedb::connect(&format!("{}{}", location.uri, selected.database))
            .namespace_client_property("manifest_enabled", "false")
            .session(session.clone())
            .execute()
            .await?;
        let resource = OfflineResource::Table {
            purpose: selected.purpose,
            database: selected.database.clone(),
            table: selected.table.clone(),
        };
        let key = serde_json::to_string(&resource)?;
        let local_name = format!("table_{}", blake3::hash(key.as_bytes()).to_hex());
        let overlay = Arc::new(TableOverlay {
            manager: Arc::downgrade(&manager),
            key: key.clone(),
            resource,
            selection: selected.clone(),
            remote: connection,
            local_name,
            generation: AtomicU64::new(0),
            last_refresh: AtomicI64::new(0),
        });
        overlay.initialize(&manager).await?;
        configured.insert((path.to_string(), selected.table.clone()), overlay.clone());
        manager.tables.lock().await.insert(key, overlay);
    }
    let selected = Arc::new(configured);
    let predicate = selected.clone();
    runtime.register_database_table_is_managed(Arc::new(move |path, table| {
        predicate.contains_key(&(path.to_string(), table.to_owned()))
    }));
    let decorated = selected.clone();
    runtime.register_database_decorator(Arc::new(move |path,store| {
        let overlay = decorated.get(&(path.to_string(),store.table_name().to_owned())).cloned();
        Box::pin(async move {
            match overlay {
                Some(overlay) => {
                    let selector = store.selector();
                    ensure!(selector.branch == "main" && selector.version.is_none() && selector.tag.is_none(), "Buffered tables expose the current local view; historical and branch selectors require a cloud-only placement");
                    Ok(store.with_mutation_adapter(overlay))
                },
                None => Ok(store),
            }
        })
    }));
    let inventory = selected;
    let locations = manager.credentials.locations.clone();
    runtime.register_database_table_names(Arc::new(move |path| {
        let inventory = inventory.clone(); let locations = locations.clone(); let session = session.clone();
        Box::pin(async move {
            let location = locations.values().find(|location| path.as_ref().starts_with(&location.prefix)).context("Database inventory is outside the project scope")?;
            let connection = lancedb::connect(&super::database_uri(location,&path)).namespace_client_property("manifest_enabled", "false").session(session).execute().await?;
            let local: Vec<_> = inventory.iter().filter(|((root,_),_)| root == path.as_ref()).map(|((_,table),_)| table.clone()).collect();
            let mut names = match connection.table_names().execute().await { Ok(names) => names, Err(error) if !local.is_empty() => { tracing::debug!(%error,"Showing initialized offline tables while cloud inventory is unavailable"); local.clone() }, Err(error) => return Err(error.into()) };
            names.extend(local); names.sort(); names.dedup(); Ok(names)
        })
    }));
    let weak = Arc::downgrade(&manager);
    tokio::spawn(async move {
        let mut failures = 0u32;
        let mut immediate = false;
        let mut retrying = false;
        loop {
            let Some(manager) = weak.upgrade() else { break };
            if immediate {
                // Drain healthy queues at network speed while yielding to local flows.
                tokio::task::yield_now().await;
            } else if retrying {
                // New local writes must not bypass an outage's retry backoff.
                tokio::time::sleep(Duration::from_secs((1u64 << failures.min(6)).min(60))).await;
            } else {
                tokio::select! { _ = tokio::time::sleep(Duration::from_secs(15)) => (), _ = manager.wake.notified() => () }
            }
            let result = manager.drain_one().await;
            if matches!(result, Ok(false)) {
                let tables: Vec<_> = manager.tables.lock().await.values().cloned().collect();
                for table in tables {
                    if let Err(error) = table.refresh_idle(&manager).await {
                        tracing::debug!(%error,"Keeping the last complete offline table snapshot");
                        let _ = manager.queue.mirror_error(&table.key, Some("Cloud table refresh failed; the last complete local snapshot remains active. Check connectivity and the mirror disk budget."));
                    }
                }
            }
            match result {
                Ok(true) => {
                    failures = 0;
                    immediate = true;
                    retrying = false;
                }
                Ok(false) => {
                    immediate = false;
                    retrying = manager.queue.head_state().ok().flatten().as_deref()
                        == Some("outcome_unknown");
                    failures = if retrying {
                        failures.saturating_add(1)
                    } else {
                        0
                    };
                }
                Err(error) => {
                    tracing::warn!(%error,"Offline replay paused");
                    immediate = false;
                    retrying = true;
                    failures = failures.saturating_add(1);
                }
            }
            drop(manager);
        }
    });
    // Callbacks retain table adapters, which retain the manager through this closure.
    // A separate strong lifetime is installed below to cover file-only configurations.
    let keepalive = manager.clone();
    let previous = runtime
        .callbacks
        .decorate_database
        .clone()
        .expect("installed decorator");
    runtime.register_database_decorator(Arc::new(move |path, store| {
        let _keepalive = keepalive.clone();
        previous(path, store)
    }));
    Ok(manager)
}

fn clean_orphaned_tables(root: &Path, retained: &HashSet<String>) -> Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(name) = name.strip_suffix(".lance") else {
            continue;
        };
        if retained.contains(name) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path())?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Offline tables must be private directories"
        );
        let mut stack = vec![std::fs::read_dir(entry.path())?];
        while let Some(directory) = stack.last_mut() {
            let Some(child) = directory.next() else {
                stack.pop();
                continue;
            };
            let child = child?;
            let metadata = std::fs::symlink_metadata(child.path())?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "Orphaned offline tables must not contain symlinks"
            );
            if metadata.is_dir() {
                ensure!(
                    stack.len() < 64,
                    "Offline table directory nesting exceeds its limit"
                );
                stack.push(std::fs::read_dir(child.path())?);
            } else {
                ensure!(metadata.is_file(), "Unexpected file type in offline table");
            }
        }
        std::fs::remove_dir_all(entry.path())?;
    }
    Ok(())
}

impl WriteManager {
    fn authorize(&self) -> Result<()> {
        if self.credentials.authorization_current().is_err() {
            self.queue
                .quarantine("Project resource authorization revoked")?;
        }
        self.queue.check_authorized()
    }
    async fn drain_one(&self) -> Result<bool> {
        self.authorize()?;
        let guard = self.gate.lock().await;
        if let Some((id, reason)) = self.queue.requested_skip()? {
            if let Some(head) = self.queue.head()?.filter(|head| head.operation_id == id) {
                if let Some(table) = self.tables.lock().await.get(&head.resource).cloned() {
                    table.skip(self, &id, &reason).await?;
                } else {
                    self.queue.complete_skip(&id, &reason, None, "main", None)?;
                }
            } else {
                self.queue.clear_finished_skip(&id)?;
            }
        }
        let Some(mut operation) = self.queue.head()? else {
            return Ok(false);
        };
        if !matches!(
            operation.state.as_str(),
            "pending" | "attempting" | "outcome_unknown"
        ) {
            return Ok(false);
        }
        if let Some(table) = self.tables.lock().await.get(&operation.resource).cloned() {
            table.recover_local(self).await?;
            operation = self.queue.head()?.context("Queue head disappeared")?;
        }
        let mut request: OfflineReplayRequest = serde_json::from_value(operation.payload.clone())?;
        if matches!(request.resource, OfflineResource::File { .. })
            && operation.local_version.is_none()
        {
            // File bytes/tombstones are already durable in the queue payload.
            // Recover a crash between enqueue and the ready marker.
            self.queue.mark_local(&operation.operation_id, 1)?;
        }
        request.operation_id = operation.operation_id.clone();
        if operation.attempts == 0 {
            request.expected = serde_json::from_value(
                self.queue
                    .resource_revision(&operation.resource)?
                    .context("Missing cloud base revision")?,
            )?;
            request.validate()?;
            self.queue
                .prepare_attempt(&operation.operation_id, &serde_json::to_value(&request)?)?;
        }
        drop(guard);
        let result = self
            .credentials
            .client
            .request::<OfflineReplayResponse>(
                reqwest::Method::POST,
                "offline/replay",
                Some(&serde_json::to_value(&request)?),
            )
            .await;
        let _guard = self.gate.lock().await;
        match result {
            Ok(response) => {
                ensure!(
                    response.operation_id == request.operation_id
                        && response.digest == request.digest()?,
                    "Replay response does not match the queued operation"
                );
                match response.status {
                    OfflineReplayStatus::Applied => {
                        let revision = response
                            .result
                            .as_ref()
                            .context("Replay acknowledgement lacks a resource revision")?;
                        files::remember_acknowledged(self, &request, revision)?;
                        self.queue.acknowledge(
                            &operation.operation_id,
                            &serde_json::to_value(revision)?,
                            &serde_json::to_value(&response)?,
                        )?;
                        Ok(true)
                    }
                    status => {
                        let state = match status {
                            OfflineReplayStatus::Conflict => "conflict",
                            OfflineReplayStatus::OutcomeUnknown => "outcome_unknown",
                            _ => "blocked",
                        };
                        self.queue.block(
                            &operation.operation_id,
                            state,
                            response
                                .message
                                .as_deref()
                                .unwrap_or("Cloud write is blocked"),
                        )?;
                        Ok(false)
                    }
                }
            }
            Err(error) => {
                if super::authorization_error(&error)
                    == flow_like_types_contracts::authorization::AuthorizationError::Denied
                {
                    self.queue.quarantine(
                        "Replay authorization denied; queued data remains on this device",
                    )?;
                } else if crate::enrollment::api_status(&error).is_some_and(|status| {
                    status.is_client_error()
                        && status != reqwest::StatusCode::UNAUTHORIZED
                        && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                }) {
                    self.queue
                        .block(&operation.operation_id, "blocked", &error.to_string())?;
                }
                // Attempting remains immutable after a lost response. The API
                // reconciles the same operation ID before any repeated effect.
                Err(error)
            }
        }
    }
}

struct TableOverlay {
    manager: std::sync::Weak<WriteManager>,
    key: String,
    resource: OfflineResource,
    selection: BufferedTable,
    remote: Connection,
    local_name: String,
    generation: AtomicU64,
    last_refresh: AtomicI64,
}

async fn optional_table(connection: &Connection, name: &str) -> Result<Option<Table>> {
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

struct MutationSize(usize);
impl std::io::Write for MutationSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 > flow_like_device_protocol::MAX_OFFLINE_OPERATION_BYTES {
            return Err(std::io::Error::other(
                "Offline mutation exceeds 8 MiB; split the logical batch",
            ));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

async fn rows(table: &Table, filter: &str, select: Select) -> Result<Vec<Value>> {
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
            batch.get_array_memory_size() <= flow_like_device_protocol::MAX_OFFLINE_OPERATION_BYTES,
            "Offline mutation batch exceeds 8 MiB; use a narrower predicate"
        );
        for row in record_batch_to_value(&batch)? {
            bytes = bytes.saturating_add(serde_json::to_vec(&row)?.len());
            ensure!(
                bytes <= flow_like_device_protocol::MAX_OFFLINE_OPERATION_BYTES,
                "Offline mutation exceeds 8 MiB; split the logical operation"
            );
            result.push(row);
        }
    }
    Ok(result)
}

impl TableOverlay {
    async fn refresh_idle(&self, manager: &WriteManager) -> Result<()> {
        manager.authorize()?;
        let now = crate::enrollment::unix_time()?;
        if now.saturating_sub(self.last_refresh.load(Ordering::Acquire)) < 30
            || manager.queue.has_pending(&self.key)?
        {
            return Ok(());
        }
        self.last_refresh.store(now, Ordering::Release);
        let previous = manager
            .queue
            .resource_revision(&self.key)?
            .context("Missing offline table revision")?;
        let remote = optional_table(&self.remote, &self.selection.table).await?;
        let Some(remote) = remote else {
            let _guard = manager.gate.lock().await;
            manager.authorize()?;
            let absent = serde_json::to_value(OfflineExpected::TableVersion {
                version: 0,
                fingerprint: None,
            })?;
            if previous != absent
                && manager.queue.checkpoint(
                    &self.key,
                    &format!("absent_{}", uuid::Uuid::new_v4().simple()),
                    None,
                    &absent,
                    &previous,
                )?
            {
                self.generation.fetch_add(1, Ordering::Release);
            }
            manager.queue.mirror_error(&self.key, None)?;
            return Ok(());
        };
        let (version, fingerprint) = offline_replay::revision(&remote).await?;
        let expected = OfflineExpected::TableVersion {
            version,
            fingerprint: Some(fingerprint),
        };
        if manager.queue.resource_revision(&self.key)? == Some(serde_json::to_value(&expected)?) {
            manager.queue.mirror_error(&self.key, None)?;
            return Ok(());
        }
        let new_name = format!("snapshot_{}", uuid::Uuid::new_v4().simple());
        let snapshot = offline_replay::materialize(
            &remote,
            &manager.local,
            &new_name,
            manager.max_mirror_bytes,
        )
        .await?;
        let _guard = manager.gate.lock().await;
        manager.authorize()?;
        if manager.queue.has_pending(&self.key)? {
            manager.local.drop_table(&new_name, &[]).await?;
            return Ok(());
        }
        if !manager.queue.checkpoint(
            &self.key,
            &new_name,
            Some(snapshot.table.version().await?),
            &serde_json::to_value(OfflineExpected::TableVersion {
                version: snapshot.source_version,
                fingerprint: Some(snapshot.source_fingerprint),
            })?,
            &previous,
        )? {
            manager.local.drop_table(&new_name, &[]).await?;
            return Ok(());
        }
        self.generation.fetch_add(1, Ordering::Release);
        manager.queue.mirror_error(&self.key, None)?;
        Ok(())
    }
    fn local_view(&self, manager: &WriteManager) -> Result<(String, String, Option<u64>)> {
        let (name, branch, base) = manager.queue.local_view(&self.key)?;
        Ok((
            name.unwrap_or_else(|| self.local_name.clone()),
            branch,
            base,
        ))
    }
    async fn local_table(&self, manager: &WriteManager) -> Result<Option<Table>> {
        let (name, branch, _) = self.local_view(manager)?;
        match manager
            .local
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
    async fn skip(&self, manager: &WriteManager, id: &str, reason: &str) -> Result<()> {
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
    async fn initialize(&self, manager: &WriteManager) -> Result<()> {
        if manager.queue.resource_revision(&self.key)?.is_some() {
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
        let remote = optional_table(&self.remote, &self.selection.table).await?;
        match remote {
            Some(table) => {
                let name = format!("snapshot_{}", uuid::Uuid::new_v4().simple());
                let snapshot = offline_replay::materialize(
                    &table,
                    &manager.local,
                    &name,
                    manager.max_mirror_bytes,
                )
                .await?;
                let expected = OfflineExpected::TableVersion {
                    version: snapshot.source_version,
                    fingerprint: Some(snapshot.source_fingerprint),
                };
                manager.queue.initialize_table(
                    &self.key,
                    &serde_json::to_value(expected)?,
                    snapshot.table.version().await?,
                    &name,
                )?;
            }
            None => manager.queue.initialize_resource(
                &self.key,
                &serde_json::to_value(OfflineExpected::TableVersion {
                    version: 0,
                    fingerprint: None,
                })?,
                None,
            )?,
        }
        Ok(())
    }
    async fn recover_local(&self, manager: &WriteManager) -> Result<()> {
        let result = self.recover_local_inner(manager).await;
        if result.is_err()
            && let Ok(Some(operation)) = manager.queue.next_unmaterialized(&self.key)
        {
            // Keep detailed provider/schema diagnostics local. Management
            // needs the blocked operation and a safe recovery instruction.
            let _ = manager.queue.block(
                &operation.operation_id,
                "blocked",
                "The local table view could not be updated. Check the device disk budget and table schema, then retry or explicitly skip this retained operation.",
            );
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
                    offline_replay::create(&manager.local, &name, &marker, items).await?
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
        table: Option<&Table>,
        mutation: LogicalTableMutation,
    ) -> Result<(OfflineMutation, Option<String>)> {
        let key = &self.selection.primary_key;
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
                let values = rows(table, &filter, Select::Dynamic(columns)).await?;
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
                let values = rows(table, &filter, Select::Columns(vec![key.clone()])).await?;
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
            let existing = rows(table, &filter, Select::Columns(vec![key.clone()])).await?;
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

fn replay_mutation(mutation: OfflineMutation) -> Result<ReplayMutation> {
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
        serde_json::to_writer(&mut MutationSize(0), &mutation)
            .context("Offline mutation is too large")?;
        let manager = self
            .manager
            .upgrade()
            .context("Offline write manager stopped")?;
        let _guard = manager.gate.lock().await;
        manager.authorize()?;
        self.recover_local(&manager).await?;
        let table = self.local_table(&manager).await?;
        let (mutation, key) = self.freeze(table.as_ref(), mutation).await?;
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
        request.validate()?;
        let operation = manager.queue.enqueue(
            &self.key,
            serde_json::to_value(request)?,
            key.as_deref(),
            crate::enrollment::unix_time()?,
        )?;
        self.recover_local(&manager).await.with_context(||format!("Offline write {} was retained in the queue but its local table view could not be updated; inspect the queue before retrying",operation.operation_id))?;
        manager.wake.notify_one();
        Ok(LocalWriteReceipt {
            operation_id: operation.operation_id,
            sequence: operation.sequence,
            state: "pending".into(),
        })
    }
    async fn read_table(&self) -> Result<Option<Table>> {
        let manager = self
            .manager
            .upgrade()
            .context("Offline write manager stopped")?;
        let _guard = manager.gate.lock().await;
        manager.authorize()?;
        self.recover_local(&manager).await?;
        self.local_table(&manager).await
    }
    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{CredentialState, ProjectClient};
    use super::*;
    use flow_like_types_contracts::authorization::{
        AuthorizationFuture, AuthorizationRequest, RequestAuthorization, RequestAuthorizer,
        ResourceAudience,
    };
    use serde_json::json;
    use std::sync::atomic::AtomicBool;

    struct Authorization(String);
    impl RequestAuthorizer for Authorization {
        fn resource_base_url(&self, audience: ResourceAudience) -> Option<String> {
            (audience == ResourceAudience::ProjectApi).then(|| self.0.clone())
        }
        fn authorize<'a>(&'a self, _request: AuthorizationRequest<'a>) -> AuthorizationFuture<'a> {
            Box::pin(async {
                RequestAuthorization::new(
                    "DPoP test".into(),
                    Some("proof".into()),
                    std::time::SystemTime::now() + Duration::from_secs(60),
                )
            })
        }
    }
    async fn manager(
        root: &Path,
        remote: Connection,
        base: &str,
    ) -> Result<(Arc<WriteManager>, Arc<TableOverlay>)> {
        let config = super::super::tests::config(root);
        let identity = WorkloadIdentity {
            instance_id: "instance".into(),
            device_id: "device".into(),
            device_auth_epoch: 1,
            key_epoch: 1,
        };
        let queue = Arc::new(Outbox::open(
            root,
            "placement",
            &"a".repeat(64),
            BufferingConfig::default(),
        )?);
        let lock = queue.acquire_writer()?;
        let local_path = queue.root().join("tables");
        crate::outbox::private_directory(&local_path)?;
        let local =
            offline_replay::budgeted_local_connection(&local_path, 32 * 1024 * 1024).await?;
        let cache_path = root.join("cache");
        crate::outbox::private_directory(&cache_path)?;
        let cache = super::super::cache::CacheControl::new(
            &cache_path,
            &"a".repeat(64),
            vec!["apps/project/storage/".into()],
            1024 * 1024,
        )?;
        let credentials = Arc::new(ProjectCredentials {
            client: ProjectClient::new(Arc::new(Authorization(base.into())))?,
            config,
            identity,
            locations: BTreeMap::new(),
            delegating_user_id: "owner".into(),
            access: OnlineProjectAccess::ReadWrite,
            state: Mutex::new(CredentialState {
                lease: None,
                denied: false,
                failures: 0,
                next_refresh: i64::MAX,
                last_error: None,
            }),
            refreshing: Mutex::new(()),
            cache,
            scope: "a".repeat(64),
            cache_root: root.to_path_buf(),
            grant_expires_at: None,
            snapshot: None,
            revoked: tokio_util::sync::CancellationToken::new(),
        });
        let manager = Arc::new(WriteManager {
            queue,
            credentials,
            local,
            tables: Mutex::new(BTreeMap::new()),
            gate: Arc::new(Mutex::new(())),
            wake: Arc::new(Notify::new()),
            _lock: lock,
            max_mirror_bytes: 32 * 1024 * 1024,
        });
        let resource = OfflineResource::Table {
            purpose: flow_like_device_protocol::StoragePurpose::Storage,
            database: "db".into(),
            table: "measurements".into(),
        };
        let key = serde_json::to_string(&resource)?;
        let table = Arc::new(TableOverlay {
            manager: Arc::downgrade(&manager),
            key: key.clone(),
            resource,
            selection: BufferedTable {
                purpose: flow_like_device_protocol::StoragePurpose::Storage,
                database: "db".into(),
                table: "measurements".into(),
                primary_key: "id".into(),
            },
            remote,
            local_name: "measurements".into(),
            generation: AtomicU64::new(0),
            last_refresh: AtomicI64::new(0),
        });
        table.initialize(&manager).await?;
        manager.tables.lock().await.insert(key, table.clone());
        Ok((manager, table))
    }
    async fn seed(root: &Path) -> Result<Connection> {
        let db = lancedb::connect(root.to_str().unwrap()).execute().await?;
        db.create_table(
            "measurements",
            flow_like_storage::arrow_utils::value_to_batch_reader(vec![
                json!({"id":1,"value":10}),
                json!({"id":2,"value":20}),
            ])?,
        )
        .execute()
        .await?;
        Ok(db)
    }
    #[tokio::test]
    async fn frozen_updates_preserve_numeric_keys_and_quoted_columns() -> Result<()> {
        let root = tempfile::tempdir()?;
        let cloud = tempfile::tempdir()?;
        let remote = lancedb::connect(cloud.path().to_str().unwrap())
            .execute()
            .await?;
        remote
            .create_table(
                "measurements",
                flow_like_storage::arrow_utils::value_to_batch_reader(vec![json!({
                    "id": 1,
                    "value": 10,
                    "sensor label": "north",
                    "select": 17,
                    "counter": u64::MAX
                })])?,
            )
            .execute()
            .await?;
        let (writer, table) =
            manager(root.path(), remote, "http://127.0.0.1:1/instances/project").await?;
        table
            .apply(LogicalTableMutation::Update {
                filter: "id = 1".into(),
                updates: vec![("value".into(), "0".into())],
            })
            .await?;
        let request: OfflineReplayRequest =
            serde_json::from_value(writer.queue.head()?.unwrap().payload)?;
        let OfflineMutation::TableUpsert { rows: frozen, .. } = request.mutation else {
            anyhow::bail!("Update was not frozen to complete rows")
        };
        assert_eq!(
            frozen,
            vec![json!({
                "id": 1,
                "value": 0,
                "sensor label": "north",
                "select": 17,
                "counter": u64::MAX
            })]
        );
        assert_eq!(
            rows(&table.read_table().await?.unwrap(), "true", Select::All).await?,
            frozen
        );
        table
            .apply(LogicalTableMutation::Delete {
                filter: "id = 1".into(),
            })
            .await?;
        assert_eq!(
            table.read_table().await?.unwrap().count_rows(None).await?,
            0
        );
        Ok(())
    }

    #[tokio::test]
    async fn offline_update_then_delete_compacts_and_reads_survive_restart() -> Result<()> {
        let root = tempfile::tempdir()?;
        let cloud = tempfile::tempdir()?;
        let remote = seed(cloud.path()).await?;
        let (writer, table) = manager(
            root.path(),
            remote.clone(),
            "http://127.0.0.1:1/instances/project",
        )
        .await?;
        table
            .apply(LogicalTableMutation::Update {
                filter: "id = 1".into(),
                updates: vec![("value".into(), "value + 5".into())],
            })
            .await?;
        assert_eq!(
            rows(&table.read_table().await?.unwrap(), "id = 1", Select::All).await?[0]["value"],
            15
        );
        table
            .apply(LogicalTableMutation::Delete {
                filter: "id = 1".into(),
            })
            .await?;
        assert_eq!(writer.queue.status()?.pending_count, 1);
        assert_eq!(
            table.read_table().await?.unwrap().count_rows(None).await?,
            1
        );
        assert_eq!(
            remote
                .open_table("measurements")
                .execute()
                .await?
                .count_rows(None)
                .await?,
            2
        );
        drop(table);
        drop(writer);
        let (writer, table) =
            manager(root.path(), remote, "http://127.0.0.1:1/instances/project").await?;
        assert_eq!(
            table.read_table().await?.unwrap().count_rows(None).await?,
            1
        );
        assert_eq!(writer.queue.status()?.pending_count, 1);
        Ok(())
    }
    #[tokio::test]
    async fn skip_rebuilds_local_branch_without_the_blocked_update() -> Result<()> {
        let root = tempfile::tempdir()?;
        let cloud = tempfile::tempdir()?;
        let (writer, table) = manager(
            root.path(),
            seed(cloud.path()).await?,
            "http://127.0.0.1:1/instances/project",
        )
        .await?;
        let first = table
            .apply(LogicalTableMutation::Upsert {
                id_field: "id".into(),
                items: vec![json!({"id":1,"value":99})],
            })
            .await?;
        table
            .apply(LogicalTableMutation::Upsert {
                id_field: "id".into(),
                items: vec![json!({"id":2,"value":42})],
            })
            .await?;
        writer.queue.block(
            &first.operation_id,
            "conflict",
            "External writer advanced the table",
        )?;
        writer.queue.request_skip(
            &first.operation_id,
            "Operator discarded the conflicted update",
            false,
        )?;
        table
            .skip(
                &writer,
                &first.operation_id,
                "Operator discarded the conflicted update",
            )
            .await?;
        let local = table.read_table().await?.unwrap();
        assert_eq!(rows(&local, "id = 1", Select::All).await?[0]["value"], 10);
        assert_eq!(rows(&local, "id = 2", Select::All).await?[0]["value"], 42);
        assert_eq!(writer.queue.status()?.pending_count, 1);
        Ok(())
    }
    #[tokio::test]
    async fn empty_updates_are_not_queued_and_revocation_locks_the_overlay() -> Result<()> {
        let root = tempfile::tempdir()?;
        let cloud = tempfile::tempdir()?;
        let (writer, table) = manager(
            root.path(),
            seed(cloud.path()).await?,
            "http://127.0.0.1:1/instances/project",
        )
        .await?;
        assert_eq!(
            table
                .apply(LogicalTableMutation::Update {
                    filter: "id = 404".into(),
                    updates: vec![("value".into(), "0".into())]
                })
                .await?
                .state,
            "unchanged"
        );
        assert_eq!(writer.queue.status()?.pending_count, 0);
        table
            .apply(LogicalTableMutation::Delete {
                filter: "id = 1".into(),
            })
            .await?;
        writer.credentials.cache.revoke()?;
        assert!(table.read_table().await.is_err());
        assert_eq!(writer.queue.status()?.pending_count, 1);
        assert!(writer.queue.status()?.quarantined);
        Ok(())
    }
    #[derive(Clone)]
    struct ReplayServer {
        cloud: Connection,
        unavailable: Arc<AtomicBool>,
        lose_ack: Arc<AtomicBool>,
    }
    async fn replay_endpoint(
        axum::extract::State(state): axum::extract::State<ReplayServer>,
        axum::Json(request): axum::Json<OfflineReplayRequest>,
    ) -> axum::response::Response {
        use axum::response::IntoResponse;
        if state.unavailable.load(Ordering::Acquire) {
            return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        let result = async {
            request.validate()?;
            let OfflineExpected::TableVersion {
                version,
                fingerprint,
            } = &request.expected
            else {
                anyhow::bail!("Expected table mutation")
            };
            let marker = ReplayMarker {
                operation_id: request.operation_id.clone(),
                digest: request.digest()?,
                expected_version: *version,
                expected_fingerprint: fingerprint.clone(),
            };
            let outcome = if *version == 0 {
                let items = match &request.mutation {
                    OfflineMutation::TableInsert { rows }
                    | OfflineMutation::TableUpsert { rows, .. } => rows.clone(),
                    _ => anyhow::bail!("Absent tables require a schema-bearing create"),
                };
                offline_replay::create(&state.cloud, "measurements", &marker, items).await?
            } else {
                let table = state.cloud.open_table("measurements").execute().await?;
                offline_replay::replay(&table, &marker, replay_mutation(request.mutation.clone())?)
                    .await?
            };
            let (status, result) = match outcome {
                ReplayOutcome::Applied {
                    version,
                    fingerprint,
                } => (
                    OfflineReplayStatus::Applied,
                    Some(OfflineExpected::TableVersion {
                        version,
                        fingerprint: Some(fingerprint),
                    }),
                ),
                ReplayOutcome::Conflict { .. } => (OfflineReplayStatus::Conflict, None),
                _ => (OfflineReplayStatus::OutcomeUnknown, None),
            };
            Ok::<_, anyhow::Error>(OfflineReplayResponse {
                operation_id: request.operation_id.clone(),
                digest: request.digest()?,
                status,
                result,
                message: None,
            })
        }
        .await;
        match result {
            Ok(_) if state.lose_ack.swap(false, Ordering::AcqRel) => {
                axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response()
            }
            Ok(response) => axum::Json(response).into_response(),
            Err(error) => (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                error.to_string(),
            )
                .into_response(),
        }
    }
    #[tokio::test]
    async fn absent_table_creation_survives_a_following_upsert_or_delete() -> Result<()> {
        for delete in [false, true] {
            let root = tempfile::tempdir()?;
            let cloud = tempfile::tempdir()?;
            let remote = lancedb::connect(cloud.path().to_str().unwrap())
                .execute()
                .await?;
            let state = ReplayServer {
                cloud: remote.clone(),
                unavailable: Arc::new(AtomicBool::new(false)),
                lose_ack: Arc::new(AtomicBool::new(false)),
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
            let base = format!("http://{}/instances/project", listener.local_addr()?);
            let router = axum::Router::new()
                .route(
                    "/instances/project/offline/replay",
                    axum::routing::post(replay_endpoint),
                )
                .with_state(state);
            let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let (writer, table) = manager(root.path(), remote.clone(), &base).await?;
            let first = table
                .apply(LogicalTableMutation::Upsert {
                    items: vec![json!({"id":1,"value":10})],
                    id_field: "id".into(),
                })
                .await?;
            let second = if delete {
                LogicalTableMutation::Delete {
                    filter: "id = 1".into(),
                }
            } else {
                LogicalTableMutation::Upsert {
                    items: vec![json!({"id":1,"value":20})],
                    id_field: "id".into(),
                }
            };
            table.apply(second).await?;
            assert_eq!(writer.queue.status()?.pending_count, 2);
            assert_eq!(
                writer.queue.head()?.unwrap().operation_id,
                first.operation_id
            );
            assert!(writer.drain_one().await?);
            let created = remote.open_table("measurements").execute().await?;
            assert_eq!(rows(&created, "true", Select::All).await?[0]["value"], 10);
            assert!(writer.drain_one().await?);
            assert_eq!(writer.queue.status()?.pending_count, 0);
            let final_table = remote.open_table("measurements").execute().await?;
            let remaining = rows(&final_table, "true", Select::All).await?;
            if delete {
                assert!(remaining.is_empty());
            } else {
                assert_eq!(remaining[0]["value"], 20);
            }
            server.abort();
        }
        Ok(())
    }

    #[tokio::test]
    async fn offline_replay_recovers_lost_ack_after_restart_without_duplicate_effect() -> Result<()>
    {
        let root = tempfile::tempdir()?;
        let cloud = tempfile::tempdir()?;
        let remote = seed(cloud.path()).await?;
        let state = ReplayServer {
            cloud: remote.clone(),
            unavailable: Arc::new(AtomicBool::new(true)),
            lose_ack: Arc::new(AtomicBool::new(true)),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}/instances/project", listener.local_addr()?);
        let router = axum::Router::new()
            .route(
                "/instances/project/offline/replay",
                axum::routing::post(replay_endpoint),
            )
            .with_state(state.clone());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let (writer, table) = manager(root.path(), remote.clone(), &base).await?;
        table
            .apply(LogicalTableMutation::Insert {
                items: vec![json!({"id":3,"value":30})],
            })
            .await?;
        assert!(writer.drain_one().await.is_err());
        assert_eq!(
            table.read_table().await?.unwrap().count_rows(None).await?,
            3
        );
        state.unavailable.store(false, Ordering::Release);
        assert!(writer.drain_one().await.is_err());
        assert_eq!(
            remote
                .open_table("measurements")
                .execute()
                .await?
                .count_rows(None)
                .await?,
            3
        );
        drop(table);
        drop(writer);
        let (writer, table) = manager(root.path(), remote.clone(), &base).await?;
        assert!(writer.drain_one().await?);
        assert_eq!(writer.queue.status()?.pending_count, 0);
        assert_eq!(
            remote
                .open_table("measurements")
                .execute()
                .await?
                .count_rows(None)
                .await?,
            3
        );
        assert_eq!(
            table.read_table().await?.unwrap().count_rows(None).await?,
            3
        );
        server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn idle_snapshot_refresh_observes_cloud_updates_and_table_deletion() -> Result<()> {
        let root = tempfile::tempdir()?;
        let cloud = tempfile::tempdir()?;
        let remote = seed(cloud.path()).await?;
        let (writer, table) = manager(
            root.path(),
            remote.clone(),
            "http://127.0.0.1:1/instances/project",
        )
        .await?;
        let source = remote.open_table("measurements").execute().await?;
        source
            .update()
            .only_if("id = 1")
            .column("value", "50")
            .execute()
            .await?;
        source.delete("id = 2").await?;

        table.refresh_idle(&writer).await?;
        let local = table
            .read_table()
            .await?
            .context("refreshed snapshot is missing")?;
        assert_eq!(local.count_rows(None).await?, 1);
        assert_eq!(rows(&local, "id = 1", Select::All).await?[0]["value"], 50);
        assert_eq!(writer.queue.status()?.pending_count, 0);

        remote.drop_table("measurements", &[]).await?;
        table.last_refresh.store(0, Ordering::Release);
        table.refresh_idle(&writer).await?;
        assert!(table.read_table().await?.is_none());
        assert_eq!(
            writer.queue.resource_revision(&table.key)?,
            Some(serde_json::to_value(OfflineExpected::TableVersion {
                version: 0,
                fingerprint: None
            })?)
        );
        Ok(())
    }

    #[tokio::test]
    async fn failed_local_schema_recovery_is_visible_without_disclosing_payload() -> Result<()> {
        let root = tempfile::tempdir()?;
        let cloud = tempfile::tempdir()?;
        let remote = seed(cloud.path()).await?;
        let (writer, table) =
            manager(root.path(), remote, "http://127.0.0.1:1/instances/project").await?;
        let request = OfflineReplayRequest {
            operation_id: uuid::Uuid::new_v4().to_string(),
            resource: table.resource.clone(),
            expected: serde_json::from_value(writer.queue.resource_revision(&table.key)?.unwrap())?,
            mutation: OfflineMutation::TableInsert {
                rows: vec![json!({"id":3,"value":{"private":"secret-offline-payload"}})],
            },
        };
        let payload = serde_json::to_value(&request)?;
        let operation = writer.queue.enqueue(
            &table.key,
            payload.clone(),
            None,
            crate::enrollment::unix_time()?,
        )?;
        assert!(table.recover_local(&writer).await.is_err());
        let retained = writer.queue.head()?.unwrap();
        assert_eq!(retained.operation_id, operation.operation_id);
        assert_eq!(retained.state, "blocked");
        assert_eq!(retained.payload, payload);
        assert!(
            retained
                .error
                .as_deref()
                .is_some_and(|message| message.contains("disk budget and table schema"))
        );
        let status = writer.queue.status()?;
        assert!(status.head.as_ref().unwrap().payload.is_null());
        assert!(!serde_json::to_string(&status)?.contains("secret-offline-payload"));
        Ok(())
    }

    #[tokio::test]
    async fn local_commit_before_sqlite_marker_is_recovered_without_reapplying() -> Result<()> {
        let root = tempfile::tempdir()?;
        let cloud = tempfile::tempdir()?;
        let remote = seed(cloud.path()).await?;
        let (writer, table) = manager(
            root.path(),
            remote.clone(),
            "http://127.0.0.1:1/instances/project",
        )
        .await?;
        let request = OfflineReplayRequest {
            operation_id: uuid::Uuid::new_v4().to_string(),
            resource: table.resource.clone(),
            expected: serde_json::from_value(writer.queue.resource_revision(&table.key)?.unwrap())?,
            mutation: OfflineMutation::TableInsert {
                rows: vec![json!({"id": 3, "value": 30})],
            },
        };
        let operation = writer.queue.enqueue(
            &table.key,
            serde_json::to_value(&request)?,
            None,
            crate::enrollment::unix_time()?,
        )?;
        let local = table.local_table(&writer).await?.unwrap();
        let (version, fingerprint) = offline_replay::revision(&local).await?;
        let expected = OfflineExpected::TableVersion {
            version,
            fingerprint: Some(fingerprint.clone()),
        };
        writer
            .queue
            .prepare_local(&operation.operation_id, &serde_json::to_value(expected)?)?;
        let marker = ReplayMarker {
            operation_id: operation.operation_id.clone(),
            digest: request.digest()?,
            expected_version: version,
            expected_fingerprint: Some(fingerprint),
        };
        let applied =
            offline_replay::replay(&local, &marker, replay_mutation(request.mutation)?).await?;
        let ReplayOutcome::Applied {
            version: applied_version,
            ..
        } = applied
        else {
            anyhow::bail!("local commit was not applied")
        };
        assert!(writer.queue.next_unmaterialized(&table.key)?.is_some());

        // Simulate process loss after the durable Lance commit and before mark_local.
        drop(local);
        drop(table);
        drop(writer);
        let (writer, table) = manager(
            root.path(),
            remote.clone(),
            "http://127.0.0.1:1/instances/project",
        )
        .await?;
        let recovered = table.read_table().await?.unwrap();
        assert_eq!(recovered.count_rows(None).await?, 3);
        assert_eq!(
            offline_replay::revision(&recovered).await?.0,
            applied_version
        );
        assert_eq!(
            writer.queue.head()?.unwrap().local_version,
            Some(applied_version)
        );
        assert!(writer.queue.next_unmaterialized(&table.key)?.is_none());
        assert_eq!(
            remote
                .open_table("measurements")
                .execute()
                .await?
                .count_rows(None)
                .await?,
            2
        );
        Ok(())
    }
}

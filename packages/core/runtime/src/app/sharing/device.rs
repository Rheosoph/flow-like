//! Private deployment snapshots. A Lance table is materialized from one pinned
//! version; mutable project objects must keep their revisions throughout export.

use super::{App, AppVisibility, FlowLikeState};
use crate::flow::{
    board::Board,
    event::{Event, filter_event_secrets},
    node::Node,
};
use flow_like_storage::{
    Path,
    databases::vector::offline_replay::{budgeted_local_connection, materialize, revision},
    files::store::FlowLikeStore,
    object_store::{GetOptions, ObjectMeta, ObjectStore},
};
use flow_like_types::{
    FromProto, Message, Result, ToProto, anyhow, bail, tokio::io::AsyncWriteExt,
};
use futures::TryStreamExt;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io::{Read, Seek, SeekFrom},
    sync::Arc,
};

pub const MAX_DEVICE_EXPORT_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub const MAX_DEVICE_EXPORT_FILES: usize = 8192;
pub const MAX_DEVICE_EXPORT_CHUNK: usize = 1024 * 1024;
const MAX_VALIDATED_COMPRESSED: u64 = 16 * 1024 * 1024;
const MAX_VALIDATED_EXPANDED: u32 = 64 * 1024 * 1024;

pub fn validate_local_source(store: &FlowLikeStore, location: &Path) -> Result<()> {
    if let FlowLikeStore::Local(local) = store {
        let root = local.directory_to_filesystem(&Path::default())?;
        let source = local.path_to_filesystem(location)?;
        let relative = source.strip_prefix(&root)?;
        let mut current = root.clone();
        for component in relative.components() {
            if !matches!(component, std::path::Component::Normal(_)) {
                bail!("Export source is outside the local store");
            }
            current.push(component);
            if std::fs::symlink_metadata(&current)?
                .file_type()
                .is_symlink()
            {
                bail!("Deployment export cannot follow symbolic links");
            }
        }
        if !std::fs::metadata(current)?.is_file() {
            bail!("Deployment source must be a regular file");
        }
    }
    Ok(())
}

#[derive(Clone, Serialize)]
pub struct DeviceExportFile {
    pub path: String,
    pub size: u64,
}

/// Owns staged bytes until upload completes or its owning session expires.
pub struct DeviceProjectSnapshot {
    directory: tempfile::TempDir,
    files: BTreeMap<String, u64>,
    bytes: u64,
}

impl DeviceProjectSnapshot {
    pub async fn online_dependencies(project: &str) -> Result<Self> {
        validate_path(project)?;
        if project.contains('/') {
            bail!("Invalid project identifier");
        }
        let mut snapshot = Self {
            directory: tempfile::Builder::new()
                .prefix("flow-like-device-export-")
                .tempdir()?,
            files: BTreeMap::new(),
            bytes: 0,
        };
        snapshot
            .add_bytes(
                &format!("apps/{project}/online-source.json"),
                &serde_json::to_vec(
                    &serde_json::json!({"version":1,"project_id":project,"source":"online"}),
                )?,
            )
            .await?;
        Ok(snapshot)
    }
    pub fn files(&self) -> Vec<DeviceExportFile> {
        self.files
            .iter()
            .map(|(path, size)| DeviceExportFile {
                path: path.clone(),
                size: *size,
            })
            .collect()
    }

    pub fn read_chunk(&self, path: &str, offset: u64, length: usize) -> Result<Vec<u8>> {
        let size = self
            .files
            .get(path)
            .ok_or_else(|| anyhow!("File is outside this export session"))?;
        if length > MAX_DEVICE_EXPORT_CHUNK || offset > *size || length as u64 > size - offset {
            bail!("Invalid deployment export chunk range");
        }
        let mut file = std::fs::File::open(self.directory.path().join(path))?;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0; length];
        file.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    pub async fn add_bytes(&mut self, path: &str, bytes: &[u8]) -> Result<()> {
        self.reserve(path, bytes.len() as u64)?;
        let destination = self.directory.path().join(path);
        if let Some(parent) = destination.parent() {
            flow_like_types::tokio::fs::create_dir_all(parent).await?;
        }
        flow_like_types::tokio::fs::write(destination, bytes).await?;
        Ok(())
    }

    async fn replace_bytes(&mut self, path: &str, bytes: &[u8]) -> Result<()> {
        let previous = self
            .files
            .remove(path)
            .ok_or_else(|| anyhow!("Cannot replace {path}: it is outside this export session"))?;
        self.bytes -= previous;
        self.add_bytes(path, bytes).await
    }

    pub async fn add_object(
        &mut self,
        path: &str,
        store: Arc<dyn ObjectStore>,
        source: &ObjectMeta,
    ) -> Result<()> {
        self.reserve(path, source.size)?;
        let options = GetOptions {
            if_match: source.e_tag.clone(),
            version: source.version.clone(),
            ..Default::default()
        };
        let result = store.get_opts(&source.location, options).await?;
        if result.meta != *source {
            bail!("Project changed during export. Stop its writers and try again.");
        }
        let destination = self.directory.path().join(path);
        if let Some(parent) = destination.parent() {
            flow_like_types::tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = flow_like_types::tokio::fs::File::create(destination).await?;
        let mut stream = result.into_stream();
        let mut size = 0_u64;
        while let Some(chunk) = stream.try_next().await? {
            size = size
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| anyhow!("Export size overflow"))?;
            if size > source.size {
                bail!("Project object grew during export");
            }
            file.write_all(&chunk).await?;
        }
        file.sync_all().await?;
        if size != source.size {
            bail!("Project object changed size during export");
        }
        Ok(())
    }

    fn reserve(&mut self, path: &str, size: u64) -> Result<()> {
        validate_path(path)?;
        if self.files.contains_key(path) {
            bail!("Duplicate deployment export path: {path}");
        }
        let total = self
            .bytes
            .checked_add(size)
            .ok_or_else(|| anyhow!("Export size overflow"))?;
        if size > 4 * 1024 * 1024 * 1024
            || total > MAX_DEVICE_EXPORT_BYTES
            || self.files.len() >= MAX_DEVICE_EXPORT_FILES
        {
            bail!("Deployment export exceeds 8192 files, 4 GiB per file or 8 GiB total");
        }
        self.files.insert(path.to_owned(), size);
        self.bytes = total;
        Ok(())
    }

    fn register_materialized(&mut self, root: &std::path::Path) -> Result<()> {
        let mut pending = vec![root.to_path_buf()];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(directory)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    pending.push(entry.path());
                } else if kind.is_file() {
                    let path = entry.path();
                    let relative = path
                        .strip_prefix(self.directory.path())?
                        .to_str()
                        .ok_or_else(|| anyhow!("Invalid snapshot path"))?
                        .replace('\\', "/");
                    self.reserve(&relative, entry.metadata()?.len())?;
                } else {
                    bail!("Snapshot contains a non-regular file");
                }
            }
        }
        Ok(())
    }
}

fn validate_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.contains(['\\', '\0'])
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        bail!("Invalid deployment export path");
    }
    Ok(())
}

fn included(relative: &str) -> bool {
    relative != "deployment-snapshot.json"
        && !relative.starts_with("deployment-user-data/")
        && !relative.starts_with("storage/db/")
        && !relative.split('/').any(|part| {
            matches!(
                part,
                "logs"
                    | ".secrets"
                    | ".env"
                    | "secrets.key"
                    | "management.sqlite"
                    | "device-keys.json"
            )
        })
}

fn is_event_document(relative: &str) -> bool {
    let Some(rest) = relative.strip_prefix("events/") else {
        return false;
    };
    match rest.strip_prefix("versions/") {
        Some(archived) => archived.split('/').count() == 2,
        None => rest.ends_with(".event") && !rest.contains('/'),
    }
}

/// Matches every sink's credential field (endpoint, bot and access tokens,
/// webhook secrets, passwords, API keys) but no routing or display key.
fn is_credential_config_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(key.as_str(), "token" | "secret" | "password")
        || key.starts_with("secret_")
        || ["_token", "_secret", "_password", "api_key"]
            .iter()
            .any(|suffix| key.ends_with(suffix))
}

/// The device listener owns its own endpoint secret and placements supply
/// secrets privately, so saved event credentials never enter a revision.
fn device_event(mut event: Event) -> Result<Event> {
    if let Ok(serde_json::Value::Object(mut config)) =
        serde_json::from_slice::<serde_json::Value>(&event.config)
    {
        let before = config.len();
        config.retain(|key, _| !is_credential_config_key(key));
        if config.len() != before {
            event.config = serde_json::to_vec(&config)?;
        }
    }
    Ok(filter_event_secrets(event))
}

fn snapshot_node_ready(node: &Node) -> Result<()> {
    if node.name.starts_with("database_")
        || matches!(
            node.name.as_str(),
            "fts_search_local_db" | "hybrid_search_local_db" | "drop_index_db"
        )
    {
        bail!(
            "Node {} requires database history, references or indexes that a current-data deployment snapshot does not retain. Use an online deployment or remove that requirement.",
            node.name
        );
    }
    if node.name == "open_local_db" {
        for (name, expected) in [("branch", "main"), ("revision", "Latest")] {
            if let Some(pin) = node.pins.values().find(|pin| pin.name == name) {
                let value = pin
                    .default_value
                    .as_deref()
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok());
                if !pin.connected_to.is_empty()
                    || !pin.depends_on.is_empty()
                    || value.as_ref().and_then(|value| value.as_str()) != Some(expected)
                {
                    bail!(
                        "Database {} selector on node {} is dynamic or historical. Current-data export requires constant main/Latest selectors; use online deployment to preserve history.",
                        name,
                        node.id
                    );
                }
            }
        }
        if let Some(pin) = node.pins.values().find(|pin| pin.name == "reference")
            && !pin.connected_to.is_empty()
        {
            bail!(
                "A workflow consumes the source database version reference. Use online deployment to preserve its version identity."
            );
        }
    }
    Ok(())
}

fn snapshot_board_ready(board: &Board, has_tables: bool) -> Result<()> {
    for variable in board.variables.values().chain(
        board
            .layers
            .values()
            .flat_map(|layer| layer.variables.values()),
    ) {
        if variable.secret && super::secret_has_value(variable.default_value.as_deref()) {
            bail!(
                "Project contains saved secret values in board {}. Clear them and configure device secrets in the deployment dialog.",
                board.id
            );
        }
    }
    if has_tables {
        for node in board
            .nodes
            .values()
            .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
        {
            snapshot_node_ready(node)?;
        }
    }
    Ok(())
}

impl DeviceProjectSnapshot {
    fn read_compressed(&self, path: &str) -> Result<Vec<u8>> {
        let size = *self
            .files
            .get(path)
            .ok_or_else(|| anyhow!("File is outside this export session"))?;
        if size > MAX_VALIDATED_COMPRESSED {
            bail!("Project document {path} is too large for deployment validation ({size} bytes)");
        }
        let mut bytes = Vec::with_capacity(size as usize);
        let mut offset = 0;
        while offset < size {
            let length = (size - offset).min(MAX_DEVICE_EXPORT_CHUNK as u64) as usize;
            bytes.extend(self.read_chunk(path, offset, length)?);
            offset += length as u64;
        }
        let declared = bytes
            .get(..4)
            .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| anyhow!("Project document {path} is not a compressed document"))?;
        if declared > MAX_VALIDATED_EXPANDED {
            bail!(
                "Project document {path} expands to {declared} bytes, beyond the deployment validation limit"
            );
        }
        Ok(lz4_flex::decompress_size_prepended(&bytes)?)
    }

    fn validate_boards(&self, has_tables: bool) -> Result<()> {
        for path in self.files.keys() {
            if !path.ends_with(".board") && !path.ends_with(".template") {
                continue;
            }
            let plain = self.read_compressed(path)?;
            let board = Board::from_proto(flow_like_types::proto::Board::decode(plain.as_slice())?);
            snapshot_board_ready(&board, has_tables)?;
        }
        Ok(())
    }

    /// Rewrites staged live and archived events before the client hashes them.
    /// Pinned-version loads compare decoded events, so re-encoding keeps them valid.
    async fn redact_events(&mut self, base: &Path) -> Result<()> {
        let prefix = format!("{base}/");
        let paths = self
            .files
            .keys()
            .filter(|path| path.strip_prefix(&prefix).is_some_and(is_event_document))
            .cloned()
            .collect::<Vec<_>>();
        for path in paths {
            let plain = self.read_compressed(&path)?;
            let proto =
                flow_like_types::proto::Event::decode(plain.as_slice()).map_err(|error| {
                    anyhow!("Cannot decode event {path} for deployment export: {error}")
                })?;
            let stored = Event::from_proto(proto);
            let original = stored.to_proto();
            let redacted = device_event(stored)?.to_proto();
            if redacted != original {
                self.replace_bytes(
                    &path,
                    &lz4_flex::compress_prepend_size(&redacted.encode_to_vec()),
                )
                .await?;
            }
        }
        Ok(())
    }
}

async fn inventory(
    stores: &[Arc<dyn ObjectStore>],
    base: &Path,
) -> Result<BTreeMap<String, (usize, ObjectMeta)>> {
    let prefix = format!("{base}/");
    let mut files = BTreeMap::new();
    for (index, store) in stores.iter().enumerate() {
        let mut stream = store.list(Some(base));
        while let Some(file) = stream.try_next().await? {
            let relative = file
                .location
                .as_ref()
                .strip_prefix(&prefix)
                .ok_or_else(|| anyhow!("Store returned a path outside the project"))?;
            if !included(relative) {
                continue;
            }
            validate_path(relative)?;
            if file.e_tag.is_none() && file.version.is_none() {
                bail!("Project store cannot supply revisions for a consistent export");
            }
            files.insert(relative.to_owned(), (index, file));
            if files.len() > MAX_DEVICE_EXPORT_FILES {
                bail!("Too many deployment files");
            }
        }
    }
    Ok(files)
}

impl App {
    pub async fn export_device_snapshot(&self, user_sub: &str) -> Result<DeviceProjectSnapshot> {
        if user_sub.is_empty()
            || user_sub.len() > 512
            || user_sub.contains(['/', '\\', '\0'])
            || matches!(user_sub, "." | "..")
        {
            bail!("Select the account whose local project data should be exported");
        }
        if !matches!(self.visibility, AppVisibility::Offline) {
            bail!("Online projects must use their online deployment source");
        }
        let state = self
            .app_state
            .clone()
            .ok_or_else(|| anyhow!("App state not found"))?;
        for board_id in &self.boards {
            let board = self.open_board(board_id.clone(), Some(false), None).await?;
            if board.snapshot().variables.values().any(|variable| {
                variable.secret && super::secret_has_value(variable.default_value.as_deref())
            }) {
                bail!(
                    "Project contains saved secret values. Clear them and configure device secrets in the deployment dialog."
                );
            }
        }
        let sources = [
            FlowLikeState::project_meta_store(&state).await?,
            FlowLikeState::project_storage_store(&state).await?,
        ];
        let stores = [sources[0].as_generic(), sources[1].as_generic()];
        let base = super::app_base(&self.id);
        let before = inventory(&stores, &base).await?;
        if !before.contains_key("manifest.app") {
            bail!("Project manifest is missing");
        }
        let mut snapshot = DeviceProjectSnapshot {
            directory: tempfile::Builder::new()
                .prefix("flow-like-device-export-")
                .tempdir()?,
            files: BTreeMap::new(),
            bytes: 0,
        };
        for (relative, (store, metadata)) in &before {
            validate_local_source(&sources[*store], &metadata.location)?;
            snapshot
                .add_object(
                    &format!("{base}/{relative}"),
                    stores[*store].clone(),
                    metadata,
                )
                .await?;
        }
        snapshot.redact_events(&base).await?;
        // Export exactly the selected account. The target runs offline as `local`;
        // placement initialization remaps this project-scoped payload to that identity.
        let user_source = FlowLikeState::user_store(&state).await?;
        let user_store = user_source.as_generic();
        let user_base = Path::from("users")
            .join(user_sub)
            .join("apps")
            .join(self.id.clone());
        let user_before = inventory(&[user_store.clone()], &user_base)
            .await?
            .into_iter()
            .filter(|(path, _)| !path.starts_with("db/"))
            .collect::<BTreeMap<_, _>>();
        let user_destination = base.clone().join("deployment-user-data");
        for (relative, (_, metadata)) in &user_before {
            validate_local_source(&user_source, &metadata.location)?;
            snapshot
                .add_object(
                    &format!("{user_destination}/{relative}"),
                    user_store.clone(),
                    metadata,
                )
                .await?;
        }
        let db_path = base.clone().join("storage").join("db");
        let user_db_path = user_base.clone().join("db");
        let has_user_tables = user_store
            .list(Some(&user_db_path))
            .try_next()
            .await?
            .is_some();
        let has_tables = stores[1].list(Some(&db_path)).try_next().await?.is_some();
        snapshot.validate_boards(has_tables || has_user_tables)?;
        let mut table_snapshots = BTreeMap::new();
        let mut checked_databases = Vec::new();
        let mut user_table_snapshots = BTreeMap::new();
        for (user, present, path, destination_path) in [
            (false, has_tables, db_path.clone(), db_path.clone()),
            (
                true,
                has_user_tables,
                user_db_path.clone(),
                user_destination.join("db"),
            ),
        ] {
            if !present {
                continue;
            }
            let callbacks = state.config.read().await.callbacks.clone();
            let builder = if user {
                callbacks.build_user_database
            } else {
                callbacks.build_project_database
            }
            .ok_or_else(|| anyhow!("Project database builder is missing"))?;
            let connection = state.with_lance_session(builder(path)).execute().await?;
            let mut names = connection.table_names().execute().await?;
            names.sort();
            let destination = snapshot.directory.path().join(destination_path.as_ref());
            std::fs::create_dir_all(&destination)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o700))?;
            }
            let budget = MAX_DEVICE_EXPORT_BYTES.saturating_sub(snapshot.bytes);
            let local = budgeted_local_connection(&destination, budget).await?;
            let mut versions = BTreeMap::new();
            for name in &names {
                let table = connection.open_table(name).execute().await?;
                let snapshot_version = materialize(&table, &local, name, budget).await?;
                let target = if user {
                    &mut user_table_snapshots
                } else {
                    &mut table_snapshots
                };
                target.insert(name.clone(), serde_json::json!({"source_version":snapshot_version.source_version,"source_fingerprint":snapshot_version.source_fingerprint.clone()}));
                versions.insert(
                    name.clone(),
                    (
                        snapshot_version.source_version,
                        snapshot_version.source_fingerprint,
                    ),
                );
            }
            drop(local);
            snapshot.register_materialized(&destination)?;
            checked_databases.push((connection, names, versions));
        }
        for (connection, names, versions) in checked_databases {
            let mut after_names = connection.table_names().execute().await?;
            after_names.sort();
            if after_names != names {
                bail!("Project tables changed during export. Stop its writers and try again.");
            }
            for (name, version) in versions {
                if revision(&connection.open_table(&name).execute().await?).await? != version {
                    bail!("Project table changed during export. Stop its writers and try again.");
                }
            }
        }
        if (!has_tables && stores[1].list(Some(&db_path)).try_next().await?.is_some())
            || (!has_user_tables
                && user_store
                    .list(Some(&user_db_path))
                    .try_next()
                    .await?
                    .is_some())
        {
            bail!("Project database appeared during export. Stop its writers and try again.");
        }
        let user_after = inventory(&[user_store], &user_base)
            .await?
            .into_iter()
            .filter(|(path, _)| !path.starts_with("db/"))
            .collect::<BTreeMap<_, _>>();
        if user_after != user_before {
            bail!("User project data changed during export. Stop its writers and try again.");
        }
        if inventory(&stores, &base).await? != before {
            bail!("Project changed during export. Stop its writers and try again.");
        }
        let current = App::load(self.id.clone(), state).await?;
        if serde_json::to_value(&current)? != serde_json::to_value(self)? {
            bail!("Project manifest changed during export. Try again.");
        }
        snapshot.add_bytes(&format!("{base}/deployment-snapshot.json"), &serde_json::to_vec(&serde_json::json!({"version":1,"database_mode":"current_data","preserves_history":false,"preserves_indexes":false,"readiness":"validated","tables":table_snapshots,"user_data":{"source_subject":user_sub,"target_subject":"local","tables":user_table_snapshots}}))?).await?;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::{ObjectStoreExt, memory::InMemory};

    #[tokio::test]
    async fn exported_lance_table_is_a_complete_independent_version() -> Result<()> {
        use crate::{bit::Metadata, state::FlowLikeConfig, utils::http::HTTPClient};
        use flow_like_storage::{
            arrow_array::{Int64Array, RecordBatch},
            arrow_schema::{DataType, Field, Schema},
            files::store::{FlowLikeStore, local_store::LocalObjectStore},
            lancedb,
        };
        let source = tempfile::tempdir()?;
        let root = source.path().to_path_buf();
        let store = FlowLikeStore::Local(Arc::new(LocalObjectStore::new(root.clone())?));
        let mut config = FlowLikeConfig::with_default_store(store);
        let user_root = root.clone();
        config.register_build_project_database(Arc::new(move |path| {
            lancedb::connect(root.join(path.as_ref()).to_str().unwrap())
        }));
        config.register_build_user_database(Arc::new(move |path| {
            lancedb::connect(user_root.join(path.as_ref()).to_str().unwrap())
        }));
        let state = Arc::new(FlowLikeState::new(
            config,
            HTTPClient::new_without_refetch(),
        ));
        let app = App::new(Some("project".into()), Metadata::default(), vec![], state).await?;
        app.save().await?;
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
        let batch =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![1, 2]))])?;
        let database = source.path().join("apps/project/storage/db");
        let connection = lancedb::connect(database.to_str().unwrap())
            .execute()
            .await?;
        let table = connection
            .create_table("records", batch.clone())
            .execute()
            .await?;
        let user_base = Path::from("users")
            .join("auth0|selected")
            .join("apps")
            .join("project");
        let user_database = source.path().join(user_base.join("db").as_ref());
        let user_connection = lancedb::connect(user_database.to_str().unwrap())
            .execute()
            .await?;
        user_connection
            .create_table("private_records", batch.clone())
            .execute()
            .await?;
        let snapshot = app.export_device_snapshot("auth0|selected").await?;
        let user_exported = lancedb::connect(
            snapshot
                .directory
                .path()
                .join("apps/project/deployment-user-data/db")
                .to_str()
                .unwrap(),
        )
        .execute()
        .await?
        .open_table("private_records")
        .execute()
        .await?;
        assert_eq!(user_exported.count_rows(None).await?, 2);
        table.add(batch).execute().await?;
        let exported_db = snapshot.directory.path().join("apps/project/storage/db");
        let exported = lancedb::connect(exported_db.to_str().unwrap())
            .execute()
            .await?
            .open_table("records")
            .execute()
            .await?;
        assert_eq!(exported.count_rows(None).await?, 2);
        assert_eq!(table.count_rows(None).await?, 4);
        let metadata = std::fs::read(
            snapshot
                .directory
                .path()
                .join("apps/project/deployment-snapshot.json"),
        )?;
        let metadata: serde_json::Value = serde_json::from_slice(&metadata)?;
        assert_eq!(metadata["tables"]["records"]["source_version"], 1);
        assert_eq!(metadata["preserves_history"], false);
        Ok(())
    }

    #[test]
    fn current_data_snapshot_rejects_history_dynamic_selectors_and_fts() -> Result<()> {
        use crate::flow::variable::VariableType;
        for name in [
            "database_checkout",
            "database_tags",
            "fts_search_local_db",
            "hybrid_search_local_db",
        ] {
            assert!(snapshot_node_ready(&Node::new(name, name, "", "Data/Database")).is_err());
        }
        let mut open = Node::new("open_local_db", "Open", "", "Data/Database");
        open.add_input_pin("branch", "Branch", "", VariableType::String)
            .set_default_value(Some(serde_json::json!("main")));
        open.add_input_pin("revision", "Revision", "", VariableType::String)
            .set_default_value(Some(serde_json::json!("Latest")));
        snapshot_node_ready(&open)?;
        open.pins
            .values_mut()
            .find(|pin| pin.name == "branch")
            .unwrap()
            .connected_to
            .insert("dynamic".into());
        assert!(snapshot_node_ready(&open).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn complete_project_snapshot_contains_manifest_and_data_but_not_logs() -> Result<()> {
        use crate::{bit::Metadata, state::FlowLikeConfig, utils::http::HTTPClient};
        use flow_like_storage::files::store::FlowLikeStore;
        let store = Arc::new(InMemory::new());
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(FlowLikeStore::Memory(store.clone())),
            HTTPClient::new_without_refetch(),
        ));
        let app = App::new(Some("project".into()), Metadata::default(), vec![], state).await?;
        app.save().await?;
        store
            .put(&Path::from("apps/project/upload/file.txt"), "data".into())
            .await?;
        store
            .put(
                &Path::from("apps/project/logs/private.json"),
                "private".into(),
            )
            .await?;
        let selected = Path::from("users")
            .join("auth0|selected")
            .join("apps")
            .join("project")
            .join("photo.txt");
        store.put(&selected, "selected".into()).await?;
        store
            .put(
                &Path::from("users/other/apps/project/photo.txt"),
                "other account".into(),
            )
            .await?;
        let snapshot = app.export_device_snapshot("auth0|selected").await?;
        assert_eq!(
            snapshot.read_chunk("apps/project/deployment-user-data/photo.txt", 0, 8)?,
            b"selected"
        );
        assert!(
            snapshot
                .files()
                .iter()
                .all(|file| !file.path.starts_with("users/") && !file.path.contains("other"))
        );
        assert!(app.export_device_snapshot("../other").await.is_err());
        assert!(
            snapshot
                .files()
                .iter()
                .any(|file| file.path == "apps/project/manifest.app")
        );
        assert_eq!(
            snapshot.read_chunk("apps/project/upload/file.txt", 0, 4)?,
            b"data"
        );
        assert!(
            snapshot
                .files()
                .iter()
                .all(|file| !file.path.contains("logs/"))
        );
        Ok(())
    }

    fn test_event(id: &str) -> Event {
        use crate::flow::event::{EventExecutionMode, EventExposure};
        Event {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            board_id: "board".to_string(),
            board_version: Some((1, 0, 0)),
            node_id: "node".to_string(),
            variables: Default::default(),
            config: Vec::new(),
            active: true,
            canary: None,
            variants: Vec::new(),
            priority: 0,
            event_type: "http".to_string(),
            notes: None,
            event_version: (1, 0, 0),
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
            default_page_id: None,
            inputs: Vec::new(),
            route: None,
            is_default: false,
            execution_mode: EventExecutionMode::Local,
            exposure: EventExposure::Public,
            correlation_mappings: None,
        }
    }

    fn staged_event(snapshot: &DeviceProjectSnapshot, path: &str) -> Result<Event> {
        let plain = snapshot.read_compressed(path)?;
        Ok(Event::from_proto(flow_like_types::proto::Event::decode(
            plain.as_slice(),
        )?))
    }

    #[test]
    fn event_documents_are_live_and_archived_events_only() {
        assert!(is_event_document("events/hook.event"));
        assert!(is_event_document("events/versions/hook/1.0.0"));
        assert!(is_event_document("events/versions.event"));
        assert!(!is_event_document("events/versions/hook"));
        assert!(!is_event_document("events/nested/hook.event"));
        assert!(!is_event_document("deployment-user-data/events/hook.event"));
        assert!(!is_event_document("boards/board.board"));
    }

    #[test]
    fn device_event_keeps_non_json_and_credential_free_configs() -> Result<()> {
        let mut opaque = test_event("opaque");
        opaque.config = b"not json".to_vec();
        assert_eq!(device_event(opaque)?.config, b"not json");
        let mut routed = test_event("routed");
        routed.config = br#"{"path":"/hook","method":"POST"}"#.to_vec();
        assert_eq!(
            device_event(routed)?.config,
            br#"{"path":"/hook","method":"POST"}"#
        );
        let mut mail = test_event("mail");
        mail.config =
            br#"{"imap_server":"imap.example","Secret_IMAP_Password":"x","AUTH_TOKEN":"y"}"#
                .to_vec();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&device_event(mail)?.config)?,
            serde_json::json!({"imap_server":"imap.example"})
        );
        Ok(())
    }

    #[test]
    fn every_sink_credential_field_is_stripped_and_settings_survive() -> Result<()> {
        let credentials = [
            "auth_token",
            "token",
            "bot_token",
            "app_token",
            "integration_token",
            "personal_access_token",
            "webhook_secret",
            "secret",
            "password",
            "api_key",
            "secret_smtp_password",
        ];
        let settings = serde_json::json!({
            "path": "/hook",
            "bot_name": "Helper",
            "username": "robot",
            "smtp_username": "robot",
            "key_combination": "Cmd+K",
            "respond_to_mentions": true,
            "public_endpoint": false,
            "command_prefix": "!",
            "tools": [{"name": "lookup", "token": "tool-parameter-schema"}]
        });
        let mut config = settings.as_object().cloned().unwrap_or_default();
        for key in credentials {
            config.insert(key.to_string(), serde_json::json!("leaked"));
        }
        let mut event = test_event("sink");
        event.config = serde_json::to_vec(&config)?;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&device_event(event)?.config)?,
            settings
        );
        Ok(())
    }

    #[tokio::test]
    async fn staged_events_drop_saved_secrets_and_endpoint_credentials() -> Result<()> {
        use crate::{
            bit::Metadata,
            flow::{
                event::{CanaryEvent, EventVariant, EventVariantMode},
                pin::ValueType,
                variable::{Variable, VariableType},
            },
            state::FlowLikeConfig,
            utils::http::HTTPClient,
        };
        use flow_like_storage::files::store::FlowLikeStore;
        let store = Arc::new(InMemory::new());
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(FlowLikeStore::Memory(store.clone())),
            HTTPClient::new_without_refetch(),
        ));
        let app = App::new(Some("project".into()), Metadata::default(), vec![], state).await?;
        app.save().await?;

        let mut secret = Variable::new("api_key", VariableType::String, ValueType::Normal);
        secret
            .set_secret(true)
            .set_default_value(serde_json::json!("sk-desktop-secret"));
        let mut plain = Variable::new("region", VariableType::String, ValueType::Normal);
        plain.set_default_value(serde_json::json!("eu"));
        let only_secret = || [(secret.id.clone(), secret.clone())].into_iter().collect();

        let mut event = test_event("hook");
        event.config = serde_json::to_vec(&serde_json::json!({
            "path": "/hook",
            "method": "POST",
            "auth_token": "endpoint-secret",
            "secret_smtp_password": "hunter2"
        }))?;
        event.variables = [
            (secret.id.clone(), secret.clone()),
            (plain.id.clone(), plain.clone()),
        ]
        .into_iter()
        .collect();
        event.canary = Some(CanaryEvent {
            weight: 0.5,
            variables: only_secret(),
            board_id: "board".into(),
            board_version: Some((1, 0, 0)),
            node_id: "node".into(),
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
        });
        event.variants = vec![EventVariant {
            name: "shadow".into(),
            board_id: "board".into(),
            board_version: Some((1, 0, 0)),
            node_id: "node".into(),
            variables: only_secret(),
            default_page_id: None,
            mode: EventVariantMode::Shadow { sample_rate: 0.1 },
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
        }];
        event.save(&app, None).await?;
        event.save(&app, Some((1, 0, 0))).await?;

        let mut clean = test_event("clean");
        clean.config = br#"{"path":"/clean","method":"GET"}"#.to_vec();
        clean.variables = [(plain.id.clone(), plain.clone())].into_iter().collect();
        clean.save(&app, None).await?;

        let snapshot = app.export_device_snapshot("auth0|selected").await?;
        let live = staged_event(&snapshot, "apps/project/events/hook.event")?;
        let archived = staged_event(&snapshot, "apps/project/events/versions/hook/1.0.0")?;
        assert_eq!(
            serde_json::to_value(&live)?,
            serde_json::to_value(&archived)?
        );
        for staged in [&live, &archived] {
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&staged.config)?,
                serde_json::json!({"path": "/hook", "method": "POST"})
            );
            assert_eq!(staged.variables[&secret.id].default_value, None);
            assert!(staged.variables[&secret.id].secret);
            assert_eq!(
                staged.variables[&plain.id].default_value,
                plain.default_value
            );
            let canary = staged.canary.as_ref().expect("canary survives redaction");
            assert_eq!(canary.variables[&secret.id].default_value, None);
            assert_eq!(staged.variants[0].variables[&secret.id].default_value, None);
        }
        for path in [
            "apps/project/events/hook.event",
            "apps/project/events/versions/hook/1.0.0",
        ] {
            let plain_bytes = snapshot.read_compressed(path)?;
            for leaked in [&b"sk-desktop-secret"[..], b"endpoint-secret", b"hunter2"] {
                assert!(
                    !plain_bytes
                        .windows(leaked.len())
                        .any(|window| window == leaked)
                );
            }
        }

        let files = snapshot
            .files()
            .into_iter()
            .map(|file| (file.path, file.size))
            .collect::<BTreeMap<_, _>>();
        let hook_size = std::fs::metadata(
            snapshot
                .directory
                .path()
                .join("apps/project/events/hook.event"),
        )?
        .len();
        assert_eq!(files["apps/project/events/hook.event"], hook_size);
        assert_eq!(snapshot.bytes, files.values().sum::<u64>());

        let clean_path = "apps/project/events/clean.event";
        let source = store.get(&Path::from(clean_path)).await?.bytes().await?;
        assert_eq!(
            snapshot.read_chunk(clean_path, 0, files[clean_path] as usize)?,
            source.to_vec()
        );
        Ok(())
    }

    #[tokio::test]
    async fn staged_objects_are_private_bounded_and_immutable() -> Result<()> {
        let store = Arc::new(InMemory::new());
        let path = Path::from("apps/test/upload/file");
        store.put(&path, "original".into()).await?;
        let metadata = store.head(&path).await?;
        let mut snapshot = DeviceProjectSnapshot {
            directory: tempfile::tempdir()?,
            files: BTreeMap::new(),
            bytes: 0,
        };
        snapshot
            .add_object(path.as_ref(), store.clone(), &metadata)
            .await?;
        store.put(&path, "changed!".into()).await?;
        assert_eq!(snapshot.read_chunk(path.as_ref(), 0, 8)?, b"original");
        assert!(snapshot.read_chunk("../outside", 0, 1).is_err());
        assert!(snapshot.read_chunk(path.as_ref(), 7, 2).is_err());
        assert!(
            snapshot
                .add_bytes("apps/test/../escape", b"x")
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn changed_revision_is_rejected_before_copy() -> Result<()> {
        let store = Arc::new(InMemory::new());
        let path = Path::from("apps/test/manifest.app");
        store.put(&path, "first".into()).await?;
        let metadata = store.head(&path).await?;
        store.put(&path, "other".into()).await?;
        let mut snapshot = DeviceProjectSnapshot {
            directory: tempfile::tempdir()?,
            files: BTreeMap::new(),
            bytes: 0,
        };
        assert!(
            snapshot
                .add_object(path.as_ref(), store, &metadata)
                .await
                .is_err()
        );
        assert!(!included("storage/db/table.lance/_versions/1.manifest"));
        assert!(!included("logs/run.json"));
        Ok(())
    }
}

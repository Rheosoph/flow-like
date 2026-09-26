use super::{ProjectCredentials, WorkloadIdentity};
use crate::{
    config::PlacementConfig,
    outbox::{BufferedTable, BufferingConfig},
};
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::{
    OfflineExpected, OfflineReplayRequest, OfflineReplayResponse, OnlineProjectAccess,
    StoragePurpose,
};
use flow_like_offline_writes::{
    OfflineHost, ReplayError, ReplayErrorKind, TableActivation, TableSetup, WriteManagerOptions,
    optional_table,
};
use flow_like_runtime::state::FlowLikeConfig;
use flow_like_storage::{
    databases::vector::lancedb::connect_lance,
    lance_io::object_store::ObjectStoreRegistry,
    lancedb::{Connection, Table},
    object_store::{ObjectMeta, path::Path as ObjectPath},
};
use flow_like_types_contracts::authorization::AuthorizationError;
use std::{
    path::Path,
    sync::{Arc, RwLock},
};

pub(super) mod files;

pub(super) struct WriteManager {
    engine: Arc<flow_like_offline_writes::WriteManager>,
    credentials: Arc<ProjectCredentials>,
}

struct StandaloneHost {
    credentials: Arc<ProjectCredentials>,
    remotes: RwLock<Vec<(BufferedTable, Connection)>>,
    session: Arc<flow_like_storage::lance::session::Session>,
}

#[async_trait::async_trait]
impl OfflineHost for StandaloneHost {
    fn authorization_current(&self) -> Result<(), AuthorizationError> {
        self.credentials.authorization_current().map_err(|error| {
            error
                .downcast_ref::<AuthorizationError>()
                .copied()
                .unwrap_or(AuthorizationError::Denied)
        })
    }
    async fn replay(
        &self,
        request: &OfflineReplayRequest,
    ) -> Result<OfflineReplayResponse, ReplayError> {
        let body = serde_json::to_value(request).map_err(|error| ReplayError {
            kind: ReplayErrorKind::Unavailable,
            code: None,
            message: error.to_string(),
        })?;
        self.credentials
            .client
            .request::<OfflineReplayResponse>(reqwest::Method::POST, "offline/replay", Some(&body))
            .await
            .map_err(|error| {
                let kind = if super::authorization_error(&error) == AuthorizationError::Denied {
                    ReplayErrorKind::Denied
                } else if crate::enrollment::api_status(&error).is_some_and(|status| {
                    status.is_client_error()
                        && status != reqwest::StatusCode::UNAUTHORIZED
                        && status != reqwest::StatusCode::TOO_MANY_REQUESTS
                }) {
                    ReplayErrorKind::Rejected
                } else {
                    ReplayErrorKind::Unavailable
                };
                ReplayError {
                    kind,
                    code: None,
                    message: error.to_string(),
                }
            })
    }
    fn location_prefix(&self, purpose: StoragePurpose) -> Option<String> {
        self.credentials
            .locations
            .get(&purpose)
            .map(|location| location.prefix.clone())
    }
    async fn remote_table(&self, table: &BufferedTable) -> Result<Option<Table>> {
        let connection = self
            .remotes
            .read()
            .map_err(|_| anyhow::anyhow!("Offline table connections poisoned"))?
            .iter()
            .find(|(selected, _)| selected == table)
            .map(|(_, connection)| connection.clone())
            .context("Missing buffered database scope")?;
        optional_table(&connection, &table.table).await
    }
    async fn remote_table_names(&self, path: &ObjectPath) -> Result<Vec<String>> {
        let location = self
            .credentials
            .locations
            .values()
            .find(|location| path.as_ref().starts_with(&location.prefix))
            .context("Database inventory is outside the project scope")?;
        let connection = connect_lance(&super::database_uri(location, path))
            .session(self.session.clone())
            .execute()
            .await?;
        Ok(connection.table_names().execute().await?)
    }
    fn file_acknowledged(
        &self,
        path: &ObjectPath,
        _operation_id: &str,
        bytes: &[u8],
        revision: &OfflineExpected,
    ) -> Result<()> {
        let OfflineExpected::FileRevision { e_tag, version } = revision else {
            anyhow::bail!("Acknowledged file revision does not match its mutation");
        };
        let meta = ObjectMeta {
            location: path.clone(),
            size: bytes.len() as u64,
            last_modified: std::time::SystemTime::now().into(),
            e_tag: e_tag.clone(),
            version: version.clone(),
        };
        Ok(self.credentials.cache.remember_file(&meta, bytes)?)
    }
    fn file_deleted(&self, path: &ObjectPath, _operation_id: &str) -> Result<()> {
        Ok(self.credentials.cache.remember_file_deleted(path)?)
    }
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
    let session = Arc::new(flow_like_storage::lance::session::Session::new(
        flow_like_storage::lance::dataset::DEFAULT_INDEX_CACHE_SIZE,
        flow_like_storage::lance::dataset::DEFAULT_METADATA_CACHE_SIZE,
        registry,
    ));
    let host = Arc::new(StandaloneHost {
        credentials: credentials.clone(),
        remotes: RwLock::new(Vec::new()),
        session: session.clone(),
    });
    let engine = flow_like_offline_writes::WriteManager::open(
        WriteManagerOptions::standalone(
            local_data.to_path_buf(),
            config.id.clone(),
            credentials.scope.clone(),
            buffering.clone(),
        ),
        host.clone(),
    )
    .await?;
    for selected in &buffering.tables {
        let location = credentials
            .locations
            .get(&selected.purpose)
            .context("Missing buffered database scope")?;
        let connection = connect_lance(&format!("{}{}", location.uri, selected.database))
            .session(session.clone())
            .execute()
            .await?;
        host.remotes
            .write()
            .map_err(|_| anyhow::anyhow!("Offline table connections poisoned"))?
            .push((selected.clone(), connection));
        engine
            .add_table(
                selected.clone(),
                TableSetup {
                    activation: TableActivation::Active,
                    validate_key: false,
                    prefetch: false,
                },
            )
            .await?;
    }
    let predicate = engine.clone();
    runtime.register_database_table_is_managed(Arc::new(move |path, table| {
        predicate.table_is_managed(path, table)
    }));
    let decorated = engine.clone();
    runtime.register_database_decorator(Arc::new(move |path, store| {
        let decorated = decorated.clone();
        Box::pin(async move { decorated.decorate(&path, store).await })
    }));
    let inventory = engine.clone();
    runtime.register_database_table_names(Arc::new(move |path| {
        let inventory = inventory.clone();
        Box::pin(async move { inventory.table_names(&path).await })
    }));
    engine.spawn_drain();
    // Callbacks retain the manager. A separate strong lifetime is installed
    // below to cover file-only configurations.
    let keepalive = engine.clone();
    let previous = runtime
        .callbacks
        .decorate_database
        .clone()
        .expect("installed decorator");
    runtime.register_database_decorator(Arc::new(move |path, store| {
        let _keepalive = keepalive.clone();
        previous(path, store)
    }));
    Ok(Arc::new(WriteManager {
        engine,
        credentials,
    }))
}

use super::{ProjectCredentials, WorkloadIdentity};
use crate::{
    config::PlacementConfig,
    enrollment::{api_error_code, api_status},
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

/// A denial quarantines every queued write, so a replay treats only a 403 or a
/// 401 with a specific code as one. An API without proof codes answers a
/// project token that expired during a long upload with a plain or generic
/// 401, which is retried; a real revocation still reaches the broker token and
/// storage lease paths.
fn replay_error_kind(error: &anyhow::Error) -> ReplayErrorKind {
    let status = api_status(error);
    if status == Some(reqwest::StatusCode::UNAUTHORIZED)
        && api_error_code(error).is_none_or(|code| code == "UNAUTHORIZED")
    {
        return ReplayErrorKind::Unavailable;
    }
    match super::authorization_error(error) {
        AuthorizationError::Denied => ReplayErrorKind::Denied,
        AuthorizationError::Unavailable => ReplayErrorKind::Unavailable,
        _ if status.is_some_and(|status| status.is_client_error()) => ReplayErrorKind::Rejected,
        _ => ReplayErrorKind::Unavailable,
    }
}

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
    /// Any error here quarantines every queued write, so only a durable
    /// revocation counts. An elapsed deadline already stops this process, and a
    /// local read failure fails the write it guards.
    fn authorization_current(&self) -> Result<(), AuthorizationError> {
        match self.credentials.authorization_current() {
            Err(error)
                if super::local_authorization_error(&error) == AuthorizationError::Denied =>
            {
                Err(AuthorizationError::Denied)
            }
            Err(error) => {
                tracing::debug!(
                    placement_id = %self.credentials.config.id,
                    "Offline writes keep their queue without a confirmed denial: {error:#}"
                );
                Ok(())
            }
            Ok(()) => Ok(()),
        }
    }
    async fn replay(
        &self,
        request: &OfflineReplayRequest,
    ) -> Result<OfflineReplayResponse, ReplayError> {
        let body = serde_json::to_vec(request).map_err(|error| ReplayError {
            kind: ReplayErrorKind::Unavailable,
            code: None,
            message: error.to_string(),
        })?;
        self.credentials
            .client
            .replay::<OfflineReplayResponse>("offline/replay", body)
            .await
            .map_err(|error| ReplayError {
                kind: replay_error_kind(&error),
                code: None,
                message: error.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::api_error;

    #[tokio::test]
    async fn only_a_durable_revocation_quarantines_offline_writes() -> Result<()> {
        use crate::online::tests::{config, identity, lease, lease_server};
        let root = tempfile::tempdir()?;
        let config = config(root.path());
        let status = Arc::new(std::sync::atomic::AtomicU16::new(200));
        let (client, server) =
            lease_server(Arc::new(tokio::sync::Mutex::new(lease())), status).await?;
        let credentials =
            ProjectCredentials::new(client, &config, &identity(), root.path()).await?;
        let host = Arc::new(StandaloneHost {
            credentials: credentials.clone(),
            remotes: RwLock::new(Vec::new()),
            session: Arc::new(flow_like_storage::lance::session::Session::default()),
        });
        let engine = flow_like_offline_writes::WriteManager::open(
            WriteManagerOptions::standalone(
                root.path().to_path_buf(),
                config.id.clone(),
                credentials.scope.clone(),
                BufferingConfig::default(),
            ),
            host,
        )
        .await?;
        let database = rusqlite::Connection::open(credentials.cache.database_path())?;
        database.execute_batch("ALTER TABLE cache_state RENAME TO cache_state_unavailable")?;
        assert!(credentials.cache.is_revoked().is_err());
        engine.drain_once().await?;
        assert!(
            engine.queue().check_authorized().is_ok(),
            "An unreadable cache state quarantined the outbox"
        );
        database.execute_batch("ALTER TABLE cache_state_unavailable RENAME TO cache_state")?;
        credentials.grant_expires_at.store(
            crate::enrollment::unix_time()? - 1,
            std::sync::atomic::Ordering::Release,
        );
        engine.drain_once().await?;
        assert!(
            engine.queue().check_authorized().is_ok(),
            "An elapsed grant deadline quarantined the outbox"
        );
        credentials.cache.revoke()?;
        assert!(engine.drain_once().await.is_err());
        assert!(engine.queue().check_authorized().is_err());
        engine.close().await?;
        server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn replay_quarantines_only_on_a_confirmed_denial() {
        for (status, body, expected) in [
            (401, "", ReplayErrorKind::Unavailable),
            (
                401,
                r#"{"error":{"code":"UNAUTHORIZED","message":"Unauthorized"}}"#,
                ReplayErrorKind::Unavailable,
            ),
            (
                401,
                r#"{"error":{"code":"INSTANCE_PROOF_INVALID"}}"#,
                ReplayErrorKind::Unavailable,
            ),
            (
                401,
                r#"{"error":{"code":"GRANT_REVOKED"}}"#,
                ReplayErrorKind::Denied,
            ),
            (403, "", ReplayErrorKind::Denied),
            (402, "", ReplayErrorKind::Unavailable),
            (409, "", ReplayErrorKind::Rejected),
            (503, "", ReplayErrorKind::Unavailable),
        ] {
            assert_eq!(
                replay_error_kind(&api_error(status, body).await),
                expected,
                "{status} {body}"
            );
        }
        assert_eq!(
            replay_error_kind(&AuthorizationError::Denied.into()),
            ReplayErrorKind::Denied
        );
        assert_eq!(
            replay_error_kind(&AuthorizationError::Expired.into()),
            ReplayErrorKind::Unavailable
        );
    }
}

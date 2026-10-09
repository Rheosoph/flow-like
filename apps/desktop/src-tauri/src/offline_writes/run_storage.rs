//! Run storage modes of online apps (design §4.4): Hosted, Disconnected or Device. No run
//! of an online app falls back to device storage.

use super::{
    OfflineWrites, hub_key,
    object_index::ContentRoots,
    scope::{self, AppOfflineScope, CacheDirs, HostedRunTicket, RunMode, ScopeKey},
    stores::{
        DesktopFileStore, FileLayer, LocalCacheView, Refusal, UNAVAILABLE_SCHEME,
        UnavailableProvider,
    },
    texts,
};
use anyhow::Context;
use flow_like::{
    app::AppVisibility,
    credentials::SharedCredentials,
    flow::execution::extract_sub_from_jwt,
    flow_like_storage::{
        Path,
        databases::vector::lancedb::connect_lance,
        files::store::{FlowLikeStore, local_store::LocalObjectStore},
        lance_io::object_store::ObjectStoreRegistry,
        lancedb::connection::ConnectBuilder,
    },
    state::{FlowLikeConfig, FlowLikeState},
};
use flow_like_types::{anyhow, authorization::AuthorizationError, sync::RwLock};
use futures::future::BoxFuture;
use std::{path::PathBuf, sync::Arc};
use tauri::{AppHandle, Manager};

pub(crate) struct RunStorageRequest<'a> {
    pub visibility: &'a AppVisibility,
    pub app_id: &'a str,
    /// `profile.hub_profile.hub`, raw.
    pub hub: &'a str,
    pub secure: bool,
    pub token: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub webview: Option<&'a str>,
    pub event_id: Option<&'a str>,
    /// Credentials the caller passed; only Offline apps use them.
    pub device_credentials: Option<SharedCredentials>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunStorageMode {
    Device,
    Hosted,
    Disconnected,
}

/// Must live until the run finishes: dropping it releases the Hosted run ticket.
pub(crate) struct RunStorage {
    mode: RunStorageMode,
    credentials: Option<SharedCredentials>,
    scope: Option<Arc<AppOfflineScope>>,
    unbound_reason: Option<String>,
    app_id: String,
    dirs: Option<CacheDirs>,
    _ticket: Option<HostedRunTicket>,
}

/// The hub cannot be reached or answers garbage; any other refusal still stops the run.
pub(crate) fn hub_unreachable(error: &flow_like_types::Error) -> bool {
    matches!(
        error.downcast_ref::<AuthorizationError>(),
        Some(AuthorizationError::Unavailable | AuthorizationError::InvalidResponse)
    )
}

fn user_content_prefix(credentials: &SharedCredentials) -> Option<&str> {
    match credentials {
        SharedCredentials::Aws(aws) => aws.user_content_path_prefix.as_deref(),
        SharedCredentials::Azure(azure) => azure.user_content_path_prefix.as_deref(),
        SharedCredentials::Gcp(gcp) => gcp.user_content_path_prefix.as_deref().or_else(|| {
            gcp.allowed_prefixes
                .iter()
                .find(|prefix| prefix.starts_with("users/"))
                .map(String::as_str)
        }),
        SharedCredentials::Mixed(mixed) => user_content_prefix(&mixed.content),
        SharedCredentials::Renewable(live) => user_content_prefix(live.initial()),
    }
}

/// The token without surrounding whitespace and a `Bearer ` scheme.
fn bare_token(token: &str) -> &str {
    let token = token.trim();
    token
        .strip_prefix("Bearer ")
        .or_else(|| token.strip_prefix("bearer "))
        .unwrap_or(token)
}

/// The subject of a Hosted run: the JWT's, or for a PAT the owner named by the lease's
/// user folder `users/{sub}/apps/{app}`, percent-decoded.
pub(crate) fn hosted_subject(
    token: &str,
    credentials: &SharedCredentials,
    app_id: &str,
) -> Option<String> {
    let token = bare_token(token);
    if !token.starts_with("pat_") {
        return extract_sub_from_jwt(token)
            .ok()
            .filter(|subject| !subject.is_empty());
    }
    let prefix = user_content_prefix(credentials)?;
    let mut parts = prefix.trim_matches('/').split('/');
    if parts.next()? != "users" {
        return None;
    }
    let subject = parts.next()?;
    if parts.next()? != "apps" || parts.next()? != app_id || parts.next().is_some() {
        return None;
    }
    urlencoding::decode(subject)
        .ok()
        .map(|subject| subject.into_owned())
        .filter(|subject| !subject.is_empty())
}

/// The subject of a Disconnected run: the JWT's, or for a PAT the account the hub last
/// confirmed for it on this device.
pub(crate) fn disconnected_subject(
    token: &str,
    confirmed: impl FnOnce(&str) -> Option<String>,
) -> Option<String> {
    if bare_token(token).starts_with("pat_") {
        confirmed(token)
    } else {
        extract_sub_from_jwt(token)
            .ok()
            .filter(|subject| !subject.is_empty())
    }
}

impl RunStorage {
    fn device(credentials: Option<SharedCredentials>, app_id: &str) -> Self {
        Self {
            mode: RunStorageMode::Device,
            credentials,
            scope: None,
            unbound_reason: None,
            app_id: app_id.to_owned(),
            dirs: None,
            _ticket: None,
        }
    }

    /// Boxed: the engine's futures nest deep enough to overflow the auto-trait checks of
    /// the run futures that await this.
    pub(crate) async fn resolve<'a>(
        app_handle: &'a AppHandle,
        request: RunStorageRequest<'a>,
    ) -> flow_like_types::Result<Self> {
        let resolving: BoxFuture<'a, flow_like_types::Result<Self>> =
            Box::pin(Self::resolve_unboxed(app_handle, request));
        resolving.await
    }

    async fn resolve_unboxed(
        app_handle: &AppHandle,
        request: RunStorageRequest<'_>,
    ) -> flow_like_types::Result<Self> {
        if matches!(request.visibility, AppVisibility::Offline) {
            return Ok(Self::device(request.device_credentials, request.app_id));
        }
        let prepared = crate::execution_credentials::prepare(
            request.hub,
            request.app_id,
            request.token,
            request.session_id,
            request.webview,
        )
        .await;
        let dirs = scope::cache_dirs(app_handle).await?;
        let registry = app_handle.try_state::<OfflineWrites>();
        let (hub, app_id) = (request.hub, request.app_id);
        let confirmed =
            |token: &str| crate::execution_identity::confirmed_subject(hub, app_id, token);
        Self::from_prepared(
            registry.as_deref(),
            Some(app_handle),
            dirs,
            request,
            prepared,
            confirmed,
        )
        .await
    }

    pub(crate) async fn from_prepared(
        registry: Option<&OfflineWrites>,
        app_handle: Option<&AppHandle>,
        dirs: CacheDirs,
        request: RunStorageRequest<'_>,
        prepared: flow_like_types::Result<SharedCredentials>,
        confirmed: impl FnOnce(&str) -> Option<String>,
    ) -> flow_like_types::Result<Self> {
        if matches!(request.visibility, AppVisibility::Offline) {
            return Ok(Self::device(request.device_credentials, request.app_id));
        }
        let hub = hub_key(request.hub, request.secure);
        match prepared {
            Ok(credentials) => {
                Self::hosted(registry, app_handle, dirs, hub, &request, credentials).await
            }
            Err(error) if hub_unreachable(&error) => {
                tracing::warn!(app_id = %request.app_id, %error, "Hub unreachable; the run keeps its offline tables and queues its file writes");
                let subject = request
                    .token
                    .and_then(|token| disconnected_subject(token, confirmed));
                Self::disconnected(registry, app_handle, dirs, hub, &request, subject).await
            }
            Err(error) => Err(error),
        }
    }

    async fn hosted(
        registry: Option<&OfflineWrites>,
        app_handle: Option<&AppHandle>,
        dirs: CacheDirs,
        hub: Option<String>,
        request: &RunStorageRequest<'_>,
        credentials: SharedCredentials,
    ) -> flow_like_types::Result<Self> {
        let mut storage = Self {
            mode: RunStorageMode::Hosted,
            credentials: Some(credentials.clone()),
            scope: None,
            unbound_reason: None,
            app_id: request.app_id.to_owned(),
            dirs: Some(dirs.clone()),
            _ticket: None,
        };
        let SharedCredentials::Renewable(live) = &credentials else {
            return Ok(storage);
        };
        let subject = request
            .token
            .and_then(|token| hosted_subject(token, live.initial(), request.app_id));
        if let (Some(hub), Some(subject)) = (&hub, &subject) {
            let namespace = blake3::hash(&flow_like_types::json::to_vec(&(
                hub,
                subject,
                request.app_id,
            ))?)
            .to_hex()
            .to_string();
            let directory = dirs.project.join(".lance-read-cache");
            let cache = flow_like_types::tokio::task::spawn_blocking(move || {
                flow_like_offline_writes::fs::private_directory(&directory)?;
                Ok::<_, anyhow::Error>(
                    flow_like::flow_like_storage::files::immutable_lance_cache::LanceRangeCache::open(directory)?,
                )
            }).await;
            match cache {
                Ok(Ok(cache)) => {
                    live.install_lance_read_cache(cache, namespace);
                }
                error => tracing::debug!(?error, "Lance disk read cache is unavailable"),
            }
        }
        let (Some(registry), Some(hub)) = (registry, hub) else {
            return Ok(storage);
        };
        let Some(subject) = subject else {
            if registry.has_configured_tables(&hub, request.app_id) {
                return Err(anyhow!(texts::UNATTRIBUTED));
            }
            return Ok(storage);
        };
        let key = ScopeKey {
            hub,
            subject,
            app_id: request.app_id.to_owned(),
        };
        let scope = registry.obtain(app_handle.cloned(), dirs, key)?;
        storage._ticket = Some(scope.hosted_run_started());
        if let Some(token) = request.token {
            if let (Some(event_id), true) = (request.event_id, token.starts_with("pat_"))
                && let Err(error) = scope.record_sink(event_id, token)
            {
                tracing::warn!(%error, "Could not remember the sink of an offline scope");
            }
            scope.refresh_capabilities_in_background(token);
        }
        if scope.needs_manager()
            && let Err(error) = scope.manager().await
        {
            tracing::warn!(app_id = %request.app_id, %error, "Offline changes are unavailable for this run");
        }
        if scope.descriptor().hub_support == Some(true) && scope.open_manager().is_some() {
            live.install_content_decorator(scope.content_decorator());
        }
        scope.offer_lease(Arc::downgrade(live));
        scope.wake();
        storage.scope = Some(scope);
        Ok(storage)
    }

    async fn disconnected(
        registry: Option<&OfflineWrites>,
        app_handle: Option<&AppHandle>,
        dirs: CacheDirs,
        hub: Option<String>,
        request: &RunStorageRequest<'_>,
        subject: Option<String>,
    ) -> flow_like_types::Result<Self> {
        let registry = registry.context("Offline changes are not initialized on this device")?;
        let mut storage = Self {
            mode: RunStorageMode::Disconnected,
            credentials: None,
            scope: None,
            unbound_reason: None,
            app_id: request.app_id.to_owned(),
            dirs: Some(dirs.clone()),
            _ticket: None,
        };
        let (Some(hub), Some(subject)) = (hub, subject) else {
            storage.unbound_reason = Some(texts::UNATTRIBUTED.to_owned());
            return Ok(storage);
        };
        let key = ScopeKey {
            hub,
            subject,
            app_id: request.app_id.to_owned(),
        };
        let scope = registry.obtain(app_handle.cloned(), dirs, key)?;
        scope.observe_unreachable_hub();
        if scope.needs_manager()
            && let Err(error) = scope.manager().await
        {
            tracing::warn!(app_id = %request.app_id, %error, "Offline changes are unavailable for this run");
        }
        storage.scope = Some(scope);
        Ok(storage)
    }

    #[cfg(test)]
    pub(crate) fn mode(&self) -> RunStorageMode {
        self.mode
    }

    #[cfg(test)]
    pub(crate) fn scope(&self) -> Option<&Arc<AppOfflineScope>> {
        self.scope.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn unbound_reason(&self) -> Option<&str> {
        self.unbound_reason.as_deref()
    }

    /// Installs the Lance registry and the per-run config into `state`; returns the run's credentials.
    pub(crate) async fn install(
        &self,
        state: &mut FlowLikeState,
    ) -> flow_like_types::Result<Option<SharedCredentials>> {
        match self.mode {
            RunStorageMode::Device => Ok(self.credentials.clone()),
            RunStorageMode::Hosted => {
                let credentials = self
                    .credentials
                    .clone()
                    .context("A Hosted run has no hub credentials")?;
                crate::execution_credentials::install_registry(state, &credentials)?;
                if let Some(scope) = &self.scope {
                    let config = hosted_config(state.config.read().await.clone(), scope);
                    state.config = Arc::new(RwLock::new(config));
                }
                Ok(Some(credentials))
            }
            RunStorageMode::Disconnected => {
                let base = state.config.read().await.clone();
                let config = self.disconnected_config(base)?;
                state.set_lance_store_registry(self.disconnected_registry());
                state.config = Arc::new(RwLock::new(config));
                Ok(None)
            }
        }
    }

    /// Lance's default providers plus `flow-like-offline` for every unavailable database.
    pub(crate) fn disconnected_registry(&self) -> Arc<ObjectStoreRegistry> {
        let registry = ObjectStoreRegistry::default();
        let unready = self
            .scope
            .as_ref()
            .map(|scope| scope.unready_tables())
            .unwrap_or_default();
        registry.insert(
            UNAVAILABLE_SCHEME,
            Arc::new(UnavailableProvider::new(unready)),
        );
        Arc::new(registry)
    }

    fn unbound_store(&self, dirs: &CacheDirs) -> flow_like_types::Result<DesktopFileStore> {
        Ok(DesktopFileStore {
            layer: FileLayer::Refused(Refusal::Unattributed),
            cloud: None,
            cache: LocalCacheView {
                index: None,
                project: Arc::new(LocalObjectStore::new(dirs.project.clone())?),
                user: Arc::new(LocalObjectStore::new(dirs.user.clone())?),
                roots: ContentRoots::new(&self.app_id, None),
            },
        })
    }

    /// The per-run config of a Disconnected run (§4.4).
    pub(crate) fn disconnected_config(
        &self,
        mut config: FlowLikeConfig,
    ) -> flow_like_types::Result<FlowLikeConfig> {
        let dirs = self
            .dirs
            .clone()
            .context("A Disconnected run has no local cache directories")?;
        let store = match &self.scope {
            Some(scope) => scope.disconnected_store()?,
            None => self.unbound_store(&dirs)?,
        };
        let store = FlowLikeStore::Signed(Arc::new(store));
        config.register_app_storage_store(store.clone());
        config.register_user_store(store);
        let project_dir = dirs.project.clone();
        config.register_build_project_database(Arc::new(move |path: Path| {
            disconnected_database(&project_dir, &path)
        }));
        config.register_build_user_database(Arc::new(|path: Path| unavailable_database(&path)));
        let Some(scope) = self.scope.clone() else {
            let reason = self
                .unbound_reason
                .clone()
                .unwrap_or_else(|| texts::UNATTRIBUTED.to_owned());
            config.register_database_decorator(Arc::new(move |_, _| {
                let reason = reason.clone();
                Box::pin(async move { Err(anyhow!(reason)) })
            }));
            config.register_database_table_is_managed(Arc::new(|_, _| false));
            return Ok(config);
        };
        let decorating = scope.clone();
        config.register_database_decorator(Arc::new(move |path, store| {
            let scope = decorating.clone();
            Box::pin(async move { scope.decorate(&path, store, RunMode::Disconnected).await })
        }));
        let managed = scope.clone();
        config.register_database_table_is_managed(Arc::new(move |path, table| {
            managed.is_managed(path, table)
        }));
        config.register_database_table_names(Arc::new(move |path| {
            let names = scope
                .open_manager()
                .map(|manager| manager.managed_table_names(&path))
                .unwrap_or_default();
            Box::pin(async move { Ok(names) })
        }));
        Ok(config)
    }
}

/// The per-run config of a Hosted run: every online app gets the offline decorator.
pub(crate) fn hosted_config(
    mut config: FlowLikeConfig,
    scope: &Arc<AppOfflineScope>,
) -> FlowLikeConfig {
    let decorating = scope.clone();
    config.register_database_decorator(Arc::new(move |path, store| {
        let scope = decorating.clone();
        Box::pin(async move { scope.decorate(&path, store, RunMode::Hosted).await })
    }));
    let managed = scope.clone();
    config.register_database_table_is_managed(Arc::new(move |path, table| {
        managed.is_managed(path, table)
    }));
    if let Some(manager) = scope.open_manager() {
        config.register_database_table_names(Arc::new(move |path| {
            let manager = manager.clone();
            Box::pin(async move { manager.table_names(&path).await })
        }));
    }
    let noticing = scope.clone();
    config.register_database_table_notice(Arc::new(move |path, table| {
        noticing.table_notice(path, table)
    }));
    config
}

/// The regenerable function-tool index stays device-local (§1.6); every other database is unavailable.
fn disconnected_database(project_dir: &std::path::Path, path: &Path) -> ConnectBuilder {
    if path.as_ref().ends_with("/storage/.agents") {
        let directory: PathBuf = project_dir.join(path.to_string());
        let _ = std::fs::create_dir_all(&directory);
        return connect_lance(directory.to_string_lossy().as_ref());
    }
    unavailable_database(path)
}

fn unavailable_database(path: &Path) -> ConnectBuilder {
    connect_lance(&format!("{UNAVAILABLE_SCHEME}://unavailable/{path}"))
}

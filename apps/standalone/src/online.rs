use crate::{
    config::{PlacementConfig, WorkloadIdentity},
    enrollment::{http_client, response_json, unix_time},
};
use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use flow_like_device_protocol::{
    InstanceStorageCredential, InstanceStorageLease, OnlineProjectAccess, StoragePurpose,
};
use flow_like_runtime::{
    app::App,
    flow::{board::Board, event::Event},
    state::FlowLikeConfig,
    utils::compression::{compress_to_file, compress_to_file_json},
};
use flow_like_storage::{
    files::{
        credentials::{
            RenewableCredentials, StorageCredential, StorageCredentialLease,
            StorageCredentialProvider, StorageCredentialScope,
        },
        store::{FlowLikeStore, local_store::LocalObjectStore},
    },
    object_store::{
        self, CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta,
        ObjectStore, PutMultipartOptions, PutOptions, PutPayload, PutResult, RenameOptions,
        path::Path as ObjectPath,
    },
    renewable_lance::{LanceStorageBinding, scoped_registry},
};
use flow_like_types::ToProto;
use flow_like_types_contracts::authorization::{
    AuthorizationError, AuthorizationRequest, RequestAuthorizer, ResourceAudience,
};
use futures_util::{StreamExt, stream::BoxStream};
use serde::de::DeserializeOwned;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};
use tokio::sync::Mutex;

mod cache;
mod local_lance;
mod metadata;
pub mod outage;
mod writes;

#[derive(Clone)]
struct ProjectClient {
    authorizer: Arc<dyn RequestAuthorizer>,
    client: reqwest::Client,
    base: String,
}
impl ProjectClient {
    fn new(authorizer: Arc<dyn RequestAuthorizer>) -> Result<Self> {
        let base = authorizer
            .resource_base_url(ResourceAudience::ProjectApi)
            .context("Missing project API authorization")?;
        let url = url::Url::parse(&base)?;
        ensure!(
            url.scheme() == "https"
                || (url.scheme() == "http"
                    && url
                        .host_str()
                        .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "[::1]"))),
            "Project resources require HTTPS"
        );
        ensure!(
            url.query().is_none()
                && url.fragment().is_none()
                && url.username().is_empty()
                && url.password().is_none(),
            "Invalid project resource base"
        );
        Ok(Self {
            authorizer,
            client: http_client()?,
            base,
        })
    }
    async fn fetch<T: DeserializeOwned>(&self, method: reqwest::Method, path: &str) -> Result<T> {
        self.request(method, path, None).await
    }
    async fn request<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<T> {
        let url = format!("{}/{}", self.base, path);
        let authorization = self
            .authorizer
            .authorize(AuthorizationRequest {
                audience: ResourceAudience::ProjectApi,
                method: method.as_str(),
                url: &url,
            })
            .await?;
        let mut auth = reqwest::header::HeaderValue::from_str(authorization.authorization())?;
        auth.set_sensitive(true);
        let mut proof = reqwest::header::HeaderValue::from_str(
            authorization
                .dpop()
                .context("Missing project possession proof")?,
        )?;
        proof.set_sensitive(true);
        let mut request = self
            .client
            .request(method, url)
            .header("authorization", auth)
            .header("dpop", proof);
        if let Some(body) = body {
            request = request.json(body);
        }
        response_json(request.send().await?).await
    }
}

fn validate_lease(
    config: &PlacementConfig,
    identity: &WorkloadIdentity,
    lease: &InstanceStorageLease,
) -> Result<()> {
    let grant = config
        .resource_grant
        .as_ref()
        .context("Missing project grant")?;
    ensure!(
        lease.instance_id == identity.instance_id
            && lease.device_id == identity.device_id
            && lease.device_auth_epoch == identity.device_auth_epoch
            && lease.key_epoch == identity.key_epoch
            && lease.grant_id == grant.grant_id
            && lease.authz_version == grant.authz_version
            && lease.project_id == config.project_id
            && lease.placement_id == config.id
            && lease.deployment_id == config.deployment_id,
        "Storage lease changed its workload authorization"
    );
    let now = unix_time()?;
    ensure!(
        lease.expires_at > now
            && lease.expires_at
                <= now + flow_like_device_protocol::MAX_INSTANCE_STORAGE_LEASE_SECONDS,
        "Storage lease exceeds the provider deadline"
    );
    ensure!(
        !lease.delegating_user_id.is_empty()
            && !lease.delegating_user_id.contains("..")
            && lease.delegating_user_id.chars().all(|character| {
                character.is_alphanumeric() || matches!(character, '-' | '_' | '.' | '|' | ':')
            })
            && lease.delegating_user_id != ".",
        "Storage lease has an invalid delegating user"
    );
    let temporary = temporary_prefix(&lease.delegating_user_id, &config.project_id);
    let expected = BTreeMap::from([
        (
            StoragePurpose::Files,
            format!("apps/{}/upload/", config.project_id),
        ),
        (
            StoragePurpose::Storage,
            format!("apps/{}/storage/", config.project_id),
        ),
        (
            StoragePurpose::User,
            user_prefix(&lease.delegating_user_id, &config.project_id),
        ),
        (StoragePurpose::Temporary, temporary),
    ]);
    ensure!(
        lease.locations.len() == expected.len(),
        "Storage lease is missing DeviceExecute directories"
    );
    for (purpose, prefix) in expected {
        let location = lease
            .locations
            .get(&purpose)
            .context("Storage lease is missing DeviceExecute directory")?;
        let credential = lease
            .credentials
            .get(&location.credential_id)
            .context("Storage lease is missing its provider credential")?;
        let uri = url::Url::parse(&location.uri)?;
        // Inspect the original path before URL parsing can normalize dot segments.
        // Object-store paths retain their canonical escapes, including Auth0's
        // encoded pipe. Preserve the server's spelling without adding aliases.
        let raw_path = location
            .uri
            .split_once("://")
            .and_then(|(_, authority)| authority.split_once('/'))
            .map(|(_, path)| path)
            .context("Storage lease is missing its object directory")?;
        let decoded_path = ObjectPath::from_url_path(raw_path)?;
        let raw_object_path = ObjectPath::parse(raw_path)?;
        ensure!(
            location.prefix == prefix
                && raw_path.ends_with('/')
                && (format!("{raw_object_path}/") == prefix
                    || format!("{decoded_path}/") == prefix)
                && raw_object_path.parts().count() == decoded_path.parts().count()
                && !raw_path.to_ascii_lowercase().contains("%2f")
                && !decoded_path.as_ref().contains(['%', '\\'])
                && uri.host_str().is_some()
                && uri.username().is_empty()
                && uri.password().is_none()
                && uri.port().is_none()
                && uri.query().is_none()
                && uri.fragment().is_none(),
            "Storage lease changed its project directory"
        );
        ensure!(
            matches!(
                (uri.scheme(), credential),
                ("s3", InstanceStorageCredential::AwsSession { .. })
                    | ("az", InstanceStorageCredential::AzureSas { .. })
                    | ("gs", InstanceStorageCredential::GcpBearer { .. })
            ),
            "Storage credential does not match its provider"
        );
        for key in ["aws_endpoint", "azure_storage_endpoint"] {
            if let Some(endpoint) = location.options.get(key) {
                let endpoint = url::Url::parse(endpoint)?;
                ensure!(
                    endpoint.scheme() == "https"
                        && endpoint.username().is_empty()
                        && endpoint.password().is_none()
                        && endpoint.query().is_none()
                        && endpoint.fragment().is_none(),
                    "Storage transport requires HTTPS"
                );
            }
        }
        ensure!(
            !location.options.contains_key("allow_http"),
            "Storage transport cannot downgrade to HTTP"
        );
    }
    ensure!(
        lease.credentials.keys().all(|id| lease
            .locations
            .values()
            .any(|location| &location.credential_id == id)),
        "Storage lease contains an unreferenced credential"
    );
    Ok(())
}

fn temporary_prefix(sub: &str, project: &str) -> String {
    format!(
        "{}/",
        flow_like_types_contracts::storage::temporary_prefixes(sub, project).0
    )
}

fn user_prefix(sub: &str, project: &str) -> String {
    // Match get_user_dir and server db_path_from_base. Path::from encodes each
    // component, so auth0|user belongs to auth0%7Cuser in the object store.
    format!(
        "{}/",
        ObjectPath::from(format!("users/{sub}/apps/{project}"))
    )
}

struct CredentialState {
    lease: Option<InstanceStorageLease>,
    denied: bool,
    failures: u32,
    next_refresh: i64,
    last_error: Option<AuthorizationError>,
}

struct ProjectCredentials {
    client: ProjectClient,
    config: PlacementConfig,
    identity: WorkloadIdentity,
    locations: BTreeMap<StoragePurpose, flow_like_device_protocol::InstanceStorageLocation>,
    delegating_user_id: String,
    access: OnlineProjectAccess,
    state: Mutex<CredentialState>,
    refreshing: Mutex<()>,
    cache: Arc<cache::CacheControl>,
    scope: String,
    cache_root: PathBuf,
    grant_expires_at: Option<i64>,
    snapshot: Option<Arc<outage::SnapshotStore>>,
    revoked: tokio_util::sync::CancellationToken,
}

pub(crate) fn authorization_error(error: &anyhow::Error) -> AuthorizationError {
    if let Some(error) = error.downcast_ref::<AuthorizationError>() {
        return *error;
    }
    match crate::enrollment::api_status(error) {
        Some(reqwest::StatusCode::FORBIDDEN) => AuthorizationError::Denied,
        Some(status)
            if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS =>
        {
            AuthorizationError::Unavailable
        }
        Some(_) => AuthorizationError::InvalidResponse,
        None if error.chain().any(|source| {
            source
                .downcast_ref::<reqwest::Error>()
                .is_some_and(|error| {
                    error.status().is_none()
                        && (error.is_connect()
                            || error.is_timeout()
                            || error.is_body()
                            || error.is_request())
                })
        }) =>
        {
            AuthorizationError::Unavailable
        }
        _ => AuthorizationError::InvalidResponse,
    }
}

impl ProjectCredentials {
    #[cfg(test)]
    async fn new(
        client: ProjectClient,
        config: &PlacementConfig,
        identity: &WorkloadIdentity,
        cache_root: &Path,
    ) -> Result<Arc<Self>> {
        Ok(Self::initialize(client, config, identity, cache_root, None)
            .await?
            .0)
    }

    async fn initialize(
        client: ProjectClient,
        config: &PlacementConfig,
        identity: &WorkloadIdentity,
        cache_root: &Path,
        snapshot: Option<Arc<outage::SnapshotStore>>,
    ) -> Result<(Arc<Self>, Option<Arc<dyn ObjectStore>>)> {
        let max_cache_bytes = read_cache_budget()?;
        let result = client
            .fetch::<InstanceStorageLease>(reqwest::Method::POST, "storage")
            .await;
        let (lease, context, metadata) = match result {
            Ok(lease) => {
                validate_lease(config, identity, &lease)?;
                let context = outage::StorageContext {
                    locations: lease.locations.clone(),
                    delegating_user_id: lease.delegating_user_id.clone(),
                    access: lease.access,
                    scope: cache_scope(config, identity, &lease, &client.base)?,
                    grant_expires_at: lease.grant_expires_at.unwrap_or(0),
                };
                (Some(lease), context, None)
            }
            Err(error)
                if authorization_error(&error) == AuthorizationError::Unavailable
                    && snapshot.is_some() =>
            {
                let restored = match snapshot
                    .as_ref()
                    .context("Missing outage authority")?
                    .restore()
                    .await
                {
                    Ok(restored) => restored,
                    Err(error) => {
                        if authorization_error(&error) == AuthorizationError::Denied {
                            cache::CacheControl::revoke_placement(cache_root, &config.id)?;
                            quarantine_queues(cache_root, config)?;
                        }
                        return Err(error);
                    }
                };
                (None, restored.context, Some(restored.metadata))
            }
            Err(error) => {
                if authorization_error(&error) == AuthorizationError::Denied {
                    cache::CacheControl::revoke_placement(cache_root, &config.id)?;
                    quarantine_queues(cache_root, config)?;
                    if let Some(snapshot) = &snapshot {
                        snapshot.revoke().await?;
                    }
                }
                return Err(match crate::enrollment::api_status(&error) {
                    Some(
                        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED,
                    ) => anyhow::anyhow!(
                        "Online device execution requires an API with DeviceExecute storage support"
                    ),
                    _ => error,
                });
            }
        };
        let scope = context.scope.clone();
        let root = private_cache(cache_root, &config.id, &scope)?;
        cache::CacheControl::retain_placement_scope(cache_root, &config.id, &scope)?;
        let cache = cache::CacheControl::new(
            &root,
            &scope,
            context
                .locations
                .values()
                .map(|location| location.prefix.clone())
                .collect(),
            max_cache_bytes,
        )?;
        ensure!(
            !cache.is_revoked(),
            "Device storage authorization was revoked; deploy a new project grant"
        );
        let provider = Arc::new(Self {
            client,
            config: config.clone(),
            identity: identity.clone(),
            locations: context.locations,
            delegating_user_id: context.delegating_user_id,
            access: context.access,
            state: Mutex::new(CredentialState {
                next_refresh: lease.as_ref().map_or(unix_time()? + 2, refresh_at),
                lease,
                denied: false,
                failures: 0,
                last_error: None,
            }),
            refreshing: Mutex::new(()),
            cache,
            scope,
            cache_root: cache_root.to_path_buf(),
            grant_expires_at: (context.grant_expires_at > 0).then_some(context.grant_expires_at),
            snapshot,
            revoked: tokio_util::sync::CancellationToken::new(),
        });
        if let Some(expiry) = provider.grant_expires_at {
            let weak = Arc::downgrade(&provider);
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(
                    (expiry - unix_time().unwrap_or(expiry)).max(0) as u64,
                ))
                .await;
                if let Some(provider) = weak.upgrade() {
                    // The authenticated deadline is already durable in the
                    // supervisor. A slow refresh must not extend execution.
                    let _ = provider.cache.revoke();
                    provider.revoked.cancel();
                    let _ = provider.revoke().await;
                }
            });
        }
        let weak = Arc::downgrade(&provider);
        tokio::spawn(async move {
            loop {
                let Some(provider) = weak.upgrade() else {
                    return;
                };
                if provider.cache.is_revoked() {
                    let _ = provider.revoke().await;
                    return;
                }
                let state = provider.state.lock().await;
                if state.denied {
                    return;
                }
                let next = provider
                    .grant_expires_at
                    .map_or(state.next_refresh, |expiry| expiry.min(state.next_refresh));
                let wait = (next - unix_time().unwrap_or(0)).clamp(1, 5) as u64;
                drop(state);
                drop(provider);
                tokio::time::sleep(Duration::from_secs(wait)).await;
                let Some(provider) = weak.upgrade() else {
                    return;
                };
                let _ = provider.refresh(false).await;
            }
        });
        Ok((provider, metadata))
    }

    fn provider(self: &Arc<Self>, purpose: StoragePurpose) -> Result<Arc<DirectoryCredentials>> {
        let location = self
            .locations
            .get(&purpose)
            .context("Missing DeviceExecute location")?;
        Ok(Arc::new(DirectoryCredentials {
            project: self.clone(),
            credential_id: location.credential_id.clone(),
            scope: StorageCredentialScope {
                instance_id: self.identity.instance_id.clone(),
                project_id: self.config.project_id.clone(),
                placement_id: self.config.id.clone(),
                grant_id: self
                    .config
                    .resource_grant
                    .as_ref()
                    .context("Missing grant")?
                    .grant_id
                    .clone(),
                resource: format!("device-execute:{}", location.credential_id),
            },
        }))
    }

    fn authorization_current(&self) -> Result<()> {
        if self
            .grant_expires_at
            .is_some_and(|expiry| unix_time().map_or(true, |now| now >= expiry))
        {
            self.cache.revoke()?;
            self.revoked.cancel();
        }
        if self.cache.is_revoked() {
            return Err(AuthorizationError::Denied.into());
        }
        Ok(())
    }

    async fn revoke(&self) -> Result<()> {
        let cache_result = self.cache.revoke();
        let mut state = self.state.lock().await;
        state.denied = true;
        state.lease = None;
        drop(state);
        let snapshot_result = match &self.snapshot {
            Some(snapshot) => snapshot.revoke().await,
            None => Ok(()),
        };
        let queue_result = quarantine_queues(&self.cache_root, &self.config);
        // Drain only after attempting each durable fence, including when a
        // damaged child cache cannot be wiped successfully.
        self.revoked.cancel();
        snapshot_result.and(cache_result).and(queue_result)
    }

    async fn refresh(&self, force: bool) -> std::result::Result<(), AuthorizationError> {
        let _refresh = self.refreshing.lock().await;
        let now = unix_time().map_err(|_| AuthorizationError::Unavailable)?;
        if self.grant_expires_at.is_some_and(|expiry| now >= expiry) {
            self.revoke()
                .await
                .map_err(|_| AuthorizationError::Unavailable)?;
            return Err(AuthorizationError::Denied);
        }
        {
            let state = self.state.lock().await;
            if state.denied {
                return Err(AuthorizationError::Denied);
            }
            if !force && now < state.next_refresh {
                return Ok(());
            }
        }
        let result = self
            .client
            .fetch::<InstanceStorageLease>(reqwest::Method::POST, "storage")
            .await
            .map_err(|error| authorization_error(&error))
            .and_then(|lease| {
                validate_lease(&self.config, &self.identity, &lease)
                    .map_err(|_| AuthorizationError::InvalidResponse)?;
                if self
                    .grant_expires_at
                    .zip(lease.grant_expires_at)
                    .is_some_and(|(previous, current)| current < previous)
                {
                    return Err(AuthorizationError::Denied);
                }
                if lease.locations != self.locations
                    || lease.access != self.access
                    || lease.delegating_user_id != self.delegating_user_id
                {
                    return Err(AuthorizationError::InvalidResponse);
                }
                Ok(lease)
            });
        let mut state = self.state.lock().await;
        match result {
            Ok(lease) => {
                state.next_refresh = refresh_at(&lease);
                state.lease = Some(lease);
                state.failures = 0;
                state.last_error = None;
                Ok(())
            }
            Err(error) => {
                state.last_error = Some(error);
                if error == AuthorizationError::Denied {
                    state.denied = true;
                    state.lease = None;
                    drop(state);
                    self.revoke()
                        .await
                        .map_err(|_| AuthorizationError::Unavailable)?;
                } else {
                    // Keep the prior credential through its actual expiry. A failed
                    // refresh must not interrupt a healthy provider session.
                    state.failures = state.failures.saturating_add(1);
                    state.next_refresh = now + (1i64 << state.failures.min(6)).min(60);
                }
                Err(error)
            }
        }
    }
}

fn refresh_at(lease: &InstanceStorageLease) -> i64 {
    let now = unix_time().unwrap_or(0);
    // Short grant deadlines still get a refresh before their last usable second.
    let margin = ((lease.expires_at - now) / 5).clamp(1, 300);
    lease.expires_at - margin
}

const READ_CACHE_BUDGET_VARIABLE: &str = "FLOW_LIKE_DEVICE_READ_CACHE_BYTES";
const DEFAULT_READ_CACHE_BYTES: u64 = 512 * 1024 * 1024;

fn read_cache_budget() -> Result<u64> {
    let value = std::env::var(READ_CACHE_BUDGET_VARIABLE)
        .map(Some)
        .or_else(|error| match error {
            std::env::VarError::NotPresent => Ok(None),
            std::env::VarError::NotUnicode(_) => Err(anyhow::anyhow!(
                "{READ_CACHE_BUDGET_VARIABLE} must contain an integer byte count"
            )),
        })?;
    parse_read_cache_budget(value.as_deref())
}

fn parse_read_cache_budget(value: Option<&str>) -> Result<u64> {
    let Some(value) = value else {
        return Ok(DEFAULT_READ_CACHE_BYTES);
    };
    let budget = value
        .parse::<u64>()
        .ok()
        .filter(|value| (16 * 1024 * 1024..=64 * 1024 * 1024 * 1024).contains(value));
    ensure!(
        value.bytes().all(|byte| byte.is_ascii_digit()) && budget.is_some(),
        "{READ_CACHE_BUDGET_VARIABLE} must be an integer byte count between 16777216 (16 MiB) and 68719476736 (64 GiB)"
    );
    Ok(budget.unwrap())
}

pub(crate) fn cache_scope(
    config: &PlacementConfig,
    identity: &WorkloadIdentity,
    lease: &InstanceStorageLease,
    api_base: &str,
) -> Result<String> {
    Ok(blake3::hash(&serde_json::to_vec(&serde_json::json!({
        "api_base": api_base,
        "device": identity.device_id,
        "device_auth_epoch": identity.device_auth_epoch,
        "project": config.project_id,
        "placement": config.id,
        "grant": lease.grant_id,
        "authz_version": lease.authz_version,
        "subject": lease.delegating_user_id,
        "access": lease.access,
        "locations": lease.locations,
    }))?)
    .to_hex()
    .to_string())
}

struct DirectoryCredentials {
    project: Arc<ProjectCredentials>,
    credential_id: String,
    scope: StorageCredentialScope,
}

#[async_trait]
impl StorageCredentialProvider for DirectoryCredentials {
    fn scope(&self) -> &StorageCredentialScope {
        &self.scope
    }

    async fn credential(&self) -> std::result::Result<StorageCredentialLease, AuthorizationError> {
        self.project
            .authorization_current()
            .map_err(|_| AuthorizationError::Denied)?;
        let now = unix_time().map_err(|_| AuthorizationError::Unavailable)?;
        let mut state = self.project.state.lock().await;
        if state.denied || self.project.cache.is_revoked() {
            return Err(AuthorizationError::Denied);
        }
        if state
            .lease
            .as_ref()
            .is_none_or(|lease| lease.expires_at <= now)
        {
            drop(state);
            self.project.refresh(false).await?;
            state = self.project.state.lock().await;
        }
        if state.denied {
            return Err(AuthorizationError::Denied);
        }
        let lease = state
            .lease
            .as_ref()
            .filter(|lease| lease.expires_at > unix_time().unwrap_or(i64::MAX))
            .ok_or(state.last_error.unwrap_or(AuthorizationError::Unavailable))?;
        let credential = match lease
            .credentials
            .get(&self.credential_id)
            .ok_or(AuthorizationError::InvalidResponse)?
        {
            InstanceStorageCredential::AwsSession {
                access_key_id,
                secret_access_key,
                session_token,
            } => StorageCredential::AwsSession {
                access_key_id: access_key_id.clone(),
                secret_access_key: secret_access_key.clone(),
                session_token: session_token.clone(),
            },
            InstanceStorageCredential::AzureSas { sas_token } => {
                StorageCredential::AzureSas(sas_token.clone())
            }
            InstanceStorageCredential::GcpBearer { access_token } => {
                StorageCredential::GcpBearer(access_token.clone())
            }
        };
        Ok(StorageCredentialLease {
            scope: self.scope.clone(),
            expires_at: UNIX_EPOCH
                + Duration::from_secs(
                    lease
                        .expires_at
                        .try_into()
                        .map_err(|_| AuthorizationError::InvalidResponse)?,
                ),
            credential,
        })
    }
}

#[derive(Debug, Clone)]
struct ProjectStore {
    routes: Vec<(String, Arc<dyn ObjectStore>)>,
}
impl std::fmt::Display for ProjectStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StandaloneProjectStore")
    }
}
impl ProjectStore {
    fn route(&self, path: &ObjectPath) -> object_store::Result<Arc<dyn ObjectStore>> {
        self.routes
            .iter()
            .find(|(prefix, _)| path.as_ref().starts_with(prefix))
            .map(|(_, store)| store.clone())
            .ok_or_else(|| object_store::Error::PermissionDenied {
                path: path.to_string(),
                source: "Object is outside this project's authorized directories".into(),
            })
    }
    fn same_route(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
    ) -> object_store::Result<Arc<dyn ObjectStore>> {
        let source = self.route(from)?;
        let target = self.route(to)?;
        if !Arc::ptr_eq(&source, &target) {
            return Err(object_store::Error::NotSupported {
                source: "Copy across different cloud buckets or credentials is unsupported".into(),
            });
        }
        Ok(source)
    }
    fn list_path(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> object_store::Result<(Arc<dyn ObjectStore>, ObjectPath)> {
        let prefix = prefix.ok_or_else(|| object_store::Error::NotSupported {
            source: "Project listing requires an authorized directory prefix".into(),
        })?;
        // ObjectStore adds the directory delimiter to cloud list requests.
        // Path itself strips trailing delimiters, so the directory root is
        // accepted here without widening ordinary object reads or writes.
        if let Some((_, store)) = self
            .routes
            .iter()
            .find(|(base, _)| base.trim_end_matches('/') == prefix.as_ref())
        {
            return Ok((store.clone(), prefix.clone()));
        }
        Ok((self.route(prefix)?, prefix.clone()))
    }
}
#[async_trait]
impl ObjectStore for ProjectStore {
    async fn put_opts(
        &self,
        path: &ObjectPath,
        payload: PutPayload,
        opts: PutOptions,
    ) -> object_store::Result<PutResult> {
        self.route(path)?.put_opts(path, payload, opts).await
    }
    async fn put_multipart_opts(
        &self,
        path: &ObjectPath,
        opts: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        self.route(path)?.put_multipart_opts(path, opts).await
    }
    async fn get_opts(
        &self,
        path: &ObjectPath,
        opts: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.route(path)?.get_opts(path, opts).await
    }
    fn delete_stream(
        &self,
        locations: BoxStream<'static, object_store::Result<ObjectPath>>,
    ) -> BoxStream<'static, object_store::Result<ObjectPath>> {
        let store = self.clone();
        Box::pin(locations.then(move |path| {
            let store = store.clone();
            async move {
                let path = path?;
                use object_store::ObjectStoreExt;
                store.route(&path)?.delete(&path).await?;
                Ok(path)
            }
        }))
    }
    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        match self.list_path(prefix) {
            Ok((store, path)) => store.list(Some(&path)),
            Err(error) => Box::pin(futures_util::stream::once(async { Err(error) })),
        }
    }
    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> object_store::Result<ListResult> {
        let (store, path) = self.list_path(prefix)?;
        store.list_with_delimiter(Some(&path)).await
    }
    async fn copy_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        self.same_route(from, to)?
            .copy_opts(from, to, options)
            .await
    }
    async fn rename_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: RenameOptions,
    ) -> object_store::Result<()> {
        self.same_route(from, to)?
            .rename_opts(from, to, options)
            .await
    }
}

fn private_cache(root: &Path, placement: &str, instance: &str) -> Result<PathBuf> {
    let mut path = root.to_owned();
    for component in [".standalone-cache", placement, instance] {
        path.push(component);
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Project cache contains a non-directory or symlink"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                metadata.mode() & 0o077 == 0 && metadata.uid() == unsafe { libc::geteuid() },
                "Project cache must be private and owned by this user"
            );
        }
    }
    Ok(path)
}

pub(crate) fn validate_snapshot_parent(root: &Path, placement: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for component in [".standalone-cache", placement, "outage"] {
        path.push(component);
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Invalid snapshot directory"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                metadata.mode() & 0o077 == 0 && metadata.uid() == unsafe { libc::geteuid() },
                "Snapshot directory must be private and owned"
            );
        }
    }
    Ok(path)
}

fn quarantine_queues(root: &Path, config: &PlacementConfig) -> Result<()> {
    let Some(limits) = &config.offline_writes else {
        return Ok(());
    };
    let directory = root.join(".standalone-outbox").join(&config.id);
    if !directory.try_exists()? {
        return Ok(());
    }
    crate::outbox::private_directory(&root.join(".standalone-outbox"))?;
    crate::outbox::private_directory(&directory)?;
    for (count, entry) in std::fs::read_dir(directory)?.enumerate() {
        ensure!(count < 64, "Too many retained outbox scopes");
        let entry = entry?;
        ensure!(entry.file_type()?.is_dir(), "Invalid retained outbox scope");
        let name = entry.file_name();
        let scope = name.to_str().context("Invalid outbox scope name")?;
        crate::outbox::Outbox::open(root, &config.id, scope, limits.clone())?
            .quarantine("Project authorization was revoked; queued data remains on this device")?;
    }
    Ok(())
}

pub(crate) fn revoke_initialized_cache(state_dir: &Path, config: &PlacementConfig) -> Result<()> {
    let root = state_dir
        .join("placement-data")
        .join(&config.id)
        .join("current/store");
    if !root.try_exists()? {
        return Ok(());
    }
    crate::placement_data::validate_root(&root, config)?;
    let cache = cache::CacheControl::revoke_placement(&root, &config.id);
    let queue = quarantine_queues(&root, config);
    let metadata = outage::remove_snapshot(&root, &config.id);
    cache.and(queue).and(metadata)
}

async fn retain_metadata(
    credentials: &ProjectCredentials,
    metadata: Arc<dyn ObjectStore>,
) -> Result<Arc<dyn ObjectStore>> {
    let Some(snapshot) = &credentials.snapshot else {
        return Ok(metadata);
    };
    if credentials.grant_expires_at.is_none() {
        tracing::warn!(
            "API does not provide grant expiry; restart-safe outage recovery is unavailable until the API is updated"
        );
        return Ok(metadata);
    }
    match snapshot.persist(credentials, &metadata).await {
        Ok(()) => Ok(metadata),
        Err(error) if authorization_error(&error) == AuthorizationError::Unavailable => {
            let restored = snapshot.restore().await?;
            ensure!(
                restored.context.scope == credentials.scope,
                "Cached metadata authorization no longer matches cloud storage"
            );
            Ok(metadata)
        }
        Err(error) => {
            if authorization_error(&error) == AuthorizationError::Denied {
                credentials.revoke().await?;
            }
            Err(error)
        }
    }
}

pub(crate) struct PreflightCache {
    path: PathBuf,
}

impl PreflightCache {
    pub(crate) fn new(config: &PlacementConfig, identity: &WorkloadIdentity) -> Result<Self> {
        identity.validate()?;
        crate::config::validate_id("placement", &config.id)?;
        // A retry may reuse the same validation identity. Its partial metadata
        // must never become input to a later validation attempt.
        let name = format!(
            "preflight-{}-{}",
            identity.instance_id,
            uuid::Uuid::new_v4()
        );
        Ok(Self {
            path: private_cache(&config.project_path, &config.id, &name)?,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PreflightCache {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("Unable to remove staged validation cache: {error}");
            }
        }
    }
}

pub(crate) async fn configure_preflight(
    config: &PlacementConfig,
    authorizer: Arc<dyn RequestAuthorizer>,
    runtime: &mut FlowLikeConfig,
    root: &Path,
) -> Result<()> {
    let client = ProjectClient::new(authorizer)?;
    let metadata = Arc::new(LocalObjectStore::new(root.to_path_buf())?);
    // Validation still proves the live grant; returned metadata is never executable input.
    let _: serde_json::Value = client.fetch(reqwest::Method::GET, "app").await?;
    hydrate_metadata(&client, config, metadata.clone()).await?;
    runtime.register_app_meta_store(FlowLikeStore::Local(metadata).read_only());
    Ok(())
}

pub(crate) struct OnlineRuntime {
    pub(crate) registry: Arc<flow_like_storage::lance_io::object_store::ObjectStoreRegistry>,
    pub(crate) delegating_user_id: String,
    pub(crate) revoked: tokio_util::sync::CancellationToken,
}

fn shared_cloud_store(
    location: &flow_like_device_protocol::InstanceStorageLocation,
    stores: &mut BTreeMap<String, Arc<dyn ObjectStore>>,
    create: impl FnOnce() -> Result<Arc<dyn ObjectStore>>,
) -> Result<Arc<dyn ObjectStore>> {
    let uri = url::Url::parse(&location.uri)?;
    let domain = serde_json::to_string(&(
        uri.scheme(),
        uri.authority(),
        &location.options,
        &location.credential_id,
    ))?;
    match stores.entry(domain) {
        std::collections::btree_map::Entry::Occupied(entry) => Ok(entry.get().clone()),
        std::collections::btree_map::Entry::Vacant(entry) => Ok(entry.insert(create()?).clone()),
    }
}

pub(crate) async fn configure_with_local_data(
    config: &PlacementConfig,
    identity: &WorkloadIdentity,
    authorizer: Arc<dyn RequestAuthorizer>,
    runtime: &mut FlowLikeConfig,
    local_data: &Path,
    outage_authority: Option<Arc<dyn outage::OutageAuthority>>,
) -> Result<OnlineRuntime> {
    identity.validate()?;
    let client = ProjectClient::new(authorizer)?;
    let snapshot = outage_authority
        .map(|authority| {
            outage::SnapshotStore::new(local_data, config, identity, &client.base, authority)
        })
        .transpose()?;
    let (credentials, restored_metadata) =
        ProjectCredentials::initialize(client.clone(), config, identity, local_data, snapshot)
            .await?;
    let mut routes: Vec<(String, Arc<dyn ObjectStore>)> = Vec::new();
    let mut bindings = Vec::new();
    let mut cloud_stores = BTreeMap::new();
    for (purpose, location) in &credentials.locations {
        let binding = LanceStorageBinding::new(
            &location.uri,
            location.options.clone().into_iter().collect(),
            RenewableCredentials::new(credentials.provider(*purpose)?)?,
        )?
        .with_object_prefix(&location.prefix)?
        .with_http_connector(cache::OfflineConnector);
        let store = shared_cloud_store(location, &mut cloud_stores, || {
            let store = binding.build_store()?;
            let store = if credentials.access == OnlineProjectAccess::ReadOnly {
                FlowLikeStore::Other(store).read_only().as_generic()
            } else {
                store
            };
            Ok(cache::ReadCache::new(store, credentials.cache.clone()))
        })?;
        routes.push((location.prefix.clone(), store.clone()));
        bindings.push(binding.with_store(store));
    }
    let project = Arc::new(ProjectStore { routes });
    runtime.register_app_storage_store(FlowLikeStore::Other(project.clone()));
    runtime.register_user_store(FlowLikeStore::Other(project.clone()));
    runtime.register_temporary_store(FlowLikeStore::Other(project.clone()));
    let project_location = credentials
        .locations
        .get(&StoragePurpose::Storage)
        .context("Missing project database authorization")?
        .clone();
    let user_location = credentials
        .locations
        .get(&StoragePurpose::User)
        .context("Missing user database authorization")?
        .clone();
    runtime.register_build_project_database(database_builder(project_location));
    runtime.register_build_user_database(database_builder(user_location));
    // Outage snapshots authenticate resource context only. Reconstruct executable
    // objects from the controller-approved bundle on every startup, including outages.
    let restored_from_outage = restored_metadata.is_some();
    let metadata: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());
    hydrate_metadata(&client, config, metadata.clone()).await?;
    let metadata = if restored_from_outage {
        // The verified checkpoint retains its original grant deadline. Creating a
        // new seal requires fresh cloud authorization, which an outage cannot supply.
        metadata
    } else {
        retain_metadata(&credentials, metadata).await?
    };
    runtime.register_app_meta_store(
        FlowLikeStore::Other(Arc::new(outage::FencedMetadata::new(
            metadata,
            credentials.clone(),
        )))
        .read_only(),
    );
    let registry = scoped_registry(bindings)?;
    local_lance::configure(local_data, runtime, &registry)?;
    if let Some(buffering) = &config.offline_writes {
        let manager = writes::configure(
            config,
            identity,
            buffering,
            credentials.clone(),
            runtime,
            local_data,
            registry.clone(),
        )
        .await?;
        let files = writes::files::wrap(project, manager, &buffering.files)?;
        runtime.register_app_storage_store(FlowLikeStore::Other(files.clone()));
        runtime.register_user_store(FlowLikeStore::Other(files.clone()));
        runtime.register_temporary_store(FlowLikeStore::Other(files));
    }
    Ok(OnlineRuntime {
        registry,
        delegating_user_id: credentials.delegating_user_id.clone(),
        revoked: credentials.revoked.clone(),
    })
}

fn database_builder(
    location: flow_like_device_protocol::InstanceStorageLocation,
) -> Arc<dyn Fn(ObjectPath) -> flow_like_storage::lancedb::connection::ConnectBuilder + Send + Sync>
{
    Arc::new(move |path: ObjectPath| {
        let uri = database_uri(&location, &path);
        flow_like_storage::lancedb::connect(&uri)
    })
}

fn database_uri(
    location: &flow_like_device_protocol::InstanceStorageLocation,
    path: &ObjectPath,
) -> String {
    path.as_ref()
        .strip_prefix(&location.prefix)
        .map(|suffix| format!("{}{suffix}", location.uri))
        .unwrap_or_else(|| "unsupported://outside-device-storage".into())
}

pub(crate) fn validate_approved_metadata(config: &PlacementConfig) -> Result<()> {
    metadata::validate(config)
}

async fn hydrate_metadata(
    _client: &ProjectClient,
    config: &PlacementConfig,
    store: Arc<dyn ObjectStore>,
) -> Result<()> {
    metadata::hydrate(config, store).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{EventBinding, ProjectSource, ResourceGrantRef};
    use flow_like_types_contracts::authorization::{AuthorizationFuture, RequestAuthorization};
    use object_store::ObjectStoreExt;
    use std::{
        collections::BTreeMap,
        sync::atomic::{AtomicUsize, Ordering},
    };

    pub(super) fn approve_documents(
        config: &mut PlacementConfig,
        documents: BTreeMap<String, serde_json::Value>,
    ) {
        let bytes = serde_json::to_vec(&metadata::Bundle {
            version: 1,
            project_id: config.project_id.clone(),
            documents,
        })
        .unwrap();
        let root = config.project_path.join("apps").join(&config.project_id);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("online-metadata.json"), &bytes).unwrap();
        config.online_metadata_sha256 = Some(flow_like_device_protocol::artifact_sha256(&bytes));
    }

    pub(super) fn config(root: &Path) -> PlacementConfig {
        PlacementConfig {
            id: "placement".into(),
            project_id: "project".into(),
            deployment_id: "deployment".into(),
            revision: "revision".into(),
            source: ProjectSource::Online,
            online_metadata_sha256: Some("a".repeat(64)),
            project_path: root.into(),
            events: vec![EventBinding {
                event_id: "event".into(),
                event_version: [1, 0, 0],
                board_version: [1, 0, 0],
            }],
            artifact_pins: Vec::new(),
            package_pins: Vec::new(),
            bit_pins: Vec::new(),
            max_replicas: 1,
            hosting: None,
            variables: BTreeMap::new(),
            secret_overrides: BTreeMap::new(),
            resource_grant: Some(ResourceGrantRef {
                grant_id: "grant".into(),
                authz_version: 1,
                billing_grant_id: None,
                billing_authz_version: None,
            }),
            offline_writes: None,
            resources: None,
            restart: Default::default(),
        }
    }
    fn identity() -> WorkloadIdentity {
        WorkloadIdentity {
            instance_id: "instance".into(),
            device_id: "device".into(),
            device_auth_epoch: 1,
            key_epoch: 1,
        }
    }
    fn lease() -> InstanceStorageLease {
        let prefixes = [
            (StoragePurpose::Files, "apps/project/upload/"),
            (StoragePurpose::Storage, "apps/project/storage/"),
            (StoragePurpose::User, "users/owner/apps/project/"),
            (StoragePurpose::Temporary, "tmp/user/owner/apps/project/"),
        ];
        InstanceStorageLease {
            instance_id: "instance".into(),
            device_id: "device".into(),
            device_auth_epoch: 1,
            key_epoch: 1,
            grant_id: "grant".into(),
            authz_version: 1,
            project_id: "project".into(),
            placement_id: "placement".into(),
            deployment_id: "deployment".into(),
            delegating_user_id: "owner".into(),
            access: OnlineProjectAccess::ReadWrite,
            expires_at: unix_time().unwrap() + 3600,
            grant_expires_at: Some(unix_time().unwrap() + 86400),
            locations: prefixes
                .into_iter()
                .map(|(purpose, prefix)| {
                    (
                        purpose,
                        flow_like_device_protocol::InstanceStorageLocation {
                            uri: format!("s3://content/{prefix}"),
                            prefix: prefix.into(),
                            credential_id: "device".into(),
                            options: BTreeMap::from([("aws_region".into(), "eu-central-1".into())]),
                        },
                    )
                })
                .collect(),
            credentials: BTreeMap::from([(
                "device".into(),
                InstanceStorageCredential::AwsSession {
                    access_key_id: "initial".into(),
                    secret_access_key: "secret".into(),
                    session_token: "session".into(),
                },
            )]),
        }
    }

    #[test]
    fn lease_requires_the_existing_identity_and_all_narrow_device_directories() {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let identity = identity();
        let original = lease();
        validate_lease(&config, &identity, &original).unwrap();
        for (field, value) in [
            ("instance_id", serde_json::json!("another")),
            ("device_id", serde_json::json!("another")),
            ("authz_version", serde_json::json!(2)),
            ("placement_id", serde_json::json!("another")),
            ("expires_at", serde_json::json!(unix_time().unwrap() + 3601)),
            ("delegating_user_id", serde_json::json!("another")),
        ] {
            let mut changed = serde_json::to_value(&original).unwrap();
            changed[field] = value;
            assert!(
                validate_lease(
                    &config,
                    &identity,
                    &serde_json::from_value(changed).unwrap(),
                )
                .is_err(),
                "accepted {field}"
            );
        }
        for purpose in [
            StoragePurpose::Files,
            StoragePurpose::Storage,
            StoragePurpose::User,
            StoragePurpose::Temporary,
        ] {
            let mut changed = original.clone();
            changed.locations.remove(&purpose);
            assert!(validate_lease(&config, &identity, &changed).is_err());
            let mut changed = original.clone();
            changed
                .locations
                .get_mut(&purpose)
                .unwrap()
                .prefix
                .push_str("other/");
            assert!(validate_lease(&config, &identity, &changed).is_err());
        }
        let mut changed = original.clone();
        changed
            .locations
            .get_mut(&StoragePurpose::Storage)
            .unwrap()
            .uri = "s3://content/apps/project/storage-other/".into();
        assert!(validate_lease(&config, &identity, &changed).is_err());
        let mut changed = original;
        changed
            .locations
            .get_mut(&StoragePurpose::Files)
            .unwrap()
            .options
            .insert("aws_endpoint".into(), "http://untrusted.example".into());
        assert!(validate_lease(&config, &identity, &changed).is_err());
    }

    #[test]
    fn lease_accepts_unicode_subjects_but_rejects_ambiguous_directories() {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let identity = identity();
        let mut original = lease();
        original.delegating_user_id = "josé:团队|42".into();
        for (purpose, prefix) in [
            (
                StoragePurpose::User,
                user_prefix(&original.delegating_user_id, "project"),
            ),
            (
                StoragePurpose::Temporary,
                temporary_prefix(&original.delegating_user_id, "project"),
            ),
        ] {
            let location = original.locations.get_mut(&purpose).unwrap();
            location.uri = format!("s3://content/{prefix}");
            location.prefix = prefix;
        }
        validate_lease(&config, &identity, &original).unwrap();
        let user = original.locations.get_mut(&StoragePurpose::User).unwrap();
        user.uri = url::Url::parse(&user.uri).unwrap().to_string();
        validate_lease(&config, &identity, &original).unwrap();
        let mut transport_encoded = original.clone();
        let user = transport_encoded
            .locations
            .get_mut(&StoragePurpose::User)
            .unwrap();
        user.uri = user.uri.replace('%', "%25");
        assert!(validate_lease(&config, &identity, &transport_encoded).is_err());
        let mut raw_alias = original.clone();
        let user = raw_alias.locations.get_mut(&StoragePurpose::User).unwrap();
        user.uri = "s3://content/users/josé:团队|42/apps/project/".into();
        user.prefix = "users/josé:团队|42/apps/project/".into();
        assert!(validate_lease(&config, &identity, &raw_alias).is_err());

        for uri in [
            "s3://content/users/stranger/../owner/apps/project/",
            "s3://content/users/stranger/%2e%2e/owner/apps/project/",
            "s3://content/users%2fowner/apps/project/",
            "s3://content/users%2Fowner/apps/project/",
            "s3://content/users/%252fowner/apps/project/",
            "s3://content/users/%5cowner/apps/project/",
        ] {
            let mut invalid = lease();
            invalid
                .locations
                .get_mut(&StoragePurpose::User)
                .unwrap()
                .uri = uri.into();
            assert!(
                validate_lease(&config, &identity, &invalid).is_err(),
                "accepted {uri}"
            );
        }
        for sub in [
            "owner/other",
            "owner\\other",
            "owner..other",
            "owner%2fother",
            "owner\0",
            "owner?",
            "owner*",
        ] {
            let mut invalid = lease();
            invalid.delegating_user_id = sub.into();
            assert!(
                validate_lease(&config, &identity, &invalid).is_err(),
                "accepted invalid subject"
            );
        }
    }

    #[test]
    fn persistent_cache_scope_separates_users_grants_projects_and_clouds() {
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let identity = identity();
        let original = lease();
        let scope = cache_scope(&config, &identity, &original, "https://api.example").unwrap();
        assert_ne!(
            cache_scope(&config, &identity, &original, "https://another-api.example").unwrap(),
            scope
        );
        let mut restarted = identity.clone();
        restarted.instance_id = "replacement".into();
        assert_eq!(
            cache_scope(&config, &restarted, &original, "https://api.example").unwrap(),
            scope
        );
        for (field, value) in [
            ("delegating_user_id", serde_json::json!("other")),
            ("grant_id", serde_json::json!("other")),
            ("authz_version", serde_json::json!(2)),
            ("access", serde_json::json!("read_only")),
        ] {
            let mut changed = serde_json::to_value(&original).unwrap();
            changed[field] = value;
            assert_ne!(
                cache_scope(
                    &config,
                    &identity,
                    &serde_json::from_value(changed).unwrap(),
                    "https://api.example",
                )
                .unwrap(),
                scope
            );
        }
        let mut changed = original;
        changed
            .locations
            .get_mut(&StoragePurpose::Storage)
            .unwrap()
            .uri = "s3://other/apps/project/storage/".into();
        assert_ne!(
            cache_scope(&config, &identity, &changed, "https://api.example").unwrap(),
            scope
        );
    }

    #[test]
    fn read_cache_budget_has_a_bounded_default_and_actionable_validation() {
        assert_eq!(parse_read_cache_budget(None).unwrap(), 512 * 1024 * 1024);
        assert_eq!(
            parse_read_cache_budget(Some("16777216")).unwrap(),
            16 * 1024 * 1024
        );
        assert_eq!(
            parse_read_cache_budget(Some("68719476736")).unwrap(),
            64 * 1024 * 1024 * 1024
        );
        for invalid in [
            "",
            "-1",
            "100",
            "+16777216",
            "16MB",
            "68719476737",
            " 16777216",
        ] {
            let error = parse_read_cache_budget(Some(invalid)).unwrap_err();
            assert!(error.to_string().contains(READ_CACHE_BUDGET_VARIABLE));
        }
    }

    #[tokio::test]
    async fn routed_store_denies_other_projects_and_cross_purpose_mutations() {
        let storage: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());
        let files: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());
        let router = ProjectStore {
            routes: vec![
                ("apps/project/storage/".into(), storage.clone()),
                ("apps/project/upload/".into(), files),
            ],
        };
        let allowed = ObjectPath::from("apps/project/storage/table");
        router.put(&allowed, "value".into()).await.unwrap();
        assert_eq!(
            router
                .get(&allowed)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap()
                .as_ref(),
            b"value"
        );
        for path in [
            "apps/other/storage/table",
            "apps/project/storage-neighbor/table",
            "users/owner/private",
            "apps/project/manifest.app",
        ] {
            assert!(
                router
                    .put(&ObjectPath::from(path), "value".into())
                    .await
                    .is_err()
            );
            assert!(router.get(&ObjectPath::from(path)).await.is_err());
        }
        assert!(
            router
                .copy(&allowed, &ObjectPath::from("apps/project/upload/file"))
                .await
                .is_err()
        );
        assert!(router.list(None).next().await.unwrap().is_err());
        assert!(
            router
                .list(Some(&ObjectPath::from("apps/project/storage")))
                .next()
                .await
                .unwrap()
                .is_ok()
        );
    }

    #[tokio::test]
    async fn one_cloud_client_allows_copy_between_its_authorized_directories() {
        let lease = lease();
        let calls = AtomicUsize::new(0);
        let mut cloud_stores = BTreeMap::new();
        let mut routes = Vec::new();
        for location in lease.locations.values() {
            let store = shared_cloud_store(location, &mut cloud_stores, || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(object_store::memory::InMemory::new()))
            })
            .unwrap();
            routes.push((location.prefix.clone(), store));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(cloud_stores.len(), 1);
        let router = ProjectStore { routes };
        let source = ObjectPath::from("apps/project/upload/source");
        router.put(&source, "shared payload".into()).await.unwrap();
        for location in lease.locations.values() {
            let target = ObjectPath::from(format!("{}copied", location.prefix));
            router.copy(&source, &target).await.unwrap();
            assert_eq!(
                router
                    .get(&target)
                    .await
                    .unwrap()
                    .bytes()
                    .await
                    .unwrap()
                    .as_ref(),
                b"shared payload"
            );
        }
        for target in [
            "apps/other/storage/file",
            "users/other/apps/project/file",
            "tmp/global/apps/project/file",
            "apps/project/metadata/en.meta",
        ] {
            assert!(
                router
                    .copy(&source, &ObjectPath::from(target))
                    .await
                    .is_err()
            );
        }

        // Directory SAS credentials and other buckets require distinct clients.
        let original = router.route(&source).unwrap();
        let mut location = lease.locations[&StoragePurpose::Files].clone();
        location.credential_id = "different-directory-sas".into();
        let isolated = shared_cloud_store(&location, &mut cloud_stores, || {
            Ok(Arc::new(object_store::memory::InMemory::new()))
        })
        .unwrap();
        assert!(!Arc::ptr_eq(&original, &isolated));
        location = lease.locations[&StoragePurpose::Files].clone();
        location.uri = "s3://different-bucket/apps/project/upload/".into();
        let isolated = shared_cloud_store(&location, &mut cloud_stores, || {
            Ok(Arc::new(object_store::memory::InMemory::new()))
        })
        .unwrap();
        assert!(!Arc::ptr_eq(&original, &isolated));
        assert_eq!(cloud_stores.len(), 3);
    }

    #[test]
    fn user_database_uses_the_same_cloud_path_as_server_execution() {
        let lease = lease();
        let user = &lease.locations[&StoragePurpose::User];
        assert_eq!(
            database_uri(user, &ObjectPath::from("users/owner/apps/project/db")),
            "s3://content/users/owner/apps/project/db"
        );
        assert_eq!(
            database_uri(
                &lease.locations[&StoragePurpose::Storage],
                &ObjectPath::from("apps/project/storage/db")
            ),
            "s3://content/apps/project/storage/db"
        );
        for path in [
            "users/other/apps/project/db",
            "users/owner/apps/project-other/db",
            "users/owner/apps/project2/db",
            "user/db",
        ] {
            assert!(database_uri(user, &ObjectPath::from(path)).starts_with("unsupported://"));
        }
        let mut auth0 = user.clone();
        auth0.prefix = user_prefix("auth0|user", "project");
        auth0.uri = format!("s3://content/{}", auth0.prefix);
        let path = ObjectPath::from("users")
            .join("auth0|user")
            .join("apps")
            .join("project")
            .join("db");
        assert_eq!(path.as_ref(), "users/auth0%7Cuser/apps/project/db");
        assert_eq!(
            database_uri(&auth0, &path),
            "s3://content/users/auth0%7Cuser/apps/project/db"
        );
        assert!(
            database_uri(
                &auth0,
                &ObjectPath::parse("users/auth0|user/apps/project/db").unwrap()
            )
            .starts_with("unsupported://")
        );
    }

    struct TestAuthorizer {
        base: String,
    }

    #[derive(Default)]
    struct SnapshotAuthority {
        denied: std::sync::atomic::AtomicBool,
    }
    #[async_trait]
    impl outage::OutageAuthority for SnapshotAuthority {
        async fn seal(&self, claim: &outage::SnapshotClaim) -> Result<String> {
            use hmac::Mac;
            ensure!(
                !self.denied.load(Ordering::SeqCst),
                AuthorizationError::Denied
            );
            let mut mac = <hmac::Hmac<sha2::Sha256> as Mac>::new_from_slice(&[7; 32])?;
            mac.update(&serde_json::to_vec(claim)?);
            Ok(blake3::Hash::from_bytes(mac.finalize().into_bytes().into())
                .to_hex()
                .to_string())
        }
        async fn verify(&self, claim: &outage::SnapshotClaim, seal: &str) -> Result<()> {
            ensure!(self.seal(claim).await? == seal, "Corrupted saved metadata");
            ensure!(
                claim.grant_expires_at > unix_time()?,
                AuthorizationError::Denied
            );
            Ok(())
        }
        async fn deny(&self, _: &str) -> Result<()> {
            self.denied.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn offline_restart_restores_metadata_and_cached_reads_then_refreshes_new_instance()
    -> Result<()> {
        use axum::response::IntoResponse;
        let root = tempfile::tempdir()?;
        let config = config(root.path());
        let response = Arc::new(Mutex::new(lease()));
        let status = Arc::new(std::sync::atomic::AtomicU16::new(200));
        let api_response = response.clone();
        let api_status = status.clone();
        let router = axum::Router::new().route(
            "/instances/project/storage",
            axum::routing::post(move || {
                let response = api_response.clone();
                let status = api_status.clone();
                async move {
                    let status =
                        axum::http::StatusCode::from_u16(status.load(Ordering::SeqCst)).unwrap();
                    if status.is_success() {
                        axum::Json(response.lock().await.clone()).into_response()
                    } else {
                        status.into_response()
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}/instances/project", listener.local_addr()?);
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = ProjectClient::new(Arc::new(TestAuthorizer { base: base.clone() }))?;
        let authority = Arc::new(SnapshotAuthority::default());
        let snapshot = outage::SnapshotStore::new(
            root.path(),
            &config,
            &identity(),
            &base,
            authority.clone(),
        )?;
        let (first, restored) = ProjectCredentials::initialize(
            client.clone(),
            &config,
            &identity(),
            root.path(),
            Some(snapshot.clone()),
        )
        .await?;
        assert!(restored.is_none());
        let metadata: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());
        let manifest = ObjectPath::from("apps/project/manifest.app");
        metadata
            .put(&manifest, "exact pinned metadata".into())
            .await?;
        snapshot.persist(&first, &metadata).await?;
        let saved = std::fs::read_to_string(
            root.path()
                .join(".standalone-cache/placement/outage/snapshot.json"),
        )?;
        assert!(
            !saved.contains("session_token")
                && !saved.contains("secret_access_key")
                && !saved.contains("credentials")
        );
        let cloud: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());
        let path = ObjectPath::from("apps/project/upload/asset");
        cloud.put(&path, "cached bytes".into()).await?;
        let cache = cache::ReadCache::new(cloud, first.cache.clone());
        assert_eq!(
            cache.get(&path).await?.bytes().await?.as_ref(),
            b"cached bytes"
        );
        drop(cache);
        drop(first);

        status.store(503, Ordering::SeqCst);
        let mut restarted = identity();
        restarted.instance_id = "after-reboot".into();
        let snapshot =
            outage::SnapshotStore::new(root.path(), &config, &restarted, &base, authority.clone())?;
        let (second, restored) = ProjectCredentials::initialize(
            client.clone(),
            &config,
            &restarted,
            root.path(),
            Some(snapshot.clone()),
        )
        .await?;
        assert!(second.state.lock().await.lease.is_none());
        assert_eq!(second.delegating_user_id, "owner");
        assert_eq!(
            restored
                .unwrap()
                .get(&manifest)
                .await?
                .bytes()
                .await?
                .as_ref(),
            b"exact pinned metadata"
        );
        let location = &second.locations[&StoragePurpose::Files];
        let binding = LanceStorageBinding::new(
            &location.uri,
            location.options.clone().into_iter().collect(),
            RenewableCredentials::new(second.provider(StoragePurpose::Files)?)?,
        )?;
        let cached = cache::ReadCache::new(binding.build_store()?, second.cache.clone());
        assert_eq!(
            cached.get(&path).await?.bytes().await?.as_ref(),
            b"cached bytes"
        );
        let error = cached
            .get(&ObjectPath::from("apps/project/upload/never-downloaded"))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("offline"));
        assert!(
            cached
                .put(&path, "must not become a cloud write".into())
                .await
                .is_err()
        );
        assert_eq!(
            second.refresh(true).await,
            Err(AuthorizationError::Unavailable)
        );
        assert!(second.state.lock().await.next_refresh > unix_time()?);
        response.lock().await.instance_id = restarted.instance_id.clone();
        status.store(200, Ordering::SeqCst);
        second.refresh(true).await?;
        assert_eq!(
            second
                .provider(StoragePurpose::Files)?
                .credential()
                .await?
                .scope
                .instance_id,
            restarted.instance_id
        );
        status.store(403, Ordering::SeqCst);
        assert_eq!(second.refresh(true).await, Err(AuthorizationError::Denied));
        assert!(second.cache.is_revoked());
        assert!(
            !root
                .path()
                .join(".standalone-cache/placement/outage/snapshot.json")
                .exists()
        );
        // Even restoring an earlier, authentic file cannot remove the parent's fence.
        crate::vault::write_new_private(
            &root
                .path()
                .join(".standalone-cache/placement/outage/snapshot.json"),
            saved.as_bytes(),
        )?;
        status.store(503, Ordering::SeqCst);
        assert!(
            ProjectCredentials::initialize(
                client,
                &config,
                &restarted,
                root.path(),
                Some(snapshot)
            )
            .await
            .is_err()
        );
        server.abort();
        Ok(())
    }

    #[test]
    fn outage_binding_requires_exact_configuration_and_device_epochs() -> Result<()> {
        let root = tempfile::tempdir()?;
        let config = config(root.path());
        let identity = identity();
        let base = "https://api.example/instances/project";
        let expected = outage::binding(&config, &identity, base)?;
        let mut restarted = identity.clone();
        restarted.instance_id = "new-instance".into();
        assert_eq!(outage::binding(&config, &restarted, base)?, expected);
        for field in ["device_id", "device_auth_epoch", "key_epoch"] {
            let mut changed = serde_json::to_value(&identity)?;
            changed[field] = if field == "device_id" {
                serde_json::json!("other")
            } else {
                serde_json::json!(2)
            };
            assert_ne!(
                outage::binding(&config, &serde_json::from_value(changed)?, base)?,
                expected
            );
        }
        for field in ["id", "project_id", "deployment_id", "revision"] {
            let mut changed = serde_json::to_value(&config)?;
            changed[field] = serde_json::json!("other");
            assert_ne!(
                outage::binding(&serde_json::from_value(changed)?, &identity, base)?,
                expected
            );
        }
        let mut changed = config.clone();
        changed.resource_grant.as_mut().unwrap().authz_version += 1;
        assert_ne!(outage::binding(&changed, &identity, base)?, expected);
        let mut changed = config.clone();
        changed.events[0].board_version = [2, 0, 0];
        assert_ne!(outage::binding(&changed, &identity, base)?, expected);
        assert_ne!(
            outage::binding(
                &config,
                &identity,
                "https://other.example/instances/project"
            )?,
            expected
        );
        Ok(())
    }
    impl RequestAuthorizer for TestAuthorizer {
        fn resource_base_url(&self, audience: ResourceAudience) -> Option<String> {
            (audience == ResourceAudience::ProjectApi).then(|| self.base.clone())
        }
        fn authorize<'a>(&'a self, request: AuthorizationRequest<'a>) -> AuthorizationFuture<'a> {
            Box::pin(async move {
                if request.audience != ResourceAudience::ProjectApi
                    || !((request.method == "POST"
                        && request.url == format!("{}/storage", self.base))
                        || (request.method == "GET"
                            && request.url.starts_with(&format!("{}/", self.base))))
                {
                    return Err(AuthorizationError::InvalidRequest);
                }
                RequestAuthorization::new(
                    "DPoP test".into(),
                    Some("test-proof".into()),
                    UNIX_EPOCH + Duration::from_secs((unix_time().unwrap() + 180) as u64),
                )
            })
        }
    }
    #[tokio::test]
    async fn background_refresh_keeps_valid_credentials_through_failure_and_denial_wipes_cache() {
        use axum::response::IntoResponse;
        let root = tempfile::tempdir().unwrap();
        let config = config(root.path());
        let responses = Arc::new(Mutex::new(lease()));
        let status = Arc::new(std::sync::atomic::AtomicU16::new(200));
        let count = Arc::new(AtomicUsize::new(0));
        let response_state = responses.clone();
        let calls = count.clone();
        let response_status = status.clone();
        let app = axum::Router::new().route(
            "/instances/project/storage",
            axum::routing::post(move || {
                let response = response_state.clone();
                let calls = calls.clone();
                let status = response_status.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let status =
                        axum::http::StatusCode::from_u16(status.load(Ordering::SeqCst)).unwrap();
                    if status.is_success() {
                        axum::Json(response.lock().await.clone()).into_response()
                    } else {
                        status.into_response()
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = ProjectClient::new(Arc::new(TestAuthorizer {
            base: format!("http://{address}/instances/project"),
        }))
        .unwrap();
        let project = ProjectCredentials::new(client, &config, &identity(), root.path())
            .await
            .unwrap();
        let retained =
            RenewableCredentials::new(project.provider(StoragePurpose::Storage).unwrap())
                .unwrap()
                .aws();
        let user = RenewableCredentials::new(project.provider(StoragePurpose::User).unwrap())
            .unwrap()
            .aws();
        for _ in 0..3 {
            assert_eq!(retained.get_credential().await.unwrap().key_id, "initial");
            assert_eq!(user.get_credential().await.unwrap().key_id, "initial");
        }
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "Runs and directories share one issuance"
        );
        status.store(503, Ordering::SeqCst);
        assert_eq!(
            project.refresh(true).await,
            Err(AuthorizationError::Unavailable)
        );
        assert_eq!(retained.get_credential().await.unwrap().key_id, "initial");
        assert!(project.state.lock().await.lease.is_some());
        assert!(project.state.lock().await.next_refresh > unix_time().unwrap());
        let attempts = count.load(Ordering::SeqCst);
        project.refresh(false).await.unwrap();
        assert_eq!(
            count.load(Ordering::SeqCst),
            attempts,
            "Backoff bounds outage traffic"
        );
        status.store(200, Ordering::SeqCst);
        responses.lock().await.credentials.insert(
            "device".into(),
            InstanceStorageCredential::AwsSession {
                access_key_id: "refreshed".into(),
                secret_access_key: "secret".into(),
                session_token: "new-session".into(),
            },
        );
        project.refresh(true).await.unwrap();
        assert_eq!(retained.get_credential().await.unwrap().key_id, "refreshed");
        assert_eq!(user.get_credential().await.unwrap().key_id, "refreshed");
        responses
            .lock()
            .await
            .locations
            .get_mut(&StoragePurpose::Storage)
            .unwrap()
            .uri = "s3://replacement/apps/project/storage/".into();
        assert_eq!(
            project.refresh(true).await,
            Err(AuthorizationError::InvalidResponse)
        );
        assert_eq!(retained.get_credential().await.unwrap().key_id, "refreshed");
        status.store(503, Ordering::SeqCst);
        project
            .state
            .lock()
            .await
            .lease
            .as_mut()
            .unwrap()
            .expires_at = unix_time().unwrap() - 1;
        assert!(
            retained.get_credential().await.is_err(),
            "Expired credentials cannot be returned"
        );
        status.store(403, Ordering::SeqCst);
        assert!(
            ProjectCredentials::new(project.client.clone(), &config, &identity(), root.path())
                .await
                .is_err()
        );
        assert!(
            project.cache.is_revoked(),
            "A denied replacement wipes existing scope caches"
        );
        assert_eq!(project.refresh(true).await, Err(AuthorizationError::Denied));
        assert!(project.cache.is_revoked());
        assert!(project.state.lock().await.lease.is_none());
        assert!(retained.get_credential().await.is_err());
        server.abort();
    }

    #[tokio::test]
    async fn background_refresh_runs_without_a_storage_request() {
        let root = tempfile::tempdir().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        let router = axum::Router::new().route(
            "/instances/project/storage",
            axum::routing::post(move || {
                let calls = calls.clone();
                async move {
                    let generation = calls.fetch_add(1, Ordering::SeqCst);
                    let mut lease = lease();
                    lease.expires_at =
                        unix_time().unwrap() + if generation == 0 { 3 } else { 3600 };
                    axum::Json(lease)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = ProjectClient::new(Arc::new(TestAuthorizer {
            base: format!("http://{address}/instances/project"),
        }))
        .unwrap();
        let project =
            ProjectCredentials::new(client, &config(root.path()), &identity(), root.path())
                .await
                .unwrap();
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if count.load(Ordering::SeqCst) >= 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        // No object or SDK credential request drove this renewal.
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if project
                    .state
                    .lock()
                    .await
                    .lease
                    .as_ref()
                    .unwrap()
                    .expires_at
                    > unix_time().unwrap() + 3000
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn old_api_reports_device_execute_requirement_without_local_fallback() {
        let root = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server =
            tokio::spawn(async move { axum::serve(listener, axum::Router::new()).await.unwrap() });
        let client = ProjectClient::new(Arc::new(TestAuthorizer {
            base: format!("http://{address}/instances/project"),
        }))
        .unwrap();
        let error = ProjectCredentials::new(client, &config(root.path()), &identity(), root.path())
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("DeviceExecute"));
        assert!(!root.path().join(".standalone-cache").exists());
        server.abort();
    }

    #[cfg(unix)]
    #[test]
    fn private_cache_rejects_symlink_substitution() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join(".standalone-cache")).unwrap();
        assert!(private_cache(root.path(), "placement", "instance").is_err());
    }

    #[tokio::test]
    async fn online_configuration_uses_cloud_user_and_temporary_stores_but_keeps_logs_local() {
        use flow_like_runtime::{
            app::AppVisibility, bit::Metadata, state::FlowLikeState, utils::http::HTTPClient,
        };
        use flow_like_storage::lance_io::object_store::ObjectStoreParams;
        let directory = tempfile::tempdir().unwrap();
        let persistent = tempfile::tempdir().unwrap();
        let local: Arc<dyn ObjectStore> = Arc::new(object_store::memory::InMemory::new());
        let source = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(FlowLikeStore::Other(local.clone())),
            HTTPClient::new_without_refetch(),
        ));
        let mut app = App::new(Some("project".into()), Metadata::default(), vec![], source)
            .await
            .unwrap();
        app.visibility = AppVisibility::Private;
        let approved_app = serde_json::to_value(&app).unwrap();
        let router = axum::Router::new()
            .route(
                "/instances/project/storage",
                axum::routing::post(|| async { axum::Json(lease()) }),
            )
            .route(
                "/instances/project/app",
                axum::routing::get(move || {
                    let app = app.clone();
                    async move { axum::Json(app) }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut placement = config(directory.path());
        placement.events.clear();
        approve_documents(
            &mut placement,
            BTreeMap::from([("app".into(), approved_app)]),
        );
        let mut runtime = FlowLikeConfig::with_default_store(FlowLikeStore::Other(local.clone()));
        let online = configure_with_local_data(
            &placement,
            &identity(),
            Arc::new(TestAuthorizer {
                base: format!("http://{address}/instances/project"),
            }),
            &mut runtime,
            persistent.path(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(online.delegating_user_id, "owner");
        for store in [
            runtime.stores.app_storage_store.as_ref().unwrap(),
            runtime.stores.user_store.as_ref().unwrap(),
            runtime.stores.temporary_store.as_ref().unwrap(),
        ] {
            assert!(!Arc::ptr_eq(&store.as_generic(), &local));
            assert!(
                store
                    .as_generic()
                    .head(&ObjectPath::from("users/other/apps/project/file"))
                    .await
                    .is_err()
            );
        }
        assert!(Arc::ptr_eq(
            &runtime.stores.log_store.as_ref().unwrap().as_generic(),
            &local
        ));
        let params = ObjectStoreParams::default();
        assert!(
            online
                .registry
                .get_store(
                    url::Url::parse("s3://content/users/owner/apps/project/db/table.lance")
                        .unwrap(),
                    &params
                )
                .await
                .is_ok()
        );
        assert!(
            online
                .registry
                .get_store(
                    url::Url::parse("s3://content/users/other/apps/project/db/table.lance")
                        .unwrap(),
                    &params
                )
                .await
                .is_err()
        );
        assert!(
            online
                .registry
                .get_store(
                    url::Url::parse("standalone-local://runtime/user/db").unwrap(),
                    &params
                )
                .await
                .is_err()
        );
        let logs = online
            .registry
            .get_store(
                url::Url::parse("standalone-local://runtime/logs/history").unwrap(),
                &params,
            )
            .await
            .unwrap();
        logs.inner
            .put(
                &ObjectPath::from("logs/history/example"),
                bytes::Bytes::from_static(b"local log").into(),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(persistent.path().join("logs/history/example")).unwrap(),
            b"local log"
        );
        assert!(
            persistent
                .path()
                .join(".standalone-cache/placement")
                .is_dir()
        );
        assert!(
            !directory.path().join(".standalone-cache").exists(),
            "Runtime metadata must not write into the immutable project revision"
        );
        assert!(!directory.path().join("logs").exists());
        assert!(!directory.path().join("user").exists());
        assert!(!directory.path().join("tmp").exists());
        server.abort();
    }

    #[tokio::test]
    async fn approved_metadata_recovers_without_resealing_and_still_rejects_tampering() -> Result<()>
    {
        use axum::response::IntoResponse;
        use flow_like_runtime::{
            app::AppVisibility, bit::Metadata, state::FlowLikeState, utils::http::HTTPClient,
        };

        #[derive(Default)]
        struct InitialSealOnly {
            inner: SnapshotAuthority,
            calls: AtomicUsize,
        }
        #[async_trait]
        impl outage::OutageAuthority for InitialSealOnly {
            async fn seal(&self, claim: &outage::SnapshotClaim) -> Result<String> {
                ensure!(
                    self.calls.fetch_add(1, Ordering::SeqCst) == 0,
                    "Fresh authorization is unavailable for another seal"
                );
                outage::OutageAuthority::seal(&self.inner, claim).await
            }
            async fn verify(&self, claim: &outage::SnapshotClaim, seal: &str) -> Result<()> {
                outage::OutageAuthority::verify(&self.inner, claim, seal).await
            }
            async fn deny(&self, binding: &str) -> Result<()> {
                outage::OutageAuthority::deny(&self.inner, binding).await
            }
        }

        let revision = tempfile::tempdir()?;
        let data = tempfile::tempdir()?;
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(FlowLikeStore::Memory(Arc::new(
                object_store::memory::InMemory::new(),
            ))),
            HTTPClient::new_without_refetch(),
        ));
        let mut app = App::new(Some("project".into()), Metadata::default(), vec![], state).await?;
        app.visibility = AppVisibility::Private;
        let mut placement = config(revision.path());
        placement.events.clear();
        approve_documents(
            &mut placement,
            BTreeMap::from([("app".into(), serde_json::to_value(app)?)]),
        );
        let status = Arc::new(std::sync::atomic::AtomicU16::new(200));
        let api_status = status.clone();
        let router = axum::Router::new().route(
            "/instances/project/storage",
            axum::routing::post(move || {
                let status = api_status.clone();
                async move {
                    if status.load(Ordering::SeqCst) == 200 {
                        axum::Json(lease()).into_response()
                    } else {
                        axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response()
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let authorizer = Arc::new(TestAuthorizer {
            base: format!("http://{}/instances/project", listener.local_addr()?),
        });
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let authority = Arc::new(InitialSealOnly::default());
        let mut first = FlowLikeConfig::new();
        let live = configure_with_local_data(
            &placement,
            &identity(),
            authorizer.clone(),
            &mut first,
            data.path(),
            Some(authority.clone()),
        )
        .await?;
        let checkpoint = data
            .path()
            .join(".standalone-cache/placement/outage/snapshot.json");
        let saved = std::fs::read(&checkpoint)?;
        drop(first);
        drop(live);
        status.store(503, Ordering::SeqCst);
        let mut restarted = identity();
        restarted.instance_id = "after-outage-restart".into();
        let mut offline = FlowLikeConfig::new();
        let recovered = configure_with_local_data(
            &placement,
            &restarted,
            authorizer.clone(),
            &mut offline,
            data.path(),
            Some(authority.clone()),
        )
        .await?;
        assert_eq!(recovered.delegating_user_id, "owner");
        assert!(
            offline
                .stores
                .app_meta_store
                .as_ref()
                .unwrap()
                .as_generic()
                .head(&ObjectPath::from("apps/project/manifest.app"))
                .await
                .is_ok()
        );
        assert_eq!(authority.calls.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(&checkpoint)?, saved);

        let approved_path = revision.path().join("apps/project/online-metadata.json");
        let mut substituted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&approved_path)?)?;
        substituted["documents"]["app"]["frontend"] =
            serde_json::json!({"landing_page":"substituted"});
        std::fs::write(&approved_path, serde_json::to_vec(&substituted)?)?;
        let mut rejected = FlowLikeConfig::new();
        let error = configure_with_local_data(
            &placement,
            &restarted,
            authorizer,
            &mut rejected,
            data.path(),
            Some(authority.clone()),
        )
        .await
        .err()
        .context("Tampered approved bytes were accepted during outage")?;
        assert!(error.to_string().contains("digest differs"));
        assert_eq!(authority.calls.load(Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(&checkpoint)?, saved);
        server.abort();
        Ok(())
    }

    #[test]
    fn preflight_cache_is_unique_and_removes_only_its_own_files() {
        let root = tempfile::tempdir().unwrap();
        let placement = config(root.path());
        let live = private_cache(root.path(), "placement", "instance").unwrap();
        std::fs::write(live.join("manifest"), b"live metadata").unwrap();
        let first = PreflightCache::new(&placement, &identity()).unwrap();
        let first_path = first.path().to_owned();
        let second = PreflightCache::new(&placement, &identity()).unwrap();
        let second_path = second.path().to_owned();
        assert_ne!(first_path, second_path);
        std::fs::create_dir_all(first_path.join("bits/deps-cache")).unwrap();
        std::fs::write(first_path.join("bits/deps-cache/partial"), b"partial").unwrap();
        drop(first);
        assert!(!first_path.exists());
        assert!(second_path.is_dir());
        drop(second);
        assert!(!second_path.exists());
        assert_eq!(
            std::fs::read(live.join("manifest")).unwrap(),
            b"live metadata"
        );
    }

    #[tokio::test]
    async fn cancelled_online_preflight_removes_its_partial_cache() {
        let directory = tempfile::tempdir().unwrap();
        let placement = config(directory.path());
        let called = Arc::new(tokio::sync::Notify::new());
        let pending = called.clone();
        let router = axum::Router::new().route(
            "/instances/project/app",
            axum::routing::get(move || {
                let called = pending.clone();
                async move {
                    called.notify_one();
                    std::future::pending::<axum::http::StatusCode>().await
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let validation = tokio::spawn(async move {
            crate::runtime::validate_rollout_authorized(
                &placement,
                Some(Arc::new(TestAuthorizer {
                    base: format!("http://{address}/instances/project"),
                })),
                Some(identity()),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(5), called.notified())
            .await
            .unwrap();
        let cache_root = directory.path().join(".standalone-cache/placement");
        assert_eq!(std::fs::read_dir(&cache_root).unwrap().count(), 1);
        validation.abort();
        assert!(validation.await.unwrap_err().is_cancelled());
        assert_eq!(std::fs::read_dir(&cache_root).unwrap().count(), 0);
        server.abort();
    }

    #[tokio::test]
    async fn project_cache_contains_only_verified_pins_and_is_read_only_to_runtime() {
        use crate::config::{ArtifactKind, ArtifactPin};
        use flow_like_runtime::{
            a2ui::widget::{Page, Widget},
            app::AppVisibility,
            bit::Metadata,
            flow::event::{EventExecutionMode, EventExposure},
            state::FlowLikeState,
            utils::http::HTTPClient,
        };
        use std::{collections::HashMap, time::SystemTime};
        let directory = tempfile::tempdir().unwrap();
        let mut placement = config(directory.path());
        placement.artifact_pins = vec![
            ArtifactPin {
                kind: ArtifactKind::Widget,
                id: "widget".into(),
                version: [1, 0, 0],
            },
            ArtifactPin {
                kind: ArtifactKind::Template,
                id: "template".into(),
                version: [1, 0, 0],
            },
        ];
        let source = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(FlowLikeStore::Memory(Arc::new(
                object_store::memory::InMemory::new(),
            ))),
            HTTPClient::new_without_refetch(),
        ));
        let mut app = App::new(
            Some("project".into()),
            Metadata::default(),
            vec![],
            source.clone(),
        )
        .await
        .unwrap();
        app.visibility = AppVisibility::Private;
        app.boards = vec!["board".into()];
        app.events = vec!["event".into()];
        app.widget_ids = vec!["widget".into()];
        app.templates = vec!["template".into()];
        let mut board = Board::new(
            Some("board".into()),
            ObjectPath::from("apps/project"),
            source.clone(),
        );
        board.version = (1, 0, 0);
        board.page_ids.push("page".into());
        let page = Page::new("page", "Pinned Page", "/").with_board_id("board");
        let mut template = board.clone();
        template.id = "template".into();
        let mut widget = Widget::new("widget", "Widget", "root");
        widget.version = Some((1, 0, 0));
        let now = SystemTime::now();
        let event = Event {
            id: "event".into(),
            name: "Service".into(),
            description: String::new(),
            board_id: "board".into(),
            board_version: Some((1, 0, 0)),
            node_id: "entry".into(),
            variables: HashMap::new(),
            config: vec![],
            active: true,
            canary: None,
            variants: vec![],
            priority: 0,
            event_type: "daemon".into(),
            notes: None,
            event_version: (1, 0, 0),
            created_at: now,
            updated_at: now,
            default_page_id: Some("page".into()),
            inputs: vec![],
            route: None,
            is_default: false,
            execution_mode: EventExecutionMode::Local,
            exposure: EventExposure::Public,
            correlation_mappings: None,
        };
        let documents = Arc::new(HashMap::from([
            (
                "/instances/project/app".to_string(),
                serde_json::to_value(&app).unwrap(),
            ),
            (
                "/instances/project/events/event/versions/1/0/0".into(),
                serde_json::to_value(event).unwrap(),
            ),
            (
                "/instances/project/boards/board/versions/1/0/0".into(),
                serde_json::to_value(board).unwrap(),
            ),
            (
                "/instances/project/boards/board/versions/1/0/0/pages/page".into(),
                serde_json::to_value(&page).unwrap(),
            ),
            (
                "/instances/project/widgets/widget/versions/1/0/0".into(),
                serde_json::to_value(widget).unwrap(),
            ),
            (
                "/instances/project/templates/template/versions/1/0/0/pages/page".into(),
                serde_json::to_value(&page).unwrap(),
            ),
            (
                "/instances/project/templates/template/versions/1/0/0".into(),
                serde_json::to_value(template).unwrap(),
            ),
        ]));
        approve_documents(
            &mut placement,
            documents
                .iter()
                .map(|(key, value)| {
                    (
                        key.trim_start_matches("/instances/project/").to_owned(),
                        value.clone(),
                    )
                })
                .collect(),
        );
        let requests = Arc::new(AtomicUsize::new(0));
        let calls = requests.clone();
        let router =
            axum::Router::new().fallback(axum::routing::get(move |uri: axum::http::Uri| {
                let docs = documents.clone();
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    docs.get(uri.path())
                        .cloned()
                        .map(axum::Json)
                        .ok_or(axum::http::StatusCode::NOT_FOUND)
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let authorizer = Arc::new(TestAuthorizer {
            base: format!("http://{address}/instances/project"),
        });
        let client = ProjectClient::new(authorizer.clone()).unwrap();
        let cache = Arc::new(
            LocalObjectStore::new(
                private_cache(directory.path(), "placement", "instance").unwrap(),
            )
            .unwrap(),
        );
        hydrate_metadata(&client, &placement, cache.clone())
            .await
            .unwrap();
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::with_default_store(FlowLikeStore::Local(cache).read_only()),
            HTTPClient::new_without_refetch(),
        ));
        let loaded = App::load("project".into(), state.clone()).await.unwrap();
        assert!(matches!(loaded.visibility, AppVisibility::Private));
        assert_eq!(
            loaded
                .get_event("event", Some((1, 0, 0)))
                .await
                .unwrap()
                .board_id,
            "board"
        );
        assert_eq!(
            Board::load(
                ObjectPath::from("apps/project"),
                "board",
                state.clone(),
                Some((1, 0, 0))
            )
            .await
            .unwrap()
            .version,
            (1, 0, 0)
        );
        assert_eq!(
            loaded
                .open_widget("widget".into(), None)
                .await
                .unwrap()
                .version,
            Some((1, 0, 0))
        );
        assert_eq!(
            Board::load(
                ObjectPath::from("apps/project"),
                "board",
                state.clone(),
                Some((1, 0, 0))
            )
            .await
            .unwrap()
            .load_versioned_page("page", (1, 0, 0), None)
            .await
            .unwrap()
            .name,
            "Pinned Page"
        );
        assert_eq!(
            Board::load_template(
                ObjectPath::from("apps/project"),
                "template",
                state.clone(),
                None
            )
            .await
            .unwrap()
            .version,
            (1, 0, 0)
        );
        assert!(loaded.save().await.is_err());
        placement.hosting = Some(crate::config::HostingConfig {
            host: "127.0.0.1".parse().unwrap(),
            port: 8090,
            max_in_flight: 1,
            request_timeout_secs: 5,
            auth_secret: "listener".into(),
        });
        crate::secrets::install(&placement, "listener", b"0123456789abcdef0123456789abcdef")
            .unwrap();
        crate::runtime::validate_rollout_authorized(
            &placement,
            Some(authorizer.clone()),
            Some(identity()),
        )
        .await
        .unwrap();
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        let cache_root = directory.path().join(".standalone-cache/placement");
        assert_eq!(std::fs::read_dir(&cache_root).unwrap().count(), 1);
        assert!(cache_root.join("instance").is_dir());
        assert!(!directory.path().join("user").exists());
        assert!(!directory.path().join("logs").exists());
        assert!(!directory.path().join("bits").exists());
        placement.events[0].board_version = [2, 0, 0];
        assert!(
            crate::runtime::validate_rollout_authorized(
                &placement,
                Some(authorizer),
                Some(identity()),
            )
            .await
            .is_err()
        );
        assert_eq!(std::fs::read_dir(&cache_root).unwrap().count(), 1);
        let rejected = Arc::new(object_store::memory::InMemory::new());
        assert!(
            hydrate_metadata(&client, &placement, rejected.clone())
                .await
                .is_err()
        );
        assert!(
            rejected
                .get(&ObjectPath::from("apps/project/versions/board/2_0_0.board"))
                .await
                .is_err()
        );
        placement.events[0].board_version = [1, 0, 0];
        let approved_path = directory.path().join("apps/project/online-metadata.json");
        let original = std::fs::read(&approved_path).unwrap();
        let mut substituted: metadata::Bundle = serde_json::from_slice(&original).unwrap();
        substituted
            .documents
            .get_mut("boards/board/versions/1/0/0")
            .unwrap()["name"] = serde_json::json!("Substituted under the same version");
        std::fs::write(&approved_path, serde_json::to_vec(&substituted).unwrap()).unwrap();
        let rejected = Arc::new(object_store::memory::InMemory::new());
        assert!(
            hydrate_metadata(&client, &placement, rejected.clone())
                .await
                .unwrap_err()
                .to_string()
                .contains("digest differs")
        );
        assert_eq!(rejected.list(None).count().await, 0);
        let mut incomplete: metadata::Bundle = serde_json::from_slice(&original).unwrap();
        incomplete
            .documents
            .remove("boards/board/versions/1/0/0/pages/page");
        approve_documents(&mut placement, incomplete.documents);
        assert!(
            hydrate_metadata(&client, &placement, rejected.clone())
                .await
                .unwrap_err()
                .to_string()
                .contains("incomplete")
        );
        assert_eq!(rejected.list(None).count().await, 0);
        placement.online_metadata_sha256 = None;
        assert!(
            hydrate_metadata(&client, &placement, rejected.clone())
                .await
                .unwrap_err()
                .to_string()
                .contains("Prepare and deploy")
        );
        assert_eq!(rejected.list(None).count().await, 0);
        server.abort();
    }
}

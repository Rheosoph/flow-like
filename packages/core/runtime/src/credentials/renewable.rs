use super::{SharedCredentials, StoreType, aws_credentials::s3_storage_options};
use async_trait::async_trait;
use flow_like_storage::{
    files::{
        credentials::{
            RenewableCredentials, StorageCredential, StorageCredentialLease,
            StorageCredentialProvider, StorageCredentialScope,
        },
        store::FlowLikeStore,
    },
    object_store::{self, ObjectStore, ObjectStoreExt, path::Path},
};
use flow_like_types::{Result, authorization::AuthorizationError, tokio};
use futures::{StreamExt, TryStreamExt, stream::BoxStream};
use std::{
    fmt,
    sync::{Arc, Mutex, Weak},
    time::{Duration, SystemTime},
};

/// The host retains responsibility for account identity and logout fencing.
#[async_trait]
pub trait SharedCredentialRefresh: Send + Sync {
    async fn refresh(&self) -> std::result::Result<SharedCredentials, AuthorizationError>;
    fn authorization_current(&self) -> std::result::Result<(), AuthorizationError> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    Meta,
    Content,
    User,
    Tmp,
    Draft,
    Logs,
}
impl Purpose {
    fn store_type(self) -> StoreType {
        match self {
            Self::Meta | Self::Draft => StoreType::Meta,
            Self::Logs => StoreType::Logs,
            _ => StoreType::Content,
        }
    }
}

struct LeaseState {
    credentials: SharedCredentials,
    expires: SystemTime,
    refresh_at: SystemTime,
    failures: u32,
    terminal: Option<AuthorizationError>,
    last_error: AuthorizationError,
}

/// Holds one refresh cycle shared by every file handle, database and child invocation.
pub struct RenewableSharedCredentials {
    initial: SharedCredentials,
    project: String,
    shape: serde_json::Value,
    source: Arc<dyn SharedCredentialRefresh>,
    state: Mutex<LeaseState>,
    refreshing: tokio::sync::Mutex<()>,
    scope_id: String,
    this: Weak<Self>,
}
impl fmt::Debug for RenewableSharedCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenewableSharedCredentials")
            .field("project", &self.project)
            .finish_non_exhaustive()
    }
}

fn leaf(
    credentials: &SharedCredentials,
    purpose: Purpose,
) -> std::result::Result<&SharedCredentials, AuthorizationError> {
    let mut current = credentials;
    for _ in 0..8 {
        match current {
            SharedCredentials::Mixed(mixed) => {
                current = match purpose.store_type() {
                    StoreType::Meta => &mixed.meta,
                    StoreType::Logs => &mixed.logs,
                    _ => &mixed.content,
                }
            }
            SharedCredentials::Renewable(_) => return Err(AuthorizationError::InvalidResponse),
            _ => return Ok(current),
        }
    }
    Err(AuthorizationError::InvalidResponse)
}

fn shape_and_expiry(
    credentials: &SharedCredentials,
) -> std::result::Result<(serde_json::Value, SystemTime), AuthorizationError> {
    fn visit(
        credentials: &SharedCredentials,
        depth: u8,
    ) -> std::result::Result<(serde_json::Value, SystemTime), AuthorizationError> {
        if depth > 8 {
            return Err(AuthorizationError::InvalidResponse);
        }
        if let SharedCredentials::Mixed(mixed) = credentials {
            let (meta, m) = visit(&mixed.meta, depth + 1)?;
            let (content, c) = visit(&mixed.content, depth + 1)?;
            let (logs, l) = visit(&mixed.logs, depth + 1)?;
            return Ok((
                serde_json::json!({"Mixed":{"meta":meta,"content":content,"logs":logs}}),
                m.min(c).min(l),
            ));
        }
        let expiry = match credentials {
            SharedCredentials::Aws(value) => value.expiration,
            SharedCredentials::Azure(value) if value.account_key.is_none() => value.expiration,
            SharedCredentials::Gcp(value) if value.service_account_key.is_empty() => {
                value.expiration
            }
            _ => return Err(AuthorizationError::InvalidResponse),
        }
        .ok_or(AuthorizationError::InvalidResponse)?;
        let expiry: SystemTime = expiry.into();
        let remaining = expiry
            .duration_since(SystemTime::now())
            .map_err(|_| AuthorizationError::Expired)?;
        if remaining.is_zero()
            || remaining > flow_like_storage::files::credentials::MAX_STORAGE_LEASE_LIFETIME
        {
            return Err(AuthorizationError::InvalidResponse);
        }
        let mut value =
            serde_json::to_value(credentials).map_err(|_| AuthorizationError::InvalidResponse)?;
        let body = value
            .as_object_mut()
            .and_then(|value| value.values_mut().next())
            .and_then(serde_json::Value::as_object_mut)
            .ok_or(AuthorizationError::InvalidResponse)?;
        body.remove("expiration");
        // Token presence is part of the authority shape. A renewal cannot silently
        // add a previously withheld directory or remove the SAS that guards it.
        for name in [
            "access_key_id",
            "secret_access_key",
            "session_token",
            "access_token",
            "meta_sas_token",
            "content_sas_token",
            "user_content_sas_token",
            "logs_sas_token",
            "tmp_sas_token",
            "draft_meta_sas_token",
        ] {
            if let Some(token) = body.get_mut(name) {
                *token = if name.ends_with("sas_token") && token.is_string() {
                    let url = url::Url::parse(&format!(
                        "https://storage.invalid/?{}",
                        token.as_str().unwrap().trim_start_matches('?')
                    ))
                    .map_err(|_| AuthorizationError::InvalidResponse)?;
                    let authority: std::collections::BTreeMap<_, _> = url
                        .query_pairs()
                        .filter(|(name, _)| {
                            matches!(
                                name.as_ref(),
                                "sp" | "sr" | "sdd" | "ss" | "srt" | "sip" | "spr"
                            )
                        })
                        .map(|(name, value)| (name.into_owned(), value.into_owned()))
                        .collect();
                    serde_json::to_value(authority)
                        .map_err(|_| AuthorizationError::InvalidResponse)?
                } else {
                    serde_json::Value::Bool(token.as_str().is_some_and(|value| !value.is_empty()))
                };
            }
        }
        Ok((value, expiry))
    }
    visit(credentials, 0)
}

fn refresh_time(expires: SystemTime) -> SystemTime {
    let remaining = expires
        .duration_since(SystemTime::now())
        .unwrap_or_default();
    expires - (remaining / 5).min(Duration::from_secs(300))
}

fn prefix_path(value: &str) -> std::result::Result<Path, AuthorizationError> {
    let raw = Path::parse(value).map_err(|_| AuthorizationError::InvalidResponse)?;
    let decoded = Path::from_url_path(value).map_err(|_| AuthorizationError::InvalidResponse)?;
    if raw.parts().count() != decoded.parts().count()
        || decoded.as_ref().contains(['%', '\\'])
        || value.to_ascii_lowercase().contains("%2f")
    {
        return Err(AuthorizationError::InvalidResponse);
    }
    Ok(Path::from(decoded.as_ref()))
}

fn redact(credentials: &mut SharedCredentials) {
    let redact = |value: &mut Option<String>| {
        if value.is_some() {
            *value = Some("[redacted]".into());
        }
    };
    match credentials {
        SharedCredentials::Aws(value) => {
            redact(&mut value.access_key_id);
            redact(&mut value.secret_access_key);
            redact(&mut value.session_token);
        }
        SharedCredentials::Azure(value) => {
            for token in [
                &mut value.meta_sas_token,
                &mut value.content_sas_token,
                &mut value.user_content_sas_token,
                &mut value.tmp_sas_token,
                &mut value.draft_meta_sas_token,
                &mut value.logs_sas_token,
            ] {
                redact(token);
            }
        }
        SharedCredentials::Gcp(value) => redact(&mut value.access_token),
        SharedCredentials::Mixed(value) => {
            self::redact(&mut value.meta);
            self::redact(&mut value.content);
            self::redact(&mut value.logs);
        }
        SharedCredentials::Renewable(_) => {}
    }
}

impl RenewableSharedCredentials {
    pub async fn new(
        initial: SharedCredentials,
        project_id: String,
        source: Arc<dyn SharedCredentialRefresh>,
    ) -> std::result::Result<Arc<Self>, AuthorizationError> {
        if project_id.is_empty()
            || project_id.contains(['/', '\\'])
            || project_id.chars().any(char::is_control)
        {
            return Err(AuthorizationError::InvalidRequest);
        }
        source.authorization_current()?;
        let (shape, expires) = shape_and_expiry(&initial)?;
        let mut metadata = initial.clone();
        redact(&mut metadata);
        let value = Arc::new_cyclic(|this| Self {
            initial: metadata,
            project: project_id,
            shape,
            source,
            state: Mutex::new(LeaseState {
                credentials: initial,
                expires,
                refresh_at: refresh_time(expires),
                failures: 0,
                terminal: None,
                last_error: AuthorizationError::Expired,
            }),
            refreshing: tokio::sync::Mutex::new(()),
            scope_id: flow_like_types::create_id(),
            this: this.clone(),
        });
        let weak = Arc::downgrade(&value);
        tokio::spawn(async move {
            loop {
                let Some(value) = weak.upgrade() else {
                    return;
                };
                if value.check().is_err() {
                    return;
                }
                let delay = value
                    .state
                    .lock()
                    .unwrap()
                    .refresh_at
                    .duration_since(SystemTime::now())
                    .unwrap_or_default()
                    .min(Duration::from_secs(60));
                drop(value);
                tokio::time::sleep(delay.max(Duration::from_millis(100))).await;
                let Some(value) = weak.upgrade() else {
                    return;
                };
                let _ = value.refresh_if_due().await;
            }
        });
        Ok(value)
    }

    /// Immutable routing metadata. Authentication fields retain presence only.
    pub fn initial(&self) -> &SharedCredentials {
        &self.initial
    }

    pub fn authorization_current(&self) -> std::result::Result<(), AuthorizationError> {
        self.check()
    }

    fn check(&self) -> std::result::Result<(), AuthorizationError> {
        if let Err(error) = self.source.authorization_current() {
            if error == AuthorizationError::Denied {
                self.state.lock().unwrap().terminal = Some(error);
            }
            return Err(error);
        }
        match self.state.lock().unwrap().terminal {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    // Express caches a derived session, so checking only its root provider does
    // not enforce the shorter shared lease on retained requests and streams.
    fn check_lease(&self) -> std::result::Result<(), AuthorizationError> {
        self.check()?;
        let state = self.state.lock().unwrap();
        if state.expires <= SystemTime::now() {
            return Err(state.last_error);
        }
        Ok(())
    }

    async fn ensure_lease(&self) -> std::result::Result<(), AuthorizationError> {
        self.check()?;
        if self.state.lock().unwrap().expires <= SystemTime::now() {
            let _ = self.refresh_if_due().await;
        }
        self.check_lease()
    }

    async fn while_lease_valid<T>(
        &self,
        future: impl std::future::Future<Output = object_store::Result<T>>,
    ) -> object_store::Result<T> {
        let mut future = std::pin::pin!(future);
        futures::future::poll_fn(|cx| {
            if let Err(error) = self.check_lease() {
                return std::task::Poll::Ready(Err(store_error(error)));
            }
            let result = future.as_mut().poll(cx);
            if let Err(error) = self.check_lease() {
                return std::task::Poll::Ready(Err(store_error(error)));
            }
            result
        })
        .await
    }

    async fn refresh_if_due(&self) -> std::result::Result<(), AuthorizationError> {
        self.check()?;
        let _single = self.refreshing.lock().await;
        self.check()?;
        if SystemTime::now() < self.state.lock().unwrap().refresh_at {
            return Ok(());
        }
        let result = tokio::time::timeout(Duration::from_secs(20), self.source.refresh())
            .await
            .unwrap_or(Err(AuthorizationError::Unavailable));
        self.check()?;
        let result = result.and_then(|credentials| {
            let (shape, expiry) = shape_and_expiry(&credentials)?;
            if shape != self.shape {
                return Err(AuthorizationError::InvalidResponse);
            }
            Ok((credentials, expiry))
        });
        let mut state = self.state.lock().unwrap();
        match result {
            Ok((credentials, expires)) => {
                state.credentials = credentials;
                state.expires = expires;
                state.refresh_at = refresh_time(expires);
                state.failures = 0;
                Ok(())
            }
            Err(error) => {
                if error == AuthorizationError::Denied {
                    state.terminal = Some(error);
                }
                state.failures = state.failures.saturating_add(1);
                state.refresh_at = SystemTime::now()
                    + Duration::from_secs((1u64 << state.failures.min(6)).min(60));
                state.last_error = error;
                Err(error)
            }
        }
    }

    fn provider(
        &self,
        purpose: Purpose,
    ) -> std::result::Result<RenewableCredentials, AuthorizationError> {
        RenewableCredentials::new(Arc::new(Provider {
            owner: self.this.upgrade().ok_or(AuthorizationError::Denied)?,
            purpose,
            scope: StorageCredentialScope {
                instance_id: self.scope_id.clone(),
                project_id: self.project.clone(),
                placement_id: "desktop".into(),
                grant_id: self.scope_id.clone(),
                resource: format!("{purpose:?}"),
            },
        }))
    }

    fn native_store(&self, purpose: Purpose) -> Result<FlowLikeStore> {
        self.check()?;
        let credentials = self.provider(purpose)?;
        match leaf(&self.initial, purpose)? {
            SharedCredentials::Aws(value) => {
                let (bucket, config) = match purpose.store_type() {
                    StoreType::Meta => (&value.meta_bucket, value.meta_config.as_ref()),
                    StoreType::Logs => (&value.logs_bucket, value.logs_config.as_ref()),
                    _ => (&value.content_bucket, value.content_config.as_ref()),
                };
                let mut builder = object_store::aws::AmazonS3Builder::new()
                    .with_bucket_name(bucket)
                    .with_region(&value.region);
                for (key, value) in s3_storage_options(config) {
                    builder = builder.with_config(key.parse()?, value);
                }
                if config.is_some_and(|config| config.express) {
                    builder = builder.with_s3_express(true);
                }
                let store = credentials.aws_store(builder)?;
                // Express caches a derived five-minute session above the provider.
                // Fence every operation even while that derived session is valid.
                if config.is_some_and(|config| config.express) {
                    Ok(self.router(vec![(Path::from(""), store)]))
                } else {
                    Ok(store)
                }
            }
            SharedCredentials::Azure(value) => {
                let container = match purpose.store_type() {
                    StoreType::Meta => &value.meta_container,
                    StoreType::Logs => &value.logs_container,
                    _ => &value.content_container,
                };
                Ok(credentials.azure_store(
                    object_store::azure::MicrosoftAzureBuilder::new()
                        .with_account(&value.account_name)
                        .with_container_name(container),
                )?)
            }
            SharedCredentials::Gcp(value) => {
                let bucket = match purpose.store_type() {
                    StoreType::Meta => &value.meta_bucket,
                    StoreType::Logs => &value.logs_bucket,
                    _ => &value.content_bucket,
                };
                Ok(credentials.gcp_store(
                    object_store::gcp::GoogleCloudStorageBuilder::new().with_bucket_name(bucket),
                )?)
            }
            _ => Err(AuthorizationError::InvalidResponse.into()),
        }
    }

    fn router(&self, routes: Vec<(Path, FlowLikeStore)>) -> FlowLikeStore {
        FlowLikeStore::Signed(Arc::new(RoutedStore {
            owner: self.this.upgrade().expect("live adapter"),
            routes,
        }))
    }

    pub async fn to_store(&self, meta: bool) -> Result<FlowLikeStore> {
        self.to_store_type(if meta {
            StoreType::Meta
        } else {
            StoreType::Content
        })
        .await
    }

    pub async fn to_store_type(&self, kind: StoreType) -> Result<FlowLikeStore> {
        let purpose = match kind {
            StoreType::Meta => Purpose::Meta,
            StoreType::Content => Purpose::Content,
            StoreType::Logs => Purpose::Logs,
            StoreType::Tmp => Purpose::Tmp,
        };
        let initial = leaf(&self.initial, purpose)?;
        let SharedCredentials::Azure(value) = initial else {
            return self.native_store(purpose);
        };
        if kind == StoreType::Content {
            let mut routes = Vec::new();
            if value.content_sas_token.is_some() {
                routes.push((
                    prefix_path(
                        &value
                            .content_path_prefix
                            .clone()
                            .unwrap_or_else(|| format!("apps/{}", self.project)),
                    )?,
                    self.native_store(Purpose::Content)?,
                ));
            }
            if let Some(prefix) = value.user_content_path_prefix.as_deref() {
                if value.user_content_sas_token.is_some() || value.content_sas_token.is_some() {
                    routes.push((prefix_path(prefix)?, self.native_store(Purpose::User)?));
                }
                if value.tmp_sas_token.is_some() {
                    let decoded = Path::from_url_path(prefix)
                        .map_err(|_| AuthorizationError::InvalidResponse)?;
                    if let Some(subject) = decoded
                        .as_ref()
                        .strip_prefix("users/")
                        .and_then(|prefix| prefix.strip_suffix(&format!("/apps/{}", self.project)))
                    {
                        routes.push((
                            Path::from(
                                flow_like_types::storage_paths::temporary_prefixes(
                                    subject,
                                    &self.project,
                                )
                                .0,
                            ),
                            self.native_store(Purpose::Tmp)?,
                        ));
                    }
                }
            }
            if routes.is_empty() {
                return Err(AuthorizationError::Denied.into());
            }
            return Ok(self.router(routes));
        }
        if kind == StoreType::Meta && value.draft_meta_sas_token.is_some() {
            let prefix = value
                .draft_meta_path_prefix
                .as_deref()
                .ok_or(AuthorizationError::InvalidResponse)?;
            return Ok(self.router(vec![
                (
                    prefix_path(prefix)?,
                    self.native_store(Purpose::Draft)?.read_only(),
                ),
                (
                    Path::from(format!("apps/{}", self.project)),
                    self.native_store(Purpose::Meta)?,
                ),
            ]));
        }
        self.native_store(purpose)
    }
}

#[cfg(feature = "flow-runtime")]
impl RenewableSharedCredentials {
    fn content_prefix(&self, user: bool) -> Result<Option<String>> {
        Ok(match leaf(&self.initial, Purpose::Content)? {
            SharedCredentials::Aws(value) => {
                if user {
                    value.user_content_path_prefix.clone()
                } else {
                    value.content_path_prefix.clone()
                }
            }
            SharedCredentials::Azure(value) => {
                if user {
                    value.user_content_path_prefix.clone()
                } else {
                    value.content_path_prefix.clone()
                }
            }
            SharedCredentials::Gcp(value) => {
                let explicit = if user {
                    &value.user_content_path_prefix
                } else {
                    &value.content_path_prefix
                };
                explicit.clone().or_else(|| {
                    value
                        .allowed_prefixes
                        .iter()
                        .find(|prefix| prefix.starts_with(if user { "users/" } else { "apps/" }))
                        .cloned()
                })
            }
            _ => return Err(AuthorizationError::InvalidResponse.into()),
        })
    }

    fn binding(
        &self,
        purpose: Purpose,
        prefix: &Path,
    ) -> Result<flow_like_storage::renewable_lance::LanceStorageBinding> {
        let (scheme, bucket, options) = match leaf(&self.initial, purpose)? {
            SharedCredentials::Aws(value) => {
                let (bucket, config) = if purpose == Purpose::Logs {
                    (&value.logs_bucket, value.logs_config.as_ref())
                } else {
                    (&value.content_bucket, value.content_config.as_ref())
                };
                let mut options: std::collections::HashMap<_, _> =
                    s3_storage_options(config).into_iter().collect();
                options.insert("aws_region".into(), value.region.clone());
                ("s3", bucket, options)
            }
            SharedCredentials::Azure(value) => (
                "az",
                if purpose == Purpose::Logs {
                    &value.logs_container
                } else {
                    &value.content_container
                },
                std::collections::HashMap::from([(
                    "azure_storage_account_name".into(),
                    value.account_name.clone(),
                )]),
            ),
            SharedCredentials::Gcp(value) => (
                "gs",
                if purpose == Purpose::Logs {
                    &value.logs_bucket
                } else {
                    &value.content_bucket
                },
                std::collections::HashMap::new(),
            ),
            _ => return Err(AuthorizationError::InvalidResponse.into()),
        };
        let uri = format!("{scheme}://{bucket}/{prefix}");
        Ok(
            flow_like_storage::renewable_lance::LanceStorageBinding::new(
                &uri,
                options,
                self.provider(purpose)?,
            )?
            .with_object_prefix(prefix.as_ref())?
            .with_store(self.native_store(purpose)?.as_generic()),
        )
    }

    pub fn lance_registry(
        &self,
    ) -> Result<Arc<flow_like_storage::lance_io::object_store::ObjectStoreRegistry>> {
        let app = prefix_path(
            &self
                .content_prefix(false)?
                .unwrap_or_else(|| format!("apps/{}", self.project)),
        )?;
        let mut bindings = vec![self.binding(Purpose::Content, &app)?];
        if let Some(user) = self.content_prefix(true)? {
            let user = prefix_path(&user)?;
            if user != app {
                bindings.push(self.binding(Purpose::User, &user)?);
            }
        }
        let logs = Path::from("runs").join(self.project.as_str());
        // Withheld log authorization stays withheld; creating its provider cannot
        // resolve an ambient identity if a caller nevertheless requests this path.
        if self.logs_available()? {
            bindings.push(self.binding(Purpose::Logs, &logs)?);
        }
        Ok(flow_like_storage::renewable_lance::scoped_registry(
            bindings,
        )?)
    }

    /// Desktop log databases stay local; only their explicit file provider is added.
    pub fn lance_registry_with_local(
        &self,
    ) -> Result<Arc<flow_like_storage::lance_io::object_store::ObjectStoreRegistry>> {
        let registry = self.lance_registry()?;
        let local = flow_like_storage::lance_io::object_store::ObjectStoreRegistry::default()
            .get_provider("file")
            .ok_or(AuthorizationError::InvalidResponse)?;
        registry.insert("file", local);
        Ok(registry)
    }

    fn database_uri(&self, purpose: Purpose, path: &Path) -> Result<String> {
        let (scheme, bucket) = match leaf(&self.initial, purpose)? {
            SharedCredentials::Aws(value) => (
                "s3",
                if purpose == Purpose::Logs {
                    &value.logs_bucket
                } else {
                    &value.content_bucket
                },
            ),
            SharedCredentials::Azure(value) => (
                "az",
                if purpose == Purpose::Logs {
                    &value.logs_container
                } else {
                    &value.content_container
                },
            ),
            SharedCredentials::Gcp(value) => (
                "gs",
                if purpose == Purpose::Logs {
                    &value.logs_bucket
                } else {
                    &value.content_bucket
                },
            ),
            _ => return Err(AuthorizationError::InvalidResponse.into()),
        };
        Ok(format!("{scheme}://{bucket}/{path}"))
    }

    fn session(&self) -> Result<Arc<flow_like_storage::lance::session::Session>> {
        Ok(Arc::new(flow_like_storage::lance::session::Session::new(
            flow_like_storage::lance::dataset::DEFAULT_INDEX_CACHE_SIZE,
            flow_like_storage::lance::dataset::DEFAULT_METADATA_CACHE_SIZE,
            self.lance_registry()?,
        )))
    }

    pub async fn to_db(
        &self,
        app_id: &str,
    ) -> Result<flow_like_storage::lancedb::connection::ConnectBuilder> {
        if app_id != self.project {
            return Err(AuthorizationError::Denied.into());
        }
        let prefix = self
            .content_prefix(false)?
            .unwrap_or_else(|| format!("apps/{}", self.project));
        let root = prefix_path(&prefix)?;
        let path = if prefix.starts_with("users/") {
            root.join("db")
        } else {
            root.join("storage").join("db")
        };
        Ok(
            flow_like_storage::lancedb::connect(&self.database_uri(Purpose::Content, &path)?)
                .session(self.session()?),
        )
    }

    pub async fn to_db_scoped(
        &self,
        sub: &str,
        app_id: &str,
    ) -> Result<flow_like_storage::lancedb::connection::ConnectBuilder> {
        if app_id != self.project {
            return Err(AuthorizationError::Denied.into());
        }
        let expected = self
            .content_prefix(true)?
            .ok_or(AuthorizationError::Denied)?;
        let requested = Path::from("users").join(sub).join("apps").join(app_id);
        if requested != prefix_path(&expected)? {
            return Err(AuthorizationError::Denied.into());
        }
        Ok(flow_like_storage::lancedb::connect(
            &self.database_uri(Purpose::User, &requested.join("db"))?,
        )
        .session(self.session()?))
    }

    pub fn to_logs_db_builder(&self) -> Result<super::LogsDbBuilder> {
        if !self.logs_available()? {
            return Err(AuthorizationError::Denied.into());
        }
        let session = self.session()?;
        let owner = self.this.upgrade().ok_or(AuthorizationError::Denied)?;
        Ok(Arc::new(move |path| {
            flow_like_storage::lancedb::connect(
                &owner
                    .database_uri(Purpose::Logs, &path)
                    .expect("validated log location"),
            )
            .session(session.clone())
        }))
    }

    fn logs_available(&self) -> Result<bool> {
        Ok(match leaf(&self.initial, Purpose::Logs)? {
            SharedCredentials::Aws(value) => !value.logs_bucket.is_empty(),
            SharedCredentials::Azure(value) => {
                !value.logs_container.is_empty() && value.logs_sas_token.is_some()
            }
            SharedCredentials::Gcp(value) => !value.logs_bucket.is_empty(),
            _ => false,
        })
    }
}

struct Provider {
    owner: Arc<RenewableSharedCredentials>,
    purpose: Purpose,
    scope: StorageCredentialScope,
}
#[async_trait]
impl StorageCredentialProvider for Provider {
    fn scope(&self) -> &StorageCredentialScope {
        &self.scope
    }
    async fn credential(&self) -> std::result::Result<StorageCredentialLease, AuthorizationError> {
        self.owner.ensure_lease().await?;
        let state = self.owner.state.lock().unwrap();
        if state.expires <= SystemTime::now() {
            return Err(state.last_error);
        }
        let credential = match leaf(&state.credentials, self.purpose)? {
            SharedCredentials::Aws(value) => StorageCredential::AwsSession {
                access_key_id: value
                    .access_key_id
                    .clone()
                    .ok_or(AuthorizationError::Denied)?,
                secret_access_key: value
                    .secret_access_key
                    .clone()
                    .ok_or(AuthorizationError::Denied)?,
                session_token: value
                    .session_token
                    .clone()
                    .ok_or(AuthorizationError::Denied)?,
            },
            SharedCredentials::Azure(value) => StorageCredential::AzureSas(
                match self.purpose {
                    Purpose::Meta => value.meta_sas_token.as_ref(),
                    Purpose::Content => value
                        .content_sas_token
                        .as_ref()
                        .or(value.user_content_sas_token.as_ref()),
                    Purpose::User => value
                        .user_content_sas_token
                        .as_ref()
                        .or(value.content_sas_token.as_ref()),
                    Purpose::Tmp => value
                        .tmp_sas_token
                        .as_ref()
                        .or(value.content_sas_token.as_ref()),
                    Purpose::Draft => value.draft_meta_sas_token.as_ref(),
                    Purpose::Logs => value.logs_sas_token.as_ref(),
                }
                .cloned()
                .ok_or(AuthorizationError::Denied)?,
            ),
            SharedCredentials::Gcp(value) => StorageCredential::GcpBearer(
                value
                    .access_token
                    .clone()
                    .ok_or(AuthorizationError::Denied)?,
            ),
            _ => return Err(AuthorizationError::InvalidResponse),
        };
        let lease = StorageCredentialLease {
            scope: self.scope.clone(),
            expires_at: state.expires,
            credential,
        };
        lease.validate(&self.scope)?;
        Ok(lease)
    }
}

#[derive(Clone)]
struct RoutedStore {
    owner: Arc<RenewableSharedCredentials>,
    routes: Vec<(Path, FlowLikeStore)>,
}
impl fmt::Debug for RoutedStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RenewableRoutedStore")
    }
}
impl fmt::Display for RoutedStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RenewableRoutedStore")
    }
}
fn store_error(error: AuthorizationError) -> object_store::Error {
    object_store::Error::Generic {
        store: "RenewableSharedCredentials",
        source: Box::new(error),
    }
}
impl RoutedStore {
    fn selected(&self, path: &Path) -> object_store::Result<&FlowLikeStore> {
        self.owner.check_lease().map_err(store_error)?;
        self.routes
            .iter()
            .find(|(prefix, _)| path.prefix_match(prefix).is_some())
            .map(|(_, store)| store)
            .ok_or_else(|| store_error(AuthorizationError::Denied))
    }
    fn route(&self, path: &Path) -> object_store::Result<Arc<dyn ObjectStore>> {
        Ok(self.selected(path)?.as_generic())
    }
    fn pair(&self, from: &Path, to: &Path) -> object_store::Result<Arc<dyn ObjectStore>> {
        let source = self.route(from)?;
        if !Arc::ptr_eq(&source, &self.route(to)?) {
            return Err(object_store::Error::NotSupported {
                source: "Cross-credential copies are not supported".into(),
            });
        }
        Ok(source)
    }
}

#[async_trait]
impl object_store::signer::Signer for RoutedStore {
    async fn signed_url(
        &self,
        method: flow_like_types::reqwest::Method,
        path: &Path,
        expires_in: Duration,
    ) -> object_store::Result<url::Url> {
        self.owner.ensure_lease().await.map_err(store_error)?;
        let store = self.selected(path)?;
        self.owner
            .while_lease_valid(async {
                store
                    .sign(method.as_str(), path, expires_in)
                    .await
                    .map_err(|error| object_store::Error::Generic {
                        store: "RenewableSharedCredentials",
                        source: error.into(),
                    })
            })
            .await
    }
}
#[async_trait]
impl ObjectStore for RoutedStore {
    async fn put_opts(
        &self,
        path: &Path,
        value: object_store::PutPayload,
        options: object_store::PutOptions,
    ) -> object_store::Result<object_store::PutResult> {
        self.owner.ensure_lease().await.map_err(store_error)?;
        self.owner
            .while_lease_valid(self.route(path)?.put_opts(path, value, options))
            .await
    }
    async fn put_multipart_opts(
        &self,
        path: &Path,
        options: object_store::PutMultipartOptions,
    ) -> object_store::Result<Box<dyn object_store::MultipartUpload>> {
        self.owner.ensure_lease().await.map_err(store_error)?;
        let inner = self
            .owner
            .while_lease_valid(self.route(path)?.put_multipart_opts(path, options))
            .await?;
        Ok(Box::new(FencedUpload {
            inner,
            owner: self.owner.clone(),
        }))
    }
    async fn get_opts(
        &self,
        path: &Path,
        options: object_store::GetOptions,
    ) -> object_store::Result<object_store::GetResult> {
        self.owner.ensure_lease().await.map_err(store_error)?;
        self.owner
            .while_lease_valid(self.route(path)?.get_opts(path, options))
            .await
    }
    fn delete_stream(
        &self,
        paths: BoxStream<'static, object_store::Result<Path>>,
    ) -> BoxStream<'static, object_store::Result<Path>> {
        let owner = self.clone();
        paths
            .then(move |path| {
                let owner = owner.clone();
                async move {
                    let path = path?;
                    owner.owner.ensure_lease().await.map_err(store_error)?;
                    owner
                        .owner
                        .while_lease_valid(owner.route(&path)?.delete(&path))
                        .await?;
                    Ok(path)
                }
            })
            .boxed()
    }
    fn list(
        &self,
        prefix: Option<&Path>,
    ) -> BoxStream<'static, object_store::Result<object_store::ObjectMeta>> {
        let prefix = prefix.cloned();
        let router = self.clone();
        futures::stream::once(async move {
            router.owner.ensure_lease().await.map_err(store_error)?;
            let prefix = prefix.ok_or_else(|| store_error(AuthorizationError::Denied))?;
            let mut stream = router.route(&prefix)?.list(Some(&prefix));
            let mut denied = false;
            Ok::<_, object_store::Error>(futures::stream::poll_fn(move |cx| {
                if denied {
                    return std::task::Poll::Ready(None);
                }
                if let Err(error) = router.owner.check_lease() {
                    denied = true;
                    return std::task::Poll::Ready(Some(Err(store_error(error))));
                }
                let result = stream.as_mut().poll_next(cx);
                if let Err(error) = router.owner.check_lease() {
                    denied = true;
                    return std::task::Poll::Ready(Some(Err(store_error(error))));
                }
                result
            }))
        })
        .try_flatten()
        .boxed()
    }
    async fn list_with_delimiter(
        &self,
        prefix: Option<&Path>,
    ) -> object_store::Result<object_store::ListResult> {
        self.owner.ensure_lease().await.map_err(store_error)?;
        self.owner
            .while_lease_valid(
                self.route(prefix.ok_or_else(|| store_error(AuthorizationError::Denied))?)?
                    .list_with_delimiter(prefix),
            )
            .await
    }
    async fn copy_opts(
        &self,
        from: &Path,
        to: &Path,
        options: object_store::CopyOptions,
    ) -> object_store::Result<()> {
        self.owner.ensure_lease().await.map_err(store_error)?;
        self.owner
            .while_lease_valid(self.pair(from, to)?.copy_opts(from, to, options))
            .await
    }
    async fn rename_opts(
        &self,
        from: &Path,
        to: &Path,
        options: object_store::RenameOptions,
    ) -> object_store::Result<()> {
        self.owner.ensure_lease().await.map_err(store_error)?;
        self.owner
            .while_lease_valid(self.pair(from, to)?.rename_opts(from, to, options))
            .await
    }
}

#[derive(Debug)]
struct FencedUpload {
    inner: Box<dyn object_store::MultipartUpload>,
    owner: Arc<RenewableSharedCredentials>,
}
#[async_trait]
impl object_store::MultipartUpload for FencedUpload {
    fn put_part(&mut self, payload: object_store::PutPayload) -> object_store::UploadPart {
        if let Err(error) = self.owner.check_lease() {
            return Box::pin(async move { Err(store_error(error)) });
        }
        let upload = self.inner.put_part(payload);
        let owner = self.owner.clone();
        Box::pin(async move { owner.while_lease_valid(upload).await })
    }
    async fn complete(&mut self) -> object_store::Result<object_store::PutResult> {
        self.owner.ensure_lease().await.map_err(store_error)?;
        let owner = self.owner.clone();
        owner.while_lease_valid(self.inner.complete()).await
    }
    async fn abort(&mut self) -> object_store::Result<()> {
        self.inner.abort().await
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        aws_credentials::{AwsSharedCredentials, BucketConfig},
        azure_credentials::AzureSharedCredentials,
        gcp_credentials::GcpSharedCredentials,
        mixed_credentials::MixedSharedCredentials,
    };
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Source {
        next: Mutex<std::result::Result<SharedCredentials, AuthorizationError>>,
        revoked: AtomicBool,
        calls: AtomicUsize,
    }
    #[async_trait]
    impl SharedCredentialRefresh for Source {
        async fn refresh(&self) -> std::result::Result<SharedCredentials, AuthorizationError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.next.lock().unwrap().clone()
        }
        fn authorization_current(&self) -> std::result::Result<(), AuthorizationError> {
            if self.revoked.load(Ordering::SeqCst) {
                Err(AuthorizationError::Denied)
            } else {
                Ok(())
            }
        }
    }
    fn source_fixture(next: SharedCredentials) -> Arc<Source> {
        Arc::new(Source {
            next: Mutex::new(Ok(next)),
            revoked: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        })
    }
    fn aws(generation: u8) -> SharedCredentials {
        SharedCredentials::Aws(AwsSharedCredentials {
            access_key_id: Some(format!("access-{generation}")),
            secret_access_key: Some("secret".into()),
            session_token: Some(format!("session-{generation}")),
            meta_bucket: "meta".into(),
            content_bucket: "content".into(),
            logs_bucket: String::new(),
            meta_config: None,
            content_config: Some(BucketConfig {
                endpoint: Some("https://storage.example.test".into()),
                use_path_style: true,
                kms_key_arn: Some("alias/project-key".into()),
                kms_bucket_key: true,
                ..Default::default()
            }),
            logs_config: None,
            region: "eu-central-1".into(),
            expiration: Some((SystemTime::now() + Duration::from_secs(3600)).into()),
            content_path_prefix: Some("apps/project".into()),
            user_content_path_prefix: Some("users/user/apps/project".into()),
        })
    }
    fn azure(generation: u8) -> SharedCredentials {
        let token =
            |purpose: &str| Some(format!("sv=2022-11-02&sp=rwdl&sig={purpose}-{generation}"));
        SharedCredentials::Azure(AzureSharedCredentials {
            meta_sas_token: token("meta"),
            content_sas_token: token("content"),
            user_content_sas_token: token("user"),
            logs_sas_token: None,
            tmp_sas_token: token("tmp"),
            draft_meta_sas_token: token("draft"),
            meta_container: "metadata".into(),
            content_container: "content".into(),
            logs_container: String::new(),
            account_name: "account".into(),
            account_key: None,
            expiration: Some((SystemTime::now() + Duration::from_secs(3600)).into()),
            content_path_prefix: Some("apps/project".into()),
            user_content_path_prefix: Some("users/user/apps/project".into()),
            draft_meta_path_prefix: Some("tmp/apps/project".into()),
        })
    }
    fn gcp(generation: u8) -> SharedCredentials {
        SharedCredentials::Gcp(GcpSharedCredentials {
            service_account_key: String::new(),
            access_token: Some(format!("gcp-{generation}")),
            meta_bucket: "meta".into(),
            content_bucket: "content".into(),
            logs_bucket: String::new(),
            allowed_prefixes: vec!["apps/project".into(), "users/user/apps/project".into()],
            write_access: true,
            expiration: Some((SystemTime::now() + Duration::from_secs(3600)).into()),
            content_path_prefix: Some("apps/project".into()),
            user_content_path_prefix: Some("users/user/apps/project".into()),
        })
    }
    fn due(value: &RenewableSharedCredentials) {
        value.state.lock().unwrap().refresh_at = SystemTime::UNIX_EPOCH;
    }

    #[tokio::test]
    async fn retained_store_rotates_and_synchronous_logout_fences_old_handles() -> Result<()> {
        let source = source_fixture(aws(1));
        let owner =
            RenewableSharedCredentials::new(aws(0), "project".into(), source.clone()).await?;
        let store = owner.to_store(false).await?;
        let path = Path::from("apps/project/storage/file");
        let before = store.sign("GET", &path, Duration::from_secs(30)).await?;
        due(&owner);
        owner.refresh_if_due().await?;
        let after = store.sign("GET", &path, Duration::from_secs(30)).await?;
        assert!(
            before
                .query_pairs()
                .any(|(key, value)| key == "X-Amz-Credential" && value.starts_with("access-0/"))
        );
        assert!(
            after
                .query_pairs()
                .any(|(key, value)| key == "X-Amz-Credential" && value.starts_with("access-1/"))
        );
        assert!(
            after
                .query_pairs()
                .any(|(key, value)| key == "X-Amz-Security-Token" && value == "session-1")
        );
        assert!(serde_json::to_string(&SharedCredentials::Renewable(owner.clone())).is_err());
        let metadata = serde_json::to_string(owner.initial())?;
        assert!(!metadata.contains("access-0") && !metadata.contains("session-0"));
        source.revoked.store(true, Ordering::SeqCst);
        assert!(
            store
                .sign("GET", &path, Duration::from_secs(30))
                .await
                .is_err()
        );
        assert_eq!(
            owner.authorization_current(),
            Err(AuthorizationError::Denied)
        );
        Ok(())
    }

    #[tokio::test]
    async fn failed_or_changed_scope_refresh_keeps_valid_lease_and_backs_off() -> Result<()> {
        let source = source_fixture(aws(1));
        let owner =
            RenewableSharedCredentials::new(aws(0), "project".into(), source.clone()).await?;
        let provider = owner.provider(Purpose::Content)?.aws();
        *source.next.lock().unwrap() = Err(AuthorizationError::Unavailable);
        due(&owner);
        assert_eq!(
            owner.refresh_if_due().await,
            Err(AuthorizationError::Unavailable)
        );
        assert_eq!(provider.get_credential().await?.key_id, "access-0");
        owner.refresh_if_due().await?;
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        let mut changed = aws(2);
        if let SharedCredentials::Aws(value) = &mut changed {
            value.user_content_path_prefix = Some("users/other/apps/project".into());
        }
        *source.next.lock().unwrap() = Ok(changed);
        due(&owner);
        assert_eq!(
            owner.refresh_if_due().await,
            Err(AuthorizationError::InvalidResponse)
        );
        assert_eq!(provider.get_credential().await?.key_id, "access-0");
        owner.state.lock().unwrap().expires = SystemTime::UNIX_EPOCH;
        assert!(provider.get_credential().await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn confirmed_denial_is_terminal_even_if_refresh_source_later_recovers() -> Result<()> {
        let source = source_fixture(aws(1));
        let owner =
            RenewableSharedCredentials::new(aws(0), "project".into(), source.clone()).await?;
        let provider = owner.provider(Purpose::Content)?.aws();
        *source.next.lock().unwrap() = Err(AuthorizationError::Denied);
        due(&owner);
        assert_eq!(
            owner.refresh_if_due().await,
            Err(AuthorizationError::Denied)
        );
        *source.next.lock().unwrap() = Ok(aws(2));
        due(&owner);
        assert_eq!(
            owner.refresh_if_due().await,
            Err(AuthorizationError::Denied)
        );
        assert!(provider.get_credential().await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn azure_directory_and_gcp_providers_rotate_with_mixed_bucket_selection() -> Result<()> {
        let mixed = |generation| {
            SharedCredentials::Mixed(MixedSharedCredentials {
                meta: Box::new(aws(generation)),
                content: Box::new(azure(generation)),
                logs: Box::new(gcp(generation)),
            })
        };
        let source = source_fixture(mixed(1));
        let owner = RenewableSharedCredentials::new(mixed(0), "project".into(), source).await?;
        let meta = owner.provider(Purpose::Meta)?.aws();
        let user = owner.provider(Purpose::User)?.azure();
        let scratch = owner.provider(Purpose::Tmp)?.azure();
        let logs = owner.provider(Purpose::Logs)?.gcp();
        due(&owner);
        owner.refresh_if_due().await?;
        assert_eq!(meta.get_credential().await?.key_id, "access-1");
        for (provider, expected) in [(user, "user-1"), (scratch, "tmp-1")] {
            let credential = provider.get_credential().await?;
            match credential.as_ref() {
                object_store::azure::AzureCredential::SASToken(pairs) => assert!(
                    pairs
                        .iter()
                        .any(|(name, value)| name == "sig" && value == expected)
                ),
                _ => panic!("expected scoped SAS"),
            }
        }
        assert_eq!(logs.get_credential().await?.bearer, "gcp-1");
        let content = owner.to_store(false).await?;
        let error = content
            .as_generic()
            .get(&Path::from("users/other/apps/project/file"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("denied"));
        let azure_owner =
            RenewableSharedCredentials::new(azure(0), "project".into(), source_fixture(azure(1)))
                .await?;
        let metadata = azure_owner.to_store(true).await?;
        assert!(
            metadata
                .as_generic()
                .put(&Path::from("tmp/apps/project/draft"), "untrusted".into())
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn routed_signing_uses_native_signers_and_refuses_logout_or_unscoped_paths() -> Result<()>
    {
        let source = source_fixture(aws(1));
        let owner =
            RenewableSharedCredentials::new(aws(0), "project".into(), source.clone()).await?;
        let store = owner.router(vec![
            (
                Path::from("apps/project/metadata"),
                owner.native_store(Purpose::Meta)?,
            ),
            (
                Path::from("apps/project/storage"),
                owner.native_store(Purpose::Content)?,
            ),
        ]);
        let metadata = store
            .sign(
                "GET",
                &Path::from("apps/project/metadata/file"),
                Duration::from_secs(30),
            )
            .await?;
        assert!(
            metadata
                .host_str()
                .is_some_and(|host| host.starts_with("meta."))
                || metadata.path().starts_with("/meta/")
        );
        due(&owner);
        owner.refresh_if_due().await?;
        let path = Path::from("apps/project/storage/file");
        let content = store.sign("GET", &path, Duration::from_secs(30)).await?;
        assert_eq!(content.host_str(), Some("storage.example.test"));
        assert!(content.path().starts_with("/content/apps/project/storage/"));
        assert!(
            content
                .query_pairs()
                .any(|(key, value)| key == "X-Amz-Credential" && value.starts_with("access-1/"))
        );
        assert!(
            store
                .sign(
                    "GET",
                    &Path::from("apps/other/file"),
                    Duration::from_secs(30)
                )
                .await
                .is_err()
        );
        assert!(
            store
                .read_only()
                .sign("GET", &path, Duration::from_secs(30))
                .await
                .is_err()
        );
        source.revoked.store(true, Ordering::SeqCst);
        assert!(
            store
                .sign("GET", &path, Duration::from_secs(30))
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn azure_sas_signing_keeps_native_unsupported_boundary_and_never_uses_ambient_signer()
    -> Result<()> {
        let source = source_fixture(azure(1));
        let owner =
            RenewableSharedCredentials::new(azure(0), "project".into(), source.clone()).await?;
        let content = owner.to_store(false).await?;
        for path in [
            "apps/project/storage/file",
            "users/user/apps/project/file",
            "tmp/user/user/apps/project/file",
        ] {
            let error = content
                .sign("GET", &Path::from(path), Duration::from_secs(30))
                .await
                .unwrap_err();
            assert!(format!("{error:#}").contains("SAS"), "{error:#}");
        }
        let metadata = owner.to_store(true).await?;
        assert!(
            metadata
                .sign(
                    "PUT",
                    &Path::from("tmp/apps/project/draft"),
                    Duration::from_secs(30)
                )
                .await
                .is_err()
        );
        source.revoked.store(true, Ordering::SeqCst);
        let error = content
            .sign(
                "GET",
                &Path::from("apps/project/storage/file"),
                Duration::from_secs(30),
            )
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("denied"));
        Ok(())
    }

    #[tokio::test]
    async fn retained_lazy_listing_and_multipart_are_fenced_before_dispatch() -> Result<()> {
        let source = source_fixture(aws(1));
        let owner =
            RenewableSharedCredentials::new(aws(0), "project".into(), source.clone()).await?;
        let memory = Arc::new(object_store::memory::InMemory::new());
        let root = Path::from("apps/project");
        memory
            .put(&root.clone().join("existing"), "present".into())
            .await?;
        let store = owner
            .router(vec![(root.clone(), FlowLikeStore::Other(memory.clone()))])
            .as_generic();
        let mut listing = store.list(Some(&root));
        let destination = root.join("new");
        let mut upload = store.put_multipart(&destination).await?;
        let part = upload.put_part("pending".into());
        source.revoked.store(true, Ordering::SeqCst);
        assert!(listing.next().await.unwrap().is_err());
        assert!(listing.next().await.is_none());
        assert!(part.await.is_err());
        assert!(upload.complete().await.is_err());
        upload.abort().await?;
        assert!(memory.head(&destination).await.is_err());
        Ok(())
    }

    #[cfg(feature = "flow-runtime")]
    #[tokio::test]
    async fn user_database_paths_match_existing_builders_for_encoded_subjects() -> Result<()> {
        for subject in ["auth0|owner", "issuer:alice", "auth0|josé"] {
            let raw_prefix = format!("users/{subject}/apps/project");
            let expected = super::super::db_path_from_base(&raw_prefix);
            for prefix in [
                raw_prefix.clone(),
                Path::from(raw_prefix.clone()).to_string(),
            ] {
                let mut initial = aws(0);
                if let SharedCredentials::Aws(value) = &mut initial {
                    value.user_content_path_prefix = Some(prefix);
                }
                let owner = RenewableSharedCredentials::new(
                    initial.clone(),
                    "project".into(),
                    source_fixture(initial),
                )
                .await?;
                assert!(
                    owner.to_db_scoped(subject, "project").await.is_ok(),
                    "subject {subject}"
                );
                let registry = owner.lance_registry()?;
                let provider = registry.get_provider("s3").unwrap();
                let uri = owner.database_uri(Purpose::User, &expected)?;
                assert_eq!(provider.extract_path(&uri.parse()?)?, expected);
                assert!(owner.to_db_scoped("different", "project").await.is_err());
            }
        }
        for prefix in [
            "users/other%2Fapps%2Fproject",
            "users/%252e%252e/apps/project",
        ] {
            assert!(prefix_path(prefix).is_err());
        }
        Ok(())
    }

    #[cfg(feature = "flow-runtime")]
    #[tokio::test]
    async fn retained_lance_client_fences_logout_and_preserves_subject_and_local_logs() -> Result<()>
    {
        use flow_like_storage::lance_io::object_store::ObjectStoreParams;
        let source = source_fixture(aws(1));
        let owner =
            RenewableSharedCredentials::new(aws(0), "project".into(), source.clone()).await?;
        let registry = owner.lance_registry_with_local()?;
        assert!(registry.get_provider("file").is_some());
        let store = registry
            .get_store(
                "s3://content/apps/project/storage/db/table.lance".parse()?,
                &ObjectStoreParams::default(),
            )
            .await?;
        assert!(
            registry
                .get_store(
                    "s3://content/apps/other/storage/db/table.lance".parse()?,
                    &ObjectStoreParams::default()
                )
                .await
                .is_err()
        );
        assert!(owner.to_db_scoped("other", "project").await.is_err());
        assert!(owner.to_db_scoped("user", "project").await.is_ok());
        assert!(owner.to_db("other").await.is_err());
        source.revoked.store(true, Ordering::SeqCst);
        let error = store
            .inner
            .head(&Path::from(
                "apps/project/storage/db/table.lance/_latest.manifest",
            ))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("denied"));
        Ok(())
    }
}

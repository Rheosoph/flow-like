use std::{collections::HashMap, fmt, sync::Arc};

use async_trait::async_trait;
use flow_like_storage_files::credentials::RenewableCredentials;
use flow_like_types::reqwest::Url;
use lance::{Error, Result};
use lance_io::object_store::{
    ObjectStore, ObjectStoreParams, ObjectStoreProvider, ObjectStoreRegistry,
};

fn unambiguous_object_path(url: &Url) -> bool {
    let Ok(raw) = object_store::path::Path::parse(url.path()) else {
        return false;
    };
    let Ok(decoded) = object_store::path::Path::from_url_path(url.path()) else {
        return false;
    };
    // Keep URI delimiters and object-store segments identical. Encoded percent
    // signs are excluded so a second decoder cannot introduce another delimiter.
    !url.path().to_ascii_lowercase().contains("%2f")
        && raw.parts().count() == decoded.parts().count()
        && !decoded.as_ref().contains(['%', '\\'])
}

#[derive(Clone, Debug)]
struct SharedConnector(Arc<dyn object_store::client::HttpConnector>);

impl object_store::client::HttpConnector for SharedConnector {
    fn connect(
        &self,
        options: &object_store::ClientOptions,
    ) -> object_store::Result<object_store::client::HttpClient> {
        self.0.connect(options)
    }
}

/// A database location and its authentication scope cannot change during refresh.
#[derive(Clone)]
pub struct LanceStorageBinding {
    root: Url,
    options: HashMap<String, String>,
    credentials: RenewableCredentials,
    store_override: Option<Arc<dyn object_store::ObjectStore>>,
    connector: Option<SharedConnector>,
    object_prefix: Option<object_store::path::Path>,
}

impl fmt::Debug for LanceStorageBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LanceStorageBinding")
            .field("root", &self.root)
            .field("scope", self.credentials.scope())
            .finish_non_exhaustive()
    }
}

impl LanceStorageBinding {
    pub fn new(
        root: &str,
        options: HashMap<String, String>,
        credentials: RenewableCredentials,
    ) -> Result<Self> {
        let root =
            Url::parse(root).map_err(|_| Error::invalid_input("Invalid scoped database URI"))?;
        if !matches!(root.scheme(), "s3" | "az" | "gs")
            || root.host_str().is_none()
            || root.path().trim_matches('/').is_empty()
            || !root.username().is_empty()
            || root.password().is_some()
            || root.query().is_some()
            || root.fragment().is_some()
            || !unambiguous_object_path(&root)
        {
            return Err(Error::invalid_input(
                "Scoped database URI requires a cloud bucket and project prefix",
            ));
        }
        // Only routing and transport settings belong here. Authentication always
        // comes from the provider; SDK-specific alternate backends are excluded.
        if options.keys().any(|key| {
            !matches!(
                key.as_str(),
                "aws_region"
                    | "aws_endpoint"
                    | "aws_virtual_hosted_style_request"
                    | "aws_server_side_encryption"
                    | "aws_sse_kms_key_id"
                    | "aws_sse_bucket_key_enabled"
                    | "azure_storage_account_name"
                    | "azure_storage_endpoint"
                    | "allow_http"
                    | "timeout"
                    | "connect_timeout"
            )
        }) {
            return Err(Error::invalid_input(
                "Unsupported option in scoped database routing",
            ));
        }
        if root.scheme() == "s3"
            && !options
                .get("aws_region")
                .is_some_and(|value| !value.is_empty())
        {
            return Err(Error::invalid_input(
                "Scoped S3 databases require an explicit region",
            ));
        }
        if root.scheme() == "az"
            && !options
                .get("azure_storage_account_name")
                .is_some_and(|value| !value.is_empty())
        {
            return Err(Error::invalid_input(
                "Scoped Azure databases require an explicit account",
            ));
        }
        Ok(Self {
            root,
            options,
            credentials,
            store_override: None,
            connector: None,
            object_prefix: None,
        })
    }

    pub fn with_http_connector<C: object_store::client::HttpConnector>(
        mut self,
        connector: C,
    ) -> Self {
        self.connector = Some(SharedConnector(Arc::new(connector)));
        self
    }

    /// Preserve the issuer's object-key spelling for a URL-encoded user directory.
    /// Table suffixes retain Lance's native cloud encoding.
    pub fn with_object_prefix(mut self, prefix: &str) -> Result<Self> {
        let raw = object_store::path::Path::parse(self.root.path())
            .map_err(|_| Error::invalid_input("Invalid scoped object prefix"))?;
        let decoded = object_store::path::Path::from_url_path(self.root.path())
            .map_err(|_| Error::invalid_input("Invalid scoped object prefix"))?;
        let prefix = object_store::path::Path::parse(prefix)
            .map_err(|_| Error::invalid_input("Invalid scoped object prefix"))?;
        // Existing server paths can contain literal percent escapes produced by
        // Path::from. The issuer chooses the exact spelling; never add aliases.
        if prefix != raw && prefix != decoded {
            return Err(Error::invalid_input(
                "Object prefix must match the scoped database URI",
            ));
        }
        self.object_prefix = Some(prefix);
        Ok(self)
    }

    /// Use the same scoped, cached store as file access for Lance reads and writes.
    pub fn with_store(mut self, store: Arc<dyn object_store::ObjectStore>) -> Self {
        self.store_override = Some(store);
        self
    }

    fn matches(&self, url: &Url) -> bool {
        let prefix = self.root.path().trim_end_matches('/');
        url.scheme() == self.root.scheme()
            && url.authority() == self.root.authority()
            && url.query().is_none()
            && url.fragment().is_none()
            && unambiguous_object_path(url)
            && (url.path() == prefix || url.path().starts_with(&format!("{prefix}/")))
    }

    pub fn build_store(&self) -> Result<Arc<dyn object_store::ObjectStore>> {
        if let Some(store) = &self.store_override {
            return Ok(store.clone());
        }
        let invalid = |_| Error::invalid_input("Invalid scoped cloud storage option");
        let store = match self.root.scheme() {
            "s3" => {
                let mut builder =
                    object_store::aws::AmazonS3Builder::new().with_url(self.root.as_str());
                for (key, value) in &self.options {
                    builder = builder.with_config(key.parse().map_err(invalid)?, value);
                }
                if let Some(connector) = &self.connector {
                    builder = builder.with_http_connector(connector.clone());
                }
                self.credentials.aws_store(builder)
            }
            "az" => {
                let mut builder =
                    object_store::azure::MicrosoftAzureBuilder::new().with_url(self.root.as_str());
                for (key, value) in &self.options {
                    builder = builder.with_config(key.parse().map_err(invalid)?, value);
                }
                if let Some(connector) = &self.connector {
                    builder = builder.with_http_connector(connector.clone());
                }
                self.credentials.azure_store(builder)
            }
            "gs" => {
                let mut builder = object_store::gcp::GoogleCloudStorageBuilder::new()
                    .with_url(self.root.as_str());
                for (key, value) in &self.options {
                    builder = builder.with_config(key.parse().map_err(invalid)?, value);
                }
                if let Some(connector) = &self.connector {
                    builder = builder.with_http_connector(connector.clone());
                }
                self.credentials.gcp_store(builder)
            }
            _ => return Err(Error::invalid_input("Unsupported scoped cloud provider")),
        }
        .map_err(|error| Error::io_source(Box::new(error)))?;
        Ok(store.as_generic())
    }

    fn provider_id(&self) -> String {
        format!(
            "flow-like:{}:{}",
            self.credentials.scope().cache_id(),
            self.root
        )
    }
}

#[derive(Clone)]
struct BoundStoreProvider {
    bindings: Vec<LanceStorageBinding>,
}

impl fmt::Debug for BoundStoreProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundStoreProvider")
            .field("bindings", &self.bindings)
            .finish_non_exhaustive()
    }
}

impl BoundStoreProvider {
    fn binding(&self, url: &Url) -> Result<&LanceStorageBinding> {
        self.bindings
            .iter()
            .find(|binding| binding.matches(url))
            .ok_or_else(|| {
                Error::invalid_input("Database URI is outside the registered storage bindings")
            })
    }
}

#[async_trait]
impl ObjectStoreProvider for BoundStoreProvider {
    async fn new_store(&self, base_path: Url, params: &ObjectStoreParams) -> Result<ObjectStore> {
        let binding = self.binding(&base_path)?;
        let inner = binding.build_store()?;
        let mut params = params.clone();
        // Lance's native providers merge ambient endpoint/signature settings.
        // Its public explicit-store constructor avoids that path. This deprecated
        // field remains necessary until Lance exposes a public wrapper constructor.
        #[allow(deprecated)]
        {
            params.object_store = Some((inner.clone(), base_path.clone()));
        }
        params.object_store_wrapper = None;
        params.list_is_lexically_ordered = Some(true);
        let registry = Arc::new(ObjectStoreRegistry::empty());
        registry.insert(base_path.scheme(), Arc::new(self.clone()));
        let (store, _) =
            ObjectStore::from_uri_and_params(registry, base_path.as_str(), &params).await?;
        let mut store = store.as_ref().clone();
        // The outer registry attaches tracking and caller wrappers once.
        store.inner = inner;
        Ok(store)
    }

    fn extract_path(&self, url: &Url) -> Result<object_store::path::Path> {
        let binding = self.binding(url)?;
        if let Some(prefix) = &binding.object_prefix {
            let suffix = url
                .path()
                .strip_prefix(binding.root.path().trim_end_matches('/'))
                .ok_or_else(|| {
                    Error::invalid_input("Object path is outside the database binding")
                })?;
            return object_store::path::Path::parse(format!("{prefix}{suffix}"))
                .map_err(|_| Error::invalid_input("Invalid scoped object path"));
        }
        // Match Lance's native cloud providers: percent escapes remain object-key
        // bytes. Decoding above only checks for ambiguous delimiters and traversal.
        object_store::path::Path::parse(url.path())
            .map_err(|_| Error::invalid_input("Invalid scoped object path"))
    }

    fn calculate_object_store_prefix(
        &self,
        url: &Url,
        _options: Option<&HashMap<String, String>>,
    ) -> Result<String> {
        Ok(self.binding(url)?.provider_id())
    }
}

/// Only registered cloud locations are available. Local and ambient providers
/// are absent; construct a separate registry for local/offline databases.
pub fn scoped_registry(bindings: Vec<LanceStorageBinding>) -> Result<Arc<ObjectStoreRegistry>> {
    if bindings.is_empty() {
        return Err(Error::invalid_input(
            "At least one storage binding is required",
        ));
    }
    for (index, binding) in bindings.iter().enumerate() {
        if bindings[..index]
            .iter()
            .any(|other| binding.matches(&other.root) || other.matches(&binding.root))
        {
            return Err(Error::invalid_input(
                "Scoped database bindings cannot overlap",
            ));
        }
    }
    let registry = Arc::new(ObjectStoreRegistry::empty());
    for scheme in ["s3", "az", "gs"] {
        let matches: Vec<_> = bindings
            .iter()
            .filter(|binding| binding.root.scheme() == scheme)
            .cloned()
            .collect();
        if matches.is_empty() {
            continue;
        }
        registry.insert(scheme, Arc::new(BoundStoreProvider { bindings: matches }));
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage_files::credentials::{
        StorageCredential, StorageCredentialLease, StorageCredentialProvider,
        StorageCredentialScope,
    };
    use flow_like_types::authorization::AuthorizationError;
    use object_store::ObjectStoreExt;
    use std::{
        sync::atomic::{AtomicBool, Ordering},
        time::{Duration, SystemTime},
    };

    struct Provider {
        scope: StorageCredentialScope,
        denied: AtomicBool,
    }

    #[async_trait]
    impl StorageCredentialProvider for Provider {
        fn scope(&self) -> &StorageCredentialScope {
            &self.scope
        }
        async fn credential(
            &self,
        ) -> std::result::Result<StorageCredentialLease, AuthorizationError> {
            if self.denied.load(Ordering::SeqCst) {
                return Err(AuthorizationError::Denied);
            }
            Ok(StorageCredentialLease {
                scope: self.scope.clone(),
                expires_at: SystemTime::now() + Duration::from_secs(60),
                credential: StorageCredential::AwsSession {
                    access_key_id: "access".into(),
                    secret_access_key: "secret".into(),
                    session_token: "token".into(),
                },
            })
        }
    }

    fn binding() -> (LanceStorageBinding, Arc<Provider>) {
        let provider = Arc::new(Provider {
            scope: StorageCredentialScope {
                instance_id: "instance".into(),
                project_id: "project".into(),
                placement_id: "placement".into(),
                grant_id: "grant".into(),
                resource: "database".into(),
            },
            denied: AtomicBool::new(false),
        });
        let binding = LanceStorageBinding::new(
            "s3://project-bucket/apps/project/storage/db",
            HashMap::from([("aws_region".into(), "us-east-1".into())]),
            RenewableCredentials::new(provider.clone()).unwrap(),
        )
        .unwrap();
        (binding, provider)
    }

    #[tokio::test]
    async fn retained_lance_store_observes_denial_before_dispatch_and_registry_restricts_locations()
    {
        let (binding, provider) = binding();
        let registry = scoped_registry(vec![binding]).unwrap();
        let uri = Url::parse("s3://project-bucket/apps/project/storage/db/table.lance").unwrap();
        let params = ObjectStoreParams::default();
        let store = registry.get_store(uri.clone(), &params).await.unwrap();
        let retained = registry.get_store(uri, &params).await.unwrap();
        assert!(Arc::ptr_eq(&store, &retained));
        provider.denied.store(true, Ordering::SeqCst);
        let error = retained
            .inner
            .head(&object_store::path::Path::from(
                "apps/project/storage/db/table.lance/_latest.manifest",
            ))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("denied"));
        assert!(
            registry
                .get_store(
                    Url::parse("s3://project-bucket/apps/other/storage/db/table.lance").unwrap(),
                    &params
                )
                .await
                .is_err()
        );
        assert!(
            registry
                .get_store(Url::parse("file:///tmp/unregistered").unwrap(), &params)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn lance_uses_the_supplied_cache_store_without_constructing_another_cloud_client() {
        let (binding, _) = binding();
        let memory = Arc::new(object_store::memory::InMemory::new());
        let path =
            object_store::path::Path::from("apps/project/storage/db/table.lance/data/cached.lance");
        memory.put(&path, "cached".into()).await.unwrap();
        let registry = scoped_registry(vec![binding.with_store(memory)]).unwrap();
        let store = registry
            .get_store(
                Url::parse("s3://project-bucket/apps/project/storage/db/table.lance").unwrap(),
                &ObjectStoreParams::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            store.inner.get(&path).await.unwrap().bytes().await.unwrap(),
            "cached"
        );
    }

    #[test]
    fn scoped_database_uris_reject_encoded_delimiters_and_dot_escapes() {
        let (binding, _) = binding();
        let provider = BoundStoreProvider {
            bindings: vec![binding.clone()],
        };
        for path in [
            "s3://project-bucket/apps/project/storage/db/..%2f..%2fother/table.lance",
            "s3://project-bucket/apps/project/storage/db/%252e%252e/other/table.lance",
            "s3://project-bucket/apps/project/storage/db/%5c..%5cother/table.lance",
            "s3://project-bucket/apps/project/storage/db/%2e%2e/other/table.lance",
            "s3://project-bucket/apps/project/storage/db-neighbor/table.lance",
            "s3://project-bucket/apps/project/storage/db//table.lance",
            "s3://project-bucket/apps/project/storage/db/table.lance%2f",
        ] {
            let url = Url::parse(path).unwrap();
            assert!(!binding.matches(&url), "{path}");
            assert!(provider.extract_path(&url).is_err(), "{path}");
        }
        assert!(binding.matches(
            &Url::parse("s3://project-bucket/apps/project/storage/db/table.lance").unwrap()
        ));
    }

    #[test]
    fn encoded_space_and_utf8_paths_match_native_lance_cloud_extraction() {
        let (binding, _) = binding();
        let scoped = BoundStoreProvider {
            bindings: vec![binding],
        };
        let native = ObjectStoreRegistry::default().get_provider("s3").unwrap();
        for suffix in ["table%20name.lance", "caf%C3%A9.lance"] {
            let url = Url::parse(&format!(
                "s3://project-bucket/apps/project/storage/db/{suffix}"
            ))
            .unwrap();
            let path = scoped.extract_path(&url).unwrap();
            assert_eq!(path, native.extract_path(&url).unwrap());
            assert_eq!(path.as_ref(), format!("apps/project/storage/db/{suffix}"));
        }
    }

    #[test]
    fn scoped_user_prefix_decodes_only_the_authorized_root_and_keeps_native_table_names() {
        let (_, provider) = binding();
        let binding = LanceStorageBinding::new(
            "s3://project-bucket/users/josé/apps/project/",
            HashMap::from([("aws_region".into(), "us-east-1".into())]),
            RenewableCredentials::new(provider).unwrap(),
        )
        .unwrap();
        assert!(
            binding
                .clone()
                .with_object_prefix("users/other/apps/project/")
                .is_err()
        );
        let bound = BoundStoreProvider {
            bindings: vec![
                binding
                    .with_object_prefix("users/josé/apps/project/")
                    .unwrap(),
            ],
        };
        let path = bound
            .extract_path(
                &Url::parse("s3://project-bucket/users/josé/apps/project/db/caf%C3%A9.lance")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(path.as_ref(), "users/josé/apps/project/db/caf%C3%A9.lance");
        assert!(
            bound
                .extract_path(
                    &Url::parse("s3://project-bucket/users/other/apps/project/db/table.lance")
                        .unwrap()
                )
                .is_err()
        );
    }

    #[test]
    fn scoped_user_prefix_preserves_the_servers_literal_subject_encoding() {
        let (_, provider) = binding();
        let prefix = object_store::path::Path::from("users/auth0|josé/apps/project");
        let binding = LanceStorageBinding::new(
            &format!("s3://project-bucket/{prefix}/"),
            HashMap::from([("aws_region".into(), "us-east-1".into())]),
            RenewableCredentials::new(provider).unwrap(),
        )
        .unwrap()
        .with_object_prefix(prefix.as_ref())
        .unwrap();
        let bound = BoundStoreProvider {
            bindings: vec![binding],
        };
        let path = bound
            .extract_path(
                &Url::parse(&format!("s3://project-bucket/{prefix}/db/caf%C3%A9.lance")).unwrap(),
            )
            .unwrap();
        assert_eq!(path.as_ref(), format!("{prefix}/db/caf%C3%A9.lance"));
    }

    #[test]
    fn refuses_overlapping_bindings_and_signature_bypass_configuration() {
        let (binding, _) = binding();
        assert!(scoped_registry(vec![binding.clone(), binding.clone()]).is_err());
        assert!(
            LanceStorageBinding::new(
                binding.root.as_str(),
                HashMap::from([
                    ("aws_region".into(), "us-east-1".into()),
                    ("aws_skip_signature".into(), "true".into())
                ]),
                binding.credentials.clone()
            )
            .is_err()
        );
        assert!(
            LanceStorageBinding::new(
                "az://project-files/apps/project/storage/db",
                HashMap::from([
                    ("azure_storage_account_name".into(), "projectaccount".into()),
                    ("azure_storage_use_emulator".into(), "true".into())
                ]),
                binding.credentials
            )
            .is_err()
        );
    }
}

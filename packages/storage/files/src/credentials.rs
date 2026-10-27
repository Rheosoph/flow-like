use std::{
    fmt,
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use flow_like_types_contracts::authorization::AuthorizationError;
use object_store::{
    aws::AwsCredentialProvider, azure::AzureCredentialProvider, gcp::GcpCredentialProvider,
};

/// Storage leases are temporary. The broker can impose a shorter resource TTL.
pub const MAX_STORAGE_LEASE_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StorageCredentialScope {
    pub instance_id: String,
    pub project_id: String,
    pub placement_id: String,
    pub grant_id: String,
    pub resource: String,
}

impl StorageCredentialScope {
    pub fn validate(&self) -> Result<(), AuthorizationError> {
        if [
            &self.instance_id,
            &self.project_id,
            &self.placement_id,
            &self.grant_id,
            &self.resource,
        ]
        .into_iter()
        .any(|value| value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control))
        {
            return Err(AuthorizationError::InvalidRequest);
        }
        Ok(())
    }

    pub fn cache_id(&self) -> String {
        let mut hash = blake3::Hasher::new();
        for value in [
            &self.instance_id,
            &self.project_id,
            &self.placement_id,
            &self.grant_id,
            &self.resource,
        ] {
            hash.update(&(value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        hash.finalize().to_hex().to_string()
    }
}

/// Refreshes contain only authentication material. Store routing stays fixed.
pub enum StorageCredential {
    AwsSession {
        access_key_id: String,
        secret_access_key: String,
        session_token: String,
    },
    AzureSas(String),
    GcpBearer(String),
}

impl fmt::Debug for StorageCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AwsSession { .. } => "AwsSession([redacted])",
            Self::AzureSas(_) => "AzureSas([redacted])",
            Self::GcpBearer(_) => "GcpBearer([redacted])",
        })
    }
}

#[derive(Debug)]
pub struct StorageCredentialLease {
    pub scope: StorageCredentialScope,
    pub expires_at: SystemTime,
    pub credential: StorageCredential,
}

impl StorageCredentialLease {
    pub fn validate(&self, expected: &StorageCredentialScope) -> Result<(), AuthorizationError> {
        if &self.scope != expected {
            return Err(AuthorizationError::InvalidResponse);
        }
        let remaining = self
            .expires_at
            .duration_since(SystemTime::now())
            .map_err(|_| AuthorizationError::Expired)?;
        if remaining.is_zero() || remaining > MAX_STORAGE_LEASE_LIFETIME {
            return Err(AuthorizationError::InvalidResponse);
        }
        let valid = |value: &str| {
            !value.is_empty() && value.len() <= 64 * 1024 && !value.chars().any(char::is_control)
        };
        let valid = match &self.credential {
            StorageCredential::AwsSession {
                access_key_id,
                secret_access_key,
                session_token,
            } => valid(access_key_id) && valid(secret_access_key) && valid(session_token),
            StorageCredential::AzureSas(value) => {
                valid(value)
                    && reqwest::Url::parse(&format!(
                        "https://storage.invalid/?{}",
                        value.trim_start_matches('?')
                    ))
                    .ok()
                    .is_some_and(|url| {
                        url.query_pairs()
                            .any(|(key, value)| key == "sig" && !value.is_empty())
                    })
            }
            StorageCredential::GcpBearer(value) => valid(value),
        };
        if !valid {
            return Err(AuthorizationError::InvalidResponse);
        }
        Ok(())
    }
}

#[async_trait]
pub trait StorageCredentialProvider: Send + Sync {
    fn scope(&self) -> &StorageCredentialScope;
    async fn credential(&self) -> Result<StorageCredentialLease, AuthorizationError>;
}

#[derive(Clone)]
pub struct RenewableCredentials {
    scope: StorageCredentialScope,
    provider: Arc<dyn StorageCredentialProvider>,
}

impl fmt::Debug for RenewableCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenewableCredentials")
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

impl RenewableCredentials {
    pub fn new(provider: Arc<dyn StorageCredentialProvider>) -> Result<Self, AuthorizationError> {
        let scope = provider.scope().clone();
        scope.validate()?;
        Ok(Self { scope, provider })
    }

    pub fn scope(&self) -> &StorageCredentialScope {
        &self.scope
    }

    pub async fn credential(&self) -> Result<StorageCredentialLease, AuthorizationError> {
        if self.provider.scope() != &self.scope {
            return Err(AuthorizationError::InvalidResponse);
        }
        let lease = self.provider.credential().await?;
        lease.validate(&self.scope)?;
        Ok(lease)
    }

    pub fn aws(&self) -> AwsCredentialProvider {
        files::aws(self)
    }
    pub fn azure(&self) -> AzureCredentialProvider {
        files::azure(self)
    }
    pub fn gcp(&self) -> GcpCredentialProvider {
        files::gcp(self)
    }

    pub fn aws_store(
        &self,
        builder: object_store::aws::AmazonS3Builder,
    ) -> object_store::Result<crate::store::FlowLikeStore> {
        Ok(crate::store::FlowLikeStore::AWS(Arc::new(
            files::aws_store(self, builder)?,
        )))
    }

    pub fn azure_store(
        &self,
        builder: object_store::azure::MicrosoftAzureBuilder,
    ) -> object_store::Result<crate::store::FlowLikeStore> {
        Ok(crate::store::FlowLikeStore::Azure(Arc::new(
            files::azure_store(self, builder)?,
        )))
    }

    pub fn gcp_store(
        &self,
        builder: object_store::gcp::GoogleCloudStorageBuilder,
    ) -> object_store::Result<crate::store::FlowLikeStore> {
        // GCS exposes a separate ambient signer without a public override hook.
        // A generic store permits scoped reads/writes and refuses local presigning.
        Ok(crate::store::FlowLikeStore::Other(Arc::new(
            files::gcp_store(self, builder)?,
        )))
    }
}

fn azure_sas_pairs(sas: &str) -> Result<Vec<(String, String)>, AuthorizationError> {
    let url = reqwest::Url::parse(&format!(
        "https://storage.invalid/?{}",
        sas.trim_start_matches('?')
    ))
    .map_err(|_| AuthorizationError::InvalidResponse)?;
    Ok(url.query_pairs().into_owned().collect())
}

/// Credential providers and hardened store builders for one object_store release.
/// The file layer and DataFusion link 0.13; Lance links 0.14.
macro_rules! renewable_stores {
    ($object_store:ident) => {
        use std::sync::Arc;

        use super::{AuthorizationError, RenewableCredentials, StorageCredential, azure_sas_pairs};

        fn store_error(error: AuthorizationError) -> $object_store::Error {
            $object_store::Error::Generic {
                store: "FlowLikeRenewableCredentials",
                source: Box::new(error),
            }
        }

        #[derive(Debug)]
        struct AwsProvider(RenewableCredentials);
        #[async_trait::async_trait]
        impl $object_store::CredentialProvider for AwsProvider {
            type Credential = $object_store::aws::AwsCredential;
            async fn get_credential(&self) -> $object_store::Result<Arc<Self::Credential>> {
                match self.0.credential().await.map_err(store_error)?.credential {
                    StorageCredential::AwsSession {
                        access_key_id,
                        secret_access_key,
                        session_token,
                    } => Ok(Arc::new($object_store::aws::AwsCredential {
                        key_id: access_key_id,
                        secret_key: secret_access_key,
                        token: Some(session_token),
                    })),
                    _ => Err(store_error(AuthorizationError::InvalidResponse)),
                }
            }
        }

        #[derive(Debug)]
        struct AzureProvider(RenewableCredentials);
        #[async_trait::async_trait]
        impl $object_store::CredentialProvider for AzureProvider {
            type Credential = $object_store::azure::AzureCredential;
            async fn get_credential(&self) -> $object_store::Result<Arc<Self::Credential>> {
                match self.0.credential().await.map_err(store_error)?.credential {
                    StorageCredential::AzureSas(sas) => {
                        Ok(Arc::new($object_store::azure::AzureCredential::SASToken(
                            azure_sas_pairs(&sas).map_err(store_error)?,
                        )))
                    }
                    _ => Err(store_error(AuthorizationError::InvalidResponse)),
                }
            }
        }

        #[derive(Debug)]
        struct GcpProvider(RenewableCredentials);
        #[async_trait::async_trait]
        impl $object_store::CredentialProvider for GcpProvider {
            type Credential = $object_store::gcp::GcpCredential;
            async fn get_credential(&self) -> $object_store::Result<Arc<Self::Credential>> {
                match self.0.credential().await.map_err(store_error)?.credential {
                    StorageCredential::GcpBearer(bearer) => {
                        Ok(Arc::new($object_store::gcp::GcpCredential { bearer }))
                    }
                    _ => Err(store_error(AuthorizationError::InvalidResponse)),
                }
            }
        }

        pub fn aws(
            credentials: &RenewableCredentials,
        ) -> $object_store::aws::AwsCredentialProvider {
            Arc::new(AwsProvider(credentials.clone()))
        }

        pub fn azure(
            credentials: &RenewableCredentials,
        ) -> $object_store::azure::AzureCredentialProvider {
            Arc::new(AzureProvider(credentials.clone()))
        }

        pub fn gcp(
            credentials: &RenewableCredentials,
        ) -> $object_store::gcp::GcpCredentialProvider {
            Arc::new(GcpProvider(credentials.clone()))
        }

        pub fn aws_store(
            credentials: &RenewableCredentials,
            builder: $object_store::aws::AmazonS3Builder,
        ) -> $object_store::Result<$object_store::aws::AmazonS3> {
            builder
                .with_skip_signature(false)
                .with_credentials(aws(credentials))
                .build()
        }

        pub fn azure_store(
            credentials: &RenewableCredentials,
            builder: $object_store::azure::MicrosoftAzureBuilder,
        ) -> $object_store::Result<$object_store::azure::MicrosoftAzure> {
            builder
                // The emulator branch bypasses the configured provider entirely.
                .with_use_emulator(false)
                .with_skip_signature(false)
                .with_credentials(azure(credentials))
                .build()
        }

        pub fn gcp_store(
            credentials: &RenewableCredentials,
            builder: $object_store::gcp::GoogleCloudStorageBuilder,
        ) -> $object_store::Result<$object_store::gcp::GoogleCloudStorage> {
            builder
                .with_skip_signature(false)
                .with_credentials(gcp(credentials))
                .build()
        }

        /// Lets several builders share one caller-supplied connector.
        #[derive(Clone, Debug)]
        pub struct SharedConnector(pub Arc<dyn $object_store::client::HttpConnector>);

        impl $object_store::client::HttpConnector for SharedConnector {
            fn connect(
                &self,
                options: &$object_store::ClientOptions,
            ) -> $object_store::Result<$object_store::client::HttpClient> {
                self.0.connect(options)
            }
        }

        /// The store at `root` (`s3://`, `az://` or `gs://`), routed by `options` and
        /// authenticated only through `credentials`.
        pub fn scoped_store(
            credentials: &RenewableCredentials,
            root: &str,
            options: &std::collections::HashMap<String, String>,
            connector: Option<&SharedConnector>,
        ) -> $object_store::Result<Arc<dyn $object_store::ObjectStore>> {
            macro_rules! configured {
                ($builder:expr) => {{
                    let mut builder = $builder.with_url(root);
                    for (key, value) in options {
                        builder = builder.with_config(key.parse()?, value);
                    }
                    if let Some(connector) = connector {
                        builder = builder.with_http_connector(connector.clone());
                    }
                    builder
                }};
            }
            Ok(match root.split_once("://").map(|(scheme, _)| scheme) {
                Some("s3") => Arc::new(aws_store(
                    credentials,
                    configured!($object_store::aws::AmazonS3Builder::new()),
                )?),
                Some("az") => Arc::new(azure_store(
                    credentials,
                    configured!($object_store::azure::MicrosoftAzureBuilder::new()),
                )?),
                Some("gs") => Arc::new(gcp_store(
                    credentials,
                    configured!($object_store::gcp::GoogleCloudStorageBuilder::new()),
                )?),
                _ => {
                    return Err($object_store::Error::NotSupported {
                        source: format!("Unsupported scoped cloud provider in '{root}'").into(),
                    });
                }
            })
        }
    };
}

/// Providers and builders on the file layer's object_store release.
pub mod files {
    renewable_stores!(object_store);
}

/// The same renewable providers and builders for Lance stores.
#[cfg(feature = "lance")]
pub mod lance {
    renewable_stores!(lance_object_store);
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::{aws::AmazonS3Builder, path::Path, signer::Signer};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RotatingProvider {
        scope: StorageCredentialScope,
        generation: AtomicUsize,
    }
    #[async_trait]
    impl StorageCredentialProvider for RotatingProvider {
        fn scope(&self) -> &StorageCredentialScope {
            &self.scope
        }
        async fn credential(&self) -> Result<StorageCredentialLease, AuthorizationError> {
            let generation = self.generation.load(Ordering::SeqCst);
            if generation == 2 {
                return Err(AuthorizationError::Denied);
            }
            let mut scope = self.scope.clone();
            if generation == 3 {
                scope.grant_id = "another-grant".into();
            }
            Ok(StorageCredentialLease {
                scope,
                expires_at: SystemTime::now() + Duration::from_secs(60),
                credential: StorageCredential::AwsSession {
                    access_key_id: format!("key-{generation}"),
                    secret_access_key: "secret".into(),
                    session_token: format!("session-{generation}"),
                },
            })
        }
    }

    #[tokio::test]
    async fn retained_native_store_signs_with_rotated_lease_and_refuses_denial_or_scope_change() {
        let provider = Arc::new(RotatingProvider {
            scope: StorageCredentialScope {
                instance_id: "instance".into(),
                project_id: "project".into(),
                placement_id: "placement".into(),
                grant_id: "grant".into(),
                resource: "content".into(),
            },
            generation: AtomicUsize::new(0),
        });
        let credentials = RenewableCredentials::new(provider.clone()).unwrap();
        let store = AmazonS3Builder::new()
            .with_bucket_name("project-files")
            .with_region("us-east-1")
            .with_credentials(credentials.aws())
            .build()
            .unwrap();
        let path = Path::from("apps/project/storage/file");
        let first = store
            .signed_url(reqwest::Method::GET, &path, Duration::from_secs(30))
            .await
            .unwrap();
        provider.generation.store(1, Ordering::SeqCst);
        let second = store
            .signed_url(reqwest::Method::GET, &path, Duration::from_secs(30))
            .await
            .unwrap();
        assert!(
            first
                .query_pairs()
                .any(|(key, value)| key == "X-Amz-Credential" && value.starts_with("key-0/"))
        );
        assert!(
            second
                .query_pairs()
                .any(|(key, value)| key == "X-Amz-Credential" && value.starts_with("key-1/"))
        );
        assert!(
            second
                .query_pairs()
                .any(|(key, value)| key == "X-Amz-Security-Token" && value == "session-1")
        );
        provider.generation.store(2, Ordering::SeqCst);
        assert!(
            store
                .signed_url(reqwest::Method::GET, &path, Duration::from_secs(30))
                .await
                .is_err()
        );
        provider.generation.store(3, Ordering::SeqCst);
        assert!(
            store
                .signed_url(reqwest::Method::GET, &path, Duration::from_secs(30))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn azure_emulator_cannot_replace_the_required_credential_provider() {
        let provider = Arc::new(RotatingProvider {
            scope: StorageCredentialScope {
                instance_id: "instance".into(),
                project_id: "project".into(),
                placement_id: "placement".into(),
                grant_id: "grant".into(),
                resource: "content".into(),
            },
            generation: AtomicUsize::new(2),
        });
        let store = RenewableCredentials::new(provider)
            .unwrap()
            .azure_store(
                object_store::azure::MicrosoftAzureBuilder::new()
                    .with_account("projectaccount")
                    .with_container_name("project-files")
                    .with_use_emulator(true),
            )
            .unwrap();
        let error = store
            .sign("GET", &Path::from("file"), Duration::from_secs(30))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("denied"));
    }
}

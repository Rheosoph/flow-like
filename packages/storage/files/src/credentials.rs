use std::{
    fmt,
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use flow_like_types_contracts::authorization::AuthorizationError;
use object_store::{
    CredentialProvider,
    aws::{AwsCredential, AwsCredentialProvider},
    azure::{AzureCredential, AzureCredentialProvider},
    gcp::{GcpCredential, GcpCredentialProvider},
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
        Arc::new(AwsProvider(self.clone()))
    }
    pub fn azure(&self) -> AzureCredentialProvider {
        Arc::new(AzureProvider(self.clone()))
    }
    pub fn gcp(&self) -> GcpCredentialProvider {
        Arc::new(GcpProvider(self.clone()))
    }

    pub fn aws_store(
        &self,
        builder: object_store::aws::AmazonS3Builder,
    ) -> object_store::Result<crate::store::FlowLikeStore> {
        Ok(crate::store::FlowLikeStore::AWS(Arc::new(
            builder
                .with_skip_signature(false)
                .with_credentials(self.aws())
                .build()?,
        )))
    }

    pub fn azure_store(
        &self,
        builder: object_store::azure::MicrosoftAzureBuilder,
    ) -> object_store::Result<crate::store::FlowLikeStore> {
        Ok(crate::store::FlowLikeStore::Azure(Arc::new(
            builder
                // The emulator branch bypasses the configured provider entirely.
                .with_use_emulator(false)
                .with_skip_signature(false)
                .with_credentials(self.azure())
                .build()?,
        )))
    }

    pub fn gcp_store(
        &self,
        builder: object_store::gcp::GoogleCloudStorageBuilder,
    ) -> object_store::Result<crate::store::FlowLikeStore> {
        // GCS exposes a separate ambient signer without a public override hook.
        // A generic store permits scoped reads/writes and refuses local presigning.
        Ok(crate::store::FlowLikeStore::Other(Arc::new(
            builder
                .with_skip_signature(false)
                .with_credentials(self.gcp())
                .build()?,
        )))
    }
}

fn store_error(error: AuthorizationError) -> object_store::Error {
    object_store::Error::Generic {
        store: "FlowLikeRenewableCredentials",
        source: Box::new(error),
    }
}

#[derive(Debug)]
struct AwsProvider(RenewableCredentials);
#[async_trait]
impl CredentialProvider for AwsProvider {
    type Credential = AwsCredential;
    async fn get_credential(&self) -> object_store::Result<Arc<AwsCredential>> {
        match self.0.credential().await.map_err(store_error)?.credential {
            StorageCredential::AwsSession {
                access_key_id,
                secret_access_key,
                session_token,
            } => Ok(Arc::new(AwsCredential {
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
#[async_trait]
impl CredentialProvider for AzureProvider {
    type Credential = AzureCredential;
    async fn get_credential(&self) -> object_store::Result<Arc<AzureCredential>> {
        match self.0.credential().await.map_err(store_error)?.credential {
            StorageCredential::AzureSas(sas) => {
                let url = reqwest::Url::parse(&format!(
                    "https://storage.invalid/?{}",
                    sas.trim_start_matches('?')
                ))
                .map_err(|_| store_error(AuthorizationError::InvalidResponse))?;
                Ok(Arc::new(AzureCredential::SASToken(
                    url.query_pairs().into_owned().collect(),
                )))
            }
            _ => Err(store_error(AuthorizationError::InvalidResponse)),
        }
    }
}

#[derive(Debug)]
struct GcpProvider(RenewableCredentials);
#[async_trait]
impl CredentialProvider for GcpProvider {
    type Credential = GcpCredential;
    async fn get_credential(&self) -> object_store::Result<Arc<GcpCredential>> {
        match self.0.credential().await.map_err(store_error)?.credential {
            StorageCredential::GcpBearer(bearer) => Ok(Arc::new(GcpCredential { bearer })),
            _ => Err(store_error(AuthorizationError::InvalidResponse)),
        }
    }
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

use crate::reference::SecretProviderKind;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct SecretStoreConfig {
    pub global_prefix: Option<String>,
    pub cache_ttl: Duration,
    pub negative_cache_ttl: Duration,
    pub max_cache_entries: usize,
    pub providers: Vec<ProviderConfig>,
}

impl Default for SecretStoreConfig {
    fn default() -> Self {
        Self {
            global_prefix: None,
            cache_ttl: Duration::from_secs(300),
            negative_cache_ttl: Duration::from_secs(30),
            max_cache_entries: 10_000,
            providers: Vec::new(),
        }
    }
}

impl SecretStoreConfig {
    pub fn with_provider(mut self, provider: ProviderConfig) -> Self {
        self.providers.push(provider);
        self
    }
}

#[derive(Clone, Debug)]
pub enum ProviderConfig {
    /// Docker Compose and Kubernetes environments where secrets are exposed
    /// via environment variables.
    Env(EnvProviderConfig),
    /// Kubernetes projected secrets or mounted files.
    File(FileProviderConfig),
    AwsParameterStore(AwsParameterStoreProviderConfig),
    AwsSecretsManager(AwsSecretsManagerProviderConfig),
    GcpSecretManager(GcpSecretManagerProviderConfig),
    AzureKeyVault(AzureKeyVaultProviderConfig),
}

impl ProviderConfig {
    pub fn kind(&self) -> SecretProviderKind {
        match self {
            ProviderConfig::Env(_) => SecretProviderKind::Env,
            ProviderConfig::File(_) => SecretProviderKind::File,
            ProviderConfig::AwsParameterStore(_) => SecretProviderKind::AwsParameterStore,
            ProviderConfig::AwsSecretsManager(_) => SecretProviderKind::AwsSecretsManager,
            ProviderConfig::GcpSecretManager(_) => SecretProviderKind::GcpSecretManager,
            ProviderConfig::AzureKeyVault(_) => SecretProviderKind::AzureKeyVault,
        }
    }

    pub fn docker_compose(prefix: Option<String>) -> Self {
        Self::Env(EnvProviderConfig { prefix })
    }

    pub fn kubernetes_env(prefix: Option<String>) -> Self {
        Self::Env(EnvProviderConfig { prefix })
    }

    pub fn kubernetes_files(root_path: impl Into<PathBuf>) -> Self {
        Self::File(FileProviderConfig {
            root_path: root_path.into(),
            trim_trailing_newline: true,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct EnvProviderConfig {
    pub prefix: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FileProviderConfig {
    pub root_path: PathBuf,
    pub trim_trailing_newline: bool,
}

impl Default for FileProviderConfig {
    fn default() -> Self {
        Self {
            root_path: PathBuf::from("/var/run/secrets"),
            trim_trailing_newline: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AwsParameterStoreProviderConfig {
    pub region: Option<String>,
    pub prefix: Option<String>,
    pub with_decryption: bool,
}

#[derive(Clone, Debug, Default)]
pub struct AwsSecretsManagerProviderConfig {
    pub region: Option<String>,
    pub prefix: Option<String>,
    pub version_stage: Option<String>,
}

#[derive(Clone, Debug)]
pub struct GcpSecretManagerProviderConfig {
    pub project_id: Option<String>,
    pub endpoint: String,
    pub prefix: Option<String>,
}

impl Default for GcpSecretManagerProviderConfig {
    fn default() -> Self {
        Self {
            project_id: None,
            endpoint: "https://secretmanager.googleapis.com".to_string(),
            prefix: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AzureKeyVaultProviderConfig {
    pub vault_url: String,
    pub api_version: String,
    pub prefix: Option<String>,
    pub credential: AzureCredentialConfig,
    pub verify_challenge_resource: bool,
}

#[derive(Clone, Debug)]
pub enum AzureCredentialConfig {
    DeveloperTools,
    ManagedIdentity(AzureManagedIdentityConfig),
    ClientSecret(AzureClientSecretCredentialConfig),
}

impl Default for AzureCredentialConfig {
    fn default() -> Self {
        Self::ManagedIdentity(AzureManagedIdentityConfig::default())
    }
}

#[derive(Clone, Debug, Default)]
pub struct AzureManagedIdentityConfig {
    pub user_assigned_id: Option<AzureManagedIdentityId>,
}

#[derive(Clone, Debug)]
pub enum AzureManagedIdentityId {
    ClientId(String),
    ObjectId(String),
    ResourceId(String),
}

#[derive(Clone, Debug)]
pub struct AzureClientSecretCredentialConfig {
    pub tenant_id_env: String,
    pub client_id_env: String,
    pub client_secret_env: String,
}

impl Default for AzureClientSecretCredentialConfig {
    fn default() -> Self {
        Self {
            tenant_id_env: "AZURE_TENANT_ID".to_string(),
            client_id_env: "AZURE_CLIENT_ID".to_string(),
            client_secret_env: "AZURE_CLIENT_SECRET".to_string(),
        }
    }
}

impl Default for AzureKeyVaultProviderConfig {
    fn default() -> Self {
        Self {
            vault_url: "https://localhost".to_string(),
            api_version: "2025-07-01".to_string(),
            prefix: None,
            credential: AzureCredentialConfig::default(),
            verify_challenge_resource: true,
        }
    }
}

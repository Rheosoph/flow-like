# flow-like-secrets

`flow-like-secrets` is a provider abstraction for runtime secret resolution across:

- Docker Compose (environment variables)
- Kubernetes (environment variables and mounted secret files)
- AWS SSM Parameter Store and AWS Secrets Manager
- GCP Secret Manager
- Azure Key Vault

## Goals

- Lazy provider initialization (provider clients are initialized only when first used)
- In-memory TTL cache for resolved secrets
- Secure secret types via `secrecy::SecretString` and `secrecy::SecretBox`
- Zeroization on drop (delegated to `secrecy` wrappers)
- No secret `Display`/`Debug` output in crate types
- Backwards-compatible env override before provider lookup

## Features

- `aws`: enables AWS SSM + Secrets Manager providers
- `gcp`: enables the official `google-cloud-secretmanager-v1` provider
- `azure`: enables the official `azure_security_keyvault_secrets` provider

By default, only local providers (`env`, `file`) are available.

## Reference Format

Fully qualified references:

- `secret://env/OPENAI_API_KEY`
- `secret://file/my-app/token`
- `secret://aws-parameter-store/my/service/key`
- `secret://aws-secrets-manager/my/service/secret?version=123`
- `secret://gcp-secret-manager/my-secret?version=latest`
- `secret://azure-key-vault/my-secret`

Unqualified references (example: `OPENAI_API_KEY`) use provider fallback order from configuration.

## Resolution Order

1. Check for a process environment variable matching `SecretRef.key` exactly.
2. If no env override is present, resolve through the configured provider or fallback order.
3. Provider results still use the in-memory cache.

The env override is checked before cache and before provider lookup so existing
deployments that inject secrets directly via env vars continue to work.

`global_prefix` is a default prefix for prefix-capable remote providers such as
AWS Parameter Store, AWS Secrets Manager, GCP Secret Manager, and Azure Key
Vault. A provider-specific prefix still overrides it.

- AWS treats prefixes as part of the canonical remote path/name.
- GCP and Azure try the prefixed name first, then fall back to the raw secret
  name for compatibility with existing unprefixed secrets.

## Configuration

```rust
use flow_like_secrets::{
    ProviderConfig, SecretStore, SecretStoreConfig,
    FileProviderConfig, EnvProviderConfig,
};

let config = SecretStoreConfig {
    global_prefix: Some("/flow-like/dev/".to_string()),
    cache_ttl: std::time::Duration::from_secs(300),
    negative_cache_ttl: std::time::Duration::from_secs(30),
    max_cache_entries: 10_000,
    providers: vec![
        ProviderConfig::Env(EnvProviderConfig {
            prefix: Some("FLOW".to_string()),
        }),
        ProviderConfig::File(FileProviderConfig {
            root_path: "/var/run/secrets".into(),
            trim_trailing_newline: true,
        }),
    ],
};

let store = SecretStore::new(config)?;
# Ok::<(), flow_like_secrets::SecretError>(())
```

### Docker Compose

Use `ProviderConfig::docker_compose(Some("FLOW".to_string()))` to map keys like `OPENAI_API_KEY` to `FLOW_OPENAI_API_KEY`.

### Kubernetes

- Env secrets: `ProviderConfig::kubernetes_env(...)`
- Mounted files: `ProviderConfig::kubernetes_files("/var/run/secrets")`

### AWS

Enable `aws` feature and configure providers. The AWS implementation uses
official SDK clients (`aws-sdk-ssm` and `aws-sdk-secretsmanager`):

- Path-style prefixes such as `/flow-like/dev/` are preserved

```rust
use flow_like_secrets::{
    ProviderConfig, SecretStoreConfig,
    AwsParameterStoreProviderConfig,
    AwsSecretsManagerProviderConfig,
};

let config = SecretStoreConfig::default()
    .with_provider(ProviderConfig::AwsParameterStore(
        AwsParameterStoreProviderConfig {
            region: Some("us-east-1".to_string()),
            prefix: Some("/flow-like".to_string()),
            with_decryption: true,
        }
    ))
    .with_provider(ProviderConfig::AwsSecretsManager(
        AwsSecretsManagerProviderConfig {
            region: Some("us-east-1".to_string()),
            prefix: Some("flow-like".to_string()),
            version_stage: Some("AWSCURRENT".to_string()),
        }
    ));
# let _ = config;
```

### GCP

Enable `gcp` feature. The provider uses the official Google Secret Manager client
and native Google authentication via Application Default Credentials.

- `project_id` can be set in config, `GCP_PROJECT_ID`, or `GOOGLE_CLOUD_PROJECT`
- `prefix` can namespace relative secret IDs; path-style prefixes like
  `/flow-like/dev/` are normalized to `flow-like-dev-...`
- If the prefixed secret is not found, the provider retries the raw secret name
  before failing
- Credentials should come from standard Google auth sources such as
  `GOOGLE_APPLICATION_CREDENTIALS`, workload identity, or metadata server

```rust
use flow_like_secrets::{
    GcpSecretManagerProviderConfig, ProviderConfig, SecretStoreConfig,
};

let config = SecretStoreConfig::default().with_provider(
    ProviderConfig::GcpSecretManager(GcpSecretManagerProviderConfig {
        project_id: Some("my-project".to_string()),
        endpoint: "https://secretmanager.googleapis.com".to_string(),
        prefix: Some("flow-like".to_string()),
    }),
);
# let _ = config;
```

### Azure

Enable `azure` feature. The provider uses the official Azure Key Vault client
and official identity credentials.

Supported credential modes:

- `ManagedIdentity` for Azure-hosted workloads
- `DeveloperTools` for local development with Azure CLI / Azure Developer CLI
- `ClientSecret` for service principal auth via env vars
- `prefix` can namespace secret names; path-style prefixes like
  `/flow-like/dev/` are normalized to `flow-like-dev-...`
- If the prefixed secret is not found, the provider retries the raw secret name
  before failing

```rust
use flow_like_secrets::{
    AzureCredentialConfig, AzureKeyVaultProviderConfig, ProviderConfig,
    SecretStoreConfig,
};

let config = SecretStoreConfig::default().with_provider(
    ProviderConfig::AzureKeyVault(AzureKeyVaultProviderConfig {
        vault_url: "https://my-vault.vault.azure.net".to_string(),
        api_version: "2025-07-01".to_string(),
        prefix: Some("flow-like".to_string()),
        credential: AzureCredentialConfig::DeveloperTools,
        verify_challenge_resource: true,
    }),
);
# let _ = config;
```

For `ClientSecret`, the default env var names are:

- `AZURE_TENANT_ID`
- `AZURE_CLIENT_ID`
- `AZURE_CLIENT_SECRET`

## Usage

```rust
use flow_like_secrets::{SecretRef, SecretStore};
use flow_like_secrets::ExposeSecret;

async fn load_api_key(store: &SecretStore) -> Result<String, flow_like_secrets::SecretError> {
    let secret = store
        .get_secret_string(&SecretRef::new("OPENAI_API_KEY"))
        .await?;

    Ok(secret.expose_secret().to_string())
}
```

## Testing

- Unit tests: cache behavior, reference parsing, secret value conversions
- Integration tests: file provider, cache invalidation, lazy init behavior, fallback order

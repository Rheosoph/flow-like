use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use flow_like_audit::{
    AuditConfig,
    config_source::{ConfigError, ConfigSource},
    keys::{self, EntryKeySource},
    secret_crypto::encryption_key_material,
    worker::AuditWorkerContext,
};
use flow_like_aws_data::dsql::{DsqlConfig, DsqlDatabase};
use flow_like_db::DbDialect;
use flow_like_secrets::{
    AwsParameterStoreProviderConfig, EnvProviderConfig, ExposeSecret, ProviderConfig, SecretRef,
    SecretStore, SecretStoreConfig,
};
use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use serde::Deserialize;

const EMBEDDED_CONFIG: &str = include_str!("../../../../../flow-like.config.json");

pub struct Worker {
    pub context: AuditWorkerContext,
    pub dsql: Option<Arc<DsqlDatabase>>,
}

#[derive(Deserialize)]
struct AuditDocument {
    #[serde(default)]
    audit: AuditConfig,
}

fn parse_config(document: &str) -> Result<AuditConfig, ConfigError> {
    serde_json::from_str::<AuditDocument>(document)
        .map(|document| document.audit)
        .map_err(|_| ConfigError::Schema)
}

pub async fn initialize(application_name: &str, dsql: Option<DsqlConfig>) -> Result<Worker> {
    let prefix = std::env::var("SECRET_PREFIX").ok();
    let secrets = SecretStore::new(
        SecretStoreConfig::default()
            .with_provider(ProviderConfig::AwsParameterStore(
                AwsParameterStoreProviderConfig {
                    prefix: prefix.clone(),
                    with_decryption: true,
                    ..Default::default()
                },
            ))
            .with_provider(ProviderConfig::Env(EnvProviderConfig { prefix })),
    )?;
    secrets.warmup().await;
    let config = ConfigSource::from_env()?
        .load_with_parser(&secrets, EMBEDDED_CONFIG, parse_config)
        .await?;
    let explicit = optional_secret(&secrets, "AUDIT_ENTRY_KEY").await;
    let backend = optional_secret(&secrets, "BACKEND_KEY").await;
    let source = keys::init_entry_key(explicit.as_deref(), backend.as_deref())?;
    if config.enabled && config.require_signing && source == EntryKeySource::Ephemeral {
        return Err(anyhow!(
            "AUDIT SIGNING REQUIRED: set AUDIT_ENTRY_KEY or BACKEND_KEY so the worker can verify pending records"
        ));
    }
    keys::init_previous_entry_key(
        optional_secret(&secrets, "AUDIT_ENTRY_KEY_PREVIOUS")
            .await
            .as_deref(),
        optional_secret(&secrets, "BACKEND_KEY_PREVIOUS")
            .await
            .as_deref(),
    )?;
    if let Some(keys) = optional_secret(&secrets, "AUDIT_VERIFYING_KEYS")
        .await
        .filter(|value| !value.trim().is_empty())
    {
        flow_like_audit::signer::register_verifying_keys_json(&keys)?;
    }
    let encryption = secrets
        .get_secret_string(&SecretRef::new("SINK_TOKEN_ENCRYPTION_KEY"))
        .await
        .map(|value| value.expose_secret().to_owned());
    let material = encryption_key_material(encryption, cfg!(debug_assertions))
        .map_err(|error| anyhow!("{error}"))?;
    let encryption_key = Some(*blake3::hash(material.as_bytes()).as_bytes());
    let (database, dialect, dsql) = match dsql {
        Some(config) => {
            let connection =
                Arc::new(flow_like_aws_data::dsql::connect_as(&config, application_name).await?);
            (
                connection.connection.clone(),
                DbDialect::Dsql,
                Some(connection),
            )
        }
        None => {
            let database = connect_database(&secrets).await?;
            let dialect = DbDialect::resolve(None, &database).await;
            (database, dialect, None)
        }
    };
    // Keep the process lease across warm invocations, as the API-backed worker did.
    let context = AuditWorkerContext::from_env(database, dialect, config, encryption_key, true)?;
    Ok(Worker { context, dsql })
}

async fn optional_secret(secrets: &SecretStore, name: &str) -> Option<String> {
    secrets
        .get_secret_string(&SecretRef::new(name))
        .await
        .ok()
        .map(|value| value.expose_secret().to_owned())
}

async fn connect_database(secrets: &SecretStore) -> Result<DatabaseConnection> {
    let url = secrets
        .get_secret_string(&SecretRef::new("DATABASE_URL"))
        .await?;
    let pool = flow_like_db::pool::PoolConfig::from_env().map_err(|error| anyhow!("{error}"))?;
    let mut options = ConnectOptions::new(url.expose_secret().to_owned());
    options
        .max_connections(pool.max_connections)
        .min_connections(pool.min_connections)
        .connect_timeout(Duration::from_secs(8))
        .acquire_timeout(Duration::from_secs(8))
        .connect_lazy(true)
        .sqlx_logging(false)
        .map_sqlx_postgres_pool_opts(|pool| {
            pool.after_connect(|connection, _| {
                Box::pin(async move {
                    sea_orm::sqlx::Executor::execute(
                        connection,
                        sea_orm::sqlx::AssertSqlSafe("SET TIME ZONE 'UTC'".to_owned()),
                    )
                    .await
                    .map(|_| ())
                })
            })
        });
    Ok(Database::connect(options).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn optional_key_lookup_tolerates_provider_failure() {
        let secrets = SecretStore::new(
            SecretStoreConfig::default()
                .with_allow_env_override(false)
                .with_provider(ProviderConfig::File(
                    flow_like_secrets::FileProviderConfig {
                        root_path: env!("CARGO_MANIFEST_DIR").into(),
                        trim_trailing_newline: false,
                    },
                )),
        )
        .unwrap();
        // Reading a directory fails inside the provider, rather than reporting a missing key.
        let error = secrets
            .get_secret_string(&SecretRef::new("src"))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            flow_like_secrets::SecretError::ProviderFailure { .. }
        ));
        assert_eq!(optional_secret(&secrets, "src").await, None);
        assert!(optional_secret(&secrets, "Cargo.toml").await.is_some());
    }

    #[test]
    fn audit_configuration_keeps_policy_without_requiring_api_fields() {
        let config = parse_config(
            r#"{"audit":{"enabled":true,"level":"verbose","retention":{"activity_days":12}}}"#,
        )
        .unwrap();
        assert_eq!(config.level, flow_like_audit::AuditLevel::Verbose);
        assert_eq!(config.retention.activity_days, 12);
        assert_eq!(config.retention.seal_after_records, 500);
        assert!(parse_config(r#"{"audit":{"retention":{"activity_days":"wrong"}}}"#).is_err());
    }
}

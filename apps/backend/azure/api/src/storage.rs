use crate::config::Config;
use flow_like_storage::{files::store::FlowLikeStore, object_store::azure::MicrosoftAzureBuilder};
use std::sync::Arc;

pub fn create_cdn_store(config: &Config) -> Result<FlowLikeStore, StorageError> {
    // Config startup validation rejects static credentials, SAS tokens,
    // alternate endpoints, emulator mode, and unsigned access. `from_env`
    // intentionally retains the Container Apps managed-identity endpoint,
    // identity header, and optional user-assigned client ID.
    let store = MicrosoftAzureBuilder::from_env()
        .with_account(&config.storage_account_name)
        .with_container_name(&config.cdn_container)
        .build()
        .map_err(|error| StorageError(error.to_string()))?;

    Ok(FlowLikeStore::Azure(Arc::new(store)))
}

#[derive(Debug)]
pub struct StorageError(String);

impl std::fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "failed to initialize Azure Blob Storage: {}",
            self.0
        )
    }
}

impl std::error::Error for StorageError {}

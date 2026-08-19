use crate::config::Config;
use flow_like_storage::{files::store::FlowLikeStore, object_store::gcp::GoogleCloudStorageBuilder};
use std::sync::Arc;

pub fn create_cdn_store(config: &Config) -> Result<FlowLikeStore, StorageError> {
    // Deliberately `new()` rather than `from_env()`. The Azure image needs
    // `from_env` to pick up the Container Apps managed-identity endpoint and its
    // rotating header; GCP has no such handshake — `object_store` falls all the
    // way through to `InstanceCredentialProvider`, which discovers the metadata
    // server with no environment input at all. `from_env` would therefore add
    // nothing except a second reader of `GOOGLE_SERVICE_ACCOUNT_KEY`,
    // `GOOGLE_SKIP_SIGNATURE` and `GOOGLE_PROXY_URL`, which config startup
    // validation rejects precisely because they can redirect or de-authenticate
    // this store.
    //
    // V4 signed URLs take the same path and land on
    // `InstanceSigningCredentialProvider`, which calls IAM `signBlob` — that is
    // why the service account needs `roles/iam.serviceAccountTokenCreator` on
    // itself even though it holds no key.
    let store = GoogleCloudStorageBuilder::new()
        .with_bucket_name(&config.cdn_bucket)
        .build()
        .map_err(|error| StorageError(error.to_string()))?;

    Ok(FlowLikeStore::Google(Arc::new(store)))
}

#[derive(Debug)]
pub struct StorageError(String);

impl std::fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "failed to initialize Google Cloud Storage: {}",
            self.0
        )
    }
}

impl std::error::Error for StorageError {}

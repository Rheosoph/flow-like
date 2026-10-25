//! GCS Object Retention Lock. URLs are V4-signed by object_store; retention and
//! holds go to the XML/JSON APIs with the API's own token.

use super::backend::{
    SignedUpload, StoredObject, UploadSpec, object_store_inspect, object_store_sha256,
};
use crate::entity::sea_orm_active_enums::LockedRetentionMode;
use chrono::{DateTime, Utc};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_storage::object_store::gcp::GoogleCloudStorageBuilder;
use flow_like_types::{Result, anyhow};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const GCS_ORIGIN: &str = "https://storage.googleapis.com";
const FULL_CONTROL_SCOPE: &str = "https://www.googleapis.com/auth/devstorage.full_control";
const TOKEN_REUSE: Duration = Duration::from_secs(30 * 60);

pub struct GcsLocked {
    store: FlowLikeStore,
    bucket: String,
    service_account_key: Option<String>,
    http: reqwest::Client,
    token: moka::sync::Cache<(), String>,
}

impl GcsLocked {
    pub fn from_env(bucket: &str) -> Result<Self> {
        let service_account_key =
            crate::credentials::gcp_credentials::service_account_key_from_env();
        let mut builder = GoogleCloudStorageBuilder::new().with_bucket_name(bucket);
        if let Some(key) = &service_account_key {
            builder = builder.with_service_account_key(key);
        }
        let store = builder
            .build()
            .map_err(|error| anyhow!("building the locked GCS store failed: {error}"))?;
        Ok(Self {
            store: FlowLikeStore::Google(Arc::new(store)),
            bucket: bucket.to_owned(),
            service_account_key,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .build()?,
            token: moka::sync::Cache::builder()
                .time_to_live(TOKEN_REUSE)
                .build(),
        })
    }

    pub(super) async fn sign_upload(
        &self,
        spec: &UploadSpec<'_>,
        ttl: Duration,
    ) -> Result<SignedUpload> {
        let url = self
            .store
            .sign("PUT", &flow_like_storage::Path::from(spec.key), ttl)
            .await?;
        Ok(SignedUpload {
            url: url.to_string(),
            headers: BTreeMap::from([("content-type".to_owned(), spec.content_type.to_owned())]),
        })
    }

    /// Keys are never reused, so the key alone addresses the version.
    pub(super) async fn sign_download(
        &self,
        key: &str,
        _version_id: Option<&str>,
        _file_name: &str,
        _content_type: &str,
        ttl: Duration,
    ) -> Result<String> {
        Ok(self
            .store
            .sign("GET", &flow_like_storage::Path::from(key), ttl)
            .await?
            .to_string())
    }

    pub(super) async fn inspect(
        &self,
        key: &str,
        _sha256: &[u8; 32],
    ) -> Result<Option<StoredObject>> {
        object_store_inspect(self.store.as_generic().as_ref(), key).await
    }

    pub(super) async fn sha256(
        &self,
        key: &str,
        version_id: Option<&str>,
    ) -> Result<([u8; 32], u64)> {
        object_store_sha256(self.store.as_generic().as_ref(), key, version_id).await
    }

    pub(super) async fn lock(
        &self,
        key: &str,
        version_id: Option<&str>,
        mode: LockedRetentionMode,
        until: DateTime<Utc>,
    ) -> Result<()> {
        let mut url = self.object_url(key, "retention");
        if let Some(generation) = version_id {
            url.push_str(&format!("&generation={}", urlencoding::encode(generation)));
        }
        let response = self
            .http
            .put(url)
            .bearer_auth(self.access_token().await?)
            .header(reqwest::header::CONTENT_TYPE, "application/xml")
            .body(retention_body(mode, until))
            .send()
            .await?;
        expect_success("locking the GCS object", key, response).await
    }

    /// A temporary hold blocks deletion independently of retention, like a legal hold.
    pub(super) async fn legal_hold(
        &self,
        key: &str,
        version_id: Option<&str>,
        enabled: bool,
    ) -> Result<()> {
        let mut url = format!(
            "{GCS_ORIGIN}/storage/v1/b/{}/o/{}",
            urlencoding::encode(&self.bucket),
            urlencoding::encode(key)
        );
        if let Some(generation) = version_id {
            url.push_str(&format!("?generation={}", urlencoding::encode(generation)));
        }
        let response = self
            .http
            .patch(url)
            .bearer_auth(self.access_token().await?)
            .json(&flow_like_types::json::json!({ "temporaryHold": enabled }))
            .send()
            .await?;
        expect_success("setting the GCS temporary hold", key, response).await
    }

    pub(super) async fn delete(&self, key: &str, _version_id: Option<&str>) -> Result<()> {
        match self
            .store
            .as_generic()
            .delete(&flow_like_storage::Path::from(key))
            .await
        {
            Ok(()) | Err(flow_like_storage::object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(anyhow!(
                "deleting the locked GCS object {key} failed: {error}"
            )),
        }
    }

    fn object_url(&self, key: &str, subresource: &str) -> String {
        let path = key
            .split('/')
            .map(|segment| urlencoding::encode(segment).into_owned())
            .collect::<Vec<_>>()
            .join("/");
        format!(
            "{GCS_ORIGIN}/{}/{path}?{subresource}",
            urlencoding::encode(&self.bucket)
        )
    }

    async fn access_token(&self) -> Result<String> {
        if let Some(token) = self.token.get(&()) {
            return Ok(token);
        }
        let token = match &self.service_account_key {
            Some(key) => {
                crate::credentials::gcp_credentials::generate_access_token_standalone(
                    key,
                    FULL_CONTROL_SCOPE,
                )
                .await?
            }
            // The metadata server caches and refreshes its own token.
            None => return crate::credentials::gcp_credentials::fetch_metadata_token().await,
        };
        self.token.insert((), token.clone());
        Ok(token)
    }
}

fn retention_body(mode: LockedRetentionMode, until: DateTime<Utc>) -> String {
    let mode = match mode {
        LockedRetentionMode::Compliance => "COMPLIANCE",
        LockedRetentionMode::Governance => "GOVERNANCE",
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Retention><Mode>{mode}</Mode><RetainUntilDate>{}</RetainUntilDate></Retention>",
        until.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    )
}

async fn expect_success(operation: &str, key: &str, response: reqwest::Response) -> Result<()> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    Err(anyhow!(
        "{operation} {key} returned HTTP {status}: {}",
        body.chars().take(300).collect::<String>()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_body_uses_xml_api_modes_and_rfc3339() {
        let until = DateTime::parse_from_rfc3339("2036-02-15T05:30:00Z")
            .unwrap()
            .to_utc();
        assert_eq!(
            retention_body(LockedRetentionMode::Compliance, until),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Retention><Mode>COMPLIANCE</Mode><RetainUntilDate>2036-02-15T05:30:00Z</RetainUntilDate></Retention>"
        );
        assert!(
            retention_body(LockedRetentionMode::Governance, until)
                .contains("<Mode>GOVERNANCE</Mode>")
        );
    }
}

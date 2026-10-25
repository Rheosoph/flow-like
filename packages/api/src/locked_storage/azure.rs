//! Azure Blob version-level immutability. URLs are SAS-signed by object_store;
//! policies and legal holds are set with the API's managed identity, which needs
//! Storage Blob Data Owner on the container.

use super::backend::{
    SignedUpload, StoredObject, UploadSpec, object_store_inspect, object_store_sha256,
};
use crate::entity::sea_orm_active_enums::LockedRetentionMode;
use crate::storage_config::{AzureConfig, non_empty_env};
use azure_core::credentials::TokenCredential;
use azure_identity::{ManagedIdentityCredential, ManagedIdentityCredentialOptions, UserAssignedId};
use chrono::{DateTime, Utc};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_types::{Result, anyhow};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const STORAGE_SCOPE: &str = "https://storage.azure.com/.default";
const API_VERSION: &str = "2021-12-02";

pub struct AzureLocked {
    store: FlowLikeStore,
    account: String,
    container: String,
    credential: flow_like_types::tokio::sync::OnceCell<Arc<ManagedIdentityCredential>>,
    http: reqwest::Client,
}

impl AzureLocked {
    /// Version-level immutability needs blob versioning, which hierarchical-namespace
    /// accounts lack, so the container usually lives in its own account.
    pub fn from_env(container: &str) -> Result<Self> {
        let dedicated = non_empty_env("AZURE_LOCKED_STORAGE_ACCOUNT_NAME");
        let account = dedicated
            .clone()
            .or_else(|| non_empty_env("AZURE_STORAGE_ACCOUNT_NAME"))
            .ok_or_else(|| {
                anyhow!(
                    "AZURE_LOCKED_STORAGE_ACCOUNT_NAME or AZURE_STORAGE_ACCOUNT_NAME must be set"
                )
            })?;
        let access_key = match dedicated {
            Some(_) => non_empty_env("AZURE_LOCKED_STORAGE_ACCOUNT_KEY"),
            None => non_empty_env("AZURE_STORAGE_ACCOUNT_KEY"),
        };
        let store = AzureConfig {
            account: account.clone(),
            access_key,
        }
        .build_store(container)?;
        Ok(Self {
            store,
            account,
            container: container.to_owned(),
            credential: flow_like_types::tokio::sync::OnceCell::new(),
            http: reqwest::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .build()?,
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
            headers: BTreeMap::from([
                ("content-type".to_owned(), spec.content_type.to_owned()),
                ("x-ms-blob-type".to_owned(), "BlockBlob".to_owned()),
            ]),
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
        let mode = match mode {
            LockedRetentionMode::Compliance => "Locked",
            LockedRetentionMode::Governance => "Unlocked",
        };
        self.blob_call(
            reqwest::Method::PUT,
            "setting the immutability policy",
            key,
            &query("immutabilityPolicies", version_id),
            &[
                ("x-ms-immutability-policy-until-date", http_date(until)),
                ("x-ms-immutability-policy-mode", mode.to_owned()),
            ],
        )
        .await
    }

    pub(super) async fn legal_hold(
        &self,
        key: &str,
        version_id: Option<&str>,
        enabled: bool,
    ) -> Result<()> {
        self.blob_call(
            reqwest::Method::PUT,
            "setting the legal hold",
            key,
            &query("legalhold", version_id),
            &[("x-ms-legal-hold", enabled.to_string())],
        )
        .await
    }

    /// With blob versioning a deleted base blob lives on as a billed previous
    /// version, so the exact version is deleted as well.
    pub(super) async fn delete(&self, key: &str, version_id: Option<&str>) -> Result<()> {
        match self
            .store
            .as_generic()
            .delete(&flow_like_storage::Path::from(key))
            .await
        {
            Ok(()) | Err(flow_like_storage::object_store::Error::NotFound { .. }) => {}
            Err(error) => {
                return Err(anyhow!(
                    "deleting the locked Azure blob {key} failed: {error}"
                ));
            }
        }
        let Some(version_id) = version_id else {
            return Ok(());
        };
        self.blob_call(
            reqwest::Method::DELETE,
            "deleting the locked blob version",
            key,
            &[("versionid", version_id)],
            &[],
        )
        .await
    }

    /// One REST call on a blob with the API's own identity. A missing blob or
    /// version counts as done, so retried deletes converge.
    async fn blob_call(
        &self,
        method: reqwest::Method,
        operation: &str,
        key: &str,
        query: &[(&str, &str)],
        headers: &[(&str, String)],
    ) -> Result<()> {
        let deleting = method == reqwest::Method::DELETE;
        let url = blob_url(&self.account, &self.container, key, query);
        let mut request = self
            .http
            .request(method, url)
            .bearer_auth(self.access_token().await?)
            .header("x-ms-version", API_VERSION)
            .header("x-ms-date", http_date(Utc::now()))
            .header(reqwest::header::CONTENT_LENGTH, "0");
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        let response = request.send().await?;
        let status = response.status();
        if status.is_success() || (deleting && status == reqwest::StatusCode::NOT_FOUND) {
            return Ok(());
        }
        let request_id = response
            .headers()
            .get("x-ms-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unavailable")
            .to_owned();
        let body = response.text().await.unwrap_or_default();
        Err(anyhow!(
            "{operation} on {key} returned HTTP {status} (request {request_id}): {}",
            body.chars().take(300).collect::<String>()
        ))
    }

    async fn access_token(&self) -> Result<String> {
        let credential = self
            .credential
            .get_or_try_init(|| async {
                let user_assigned_id =
                    non_empty_env("AZURE_CLIENT_ID").map(UserAssignedId::ClientId);
                ManagedIdentityCredential::new(Some(ManagedIdentityCredentialOptions {
                    user_assigned_id,
                    ..Default::default()
                }))
                .map_err(|error| anyhow!("initializing the Azure managed identity failed: {error}"))
            })
            .await?;
        let token = credential
            .get_token(&[STORAGE_SCOPE], None)
            .await
            .map_err(|error| anyhow!("acquiring an Azure Storage token failed: {error}"))?;
        Ok(token.token.secret().to_owned())
    }
}

fn http_date(at: DateTime<Utc>) -> String {
    at.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

fn query<'a>(comp: &'a str, version_id: Option<&'a str>) -> Vec<(&'a str, &'a str)> {
    let mut query = vec![("comp", comp)];
    if let Some(version_id) = version_id {
        query.push(("versionid", version_id));
    }
    query
}

fn blob_url(account: &str, container: &str, key: &str, query: &[(&str, &str)]) -> String {
    let path = key
        .split('/')
        .map(|segment| urlencoding::encode(segment).into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let query = query
        .iter()
        .map(|(name, value)| format!("{name}={}", urlencoding::encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("https://{account}.blob.core.windows.net/{container}/{path}?{query}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_requests_address_the_exact_version() {
        assert_eq!(
            blob_url(
                "acct",
                "locked",
                "apps/a/project/i/v",
                &query("immutabilityPolicies", Some("2026-10-21T12:00:00.0000000Z"))
            ),
            "https://acct.blob.core.windows.net/locked/apps/a/project/i/v?comp=immutabilityPolicies&versionid=2026-10-21T12%3A00%3A00.0000000Z"
        );
        assert_eq!(
            blob_url(
                "acct",
                "locked",
                "apps/a/project/i/v",
                &[("versionid", "2026-10-21T12:00:00.0000000Z")]
            ),
            "https://acct.blob.core.windows.net/locked/apps/a/project/i/v?versionid=2026-10-21T12%3A00%3A00.0000000Z"
        );
        assert_eq!(query("legalhold", None), vec![("comp", "legalhold")]);
        let until = DateTime::parse_from_rfc3339("2036-02-15T05:30:00Z")
            .unwrap()
            .to_utc();
        assert_eq!(http_date(until), "Fri, 15 Feb 2036 05:30:00 GMT");
    }
}

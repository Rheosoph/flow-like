#[cfg(feature = "flow-runtime")]
use crate::credentials::{LogsDbBuilder, db_path_from_base};
use crate::credentials::{SharedCredentialsTrait, StoreType};
use flow_like_storage::files::store::FlowLikeStore;
#[cfg(feature = "flow-runtime")]
use flow_like_storage::lancedb;
#[cfg(feature = "flow-runtime")]
use flow_like_storage::lancedb::connection::ConnectBuilder;
use flow_like_storage::object_store;
use flow_like_storage::object_store::{
    GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
    PutMultipartOptions, PutOptions, PutPayload, PutResult,
    azure::{MicrosoftAzure, MicrosoftAzureBuilder},
    path::Path as ObjectPath,
};
use flow_like_types::{Result, anyhow, async_trait};
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Serialize, Deserialize)]
pub struct AzureSharedCredentials {
    /// SAS token for meta container
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_sas_token: Option<String>,
    /// SAS token for content container (app-level access: apps/{app_id})
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_sas_token: Option<String>,
    /// SAS token for user-scoped content (users/{sub}/apps/{app_id})
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_content_sas_token: Option<String>,
    /// SAS token for logs container
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logs_sas_token: Option<String>,
    /// SAS token for the caller's temporary scratch directory
    /// (`tmp/user/{sub}/apps/{app_id}`) on the content container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmp_sas_token: Option<String>,
    /// Read-only SAS for draft artifacts (`tmp/apps/{app_id}`) on the meta
    /// container. A directory SAS signs exactly one directory, so the
    /// `apps/{app_id}` meta SAS cannot also cover the draft prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_meta_sas_token: Option<String>,
    pub meta_container: String,
    pub content_container: String,
    pub logs_container: String,
    pub account_name: String,
    /// SECURITY: Never serialize account_key to prevent leaking master credentials to clients
    #[serde(default, skip_serializing)]
    pub account_key: Option<String>,
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
    /// App-level content path prefix (e.g., "apps/{app_id}")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_path_prefix: Option<String>,
    /// User-level content path prefix (e.g., "users/{sub}/apps/{app_id}")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_content_path_prefix: Option<String>,
    /// Directory signed by `draft_meta_sas_token` (e.g., "tmp/apps/{app_id}")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_meta_path_prefix: Option<String>,
}

impl std::fmt::Debug for AzureSharedCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AzureSharedCredentials")
            .field(
                "meta_sas_token",
                &self.meta_sas_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "content_sas_token",
                &self.content_sas_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "user_content_sas_token",
                &self.user_content_sas_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "logs_sas_token",
                &self.logs_sas_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "tmp_sas_token",
                &self.tmp_sas_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "draft_meta_sas_token",
                &self.draft_meta_sas_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("meta_container", &self.meta_container)
            .field("content_container", &self.content_container)
            .field("logs_container", &self.logs_container)
            .field("account_name", &self.account_name)
            .field(
                "account_key",
                &self.account_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("expiration", &self.expiration)
            .finish()
    }
}

impl AzureSharedCredentials {
    /// Whether this credential set came from `scoped_credentials` rather than from
    /// server configuration.
    ///
    /// Master credentials carry no SAS and no expiry: they are meant to run as the
    /// ambient identity. A scoped credential set is the opposite — the tokens it
    /// carries are the *only* thing enforcing the prefix isolation, so a store it
    /// cannot authorize must fail rather than quietly borrow the workload identity.
    fn is_scoped(&self) -> bool {
        self.expiration.is_some()
            || self.content_path_prefix.is_some()
            || self.user_content_path_prefix.is_some()
            || self.draft_meta_path_prefix.is_some()
            || self.meta_sas_token.is_some()
            || self.content_sas_token.is_some()
            || self.user_content_sas_token.is_some()
            || self.logs_sas_token.is_some()
            || self.tmp_sas_token.is_some()
            || self.draft_meta_sas_token.is_some()
    }

    /// The container and SAS that authorize `store_type`.
    ///
    /// `Content` falls back to the user-scoped token because the user-only scopes
    /// (`ReadUser`, `EditUser`, `InvokeNone`) mint nothing else: without this the
    /// content store for those modes has no token at all.
    fn store_credentials(&self, store_type: StoreType) -> (&String, Option<&String>) {
        match store_type {
            StoreType::Meta => (&self.meta_container, self.meta_sas_token.as_ref()),
            StoreType::Content => (
                &self.content_container,
                self.content_sas_token
                    .as_ref()
                    .or(self.user_content_sas_token.as_ref()),
            ),
            StoreType::Logs => (&self.logs_container, self.logs_sas_token.as_ref()),
            StoreType::Tmp => (
                &self.content_container,
                self.tmp_sas_token
                    .as_ref()
                    .or(self.content_sas_token.as_ref()),
            ),
        }
    }

    /// The SAS that authorises a LanceDB path, or an error when a scoped
    /// credential carries none.
    ///
    /// `make_azure_builder` drops a blank token, and a `MicrosoftAzureBuilder`
    /// with no credential configured falls through to
    /// `ImdsManagedIdentityProvider` — the workload's own managed identity,
    /// which carries no prefix restriction at all. Under master credentials
    /// that is the intended identity; under a scoped credential it would trade
    /// the isolation the token exists to enforce for availability, silently, so
    /// it fails closed here exactly as `to_store_type` does.
    #[cfg(feature = "flow-runtime")]
    fn lance_sas_or_ambient(&self, token: Option<&String>, what: &str) -> Result<String> {
        match token
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            Some(token) => Ok(token.to_string()),
            None if self.is_scoped() => Err(anyhow!(
                "scoped Azure credentials carry no SAS for {what} - refusing to fall back to the \
                 ambient storage identity, which enforces no prefix restriction"
            )),
            None => Ok(String::new()),
        }
    }

    async fn build_store(
        account: String,
        container: String,
        account_key: Option<String>,
        sas_token: Option<String>,
    ) -> Result<Arc<MicrosoftAzure>> {
        use flow_like_types::tokio;

        let store = tokio::task::spawn_blocking(move || {
            // `from_env` is required for Azure Container Apps/App Service managed
            // identity because it carries IDENTITY_ENDPOINT and AZURE_CLIENT_ID
            // into object_store's MSI credential provider. Keep scoped SAS and
            // legacy account-key credentials isolated from ambient auth settings.
            let builder = if account_key.is_none() && sas_token.is_none() {
                MicrosoftAzureBuilder::from_env()
            } else {
                MicrosoftAzureBuilder::new()
            }
            .with_account(account)
            .with_container_name(container);

            // Use account key for master credentials, SAS for scoped credentials
            if let Some(key) = account_key {
                builder.with_access_key(key).build()
            } else if let Some(sas) = sas_token {
                let sas_pairs = Self::parse_sas_token(&sas);
                builder.with_sas_authorization(sas_pairs).build()
            } else {
                builder.build()
            }
        })
        .await
        .map_err(|e| anyhow!("Failed to spawn blocking task: {}", e))??;

        Ok(Arc::new(store))
    }

    fn parse_sas_token(sas: &str) -> Vec<(String, String)> {
        let sas = sas.trim_start_matches('?');
        sas.split('&')
            .filter_map(|pair| {
                let mut parts = pair.splitn(2, '=');
                match (parts.next(), parts.next()) {
                    (Some(k), Some(v)) => {
                        // URL-decode the value since object_store will re-encode it
                        let decoded_v = percent_encoding::percent_decode_str(v)
                            .decode_utf8()
                            .map(|cow| cow.into_owned())
                            .unwrap_or_else(|_| v.to_string());
                        Some((k.to_string(), decoded_v))
                    }
                    _ => None,
                }
            })
            .collect()
    }
}

#[async_trait]
impl SharedCredentialsTrait for AzureSharedCredentials {
    #[tracing::instrument(name = "AzureSharedCredentials::to_store", skip(self, meta), fields(meta = meta), level="debug")]
    async fn to_store(&self, meta: bool) -> Result<FlowLikeStore> {
        self.to_store_type(if meta {
            StoreType::Meta
        } else {
            StoreType::Content
        })
        .await
    }

    #[tracing::instrument(name = "AzureSharedCredentials::to_store_type", skip(self), fields(store_type = ?store_type), level="debug")]
    async fn to_store_type(&self, store_type: StoreType) -> Result<FlowLikeStore> {
        let (container, sas_token) = self.store_credentials(store_type);

        let container = container.clone();
        let account_key = self
            .account_key
            .clone()
            .filter(|value| !value.trim().is_empty());
        let sas_token = sas_token.cloned().filter(|value| !value.trim().is_empty());

        // A scoped credential set that cannot authorize this store must fail here.
        // Falling through to `from_env` below would build the store from the
        // workload's own managed identity, which carries no prefix restriction at
        // all — turning a deliberately withheld scope into full container access.
        if account_key.is_none() && sas_token.is_none() && self.is_scoped() {
            return Err(anyhow!(
                "scoped Azure credentials carry no SAS for the {:?} store - refusing to fall back \
                 to the ambient storage identity, which enforces no prefix restriction",
                store_type
            ));
        }

        let store =
            Self::build_store(self.account_name.clone(), container, account_key, sas_token).await?;

        // A directory SAS signs exactly one directory, so draft artifacts under
        // `tmp/apps/{app_id}` need a token of their own; serve them from a
        // second, read-only store while every other meta path keeps the
        // `apps/{app_id}` SAS. Master credentials mint no draft SAS and keep
        // the plain store, whose identity already covers the whole container.
        if store_type == StoreType::Meta
            && let Some(draft_sas) = self
                .draft_meta_sas_token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            && let Some(draft_prefix) = self
                .draft_meta_path_prefix
                .as_deref()
                .map(|value| value.trim_matches('/'))
                .filter(|value| !value.is_empty())
        {
            let drafts = Self::build_store(
                self.account_name.clone(),
                self.meta_container.clone(),
                None,
                Some(draft_sas.to_string()),
            )
            .await?;
            return Ok(FlowLikeStore::Other(Arc::new(DraftScopedAzureMetaStore {
                primary: store,
                drafts,
                draft_prefix: ObjectPath::from(draft_prefix),
            })));
        }

        Ok(FlowLikeStore::Azure(store))
    }

    #[tracing::instrument(name = "AzureSharedCredentials::to_db", skip(self), level = "debug")]
    #[cfg(feature = "flow-runtime")]
    async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder> {
        let base_path = self
            .content_path_prefix
            .clone()
            .unwrap_or_else(|| format!("apps/{}", app_id));
        let sas_token =
            self.lance_sas_or_ambient(self.content_sas_token.as_ref(), "the app database")?;

        let path = db_path_from_base(&base_path);
        let connection = make_azure_builder(
            self.account_name.clone(),
            self.content_container.clone(),
            sas_token,
        );
        let connection = connection(path.clone());
        Ok(connection)
    }

    #[tracing::instrument(
        name = "AzureSharedCredentials::to_db_scoped",
        skip(self, sub),
        level = "debug"
    )]
    #[cfg(feature = "flow-runtime")]
    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        let base_path = format!("users/{}/apps/{}", sub, app_id);
        let sas_token = self.lance_sas_or_ambient(
            self.user_content_sas_token
                .as_ref()
                .or(self.content_sas_token.as_ref()),
            "the user database",
        )?;

        let path = db_path_from_base(&base_path);
        let connection = make_azure_builder(
            self.account_name.clone(),
            self.content_container.clone(),
            sas_token,
        );
        let connection = connection(path.clone());
        Ok(connection)
    }

    #[cfg(feature = "flow-runtime")]
    fn to_logs_db_builder(&self) -> Result<LogsDbBuilder> {
        if self.logs_container.is_empty() {
            return Err(anyhow!(
                "logs_container is empty - cannot create logs database builder"
            ));
        }
        let sas_token =
            self.lance_sas_or_ambient(self.logs_sas_token.as_ref(), "the logs database")?;
        let builder = make_azure_builder(
            self.account_name.clone(),
            self.logs_container.clone(),
            sas_token,
        );
        Ok(Arc::new(builder))
    }
}

/// Meta store for a scoped Azure credential that carries a second, read-only
/// SAS for draft artifacts.
///
/// An Azure directory SAS signs exactly one directory, so the `apps/{app_id}`
/// meta SAS cannot also cover `tmp/apps/{app_id}`
/// (see [`crate::flow::compiled::draft_artifact_dir`]). Paths under the draft
/// prefix are served by the draft store; everything else keeps the primary
/// meta store. Writes under the draft prefix are refused locally: only the
/// API writes drafts — the trust anchor for `entry_authority_revision` — so
/// this store never even presents its read-only token for one.
#[derive(Debug)]
struct DraftScopedAzureMetaStore {
    primary: Arc<MicrosoftAzure>,
    drafts: Arc<MicrosoftAzure>,
    draft_prefix: ObjectPath,
}

impl DraftScopedAzureMetaStore {
    fn in_draft_scope(&self, location: &ObjectPath) -> bool {
        location.prefix_match(&self.draft_prefix).is_some()
    }

    fn store_for(&self, location: &ObjectPath) -> &Arc<MicrosoftAzure> {
        if self.in_draft_scope(location) {
            &self.drafts
        } else {
            &self.primary
        }
    }

    fn draft_write_denied(&self, location: &ObjectPath) -> object_store::Error {
        object_store::Error::PermissionDenied {
            path: location.to_string(),
            source: format!(
                "draft artifacts under {} are read-only for executors - only the API writes them",
                self.draft_prefix
            )
            .into(),
        }
    }

    fn cross_scope_copy(&self, from: &ObjectPath) -> object_store::Error {
        object_store::Error::NotSupported {
            source: format!(
                "cannot copy {from} out of the read-only draft scope {} - the two directories \
                 are signed by different SAS tokens",
                self.draft_prefix
            )
            .into(),
        }
    }
}

impl std::fmt::Display for DraftScopedAzureMetaStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DraftScopedAzureMetaStore({}, read-only drafts at {})",
            self.primary, self.draft_prefix
        )
    }
}

#[async_trait]
impl ObjectStore for DraftScopedAzureMetaStore {
    async fn put_opts(
        &self,
        location: &ObjectPath,
        payload: PutPayload,
        opts: PutOptions,
    ) -> object_store::Result<PutResult> {
        if self.in_draft_scope(location) {
            return Err(self.draft_write_denied(location));
        }
        self.primary.put_opts(location, payload, opts).await
    }

    async fn put_multipart_opts(
        &self,
        location: &ObjectPath,
        opts: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        if self.in_draft_scope(location) {
            return Err(self.draft_write_denied(location));
        }
        self.primary.put_multipart_opts(location, opts).await
    }

    async fn get_opts(
        &self,
        location: &ObjectPath,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.store_for(location).get_opts(location, options).await
    }

    async fn delete(&self, location: &ObjectPath) -> object_store::Result<()> {
        if self.in_draft_scope(location) {
            return Err(self.draft_write_denied(location));
        }
        self.primary.delete(location).await
    }

    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        match prefix {
            Some(prefix) => self.store_for(prefix).list(Some(prefix)),
            None => self.primary.list(None),
        }
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> object_store::Result<ListResult> {
        match prefix {
            Some(prefix) => {
                self.store_for(prefix)
                    .list_with_delimiter(Some(prefix))
                    .await
            }
            None => self.primary.list_with_delimiter(None).await,
        }
    }

    async fn copy(&self, from: &ObjectPath, to: &ObjectPath) -> object_store::Result<()> {
        if self.in_draft_scope(to) {
            return Err(self.draft_write_denied(to));
        }
        if self.in_draft_scope(from) {
            return Err(self.cross_scope_copy(from));
        }
        self.primary.copy(from, to).await
    }

    async fn copy_if_not_exists(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
    ) -> object_store::Result<()> {
        if self.in_draft_scope(to) {
            return Err(self.draft_write_denied(to));
        }
        if self.in_draft_scope(from) {
            return Err(self.cross_scope_copy(from));
        }
        self.primary.copy_if_not_exists(from, to).await
    }
}

#[cfg(feature = "flow-runtime")]
fn make_azure_builder(
    account_name: String,
    container: String,
    sas_token: String,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder + Send + Sync + 'static {
    move |path| {
        let url = format!("az://{}/{}", container, path);
        let builder = lancedb::connect(&url).storage_option(
            "azure_storage_account_name".to_string(),
            account_name.clone(),
        );

        if sas_token.trim().is_empty() {
            builder
        } else {
            builder.storage_option("azure_storage_sas_token".to_string(), sas_token.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::{from_str, to_string};

    fn sample_credentials() -> AzureSharedCredentials {
        AzureSharedCredentials {
            meta_sas_token: Some("?sv=2022-11-02&ss=b&srt=sco&sp=rl&se=2025-01-15T20:00:00Z&st=2025-01-15T12:00:00Z&spr=https&sig=meta123".to_string()),
            content_sas_token: Some("?sv=2022-11-02&ss=b&srt=sco&sp=rwdlacyx&se=2025-01-15T20:00:00Z&st=2025-01-15T12:00:00Z&spr=https&sig=content456".to_string()),
            user_content_sas_token: None,
            logs_sas_token: Some("?sv=2022-11-02&ss=b&srt=sco&sp=rl&se=2025-01-15T20:00:00Z&st=2025-01-15T12:00:00Z&spr=https&sig=logs789".to_string()),
            tmp_sas_token: None,
            draft_meta_sas_token: None,
            meta_container: "meta-container".to_string(),
            content_container: "content-container".to_string(),
            logs_container: "logs-container".to_string(),
            account_name: "mystorageaccount".to_string(),
            account_key: None,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
            draft_meta_path_prefix: None,
        }
    }

    #[test]
    fn test_azure_credentials_serialization() {
        let creds = sample_credentials();
        let json = to_string(&creds).expect("Failed to serialize");

        assert!(json.contains("mystorageaccount"));
        assert!(json.contains("meta-container"));
        assert!(json.contains("content-container"));
        assert!(json.contains("sv=2022-11-02"));
    }

    #[test]
    fn test_azure_credentials_deserialization() {
        let json = r#"{
            "sas_token": "?sv=2022-11-02&ss=b&srt=sco&sp=r&se=2025-01-15T20:00:00Z&sig=test",
            "meta_container": "test-meta",
            "content_container": "test-content",
            "logs_container": "test-logs",
            "account_name": "teststorage",
            "expiration": null
        }"#;

        let creds: AzureSharedCredentials = from_str(json).expect("Failed to deserialize");

        assert_eq!(creds.account_name, "teststorage");
        assert_eq!(creds.meta_container, "test-meta");
        assert_eq!(creds.content_container, "test-content");
        assert!(creds.expiration.is_none());
    }

    #[test]
    fn test_azure_credentials_roundtrip() {
        let original = sample_credentials();
        let json = to_string(&original).expect("Failed to serialize");
        let deserialized: AzureSharedCredentials = from_str(&json).expect("Failed to deserialize");

        assert_eq!(original.meta_container, deserialized.meta_container);
        assert_eq!(original.content_container, deserialized.content_container);
        assert_eq!(original.account_name, deserialized.account_name);
    }

    #[test]
    fn test_azure_credentials_with_expiration() {
        let json = r#"{
            "sas_token": "?sv=2022-11-02&ss=b&sig=test",
            "meta_container": "meta",
            "content_container": "content",
            "logs_container": "logs",
            "account_name": "storage",
            "expiration": "2025-01-15T12:00:00Z"
        }"#;

        let creds: AzureSharedCredentials = from_str(json).expect("Failed to deserialize");
        assert!(creds.expiration.is_some());
    }

    #[test]
    fn test_parse_sas_token_with_question_mark() {
        let sas = "?sv=2022-11-02&ss=b&srt=sco&sp=rwdlacyx&se=2025-01-15T20:00:00Z";
        let pairs = AzureSharedCredentials::parse_sas_token(sas);

        assert!(pairs.iter().any(|(k, _)| k == "sv"));
        assert!(pairs.iter().any(|(k, _)| k == "ss"));
        assert!(pairs.iter().any(|(k, _)| k == "srt"));
        assert!(pairs.iter().any(|(k, _)| k == "sp"));
        assert!(pairs.iter().any(|(k, _)| k == "se"));
    }

    #[test]
    fn test_parse_sas_token_without_question_mark() {
        let sas = "sv=2022-11-02&ss=b&srt=sco";
        let pairs = AzureSharedCredentials::parse_sas_token(sas);

        assert_eq!(pairs.len(), 3);
        assert!(pairs.iter().any(|(k, v)| k == "sv" && v == "2022-11-02"));
        assert!(pairs.iter().any(|(k, v)| k == "ss" && v == "b"));
        assert!(pairs.iter().any(|(k, v)| k == "srt" && v == "sco"));
    }

    #[test]
    fn test_parse_sas_token_decodes_url_encoded_values() {
        let sas = "sv=2022-11-02&se=2025-01-15T20%3A00%3A00Z&sig=abc%2Fdef%3D";
        let pairs = AzureSharedCredentials::parse_sas_token(sas);

        // Values should be URL-decoded since object_store will re-encode them
        let se = pairs
            .iter()
            .find(|(k, _)| k == "se")
            .map(|(_, v)| v.as_str());
        assert_eq!(se, Some("2025-01-15T20:00:00Z"));

        let sig = pairs
            .iter()
            .find(|(k, _)| k == "sig")
            .map(|(_, v)| v.as_str());
        assert_eq!(sig, Some("abc/def="));
    }

    #[test]
    fn test_parse_sas_token_empty() {
        let pairs = AzureSharedCredentials::parse_sas_token("");
        assert!(pairs.is_empty());
    }

    /// A `ReadUser` / `EditUser` / `InvokeNone` credential set: the user-scoped SAS
    /// is the only token minted.
    fn user_scoped_credentials() -> AzureSharedCredentials {
        AzureSharedCredentials {
            meta_sas_token: None,
            content_sas_token: None,
            user_content_sas_token: Some("sv=2022-11-02&sr=d&sp=rwdl&sig=user".to_string()),
            logs_sas_token: None,
            tmp_sas_token: None,
            draft_meta_sas_token: None,
            meta_container: "meta".to_string(),
            content_container: "content".to_string(),
            logs_container: "logs".to_string(),
            account_name: "storage".to_string(),
            account_key: None,
            expiration: Some(chrono::Utc::now()),
            content_path_prefix: None,
            user_content_path_prefix: Some("users/test-user/apps/test-app".to_string()),
            draft_meta_path_prefix: None,
        }
    }

    fn master_credentials() -> AzureSharedCredentials {
        AzureSharedCredentials {
            meta_sas_token: None,
            content_sas_token: None,
            user_content_sas_token: None,
            logs_sas_token: None,
            tmp_sas_token: None,
            draft_meta_sas_token: None,
            meta_container: "meta".to_string(),
            content_container: "content".to_string(),
            logs_container: "logs".to_string(),
            account_name: "storage".to_string(),
            account_key: None,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
            draft_meta_path_prefix: None,
        }
    }

    /// A `ServerExecute` credential set: app-scoped meta SAS plus the
    /// read-only draft SAS for `tmp/apps/{app_id}`.
    fn server_execute_credentials() -> AzureSharedCredentials {
        AzureSharedCredentials {
            meta_sas_token: Some("sv=2022-11-02&sr=d&sp=rl&sig=meta".to_string()),
            content_sas_token: Some("sv=2022-11-02&sr=d&sp=rwdl&sig=content".to_string()),
            user_content_sas_token: None,
            logs_sas_token: Some("sv=2022-11-02&sr=d&sp=rwl&sig=logs".to_string()),
            tmp_sas_token: None,
            draft_meta_sas_token: Some("sv=2022-11-02&sr=d&sp=rl&sig=draft".to_string()),
            meta_container: "meta".to_string(),
            content_container: "content".to_string(),
            logs_container: "logs".to_string(),
            account_name: "storage".to_string(),
            account_key: None,
            expiration: Some(chrono::Utc::now()),
            content_path_prefix: Some("apps/test-app".to_string()),
            user_content_path_prefix: None,
            draft_meta_path_prefix: Some("tmp/apps/test-app".to_string()),
        }
    }

    #[test]
    fn test_scoped_and_master_credentials_are_distinguishable() {
        assert!(user_scoped_credentials().is_scoped());
        assert!(!master_credentials().is_scoped());
    }

    #[flow_like_types::tokio::test]
    async fn test_scoped_credentials_never_fall_back_to_ambient_identity() {
        let creds = user_scoped_credentials();

        for store_type in [StoreType::Meta, StoreType::Logs] {
            let error = creds
                .to_store_type(store_type)
                .await
                .expect_err("a scoped credential with no SAS for this store must not build one");
            assert!(
                error.to_string().contains("refusing to fall back"),
                "unexpected error for {store_type:?}: {error}"
            );
        }
    }

    /// The store path was hardened first; the three LanceDB paths kept the old
    /// `unwrap_or_default()` shape, and `make_azure_builder` drops a blank token
    /// so `MicrosoftAzureBuilder` resolved the workload's managed identity —
    /// unrestricted across the whole container.
    #[cfg(feature = "flow-runtime")]
    #[flow_like_types::tokio::test]
    async fn test_scoped_credentials_never_build_a_database_on_the_ambient_identity() {
        let creds = user_scoped_credentials();

        // No content SAS at all: the app database is out of scope.
        let error = creds
            .to_db("test-app")
            .await
            .expect_err("a scoped credential with no app SAS must not build an app database");
        assert!(
            error.to_string().contains("refusing to fall back"),
            "{error}"
        );

        // No logs SAS: every client invoke mode is in this shape now.
        // `LogsDbBuilder` is a boxed closure, so this cannot use `expect_err`.
        let Err(error) = creds.to_logs_db_builder() else {
            panic!("a scoped credential with no logs SAS must not build a logs database");
        };
        assert!(
            error.to_string().contains("refusing to fall back"),
            "{error}"
        );

        // The user database is the one scope this credential does carry.
        creds
            .to_db_scoped("test-user", "test-app")
            .await
            .expect("the user database is signed by the user SAS");
    }

    /// Master credentials have no SAS by design and must keep resolving the
    /// workload identity, which is the whole point of that mode.
    #[cfg(feature = "flow-runtime")]
    #[flow_like_types::tokio::test]
    async fn test_master_credentials_still_build_databases_without_a_sas() {
        let creds = master_credentials();
        assert!(creds.to_db("test-app").await.is_ok());
        assert!(creds.to_db_scoped("test-user", "test-app").await.is_ok());
        assert!(creds.to_logs_db_builder().is_ok());
    }

    #[flow_like_types::tokio::test]
    async fn test_content_store_uses_the_user_token_when_it_is_the_only_scope() {
        // Without this fallback the content store for the user-only modes has no
        // token at all, which is what made it reach for the ambient identity.
        user_scoped_credentials()
            .to_store_type(StoreType::Content)
            .await
            .expect("user-scoped content store should be built from the user SAS");
    }

    #[flow_like_types::tokio::test]
    async fn test_meta_store_without_a_draft_sas_stays_a_plain_azure_store() {
        let store = sample_credentials()
            .to_store_type(StoreType::Meta)
            .await
            .expect("meta store should be built from the meta SAS");
        assert!(matches!(store, FlowLikeStore::Azure(_)));
    }

    #[flow_like_types::tokio::test]
    async fn test_meta_store_with_a_draft_sas_routes_and_refuses_draft_writes() {
        let store = server_execute_credentials()
            .to_store_type(StoreType::Meta)
            .await
            .expect("meta store should compose the meta and draft SAS");
        assert!(
            matches!(store, FlowLikeStore::Other(_)),
            "a draft SAS must produce the prefix-routed meta store"
        );

        // The refusal is local to the wrapper, so no network round trip runs.
        let store = store.as_generic();
        let draft = ObjectPath::from("tmp/apps/test-app/compiled/drafts/board-1/aaaa_bbbb.flcb");
        let error = store
            .put(&draft, b"executors never write drafts".to_vec().into())
            .await
            .expect_err("draft writes must be refused before reaching Azure");
        assert!(
            matches!(error, object_store::Error::PermissionDenied { .. }),
            "unexpected error: {error}"
        );
        let error = store
            .delete(&draft)
            .await
            .expect_err("draft deletes must be refused before reaching Azure");
        assert!(
            matches!(error, object_store::Error::PermissionDenied { .. }),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn test_draft_scope_matches_directories_not_string_prefixes() {
        let store = DraftScopedAzureMetaStore {
            primary: Arc::new(
                MicrosoftAzureBuilder::new()
                    .with_account("storage")
                    .with_container_name("meta")
                    .with_access_key("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
                    .build()
                    .expect("primary store"),
            ),
            drafts: Arc::new(
                MicrosoftAzureBuilder::new()
                    .with_account("storage")
                    .with_container_name("meta")
                    .with_access_key("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
                    .build()
                    .expect("draft store"),
            ),
            draft_prefix: ObjectPath::from("tmp/apps/app-1"),
        };

        assert!(store.in_draft_scope(&ObjectPath::from(
            "tmp/apps/app-1/compiled/drafts/board-1/x.flcb"
        )));
        // `apps/app-10` must not match a scope for `apps/app-1`.
        assert!(!store.in_draft_scope(&ObjectPath::from(
            "tmp/apps/app-10/compiled/drafts/board-1/x.flcb"
        )));
        assert!(!store.in_draft_scope(&ObjectPath::from("apps/app-1/boards/board-1.board")));
    }
}

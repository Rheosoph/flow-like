use super::RuntimeCredentialsTrait;
#[cfg(feature = "azure")]
use crate::credentials::CredentialsAccess;
use crate::state::{AppState, State};
#[cfg(feature = "azure")]
use azure_core::credentials::TokenCredential;
#[cfg(feature = "azure")]
use azure_identity::{ManagedIdentityCredential, ManagedIdentityCredentialOptions, UserAssignedId};
#[cfg(feature = "azure")]
use azure_storage_common::models::UserDelegationKey;
#[cfg(feature = "azure")]
use azure_storage_sas::{SasBuilder, SasProtocol};
#[cfg(feature = "azure")]
use flow_like::credentials::{
    SharedCredentials, StoreType, azure_credentials::AzureSharedCredentials,
};
use flow_like::{
    flow_like_storage::lancedb::{connect, connection::ConnectBuilder},
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::object_store;
use flow_like_types::{Result, anyhow, async_trait};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

#[cfg(feature = "azure")]
const STORAGE_SCOPE: &str = "https://storage.azure.com/.default";
#[cfg(feature = "azure")]
const USER_DELEGATION_API_VERSION: &str = "2023-11-03";
#[cfg(feature = "azure")]
const USER_DELEGATION_KEY_LIFETIME_HOURS: i64 = 6;
#[cfg(feature = "azure")]
const SCOPED_SAS_LIFETIME_MINUTES: i64 = 60;

#[cfg(feature = "azure")]
#[derive(Clone)]
enum AzureDirectorySasIssuer {
    UserDelegation(AzureUserDelegationSasIssuer),
    SharedKey(String),
}

#[cfg(feature = "azure")]
#[derive(Clone)]
struct AzureUserDelegationSasIssuer {
    key: UserDelegationKey,
}

#[cfg(feature = "azure")]
static USER_DELEGATION_KEYS: OnceLock<moka::sync::Cache<String, AzureUserDelegationSasIssuer>> =
    OnceLock::new();

/// Serializes delegation-key fetches so a cold cache under load issues one
/// `userdelegationkey` request instead of one per in-flight scoped credential.
/// Cache hits never reach this lock.
#[cfg(feature = "azure")]
static USER_DELEGATION_FETCH: OnceLock<flow_like_types::tokio::sync::Mutex<()>> = OnceLock::new();

#[cfg(feature = "azure")]
#[derive(Clone, Serialize, Deserialize)]
pub struct AzureRuntimeCredentials {
    /// SAS token for meta container
    pub meta_sas_token: Option<String>,
    /// SAS token for content container (app-level: apps/{app_id})
    pub content_sas_token: Option<String>,
    /// SAS token for user-scoped content (users/{sub}/apps/{app_id})
    pub user_content_sas_token: Option<String>,
    /// SAS token for logs container
    pub logs_sas_token: Option<String>,
    /// SAS token for the caller's scratch directory (tmp/user/{sub}/apps/{app_id})
    #[serde(default)]
    pub tmp_sas_token: Option<String>,
    /// Read-only SAS for draft artifacts (tmp/apps/{app_id}) on the meta
    /// container. A directory SAS signs exactly one directory, so the
    /// `apps/{app_id}` meta SAS cannot also cover the draft prefix.
    #[serde(default)]
    pub draft_meta_sas_token: Option<String>,
    pub meta_container: String,
    pub content_container: String,
    pub logs_container: String,
    pub account_name: String,
    /// SECURITY: Never serialize account_key to prevent leaking master credentials.
    /// Mirrors the guard on `AzureSharedCredentials` — this type is cached in
    /// `AppState::credentials_cache` and one stray `to_value` on it would otherwise
    /// hand the storage account master key to whoever receives the response.
    #[serde(default, skip_serializing)]
    pub account_key: Option<String>,
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
    /// App-level content path prefix (e.g., "apps/{app_id}")
    pub content_path_prefix: Option<String>,
    /// User-level content path prefix (e.g., "users/{sub}/apps/{app_id}")
    pub user_content_path_prefix: Option<String>,
    /// Directory signed by `draft_meta_sas_token` (e.g., "tmp/apps/{app_id}")
    #[serde(default)]
    pub draft_meta_path_prefix: Option<String>,
}

#[cfg(feature = "azure")]
impl std::fmt::Debug for AzureRuntimeCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AzureRuntimeCredentials")
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

#[cfg(feature = "azure")]
impl AzureRuntimeCredentials {
    pub fn new(
        meta_container: &str,
        content_container: &str,
        logs_container: &str,
        account_name: &str,
    ) -> Self {
        AzureRuntimeCredentials {
            meta_sas_token: None,
            content_sas_token: None,
            user_content_sas_token: None,
            logs_sas_token: None,
            tmp_sas_token: None,
            draft_meta_sas_token: None,
            meta_container: meta_container.to_string(),
            content_container: content_container.to_string(),
            logs_container: logs_container.to_string(),
            account_name: account_name.to_string(),
            account_key: None,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
            draft_meta_path_prefix: None,
        }
    }

    pub fn from_env() -> Self {
        let logs_container = std::env::var("AZURE_LOG_CONTAINER")
            .or_else(|_| std::env::var("LOG_BUCKET"))
            .unwrap_or_default();
        if logs_container.is_empty() {
            tracing::warn!(
                "AZURE_LOG_CONTAINER environment variable is not set - logs will not be persisted"
            );
        }
        AzureRuntimeCredentials {
            meta_sas_token: None,
            content_sas_token: None,
            user_content_sas_token: None,
            logs_sas_token: None,
            tmp_sas_token: None,
            draft_meta_sas_token: None,
            meta_container: std::env::var("AZURE_META_CONTAINER")
                .or_else(|_| std::env::var("META_BUCKET"))
                .unwrap_or_default(),
            content_container: std::env::var("AZURE_CONTENT_CONTAINER")
                .or_else(|_| std::env::var("CONTENT_BUCKET"))
                .unwrap_or_default(),
            logs_container,
            account_name: std::env::var("AZURE_STORAGE_ACCOUNT_NAME").unwrap_or_default(),
            account_key: std::env::var("AZURE_STORAGE_ACCOUNT_KEY").ok(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
            draft_meta_path_prefix: None,
        }
    }

    pub async fn master_credentials(&self) -> Self {
        AzureRuntimeCredentials {
            meta_sas_token: None,
            content_sas_token: None,
            user_content_sas_token: None,
            logs_sas_token: None,
            tmp_sas_token: None,
            draft_meta_sas_token: None,
            meta_container: self.meta_container.clone(),
            content_container: self.content_container.clone(),
            logs_container: self.logs_container.clone(),
            account_name: self.account_name.clone(),
            account_key: self.account_key.clone(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
            draft_meta_path_prefix: None,
        }
    }

    async fn directory_sas_issuer(&self) -> Result<AzureDirectorySasIssuer> {
        if let Some(account_key) = self
            .account_key
            .clone()
            .or_else(|| std::env::var("AZURE_STORAGE_ACCOUNT_KEY").ok())
            .filter(|value| !value.trim().is_empty())
        {
            // Retained for existing non-Azure-target deployments. The dedicated
            // Azure API rejects account keys at startup, so its only reachable
            // path is Microsoft Entra user delegation below.
            return Ok(AzureDirectorySasIssuer::SharedKey(account_key));
        }

        Ok(AzureDirectorySasIssuer::UserDelegation(
            AzureUserDelegationSasIssuer::from_env(&self.account_name).await?,
        ))
    }

    #[tracing::instrument(
        name = "AzureRuntimeCredentials::scoped_credentials",
        skip(self, sub, _state),
        level = "debug"
    )]
    pub async fn scoped_credentials(
        &self,
        sub: &str,
        app_id: &str,
        _state: &State,
        mode: CredentialsAccess,
    ) -> Result<Self> {
        let issuer = self.directory_sas_issuer().await?;
        self.scoped_credentials_with_issuer(sub, app_id, mode, issuer)
    }

    /// Whether this credential set came from `scoped_credentials` rather than
    /// from server configuration. Mirrors `AzureSharedCredentials::is_scoped`.
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

    /// Mirrors `AzureSharedCredentials::lance_sas_or_ambient`. `make_azure_builder`
    /// drops a blank token and `MicrosoftAzureBuilder` then resolves the workload's
    /// own managed identity, which carries no prefix restriction — so a scoped
    /// credential with no SAS for a database registers no builder at all rather
    /// than one backed by the ambient identity.
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

    /// The single source of truth for what each access mode may reach.
    ///
    /// Tests drive this with a shared-key issuer and production with the Entra
    /// user-delegation issuer, so the scope table below is the one that ships —
    /// a parallel test-only copy would let the two drift apart unnoticed.
    fn scoped_credentials_with_issuer(
        &self,
        sub: &str,
        app_id: &str,
        mode: CredentialsAccess,
        issuer: AzureDirectorySasIssuer,
    ) -> Result<Self> {
        if sub.is_empty() || app_id.is_empty() {
            return Err(flow_like_types::anyhow!("Sub or App ID cannot be empty"));
        }
        crate::credentials::validate_path_component(sub, "sub")?;
        crate::credentials::validate_path_component(app_id, "app_id")?;

        let expiry = chrono::Utc::now() + chrono::Duration::minutes(SCOPED_SAS_LIFETIME_MINUTES);
        let expiry_str = expiry.format("%Y-%m-%dT%H:%M:%SZ").to_string();

        // Scratch space is the caller's own directory, so wherever it is granted at
        // all it is granted read+write. A directory SAS signs exactly one directory,
        // so unlike the AWS/GCP prefix lists this needs a token of its own — without
        // it the HTTP-sink request offload and every `/tmp` file input are
        // unreachable on Azure.
        // Both writers (`/tmp` presign and the HTTP-sink offload) run the segments
        // through `storage_path_segment`, so the signed directory has to as well —
        // a raw `auth0|123` here would sign a directory nobody ever writes to.
        let (tmp_directory, _) = crate::credentials::temporary_prefixes(sub, app_id);
        let tmp_sas = || {
            generate_directory_sas(
                &self.account_name,
                &self.content_container,
                &tmp_directory,
                "rwdl",
                &expiry_str,
                &issuer,
            )
        };

        // Only ServerExecute reads draft artifacts, mirroring the AWS/GCP/R2
        // executor policies; every other mode leaves both slots empty.
        let mut draft_meta_sas_token = None;
        let mut draft_meta_path_prefix = None;

        // Determine which containers and paths need SAS tokens based on access mode
        let (
            meta_sas,
            content_sas,
            user_content_sas,
            logs_sas,
            tmp_sas_token,
            content_path_prefix,
            user_content_path_prefix,
        ) = match mode {
            CredentialsAccess::EditApp => {
                // EditApp: need write access to meta container for app manifests
                // Also need content access for app data (stored at apps/{app_id})
                let meta_sas = generate_directory_sas(
                    &self.account_name,
                    &self.meta_container,
                    &format!("apps/{}", app_id),
                    "rwdl",
                    &expiry_str,
                    &issuer,
                )?;
                let content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("apps/{}", app_id),
                    "rwdl",
                    &expiry_str,
                    &issuer,
                )?;
                (
                    Some(meta_sas),
                    Some(content_sas),
                    None,
                    None,
                    None,
                    Some(format!("apps/{}", app_id)),
                    None,
                )
            }
            CredentialsAccess::ReadApp => {
                // ReadApp: need read access to meta container
                // Also need content access for app data (stored at apps/{app_id})
                let meta_sas = generate_directory_sas(
                    &self.account_name,
                    &self.meta_container,
                    &format!("apps/{}", app_id),
                    "rl",
                    &expiry_str,
                    &issuer,
                )?;
                let content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("apps/{}", app_id),
                    "rl",
                    &expiry_str,
                    &issuer,
                )?;
                (
                    Some(meta_sas),
                    Some(content_sas),
                    None,
                    None,
                    None,
                    Some(format!("apps/{}", app_id)),
                    None,
                )
            }
            CredentialsAccess::ReadAppContent => {
                // Content-only — boards/events/widgets/templates/pages
                // live in the meta container and may carry secrets,
                // so the desktop never gets a meta-side SAS. Source
                // app's content (metadata/, upload/, storage/) is
                // safe to expose for fork pulls.
                let content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("apps/{}", app_id),
                    "rl",
                    &expiry_str,
                    &issuer,
                )?;
                (
                    None,
                    Some(content_sas),
                    None,
                    None,
                    None,
                    Some(format!("apps/{}", app_id)),
                    None,
                )
            }
            CredentialsAccess::EditAppContent => {
                // Content-only write. Used for client-handed upload
                // credentials (presign-data-access, fork-online-begin
                // bundle uploads). Boards / events / templates /
                // widgets / pages must be written through the API
                // so role-permission gates and per-resource
                // validation run; we never hand out a meta-container
                // SAS to a client.
                let content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("apps/{}", app_id),
                    "rwdl",
                    &expiry_str,
                    &issuer,
                )?;
                (
                    None,
                    Some(content_sas),
                    None,
                    None,
                    None,
                    Some(format!("apps/{}", app_id)),
                    None,
                )
            }
            CredentialsAccess::ReadAppDb => {
                // Project LanceDB only — the SAS is restricted to the
                // db directory so connected apps cannot read the rest
                // of the app's content (e.g. upload/). The path
                // prefix stays at apps/{app_id} because it is
                // path-resolution metadata: SharedCredentials::to_db
                // appends storage/db itself.
                let content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("apps/{}/storage/db", app_id),
                    "rl",
                    &expiry_str,
                    &issuer,
                )?;
                (
                    None,
                    Some(content_sas),
                    None,
                    None,
                    None,
                    Some(format!("apps/{}", app_id)),
                    None,
                )
            }
            CredentialsAccess::EditAppDb => {
                let content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("apps/{}/storage/db", app_id),
                    "rwdl",
                    &expiry_str,
                    &issuer,
                )?;
                (
                    None,
                    Some(content_sas),
                    None,
                    None,
                    None,
                    Some(format!("apps/{}", app_id)),
                    None,
                )
            }
            CredentialsAccess::EditUser => {
                let user_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("users/{}/apps/{}", sub, app_id),
                    "rwdl",
                    &expiry_str,
                    &issuer,
                )?;
                (
                    None,
                    None,
                    Some(user_content_sas),
                    None,
                    None,
                    None,
                    Some(format!("users/{}/apps/{}", sub, app_id)),
                )
            }
            CredentialsAccess::ReadUser => {
                let user_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("users/{}/apps/{}", sub, app_id),
                    "rl",
                    &expiry_str,
                    &issuer,
                )?;
                (
                    None,
                    None,
                    Some(user_content_sas),
                    None,
                    None,
                    None,
                    Some(format!("users/{}/apps/{}", sub, app_id)),
                )
            }
            CredentialsAccess::InvokeNone => {
                // No app meta/content access for execute-only callers.
                let user_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("users/{}/apps/{}", sub, app_id),
                    "rwdl",
                    &expiry_str,
                    &issuer,
                )?;
                // Run logs (`runs/{app_id}`) are written only by the server
                // executor (ServerExecute) and read only through the API
                // (ReadLogs); the desktop keeps its own local log store. The
                // client-facing invoke modes therefore get no logs SAS at all.
                (
                    None,
                    None,
                    Some(user_content_sas),
                    None,
                    Some(tmp_sas()?),
                    None,
                    Some(format!("users/{}/apps/{}", sub, app_id)),
                )
            }
            CredentialsAccess::InvokeRead => {
                // App metadata remains server-only; this mode grants content reads.
                let app_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("apps/{}", app_id),
                    "rl",
                    &expiry_str,
                    &issuer,
                )?;
                let user_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("users/{}/apps/{}", sub, app_id),
                    "rl",
                    &expiry_str,
                    &issuer,
                )?;
                (
                    None,
                    Some(app_content_sas),
                    Some(user_content_sas),
                    None,
                    Some(tmp_sas()?),
                    Some(format!("apps/{}", app_id)),
                    Some(format!("users/{}/apps/{}", sub, app_id)),
                )
            }
            CredentialsAccess::InvokeWrite => {
                // App metadata remains server-only; this mode grants content writes.
                let app_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("apps/{}", app_id),
                    "rwdl",
                    &expiry_str,
                    &issuer,
                )?;
                let user_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("users/{}/apps/{}", sub, app_id),
                    "rwdl",
                    &expiry_str,
                    &issuer,
                )?;
                (
                    None,
                    Some(app_content_sas),
                    Some(user_content_sas),
                    None,
                    Some(tmp_sas()?),
                    Some(format!("apps/{}", app_id)),
                    Some(format!("users/{}/apps/{}", sub, app_id)),
                )
            }
            CredentialsAccess::ServerExecute => {
                let (meta_prefix, meta_permissions) = server_execute_meta_scope(app_id);
                let meta_sas = generate_directory_sas(
                    &self.account_name,
                    &self.meta_container,
                    &meta_prefix,
                    meta_permissions,
                    &expiry_str,
                    &issuer,
                )?;
                // Draft artifacts (compiled boards, exact `.board` snapshots,
                // prerun manifests) live under `tmp/apps/{app_id}` on the meta
                // container. A directory SAS signs exactly one directory, so
                // unlike the AWS/GCP prefix lists this needs a second,
                // read-only token beside the `apps/{app_id}` meta SAS.
                let (draft_prefix, draft_permissions) = server_execute_draft_meta_scope(app_id);
                draft_meta_sas_token = Some(generate_directory_sas(
                    &self.account_name,
                    &self.meta_container,
                    &draft_prefix,
                    draft_permissions,
                    &expiry_str,
                    &issuer,
                )?);
                draft_meta_path_prefix = Some(draft_prefix);
                let app_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("apps/{}", app_id),
                    "rwdl",
                    &expiry_str,
                    &issuer,
                )?;
                let user_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("users/{}/apps/{}", sub, app_id),
                    "rwdl",
                    &expiry_str,
                    &issuer,
                )?;
                // Append-only run logs: Lance creates and appends, never
                // deletes; without `d` a run cannot erase other runs' logs.
                let logs_sas = generate_directory_sas(
                    &self.account_name,
                    &self.logs_container,
                    &format!("runs/{}", app_id),
                    "rwl",
                    &expiry_str,
                    &issuer,
                )?;
                (
                    Some(meta_sas),
                    Some(app_content_sas),
                    Some(user_content_sas),
                    Some(logs_sas),
                    Some(tmp_sas()?),
                    Some(format!("apps/{}", app_id)),
                    Some(format!("users/{}/apps/{}", sub, app_id)),
                )
            }
            CredentialsAccess::ShadowExecute => {
                // ServerExecute with app/user content writes dropped: a shadow
                // run reads live content ("rl") but may never mutate it.
                // Scratch stays read-write and run logs stay append-only so
                // the shadow run itself is recorded.
                let (meta_prefix, meta_permissions) = server_execute_meta_scope(app_id);
                let meta_sas = generate_directory_sas(
                    &self.account_name,
                    &self.meta_container,
                    &meta_prefix,
                    meta_permissions,
                    &expiry_str,
                    &issuer,
                )?;
                let (draft_prefix, draft_permissions) = server_execute_draft_meta_scope(app_id);
                draft_meta_sas_token = Some(generate_directory_sas(
                    &self.account_name,
                    &self.meta_container,
                    &draft_prefix,
                    draft_permissions,
                    &expiry_str,
                    &issuer,
                )?);
                draft_meta_path_prefix = Some(draft_prefix);
                let app_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("apps/{}", app_id),
                    "rl",
                    &expiry_str,
                    &issuer,
                )?;
                let user_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("users/{}/apps/{}", sub, app_id),
                    "rl",
                    &expiry_str,
                    &issuer,
                )?;
                let logs_sas = generate_directory_sas(
                    &self.account_name,
                    &self.logs_container,
                    &format!("runs/{}", app_id),
                    "rwl",
                    &expiry_str,
                    &issuer,
                )?;
                (
                    Some(meta_sas),
                    Some(app_content_sas),
                    Some(user_content_sas),
                    Some(logs_sas),
                    Some(tmp_sas()?),
                    Some(format!("apps/{}", app_id)),
                    Some(format!("users/{}/apps/{}", sub, app_id)),
                )
            }
            CredentialsAccess::ReadLogs => {
                // ReadLogs: read access to logs container only
                let logs_sas = generate_directory_sas(
                    &self.account_name,
                    &self.logs_container,
                    &format!("runs/{}", app_id),
                    "rl",
                    &expiry_str,
                    &issuer,
                )?;
                (None, None, None, Some(logs_sas), None, None, None)
            }
        };

        Ok(Self {
            meta_sas_token: meta_sas,
            content_sas_token: content_sas,
            user_content_sas_token: user_content_sas,
            logs_sas_token: logs_sas,
            tmp_sas_token,
            draft_meta_sas_token,
            meta_container: self.meta_container.clone(),
            content_container: self.content_container.clone(),
            logs_container: self.logs_container.clone(),
            account_name: self.account_name.clone(),
            account_key: None,
            expiration: Some(expiry),
            content_path_prefix,
            user_content_path_prefix,
            draft_meta_path_prefix,
        })
    }

    /// Drives the production scope table through the legacy shared-key signer so
    /// tests need neither `State` nor a Microsoft Entra round trip.
    ///
    /// This deliberately does *not* reimplement the scope table: an earlier
    /// test-only copy drifted from production (different permission strings, and a
    /// content-container SAS returned in the `meta_sas_token` slot), which left the
    /// assertions below validating code that never shipped.
    #[cfg(test)]
    pub async fn scoped_credentials_for_test(
        &self,
        sub: &str,
        app_id: &str,
        mode: CredentialsAccess,
    ) -> Result<Self> {
        let account_key = self
            .account_key
            .clone()
            .or_else(|| std::env::var("AZURE_STORAGE_ACCOUNT_KEY").ok())
            .ok_or_else(|| {
                flow_like_types::anyhow!("AZURE_STORAGE_ACCOUNT_KEY environment variable not set")
            })?;

        self.scoped_credentials_with_issuer(
            sub,
            app_id,
            mode,
            AzureDirectorySasIssuer::SharedKey(account_key),
        )
    }
}

#[cfg(all(test, feature = "azure"))]
mod sas_tests {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use urlencoding;

    /// Test that our SAS signature generation matches Azure CLI output
    /// Run: az storage account generate-sas --account-name <your-account> --services b --resource-types sco --permissions rwdlac --expiry <date> --https-only
    /// Then compare the generated signature with the test output
    #[test]
    #[ignore]
    fn test_sas_signature_matches_azure_cli() {
        dotenv::dotenv().ok();

        let account_name = "flowliketest";
        let account_key = std::env::var("AZURE_STORAGE_ACCOUNT_KEY")
            .expect("AZURE_STORAGE_ACCOUNT_KEY must be set");
        let permissions = "rwdlac";
        let services = "b";
        let resource_types = "sco";
        let expiry = "2025-12-17T00:00:00Z";
        let protocol = "https";
        let version = "2022-11-02";

        // String to sign for Account SAS (version 2020-12-06+)
        // 10 fields with 10 newlines (trailing newline after empty encryption scope)
        let string_to_sign = format!(
            "{}\n{}\n{}\n{}\n\n{}\n\n{}\n{}\n\n",
            account_name,
            permissions,
            services,
            resource_types,
            // start is empty
            expiry,
            // IP is empty
            protocol,
            version,
            // encryption scope is empty
        );

        eprintln!("String to sign (escaped): {:?}", string_to_sign);
        eprintln!(
            "Newline count: {}",
            string_to_sign.chars().filter(|&c| c == '\n').count()
        );

        let key_bytes = STANDARD.decode(&account_key).expect("Failed to decode key");

        type HmacSha256 = Hmac<Sha256>;
        let mut mac = HmacSha256::new_from_slice(&key_bytes).expect("HMAC error");
        mac.update(string_to_sign.as_bytes());
        let signature = STANDARD.encode(mac.finalize().into_bytes());

        eprintln!("Generated signature: {}", signature);

        // The signature won't match because the account key might be different
        // But we can verify the format is correct by testing the SAS works
        let sas_token = format!(
            "se={}&sp={}&spr={}&sv={}&ss={}&srt={}&sig={}",
            urlencoding::encode(expiry),
            permissions,
            protocol,
            version,
            services,
            resource_types,
            urlencoding::encode(&signature),
        );

        eprintln!("Generated SAS: {}", sas_token);
    }
}

#[cfg(feature = "azure")]
impl AzureUserDelegationSasIssuer {
    async fn from_env(account_name: &str) -> Result<Self> {
        let client_id = std::env::var("AZURE_CLIENT_ID")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let cache_key = format!(
            "{}|{}",
            account_name,
            client_id.as_deref().unwrap_or("system")
        );
        let cache = USER_DELEGATION_KEYS.get_or_init(|| {
            moka::sync::Cache::builder()
                .max_capacity(32)
                .time_to_live(std::time::Duration::from_secs(5 * 60 * 60))
                .build()
        });
        if let Some(issuer) = cache.get(&cache_key) {
            return Ok(issuer);
        }

        let _fetch_guard = USER_DELEGATION_FETCH
            .get_or_init(|| flow_like_types::tokio::sync::Mutex::new(()))
            .lock()
            .await;
        // A concurrent caller may have populated the cache while we waited.
        if let Some(issuer) = cache.get(&cache_key) {
            return Ok(issuer);
        }

        let user_assigned_id = client_id.map(UserAssignedId::ClientId);
        let credential = ManagedIdentityCredential::new(Some(ManagedIdentityCredentialOptions {
            user_assigned_id,
            ..Default::default()
        }))
        .map_err(|error| anyhow!("failed to initialize Azure managed identity: {error}"))?;
        let token = credential
            .get_token(&[STORAGE_SCOPE], None)
            .await
            .map_err(|error| anyhow!("failed to acquire Azure Storage identity token: {error}"))?;

        let start = chrono::Utc::now() - chrono::Duration::minutes(5);
        let expiry =
            chrono::Utc::now() + chrono::Duration::hours(USER_DELEGATION_KEY_LIFETIME_HOURS);
        let start_rfc3339 = start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let expiry_rfc3339 = expiry.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let request_body = format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?><KeyInfo><Start>{start_rfc3339}</Start><Expiry>{expiry_rfc3339}</Expiry></KeyInfo>"
        );
        let endpoint = format!(
            "https://{account_name}.blob.core.windows.net/?restype=service&comp=userdelegationkey"
        );
        let response = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|error| anyhow!("failed to construct Azure Storage client: {error}"))?
            .post(endpoint)
            .bearer_auth(token.token.secret())
            .header("x-ms-version", USER_DELEGATION_API_VERSION)
            .header(
                "x-ms-date",
                chrono::Utc::now()
                    .format("%a, %d %b %Y %H:%M:%S GMT")
                    .to_string(),
            )
            .header(reqwest::header::CONTENT_TYPE, "application/xml")
            .body(request_body)
            .send()
            .await
            .map_err(|error| anyhow!("Azure user delegation key request failed: {error}"))?;

        let status = response.status();
        let request_id = response
            .headers()
            .get("x-ms-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unavailable")
            .to_string();
        if !status.is_success() {
            return Err(anyhow!(
                "Azure user delegation key request returned HTTP {status} (request {request_id})"
            ));
        }
        let response_body = response
            .text()
            .await
            .map_err(|error| anyhow!("failed to read Azure user delegation key: {error}"))?;
        let key: UserDelegationKey = quick_xml::de::from_str(&response_body)
            .map_err(|error| anyhow!("invalid Azure user delegation key response: {error}"))?;
        SasBuilder::new(
            account_name,
            &key,
            time::OffsetDateTime::now_utc() + time::Duration::minutes(1),
        )
        .map_err(|error| anyhow!("Azure returned an incomplete user delegation key: {error}"))?;

        let issuer = Self { key };
        cache.insert(cache_key, issuer.clone());
        Ok(issuer)
    }
}

/// Read-only meta scope for the executor: version artifacts and boards under
/// `apps/{app_id}`.
///
/// Draft artifacts live under `tmp/apps/{app_id}` and are covered by the
/// separate SAS from [`server_execute_draft_meta_scope`] — an Azure directory
/// SAS signs exactly one directory, and widening this one to the container
/// root would expose every app's metadata.
#[cfg(feature = "azure")]
fn server_execute_meta_scope(app_id: &str) -> (String, &'static str) {
    (format!("apps/{app_id}"), "rl")
}

/// Read-only draft-artifact scope for the executor: compiled drafts, exact
/// `.board` snapshots and prerun manifests under `tmp/apps/{app_id}`
/// (see `flow_like::flow::compiled::draft_artifact_dir`), lifecycle-reclaimed
/// via the `tmp/` prefix. Read+list only — only the API writes drafts, which
/// anchors `entry_authority_revision`. The executor routes meta reads under
/// this directory through the token (`AzureSharedCredentials::to_store_type`).
#[cfg(feature = "azure")]
fn server_execute_draft_meta_scope(app_id: &str) -> (String, &'static str) {
    (format!("tmp/apps/{app_id}"), "rl")
}

/// Generates an HTTPS-only directory SAS for an HNS/ADLS Gen2 path.
/// The secure Azure API reaches this through a Microsoft Entra user delegation
/// key; shared-key signing remains only for backward-compatible callers that
/// explicitly supplied a legacy account key.
#[cfg(feature = "azure")]
fn generate_directory_sas(
    account_name: &str,
    container: &str,
    directory: &str,
    permissions: &str,
    expiry: &str,
    issuer: &AzureDirectorySasIssuer,
) -> Result<String> {
    match issuer {
        AzureDirectorySasIssuer::UserDelegation(issuer) => {
            let expiry =
                time::OffsetDateTime::parse(expiry, &time::format_description::well_known::Rfc3339)
                    .map_err(|error| anyhow!("invalid scoped SAS expiry: {error}"))?;
            let start = time::OffsetDateTime::now_utc() - time::Duration::minutes(5);
            let mut builder = SasBuilder::new(account_name, &issuer.key, expiry)
                .map_err(|error| anyhow!("failed to initialize user delegation SAS: {error}"))?
                .directory(container, directory.trim_matches('/'))
                .start(start)
                .protocol(SasProtocol::Https);

            for permission in permissions.chars() {
                builder = match permission {
                    'r' => builder.read(),
                    'a' => builder.add(),
                    'c' => builder.create(),
                    'w' => builder.write(),
                    'd' => builder.delete(),
                    'l' => builder.list(),
                    other => {
                        return Err(anyhow!(
                            "unsupported Azure directory SAS permission: {other}"
                        ));
                    }
                };
            }
            Ok(builder.build())
        }
        AzureDirectorySasIssuer::SharedKey(account_key) => {
            let start = (chrono::Utc::now() - chrono::Duration::minutes(5))
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string();
            generate_shared_key_directory_sas(
                account_name,
                container,
                directory,
                permissions,
                &start,
                expiry,
                account_key,
            )
        }
    }
}

/// Legacy account-key Directory SAS signer retained for existing deployments.
///
/// String-to-sign format for Service SAS (version 2020-12-06):
/// - signedPermissions
/// - signedStart
/// - signedExpiry
/// - canonicalizedResource (/blob/{account}/{container}/{directory})
/// - signedIdentifier (empty)
/// - signedIP (empty)
/// - signedProtocol
/// - signedVersion
/// - signedResource (d for directory)
/// - signedSnapshotTime (empty)
/// - signedEncryptionScope (empty)
/// - rscc, rscd, rsce, rscl, rsct (all empty - response headers)
#[cfg(feature = "azure")]
fn generate_shared_key_directory_sas(
    account_name: &str,
    container: &str,
    directory: &str,
    permissions: &str,
    start: &str,
    expiry: &str,
    account_key: &str,
) -> Result<String> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    // Azure SAS version - 2020-12-06 has well-documented directory SAS support with encryption scope
    let signed_version = "2020-12-06";
    // "d" = directory (requires HNS enabled)
    let signed_resource = "d";
    let signed_protocol = "https";

    // Directory depth - count the path segments (0 for root, 1 for first level, etc.)
    // Strip leading/trailing slashes for consistent counting
    let clean_dir = directory.trim_matches('/');
    let directory_depth = if clean_dir.is_empty() {
        0
    } else {
        clean_dir.split('/').filter(|s| !s.is_empty()).count()
    };

    // Canonicalized resource: /blob/{account}/{container}/{directory}
    // Note: Use /blob/ even for ADLS Gen2 when using Blob API
    let canonicalized_resource = if clean_dir.is_empty() {
        format!("/blob/{}/{}", account_name, container)
    } else {
        format!("/blob/{}/{}/{}", account_name, container, clean_dir)
    };

    // String to sign format for Service SAS (version 2020-12-06)
    // 16 fields total, each separated by newline
    // Note: sdd (signedDirectoryDepth) is NOT in string-to-sign, only in the final token
    let string_to_sign = format!(
        "{}\n{}\n{}\n{}\n\n\n{}\n{}\n{}\n\n\n\n\n\n\n",
        permissions,            // sp: signedPermissions
        start,                  // st: signedStart
        expiry,                 // se: signedExpiry
        canonicalized_resource, // canonicalizedResource
        // signedIdentifier (empty)
        // signedIP (empty)
        signed_protocol, // spr: signedProtocol
        signed_version,  // sv: signedVersion
        signed_resource, // sr: signedResource (d)
                         // signedSnapshotTime (empty)
                         // signedEncryptionScope (empty)
                         // rscc, rscd, rsce, rscl, rsct (all empty)
    );

    #[cfg(test)]
    {
        eprintln!("=== Directory SAS Debug ===");
        eprintln!("Account: {}", account_name);
        eprintln!("Container: {}", container);
        eprintln!("Directory: {}", clean_dir);
        eprintln!("Directory depth: {}", directory_depth);
        eprintln!("Canonicalized resource: {}", canonicalized_resource);
        eprintln!("Permissions: {}", permissions);
        eprintln!("String to sign (escaped): {:?}", string_to_sign);
        eprintln!(
            "Newline count: {}",
            string_to_sign.chars().filter(|&c| c == '\n').count()
        );
        eprintln!("===========================");
    }

    let key_bytes = STANDARD
        .decode(account_key)
        .map_err(|e| anyhow!("Failed to decode account key: {}", e))?;

    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(&key_bytes)
        .map_err(|e| anyhow!("Failed to create HMAC: {}", e))?;
    mac.update(string_to_sign.as_bytes());
    let signature = STANDARD.encode(mac.finalize().into_bytes());

    // Build SAS token - sdd IS included in the token (but not in string-to-sign)
    let sas_token = format!(
        "sp={}&st={}&se={}&spr={}&sv={}&sr={}&sdd={}&sig={}",
        permissions,
        urlencoding::encode(start),
        urlencoding::encode(expiry),
        signed_protocol,
        signed_version,
        signed_resource,
        directory_depth,
        urlencoding::encode(&signature),
    );

    #[cfg(test)]
    eprintln!("Generated Directory SAS: {}", sas_token);

    Ok(sas_token)
}

#[cfg(feature = "azure")]
#[async_trait]
impl RuntimeCredentialsTrait for AzureRuntimeCredentials {
    fn into_shared_credentials(&self) -> SharedCredentials {
        SharedCredentials::Azure(AzureSharedCredentials {
            meta_sas_token: self.meta_sas_token.clone(),
            content_sas_token: self.content_sas_token.clone(),
            user_content_sas_token: self.user_content_sas_token.clone(),
            logs_sas_token: self.logs_sas_token.clone(),
            tmp_sas_token: self.tmp_sas_token.clone(),
            draft_meta_sas_token: self.draft_meta_sas_token.clone(),
            meta_container: self.meta_container.clone(),
            content_container: self.content_container.clone(),
            logs_container: self.logs_container.clone(),
            account_name: self.account_name.clone(),
            account_key: self.account_key.clone(),
            expiration: self.expiration,
            content_path_prefix: self.content_path_prefix.clone(),
            user_content_path_prefix: self.user_content_path_prefix.clone(),
            draft_meta_path_prefix: self.draft_meta_path_prefix.clone(),
        })
    }

    async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder> {
        self.into_shared_credentials().to_db(app_id).await
    }

    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        self.into_shared_credentials()
            .to_db_scoped(sub, app_id)
            .await
    }

    #[tracing::instrument(
        name = "AzureRuntimeCredentials::to_state",
        skip(self, state),
        level = "debug"
    )]
    async fn to_state(&self, state: AppState) -> Result<FlowLikeState> {
        let shared = self.into_shared_credentials();
        let (meta_store, content_store, tmp_store) = {
            use flow_like_types::tokio;

            tokio::join!(
                async { shared.to_store_type(StoreType::Meta).await },
                async { shared.to_store_type(StoreType::Content).await },
                async { shared.to_store_type(StoreType::Tmp).await },
            )
        };
        let http_client = HTTPClient::new_without_refetch();

        let meta_store = meta_store?;
        let content_store = content_store?;

        let mut config = {
            let mut cfg = FlowLikeConfig::with_default_store(content_store);
            cfg.register_app_meta_store(meta_store.clone());
            cfg
        };

        // `with_default_store` aims the scratch store at the content store, whose
        // SAS only signs `apps/{app_id}` — tmp/* needs the token minted for it.
        match tmp_store {
            Ok(store) => config.register_temporary_store(store),
            Err(error) => tracing::warn!(
                %error,
                "no scratch storage credentials - tmp/* paths will be unavailable"
            ),
        }

        let account = self.account_name.clone();
        let container = self.content_container.clone();

        match self.lance_sas_or_ambient(self.content_sas_token.as_ref(), "the app database") {
            Ok(sas) => config.register_build_project_database(Arc::new(make_azure_builder(
                account.clone(),
                container.clone(),
                sas,
            ))),
            Err(error) => tracing::warn!(
                %error,
                "no app database builder - the app database is unreachable under this scope"
            ),
        }
        // The user database lives at users/{sub}/apps/{app_id}/db, which the
        // app-scoped content SAS does not sign.
        match self.lance_sas_or_ambient(
            self.user_content_sas_token
                .as_ref()
                .or(self.content_sas_token.as_ref()),
            "the user database",
        ) {
            Ok(sas) => config.register_build_user_database(Arc::new(make_azure_builder(
                account, container, sas,
            ))),
            Err(error) => tracing::warn!(
                %error,
                "no user database builder - the user database is unreachable under this scope"
            ),
        }
        // Run logs live in the logs container under runs/{app_id}; signing them
        // with the content SAS pointed the log database at the wrong container.
        match shared.to_logs_db_builder() {
            Ok(builder) => config.register_build_logs_database(builder),
            Err(error) => tracing::warn!(
                %error,
                "no logs database builder - run logs will not be persisted"
            ),
        }

        let mut flow_like_state = FlowLikeState::new(config, http_client);

        flow_like_state.model_provider_config = state.provider.clone();
        flow_like_state.node_registry.write().await.node_registry = state.registry.clone();

        Ok(flow_like_state)
    }
}

#[cfg(feature = "azure")]
fn make_azure_builder(
    account_name: String,
    container: String,
    sas_token: String,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder {
    move |path| {
        let url = format!("az://{}/{}", container, path);
        let builder = connect(&url).storage_option(
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

// ============================================================================
// Integration Tests
// ============================================================================

#[cfg(all(test, feature = "azure"))]
mod integration_tests {
    use super::*;
    use crate::credentials::CredentialsAccess;
    use crate::credentials::RuntimeCredentialsTrait;
    use flow_like_storage::Path;
    use flow_like_storage::object_store::ObjectStore;
    use flow_like_types::json::{from_str, to_string};
    use flow_like_types::tokio;
    use std::sync::Once;

    const TEST_SUB: &str = "test-user-123";
    const TEST_APP_ID: &str = "test-app-456";

    static INIT: Once = Once::new();

    fn init_env() {
        INIT.call_once(|| {
            if dotenv::from_filename("packages/api/.env").is_err() {
                let _ = dotenv::dotenv();
            }
        });
    }

    fn test_azure_runtime_credentials() -> AzureRuntimeCredentials {
        AzureRuntimeCredentials {
            meta_sas_token: None,
            content_sas_token: None,
            user_content_sas_token: None,
            logs_sas_token: None,
            tmp_sas_token: None,
            draft_meta_sas_token: None,
            meta_container: "meta-secret".to_string(),
            content_container: "content-data".to_string(),
            logs_container: "logs".to_string(),
            account_name: "account".to_string(),
            account_key: Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string()),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
            draft_meta_path_prefix: None,
        }
    }

    fn sas_permissions(token: &str) -> Option<&str> {
        token.split('&').find_map(|part| part.strip_prefix("sp="))
    }

    /// Signed directory depth. The container and directory are signed but not
    /// emitted in the token, so `sdd` is the only handle a unit test has on which
    /// directory a SAS actually covers: `apps/{app}` is 2, `tmp/apps/{app}` is 3,
    /// `users/{sub}/apps/{app}` is 4, `tmp/user/{sub}/apps/{app}` is 5.
    fn sas_directory_depth(token: &str) -> Option<u32> {
        token
            .split('&')
            .find_map(|part| part.strip_prefix("sdd="))
            .and_then(|value| value.parse().ok())
    }

    #[tokio::test]
    async fn test_azure_invoke_scopes_do_not_expose_meta_access() {
        let creds = test_azure_runtime_credentials();

        for mode in [
            CredentialsAccess::InvokeNone,
            CredentialsAccess::InvokeRead,
            CredentialsAccess::InvokeWrite,
        ] {
            let scoped = creds
                .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, mode.clone())
                .await
                .expect("scoped credentials should be generated");

            assert!(
                scoped.meta_sas_token.is_none(),
                "{mode} must not expose app metadata"
            );
            assert!(
                scoped.draft_meta_sas_token.is_none(),
                "{mode} must not expose draft artifacts - only ServerExecute reads them"
            );
        }
    }

    #[tokio::test]
    async fn test_azure_invoke_none_does_not_grant_app_content_access() {
        let creds = test_azure_runtime_credentials();
        let scoped = creds
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::InvokeNone)
            .await
            .expect("scoped credentials should be generated");

        assert!(scoped.content_sas_token.is_none());
        assert_eq!(scoped.content_path_prefix, None);
        assert!(scoped.user_content_sas_token.is_some());
        assert_eq!(
            scoped.user_content_path_prefix,
            Some(format!("users/{}/apps/{}", TEST_SUB, TEST_APP_ID))
        );
    }

    #[tokio::test]
    async fn test_azure_invoke_read_and_write_app_content_permissions() {
        let creds = test_azure_runtime_credentials();

        let read = creds
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::InvokeRead)
            .await
            .expect("read credentials should be generated");
        let read_content = read
            .content_sas_token
            .as_deref()
            .expect("InvokeRead should include app content read credentials");
        assert_eq!(sas_permissions(read_content), Some("rl"));

        let write = creds
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::InvokeWrite)
            .await
            .expect("write credentials should be generated");
        let write_content = write
            .content_sas_token
            .as_deref()
            .expect("InvokeWrite should include app content write credentials");
        let permissions = sas_permissions(write_content).expect("SAS should include permissions");
        assert!(permissions.contains('w'));
        assert!(permissions.contains('d'));
    }

    #[tokio::test]
    async fn test_azure_server_execute_has_meta_read_and_content_write() {
        let creds = test_azure_runtime_credentials();
        let scoped = creds
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::ServerExecute)
            .await
            .expect("server execute credentials should be generated");

        let meta = scoped
            .meta_sas_token
            .as_deref()
            .expect("ServerExecute should include meta read credentials");
        assert_eq!(sas_permissions(meta), Some("rl"));
        assert_eq!(sas_directory_depth(meta), Some(2));
        let (meta_prefix, meta_permissions) = server_execute_meta_scope(TEST_APP_ID);
        assert_eq!(meta_prefix, format!("apps/{TEST_APP_ID}"));
        assert_eq!(meta_permissions, "rl");

        // Draft artifacts live under tmp/apps/{app_id}, which a directory SAS
        // for apps/{app_id} cannot cover — ServerExecute therefore carries a
        // second meta SAS scoped to exactly that directory, read+list only:
        // only the API writes drafts (the trust anchor for
        // `entry_authority_revision`).
        let draft_meta = scoped
            .draft_meta_sas_token
            .as_deref()
            .expect("ServerExecute should include draft-artifact read credentials");
        let (draft_prefix, draft_permissions) = server_execute_draft_meta_scope(TEST_APP_ID);
        assert_eq!(draft_prefix, format!("tmp/apps/{TEST_APP_ID}"));
        assert_eq!(draft_permissions, "rl");
        assert_eq!(sas_permissions(draft_meta), Some("rl"));
        let draft_meta_permissions =
            sas_permissions(draft_meta).expect("draft SAS should carry permissions");
        assert!(
            !draft_meta_permissions.contains('w')
                && !draft_meta_permissions.contains('d')
                && !draft_meta_permissions.contains('a')
                && !draft_meta_permissions.contains('c'),
            "the draft SAS must never grant a write, got {draft_meta_permissions}"
        );
        // tmp/apps/{app_id}
        assert_eq!(sas_directory_depth(draft_meta), Some(3));
        assert_eq!(
            scoped.draft_meta_path_prefix.as_deref(),
            Some(draft_prefix.as_str()),
            "the executor routes meta reads by this prefix"
        );

        let draft_artifact = flow_like::flow::compiled::draft_artifact_path(
            TEST_APP_ID,
            "board-1",
            "etag",
            &[7; 32],
        )
        .to_string();
        let draft_source =
            flow_like::flow::compiled::draft_source_path(TEST_APP_ID, "board-1", "etag")
                .to_string();
        let draft_manifest =
            flow_like::flow::compiled::draft_manifest_path(TEST_APP_ID, "board-1", "etag")
                .to_string();
        let other_app_artifact = flow_like::flow::compiled::draft_artifact_path(
            &format!("{TEST_APP_ID}-other"),
            "board-1",
            "etag",
            &[7; 32],
        )
        .to_string();
        for path in [&draft_artifact, &draft_source, &draft_manifest] {
            assert!(
                path.starts_with(&format!("{draft_prefix}/")),
                "{path} must be inside the ServerExecute draft meta scope {draft_prefix}"
            );
        }
        assert!(!other_app_artifact.starts_with(&format!("{draft_prefix}/")));

        let content = scoped
            .content_sas_token
            .as_deref()
            .expect("ServerExecute should include content credentials");
        let content_permissions =
            sas_permissions(content).expect("content SAS should include permissions");
        assert!(content_permissions.contains('w'));
        assert!(content_permissions.contains('d'));

        let logs = scoped
            .logs_sas_token
            .as_deref()
            .expect("ServerExecute writes run logs");
        let logs_permissions = sas_permissions(logs).expect("logs SAS should carry permissions");
        assert!(logs_permissions.contains('w'));
        assert!(
            !logs_permissions.contains('d'),
            "ServerExecute run logs are append-only, got {logs_permissions}"
        );
        assert_eq!(
            scoped.content_path_prefix,
            Some(format!("apps/{}", TEST_APP_ID))
        );
    }

    /// A shadow run reads live app/user content but may never mutate it;
    /// scratch stays read-write and run logs stay append-only.
    #[tokio::test]
    async fn test_azure_shadow_execute_drops_content_writes_but_keeps_reads() {
        let creds = test_azure_runtime_credentials();
        let scoped = creds
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::ShadowExecute)
            .await
            .expect("shadow execute credentials should be generated");

        let content = scoped
            .content_sas_token
            .as_deref()
            .expect("ShadowExecute should include app content read credentials");
        assert_eq!(sas_permissions(content), Some("rl"));
        let user_content = scoped
            .user_content_sas_token
            .as_deref()
            .expect("ShadowExecute should include user content read credentials");
        assert_eq!(sas_permissions(user_content), Some("rl"));

        let meta = scoped
            .meta_sas_token
            .as_deref()
            .expect("ShadowExecute still reads the board/event definition");
        assert_eq!(sas_permissions(meta), Some("rl"));

        let logs = scoped
            .logs_sas_token
            .as_deref()
            .expect("ShadowExecute still records run logs");
        let logs_permissions = sas_permissions(logs).expect("logs SAS should carry permissions");
        assert!(logs_permissions.contains('w'));
        assert!(
            !logs_permissions.contains('d'),
            "ShadowExecute run logs are append-only, got {logs_permissions}"
        );

        let tmp = scoped
            .tmp_sas_token
            .as_deref()
            .expect("ShadowExecute keeps the caller's scratch directory");
        assert!(
            sas_permissions(tmp)
                .expect("scratch SAS should carry permissions")
                .contains('w')
        );
    }

    #[tokio::test]
    async fn test_azure_execution_scopes_grant_the_callers_scratch_directory() {
        let creds = test_azure_runtime_credentials();

        for mode in [
            CredentialsAccess::InvokeNone,
            CredentialsAccess::InvokeRead,
            CredentialsAccess::InvokeWrite,
            CredentialsAccess::ServerExecute,
        ] {
            let scoped = creds
                .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, mode.clone())
                .await
                .expect("scoped credentials should be generated");

            let tmp = scoped
                .tmp_sas_token
                .as_deref()
                .unwrap_or_else(|| panic!("{mode} must grant the caller's scratch directory"));

            // tmp/user/{sub}/apps/{app_id}
            assert_eq!(sas_directory_depth(tmp), Some(5), "{mode} scratch scope");
            let permissions = sas_permissions(tmp).expect("scratch SAS should carry permissions");
            assert!(
                permissions.contains('w'),
                "{mode} must be able to write scratch"
            );
        }
    }

    #[tokio::test]
    async fn test_azure_non_execution_scopes_grant_no_scratch_directory() {
        let creds = test_azure_runtime_credentials();

        for mode in [
            CredentialsAccess::EditApp,
            CredentialsAccess::ReadApp,
            CredentialsAccess::ReadAppContent,
            CredentialsAccess::EditAppContent,
            CredentialsAccess::ReadUser,
            CredentialsAccess::ReadLogs,
        ] {
            let scoped = creds
                .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, mode.clone())
                .await
                .expect("scoped credentials should be generated");

            assert!(
                scoped.tmp_sas_token.is_none(),
                "{mode} has no reason to reach scratch storage"
            );
            assert!(
                scoped.draft_meta_sas_token.is_none(),
                "{mode} has no reason to reach draft artifacts - only ServerExecute reads them"
            );
        }
    }

    #[tokio::test]
    async fn test_azure_client_invoke_modes_do_not_reach_run_logs() {
        for mode in [
            CredentialsAccess::InvokeNone,
            CredentialsAccess::InvokeRead,
            CredentialsAccess::InvokeWrite,
        ] {
            let scoped = test_azure_runtime_credentials()
                .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, mode.clone())
                .await
                .expect("scoped credentials should be generated");

            // runs/{app_id} spans every run of the app by every user; only the
            // server executor writes it and only the API (ReadLogs) reads it.
            assert!(
                scoped.logs_sas_token.is_none(),
                "{mode} must not carry a run-logs SAS"
            );
        }
    }

    #[tokio::test]
    async fn test_azure_app_and_user_scopes_sign_the_directories_they_claim() {
        let creds = test_azure_runtime_credentials();

        let edit_app = creds
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::EditApp)
            .await
            .expect("scoped credentials should be generated");
        // apps/{app_id} on both the meta and the content container.
        assert_eq!(
            sas_directory_depth(edit_app.meta_sas_token.as_deref().expect("meta SAS")),
            Some(2)
        );
        assert_eq!(
            sas_directory_depth(edit_app.content_sas_token.as_deref().expect("content SAS")),
            Some(2)
        );

        let read_db = creds
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::ReadAppDb)
            .await
            .expect("scoped credentials should be generated");
        // apps/{app_id}/storage/db — narrower than the rest of the app's content.
        assert_eq!(
            sas_directory_depth(read_db.content_sas_token.as_deref().expect("db SAS")),
            Some(4)
        );

        let read_user = creds
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::ReadUser)
            .await
            .expect("scoped credentials should be generated");
        // users/{sub}/apps/{app_id}
        assert_eq!(
            sas_directory_depth(
                read_user
                    .user_content_sas_token
                    .as_deref()
                    .expect("user SAS")
            ),
            Some(4)
        );
    }

    #[test]
    fn test_azure_runtime_credentials_never_serialize_the_account_key() {
        let creds = test_azure_runtime_credentials();
        assert!(creds.account_key.is_some(), "fixture should carry a key");

        let json = to_string(&creds).expect("Failed to serialize");

        assert!(
            !json.contains("account_key"),
            "the storage master key must never leave this process: {json}"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_azure_master_credentials_setup() {
        init_env();
        let creds = AzureRuntimeCredentials::from_env();
        assert!(
            !creds.account_name.is_empty(),
            "AZURE_STORAGE_ACCOUNT_NAME must be set"
        );
        assert!(
            !creds.meta_container.is_empty(),
            "AZURE_META_CONTAINER must be set"
        );
        assert!(
            !creds.content_container.is_empty(),
            "AZURE_CONTENT_CONTAINER must be set"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_azure_master_credentials_can_write() {
        init_env();
        let creds = AzureRuntimeCredentials::from_env()
            .master_credentials()
            .await;
        let shared = creds.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from master credentials");

        let test_path = format!(
            "test/master-write-test-{}.txt",
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Azure(s) => {
                s.put(&path, b"test content".to_vec().into())
                    .await
                    .expect("Master credentials should be able to write");
                s.delete(&path).await.ok();
            }
            _ => panic!("Expected Azure store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_azure_single_directory_sas_can_write() {
        init_env();
        let master = AzureRuntimeCredentials::from_env()
            .master_credentials()
            .await;

        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::EditApp)
            .await
            .expect("Failed to generate scoped credentials");

        let shared = scoped.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from scoped credentials");

        let test_path = format!(
            "apps/{}/test-{}.txt",
            TEST_APP_ID,
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Azure(s) => {
                s.put(&path, b"scoped test content".to_vec().into())
                    .await
                    .expect("Single directory SAS should be able to write in allowed path");
                s.delete(&path).await.ok();
            }
            _ => panic!("Expected Azure store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_azure_single_directory_sas_cannot_write_outside() {
        init_env();
        let master = AzureRuntimeCredentials::from_env()
            .master_credentials()
            .await;

        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::EditApp)
            .await
            .expect("Failed to generate scoped credentials");

        let shared = scoped.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from scoped credentials");

        let test_path = format!(
            "apps/different-app/unauthorized-{}.txt",
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Azure(s) => {
                let result = s.put(&path, b"should fail".to_vec().into()).await;
                assert!(
                    result.is_err(),
                    "Single directory SAS should NOT be able to write outside allowed path. Got: {:?}",
                    result
                );
            }
            _ => panic!("Expected Azure store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_azure_scoped_credentials_can_write_in_scope() {
        init_env();
        let master = AzureRuntimeCredentials::from_env()
            .master_credentials()
            .await;

        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::InvokeWrite)
            .await
            .expect("Failed to generate scoped credentials");

        let shared = scoped.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from scoped credentials");

        let test_path = format!(
            "users/{}/apps/{}/test-{}.txt",
            TEST_SUB,
            TEST_APP_ID,
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Azure(s) => {
                s.put(&path, b"scoped test content".to_vec().into())
                    .await
                    .expect("Scoped credentials should be able to write in allowed path");
                s.delete(&path).await.ok();
            }
            _ => panic!("Expected Azure store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_azure_scoped_credentials_cannot_write_outside_scope() {
        init_env();
        let master = AzureRuntimeCredentials::from_env()
            .master_credentials()
            .await;

        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::InvokeWrite)
            .await
            .expect("Failed to generate scoped credentials");

        let shared = scoped.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from scoped credentials");

        let test_path = format!(
            "users/different-user/apps/{}/unauthorized-{}.txt",
            TEST_APP_ID,
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Azure(s) => {
                let result = s.put(&path, b"should fail".to_vec().into()).await;
                assert!(
                    result.is_err(),
                    "Scoped credentials should NOT be able to write outside allowed path. Got: {:?}",
                    result
                );
            }
            _ => panic!("Expected Azure store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_azure_scoped_credentials_read_only_cannot_write() {
        init_env();
        let master = AzureRuntimeCredentials::from_env()
            .master_credentials()
            .await;

        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::ReadApp)
            .await
            .expect("Failed to generate scoped credentials");

        let shared = scoped.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from scoped credentials");

        let test_path = format!(
            "apps/{}/readonly-test-{}.txt",
            TEST_APP_ID,
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Azure(s) => {
                let result = s.put(&path, b"should fail".to_vec().into()).await;
                assert!(
                    result.is_err(),
                    "Read-only scoped credentials should NOT be able to write. Got: {:?}",
                    result
                );
            }
            _ => panic!("Expected Azure store"),
        }
    }

    #[test]
    fn test_azure_runtime_credentials_serialization() {
        let creds = AzureRuntimeCredentials {
            content_sas_token: None,
            logs_sas_token: None,
            tmp_sas_token: None,
            draft_meta_sas_token: None,
            meta_sas_token: None,
            meta_container: "meta".to_string(),
            logs_container: "logs".to_string(),
            content_container: "content".to_string(),
            account_name: "teststorage".to_string(),
            account_key: None,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
            draft_meta_path_prefix: None,
            user_content_sas_token: None,
        };

        let json = to_string(&creds).expect("Failed to serialize");
        let deserialized: AzureRuntimeCredentials = from_str(&json).expect("Failed to deserialize");

        assert_eq!(creds.account_name, deserialized.account_name);
    }
}

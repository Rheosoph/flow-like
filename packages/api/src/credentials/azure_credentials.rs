use super::RuntimeCredentialsTrait;
#[cfg(feature = "azure")]
use crate::credentials::CredentialsAccess;
use crate::state::{AppState, State};
#[cfg(feature = "azure")]
use flow_like::credentials::{SharedCredentials, azure_credentials::AzureSharedCredentials};
use flow_like::{
    flow_like_storage::lancedb::{connect, connection::ConnectBuilder},
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::object_store;
use flow_like_types::{Result, anyhow, async_trait};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
    pub meta_container: String,
    pub content_container: String,
    pub logs_container: String,
    pub account_name: String,
    pub account_key: Option<String>,
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
    /// App-level content path prefix (e.g., "apps/{app_id}")
    pub content_path_prefix: Option<String>,
    /// User-level content path prefix (e.g., "users/{sub}/apps/{app_id}")
    pub user_content_path_prefix: Option<String>,
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
            meta_container: meta_container.to_string(),
            content_container: content_container.to_string(),
            logs_container: logs_container.to_string(),
            account_name: account_name.to_string(),
            account_key: None,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    pub fn from_env() -> Self {
        let logs_container = std::env::var("AZURE_LOG_CONTAINER").unwrap_or_default();
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
            meta_container: std::env::var("AZURE_META_CONTAINER").unwrap_or_default(),
            content_container: std::env::var("AZURE_CONTENT_CONTAINER").unwrap_or_default(),
            logs_container,
            account_name: std::env::var("AZURE_STORAGE_ACCOUNT_NAME").unwrap_or_default(),
            account_key: std::env::var("AZURE_STORAGE_ACCOUNT_KEY").ok(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    pub async fn master_credentials(&self) -> Self {
        AzureRuntimeCredentials {
            meta_sas_token: None,
            content_sas_token: None,
            user_content_sas_token: None,
            logs_sas_token: None,
            meta_container: self.meta_container.clone(),
            content_container: self.content_container.clone(),
            logs_container: self.logs_container.clone(),
            account_name: self.account_name.clone(),
            account_key: std::env::var("AZURE_STORAGE_ACCOUNT_KEY").ok(),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    #[tracing::instrument(
        name = "AzureRuntimeCredentials::scoped_credentials",
        skip(self, _state),
        level = "debug"
    )]
    pub async fn scoped_credentials(
        &self,
        sub: &str,
        app_id: &str,
        _state: &State,
        mode: CredentialsAccess,
    ) -> Result<Self> {
        if sub.is_empty() || app_id.is_empty() {
            return Err(flow_like_types::anyhow!("Sub or App ID cannot be empty"));
        }
        crate::credentials::validate_path_component(sub, "sub")?;
        crate::credentials::validate_path_component(app_id, "app_id")?;

        let account_key = self
            .account_key
            .clone()
            .or_else(|| std::env::var("AZURE_STORAGE_ACCOUNT_KEY").ok())
            .ok_or_else(|| {
                flow_like_types::anyhow!("AZURE_STORAGE_ACCOUNT_KEY environment variable not set")
            })?;

        let expiry = chrono::Utc::now() + chrono::Duration::hours(1);
        let expiry_str = expiry.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let start = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        // Determine which containers and paths need SAS tokens based on access mode
        let (
            meta_sas,
            content_sas,
            user_content_sas,
            logs_sas,
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
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                let content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("apps/{}", app_id),
                    "rwdl",
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                (
                    Some(meta_sas),
                    Some(content_sas),
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
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                let content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("apps/{}", app_id),
                    "rl",
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                (
                    Some(meta_sas),
                    Some(content_sas),
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
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                (
                    None,
                    Some(content_sas),
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
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                (
                    None,
                    Some(content_sas),
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
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                (
                    None,
                    Some(content_sas),
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
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                (
                    None,
                    Some(content_sas),
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
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                (
                    None,
                    None,
                    Some(user_content_sas),
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
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                (
                    None,
                    None,
                    Some(user_content_sas),
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
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                let logs_sas = generate_directory_sas(
                    &self.account_name,
                    &self.logs_container,
                    &format!("runs/{}", app_id),
                    "rwdl",
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                (
                    None,
                    None,
                    Some(user_content_sas),
                    Some(logs_sas),
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
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                let user_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("users/{}/apps/{}", sub, app_id),
                    "rl",
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                let logs_sas = generate_directory_sas(
                    &self.account_name,
                    &self.logs_container,
                    &format!("runs/{}", app_id),
                    "rl",
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                (
                    None,
                    Some(app_content_sas),
                    Some(user_content_sas),
                    Some(logs_sas),
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
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                let user_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("users/{}/apps/{}", sub, app_id),
                    "rwdl",
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                let logs_sas = generate_directory_sas(
                    &self.account_name,
                    &self.logs_container,
                    &format!("runs/{}", app_id),
                    "rwdl",
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                (
                    None,
                    Some(app_content_sas),
                    Some(user_content_sas),
                    Some(logs_sas),
                    Some(format!("apps/{}", app_id)),
                    Some(format!("users/{}/apps/{}", sub, app_id)),
                )
            }
            CredentialsAccess::ServerExecute => {
                let meta_sas = generate_directory_sas(
                    &self.account_name,
                    &self.meta_container,
                    &format!("apps/{}", app_id),
                    "rl",
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                let app_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("apps/{}", app_id),
                    "rwdl",
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                let user_content_sas = generate_directory_sas(
                    &self.account_name,
                    &self.content_container,
                    &format!("users/{}/apps/{}", sub, app_id),
                    "rwdl",
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                let logs_sas = generate_directory_sas(
                    &self.account_name,
                    &self.logs_container,
                    &format!("runs/{}", app_id),
                    "rwdl",
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                (
                    Some(meta_sas),
                    Some(app_content_sas),
                    Some(user_content_sas),
                    Some(logs_sas),
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
                    &start,
                    &expiry_str,
                    &account_key,
                )?;
                (None, None, None, Some(logs_sas), None, None)
            }
        };

        Ok(Self {
            meta_sas_token: meta_sas,
            content_sas_token: content_sas,
            user_content_sas_token: user_content_sas,
            logs_sas_token: logs_sas,
            meta_container: self.meta_container.clone(),
            content_container: self.content_container.clone(),
            logs_container: self.logs_container.clone(),
            account_name: self.account_name.clone(),
            account_key: None,
            expiration: Some(expiry),
            content_path_prefix,
            user_content_path_prefix,
        })
    }

    /// Test-only version of scoped_credentials that doesn't require State
    /// Uses Directory SAS for path-level security (requires HNS/ADLS Gen2)
    #[cfg(test)]
    pub async fn scoped_credentials_for_test(
        &self,
        sub: &str,
        app_id: &str,
        mode: CredentialsAccess,
    ) -> Result<Self> {
        if sub.is_empty() || app_id.is_empty() {
            return Err(flow_like_types::anyhow!("Sub or App ID cannot be empty"));
        }
        crate::credentials::validate_path_component(sub, "sub")?;
        crate::credentials::validate_path_component(app_id, "app_id")?;

        let account_key = self
            .account_key
            .clone()
            .or_else(|| std::env::var("AZURE_STORAGE_ACCOUNT_KEY").ok())
            .ok_or_else(|| {
                flow_like_types::anyhow!("AZURE_STORAGE_ACCOUNT_KEY environment variable not set")
            })?;

        let permissions = match mode {
            CredentialsAccess::EditApp => "racwdl",
            CredentialsAccess::ReadApp => "rl",
            CredentialsAccess::ReadAppContent => "rl",
            CredentialsAccess::EditAppContent => "racwdl",
            CredentialsAccess::ReadAppDb => "rl",
            CredentialsAccess::EditAppDb => "racwdl",
            CredentialsAccess::EditUser => "racwdl",
            CredentialsAccess::ReadUser => "rl",
            CredentialsAccess::InvokeNone => "racwdl",
            CredentialsAccess::InvokeRead => "rl",
            CredentialsAccess::InvokeWrite => "racwdl",
            CredentialsAccess::ServerExecute => "racwdl",
            CredentialsAccess::ReadLogs => "rl",
        };

        let expiry = chrono::Utc::now() + chrono::Duration::hours(1);
        let expiry_str = expiry.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let start = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        // Determine the app/user content paths based on access mode
        // EditApp/ReadApp/ReadAppContent/EditAppContent: apps/{app_id}
        // EditUser/ReadUser: users/{sub}/apps/{app_id}
        // InvokeNone: user path only
        // InvokeRead/InvokeWrite/ServerExecute: both app and user paths
        // ReadLogs: logs/{app_id}
        let (app_directory, user_directory) = match mode {
            CredentialsAccess::EditApp
            | CredentialsAccess::ReadApp
            | CredentialsAccess::ReadAppContent
            | CredentialsAccess::EditAppContent => (Some(format!("apps/{}", app_id)), None),
            CredentialsAccess::ReadAppDb | CredentialsAccess::EditAppDb => {
                (Some(format!("apps/{}/storage/db", app_id)), None)
            }
            CredentialsAccess::EditUser | CredentialsAccess::ReadUser => {
                (None, Some(format!("users/{}/apps/{}", sub, app_id)))
            }
            CredentialsAccess::InvokeNone => (None, Some(format!("users/{}/apps/{}", sub, app_id))),
            CredentialsAccess::InvokeRead
            | CredentialsAccess::InvokeWrite
            | CredentialsAccess::ServerExecute => (
                Some(format!("apps/{}", app_id)),
                Some(format!("users/{}/apps/{}", sub, app_id)),
            ),
            CredentialsAccess::ReadLogs => (None, None),
        };

        // Use Directory SAS for path-level security (requires HNS/ADLS Gen2)
        let app_sas_token = if let Some(ref dir) = app_directory {
            Some(generate_directory_sas(
                &self.account_name,
                &self.content_container,
                dir,
                permissions,
                &start,
                &expiry_str,
                &account_key,
            )?)
        } else {
            None
        };

        let user_sas_token = if let Some(ref dir) = user_directory {
            Some(generate_directory_sas(
                &self.account_name,
                &self.content_container,
                dir,
                permissions,
                &start,
                &expiry_str,
                &account_key,
            )?)
        } else {
            None
        };

        let meta_sas_token = match mode {
            CredentialsAccess::EditApp | CredentialsAccess::ReadApp => app_sas_token.clone(),
            CredentialsAccess::ServerExecute => Some(generate_directory_sas(
                &self.account_name,
                &self.meta_container,
                &format!("apps/{}", app_id),
                "rl",
                &start,
                &expiry_str,
                &account_key,
            )?),
            _ => None,
        };

        let logs_sas_token = if matches!(mode, CredentialsAccess::ServerExecute) {
            Some(generate_directory_sas(
                &self.account_name,
                &self.logs_container,
                &format!("runs/{}", app_id),
                "racwdl",
                &start,
                &expiry_str,
                &account_key,
            )?)
        } else {
            None
        };

        let content_path_prefix = match mode {
            CredentialsAccess::ReadAppDb | CredentialsAccess::EditAppDb => {
                Some(format!("apps/{}", app_id))
            }
            _ => app_directory,
        };

        Ok(Self {
            meta_sas_token,
            content_sas_token: app_sas_token,
            user_content_sas_token: user_sas_token,
            logs_sas_token,
            meta_container: self.meta_container.clone(),
            content_container: self.content_container.clone(),
            logs_container: self.logs_container.clone(),
            account_name: self.account_name.clone(),
            account_key: None,
            expiration: Some(expiry),
            content_path_prefix,
            user_content_path_prefix: user_directory,
        })
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

/// Generate a Directory SAS token (requires HNS/Data Lake Storage Gen2)
/// This provides path-level security - access is restricted to the specified directory and its contents.
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
fn generate_directory_sas(
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
            meta_container: self.meta_container.clone(),
            content_container: self.content_container.clone(),
            logs_container: self.logs_container.clone(),
            account_name: self.account_name.clone(),
            account_key: self.account_key.clone(),
            expiration: self.expiration,
            content_path_prefix: self.content_path_prefix.clone(),
            user_content_path_prefix: self.user_content_path_prefix.clone(),
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
        let (meta_store, content_store) = {
            use flow_like_types::tokio;

            tokio::join!(
                async { self.into_shared_credentials().to_store(true).await },
                async { self.into_shared_credentials().to_store(false).await },
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

        let (account, container, sas) = (
            self.account_name.clone(),
            self.content_container.clone(),
            self.content_sas_token.clone().unwrap_or_default(),
        );

        config.register_build_logs_database(Arc::new(make_azure_builder(
            account.clone(),
            container.clone(),
            sas.clone(),
        )));
        config.register_build_project_database(Arc::new(make_azure_builder(
            account.clone(),
            container.clone(),
            sas.clone(),
        )));
        config.register_build_user_database(Arc::new(make_azure_builder(account, container, sas)));

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
        connect(&url)
            .storage_option(
                "azure_storage_account_name".to_string(),
                account_name.clone(),
            )
            .storage_option("azure_storage_sas_token".to_string(), sas_token.clone())
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
            meta_container: "meta-secret".to_string(),
            content_container: "content-data".to_string(),
            logs_container: "logs".to_string(),
            account_name: "account".to_string(),
            account_key: Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string()),
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    fn sas_permissions(token: &str) -> Option<&str> {
        token.split('&').find_map(|part| part.strip_prefix("sp="))
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

        let content = scoped
            .content_sas_token
            .as_deref()
            .expect("ServerExecute should include content credentials");
        let content_permissions =
            sas_permissions(content).expect("content SAS should include permissions");
        assert!(content_permissions.contains('w'));
        assert!(content_permissions.contains('d'));

        assert!(scoped.logs_sas_token.is_some());
        assert_eq!(
            scoped.content_path_prefix,
            Some(format!("apps/{}", TEST_APP_ID))
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
            meta_sas_token: None,
            meta_container: "meta".to_string(),
            logs_container: "logs".to_string(),
            content_container: "content".to_string(),
            account_name: "teststorage".to_string(),
            account_key: None,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
            user_content_sas_token: None,
        };

        let json = to_string(&creds).expect("Failed to serialize");
        let deserialized: AzureRuntimeCredentials = from_str(&json).expect("Failed to deserialize");

        assert_eq!(creds.account_name, deserialized.account_name);
    }
}

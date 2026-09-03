use super::RuntimeCredentialsTrait;
#[cfg(feature = "gcp")]
use crate::credentials::CredentialsAccess;
use crate::state::{AppState, State};
#[cfg(feature = "gcp")]
use flow_like::credentials::{SharedCredentials, gcp_credentials::GcpSharedCredentials};
use flow_like::{
    flow_like_storage::lancedb::{connect, connection::ConnectBuilder},
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_storage::object_store;
use flow_like_types::{Result, anyhow, async_trait};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

#[cfg(feature = "gcp")]
const GCS_STORAGE_TOKEN_OPTION: &str = "google_storage_token";
#[cfg(feature = "gcp")]
const GCS_SERVICE_ACCOUNT_KEY_OPTION: &str = "google_service_account_key";

/// Metadata-server endpoint that mints an OAuth2 token for the service account
/// bound to the running Cloud Run revision, GCE instance or GKE pod.
#[cfg(feature = "gcp")]
const METADATA_TOKEN_PATH: &str = "/computeMetadata/v1/instance/service-accounts/default/token";
#[cfg(feature = "gcp")]
const DEFAULT_METADATA_HOST: &str = "metadata.google.internal";
#[cfg(feature = "gcp")]
const DEFAULT_METADATA_IP: &str = "169.254.169.254";
#[cfg(feature = "gcp")]
const METADATA_FLAVOR_HEADER: &str = "Metadata-Flavor";
#[cfg(feature = "gcp")]
const METADATA_FLAVOR_VALUE: &str = "Google";
/// A cached metadata token is only reused while this much life remains. The
/// downscoped token handed to a client inherits the base token's remaining
/// lifetime, so serving a nearly-dead base token would mint client credentials
/// that expire minutes after they are issued.
#[cfg(feature = "gcp")]
const METADATA_TOKEN_MIN_LIFETIME_SECONDS: i64 = 5 * 60;
/// Ceiling on cache residency. Metadata tokens live ~1h and the metadata server
/// refreshes its own copy near the end of that window; re-asking every ten
/// minutes keeps this process close to a full-lifetime token without turning
/// every scoped-credential request into a metadata round trip.
#[cfg(feature = "gcp")]
const METADATA_TOKEN_CACHE_TTL_SECONDS: u64 = 10 * 60;
#[cfg(feature = "gcp")]
const METADATA_CONNECT_TIMEOUT_SECONDS: u64 = 3;
#[cfg(feature = "gcp")]
const METADATA_REQUEST_TIMEOUT_SECONDS: u64 = 10;

#[cfg(feature = "gcp")]
#[derive(Clone)]
struct CachedMetadataToken {
    token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

#[cfg(feature = "gcp")]
static METADATA_TOKENS: OnceLock<moka::sync::Cache<String, CachedMetadataToken>> = OnceLock::new();

/// GCP Runtime Credentials with downscoped access tokens
///
/// SECURITY: Uses GCP Credential Access Boundaries to create tokens that are
/// cryptographically restricted to specific paths and permissions, similar to
/// AWS STS and Azure Directory SAS. GCP enforces these restrictions server-side.
///
/// The flow is:
/// 1. Generate a base OAuth2 access token from the service account
/// 2. Exchange it for a downscoped token with Credential Access Boundary
/// 3. The downscoped token can only access the specified paths/permissions
#[cfg(feature = "gcp")]
#[derive(Clone, Serialize, Deserialize)]
pub struct GcpRuntimeCredentials {
    /// Master service account key (server-side only, never sent to clients)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_account_key: Option<String>,
    /// Short-lived downscoped OAuth2 access token (sent to clients)
    pub access_token: Option<String>,
    pub meta_bucket: String,
    pub content_bucket: String,
    pub logs_bucket: String,
    /// Allowed path prefixes (enforced by GCP via Credential Access Boundary)
    pub allowed_prefixes: Vec<String>,
    /// Whether write operations are allowed
    pub write_access: bool,
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
    pub content_path_prefix: Option<String>,
    pub user_content_path_prefix: Option<String>,
}

#[cfg(feature = "gcp")]
impl std::fmt::Debug for GcpRuntimeCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcpRuntimeCredentials")
            .field(
                "service_account_key",
                &self.service_account_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "access_token",
                &self.access_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("meta_bucket", &self.meta_bucket)
            .field("content_bucket", &self.content_bucket)
            .field("logs_bucket", &self.logs_bucket)
            .field("write_access", &self.write_access)
            .field("expiration", &self.expiration)
            .finish()
    }
}

#[cfg(feature = "gcp")]
impl GcpRuntimeCredentials {
    pub fn new(meta_bucket: &str, content_bucket: &str, logs_bucket: &str) -> Self {
        GcpRuntimeCredentials {
            service_account_key: None,
            access_token: None,
            meta_bucket: meta_bucket.to_string(),
            content_bucket: content_bucket.to_string(),
            logs_bucket: logs_bucket.to_string(),
            allowed_prefixes: Vec::new(),
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    pub fn from_env() -> Self {
        let service_account_key = service_account_key_from_env();
        let logs_bucket = std::env::var("GCP_LOG_BUCKET")
            .or_else(|_| std::env::var("LOG_BUCKET"))
            .unwrap_or_default();
        if logs_bucket.is_empty() {
            tracing::warn!(
                "GCP_LOG_BUCKET environment variable is not set - logs will not be persisted"
            );
        }

        GcpRuntimeCredentials {
            service_account_key,
            access_token: None,
            meta_bucket: std::env::var("GCP_META_BUCKET")
                .or_else(|_| std::env::var("META_BUCKET"))
                .unwrap_or_default(),
            content_bucket: std::env::var("GCP_CONTENT_BUCKET")
                .or_else(|_| std::env::var("CONTENT_BUCKET"))
                .unwrap_or_default(),
            logs_bucket,
            allowed_prefixes: Vec::new(),
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    pub async fn master_credentials(&self) -> Self {
        let service_account_key = service_account_key_from_env();

        GcpRuntimeCredentials {
            service_account_key,
            access_token: None,
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            allowed_prefixes: Vec::new(),
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        }
    }

    /// Decide where the base OAuth2 token comes from.
    ///
    /// An explicitly configured key always wins. Silently preferring the
    /// metadata identity over a key the operator deliberately supplied would
    /// swap the acting principal — and with it the effective IAM grants — with
    /// no signal at all, so the keyless path only engages where it is the only
    /// path. That is every Cloud Run / GKE Workload Identity deployment, which
    /// is precisely where the previous hard error made this unusable.
    ///
    /// A blank key never reaches here from configuration —
    /// `service_account_key_from_env` collapses an empty
    /// `GOOGLE_APPLICATION_CREDENTIALS_JSON` to `None` so that "set but empty"
    /// cannot masquerade as a key — but a deserialized payload can still carry
    /// one, and blank key material is unusable either way.
    fn base_token_source(&self) -> GcpBaseTokenSource {
        match self
            .service_account_key
            .as_ref()
            .filter(|key| !key.trim().is_empty())
        {
            Some(key) => GcpBaseTokenSource::ServiceAccountKey(key.clone()),
            None => GcpBaseTokenSource::Metadata,
        }
    }

    /// Whether this credential set came from `scoped_credentials` rather than
    /// from server configuration. Mirrors `GcpSharedCredentials::is_scoped`.
    fn is_scoped(&self) -> bool {
        !self.allowed_prefixes.is_empty()
            || self.expiration.is_some()
            || self.content_path_prefix.is_some()
            || self.user_content_path_prefix.is_some()
    }

    /// Gate in front of the keyless (ADC) database builders.
    ///
    /// ADC resolves the workload's own runtime service account, which carries no
    /// Credential Access Boundary and so is unrestricted across the bucket. That
    /// is the intended identity for master credentials under Workload Identity
    /// and the wrong one for a scoped credential, where the downscoped token is
    /// the only thing enforcing prefix isolation.
    fn ensure_keyless_allowed(&self) -> Result<()> {
        if self.is_scoped() {
            return Err(anyhow!(
                "scoped GCP credentials carry no usable access token (prefixes: {:?}, expiration: {:?}) - \
                 refusing to fall back to the ambient runtime identity, which enforces no prefix restriction",
                self.allowed_prefixes,
                self.expiration
            ));
        }
        Ok(())
    }

    #[tracing::instrument(
        name = "GcpRuntimeCredentials::scoped_credentials",
        skip(self, sub, state),
        level = "debug"
    )]
    pub async fn scoped_credentials(
        &self,
        sub: &str,
        app_id: &str,
        state: &State,
        mode: CredentialsAccess,
    ) -> Result<Self> {
        if sub.is_empty() || app_id.is_empty() {
            return Err(anyhow!("Sub or App ID cannot be empty"));
        }
        crate::credentials::validate_path_component(sub, "sub")?;
        crate::credentials::validate_path_component(app_id, "app_id")?;

        // Decided here, ahead of the prefix work, so the one place this flow
        // branches by credential shape stays visible at the top. Under Workload
        // Identity there is no key JSON anywhere in the environment, so the
        // unconditional key requirement that used to sit on this line failed
        // every scoped-credential request on Cloud Run.
        let token_source = self.base_token_source();

        let apps_prefix = format!("apps/{}", app_id);
        let db_prefix = format!("{}/storage/db", apps_prefix);
        let user_prefix = format!("users/{}/apps/{}", sub, app_id);
        let log_prefix = format!("runs/{}", app_id);
        // The writers (`/tmp` presign, HTTP-sink offload) run both segments
        // through `storage_path_segment`, so the prefixes this credential
        // authorises have to as well — a raw `sink:abc` here would name a
        // directory nothing ever writes to.
        let (temporary_user_prefix, temporary_global_prefix) =
            crate::credentials::temporary_prefixes(sub, app_id);
        let (content_path_prefix, user_content_path_prefix) =
            scoped_content_path_prefixes(&apps_prefix, &user_prefix, &mode);

        let (allowed_prefixes, write_access) = match mode {
            CredentialsAccess::EditApp => (vec![apps_prefix.clone()], true),
            CredentialsAccess::ReadApp => (vec![apps_prefix.clone()], false),
            // GCP scoping is prefix-based on a single bucket; we
            // don't have a per-bucket policy to fall back to.
            // Practical safety relies on the client only ever using
            // these creds against the content store; the fork code
            // enforces that, but a misbehaving client could read or
            // write meta paths if both stores live in the same
            // bucket. Same scope as `ReadApp` / `EditApp`.
            CredentialsAccess::ReadAppContent => (vec![apps_prefix.clone()], false),
            CredentialsAccess::EditAppContent => (vec![apps_prefix.clone()], true),
            CredentialsAccess::ReadAppDb => (vec![db_prefix.clone()], false),
            CredentialsAccess::EditAppDb => (vec![db_prefix.clone()], true),
            CredentialsAccess::EditUser => (vec![user_prefix.clone()], true),
            CredentialsAccess::ReadUser => (vec![user_prefix.clone()], false),
            // Run logs (`runs/{app_id}`) are written only by the server
            // executor (ServerExecute) and read only through the API
            // (ReadLogs); the desktop keeps its own local log store. The
            // client-facing invoke modes therefore never touch that prefix.
            CredentialsAccess::InvokeNone => (
                vec![user_prefix.clone(), temporary_user_prefix.clone()],
                true,
            ),
            CredentialsAccess::InvokeRead => (
                vec![
                    apps_prefix.clone(),
                    user_prefix.clone(),
                    temporary_user_prefix.clone(),
                    temporary_global_prefix.clone(),
                ],
                false,
            ),
            CredentialsAccess::InvokeWrite => (
                vec![
                    apps_prefix.clone(),
                    user_prefix.clone(),
                    temporary_user_prefix.clone(),
                    temporary_global_prefix.clone(),
                ],
                true,
            ),
            CredentialsAccess::ServerExecute => (
                vec![
                    apps_prefix.clone(),
                    user_prefix.clone(),
                    temporary_user_prefix.clone(),
                    temporary_global_prefix.clone(),
                    log_prefix.clone(),
                ],
                true,
            ),
            // Shadow/replay: same reachable prefixes as ServerExecute; the
            // access-boundary rules below drop writes on app/user content.
            CredentialsAccess::ShadowExecute => (
                vec![
                    apps_prefix.clone(),
                    user_prefix.clone(),
                    temporary_user_prefix.clone(),
                    temporary_global_prefix.clone(),
                    log_prefix.clone(),
                ],
                true,
            ),
            CredentialsAccess::ReadLogs => (vec![log_prefix.clone()], false),
        };

        // Generate a base access token, then downscope it with Credential Access Boundary
        let base_token = token_source.access_token(state).await?;
        let access_token = if matches!(mode, CredentialsAccess::ServerExecute) {
            downscope_token_for_rules(
                &base_token,
                &[
                    GcpAccessRule::new(
                        self.content_bucket.clone(),
                        vec![
                            apps_prefix.clone(),
                            user_prefix.clone(),
                            temporary_user_prefix.clone(),
                            temporary_global_prefix.clone(),
                        ],
                        true,
                    ),
                    // Run logs are append-only: the executor creates and
                    // appends Lance tables but never deletes; Lance's own
                    // auto-cleanup is best-effort and logs on denial.
                    GcpAccessRule::with_access(
                        self.logs_bucket.clone(),
                        vec![log_prefix.clone()],
                        GcpAccess::Append,
                    ),
                    server_execute_meta_rule(&self.meta_bucket, &apps_prefix),
                ],
            )
            .await?
        } else if matches!(mode, CredentialsAccess::ShadowExecute) {
            downscope_token_for_rules(
                &base_token,
                &[
                    // A shadow run reads live app/user content but may never
                    // mutate it; scratch stays read-write for request-file
                    // offloads and cache paths.
                    GcpAccessRule::with_access(
                        self.content_bucket.clone(),
                        vec![apps_prefix.clone(), user_prefix.clone()],
                        GcpAccess::Read,
                    ),
                    GcpAccessRule::new(
                        self.content_bucket.clone(),
                        vec![
                            temporary_user_prefix.clone(),
                            temporary_global_prefix.clone(),
                        ],
                        true,
                    ),
                    // Run logs stay append-only so the shadow run is recorded.
                    GcpAccessRule::with_access(
                        self.logs_bucket.clone(),
                        vec![log_prefix.clone()],
                        GcpAccess::Append,
                    ),
                    server_execute_meta_rule(&self.meta_bucket, &apps_prefix),
                ],
            )
            .await?
        } else if matches!(mode, CredentialsAccess::ReadLogs) {
            // Run logs live in the logs bucket (see `to_logs_db_builder`), not
            // the content bucket the other client modes are scoped to.
            downscope_token_for_rules(
                &base_token,
                &[GcpAccessRule::with_access(
                    self.logs_bucket.clone(),
                    vec![log_prefix.clone()],
                    GcpAccess::Read,
                )],
            )
            .await?
        } else {
            downscope_token(
                &base_token,
                &self.content_bucket,
                &allowed_prefixes,
                write_access,
            )
            .await?
        };
        let chrono_expiration = chrono::Utc::now() + chrono::Duration::hours(1);

        Ok(Self {
            service_account_key: None, // Never send the key to clients
            access_token: Some(access_token),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            allowed_prefixes,
            write_access,
            expiration: Some(chrono_expiration),
            content_path_prefix,
            user_content_path_prefix,
        })
    }

    /// Test-only version using the service account key directly
    /// In production, use scoped_credentials with State
    #[cfg(test)]
    pub async fn scoped_credentials_for_test(
        &self,
        sub: &str,
        app_id: &str,
        mode: CredentialsAccess,
    ) -> Result<Self> {
        if sub.is_empty() || app_id.is_empty() {
            return Err(anyhow!("Sub or App ID cannot be empty"));
        }
        crate::credentials::validate_path_component(sub, "sub")?;
        crate::credentials::validate_path_component(app_id, "app_id")?;

        let service_account_key = self
            .service_account_key
            .clone()
            .or_else(|| std::env::var("GOOGLE_APPLICATION_CREDENTIALS_JSON").ok())
            .ok_or_else(|| anyhow!("GOOGLE_APPLICATION_CREDENTIALS_JSON is not set"))?;

        let apps_prefix = format!("apps/{}", app_id);
        let db_prefix = format!("{}/storage/db", apps_prefix);
        let user_prefix = format!("users/{}/apps/{}", sub, app_id);
        let log_prefix = format!("runs/{}", app_id);
        // The writers (`/tmp` presign, HTTP-sink offload) run both segments
        // through `storage_path_segment`, so the prefixes this credential
        // authorises have to as well — a raw `sink:abc` here would name a
        // directory nothing ever writes to.
        let (temporary_user_prefix, temporary_global_prefix) =
            crate::credentials::temporary_prefixes(sub, app_id);
        let (content_path_prefix, user_content_path_prefix) =
            scoped_content_path_prefixes(&apps_prefix, &user_prefix, &mode);

        let (allowed_prefixes, write_access) = match mode {
            CredentialsAccess::EditApp => (vec![apps_prefix.clone()], true),
            CredentialsAccess::ReadApp => (vec![apps_prefix.clone()], false),
            // GCP scoping is prefix-based on a single bucket; we
            // don't have a per-bucket policy to fall back to.
            // Practical safety relies on the client only ever using
            // these creds against the content store; the fork code
            // enforces that, but a misbehaving client could read or
            // write meta paths if both stores live in the same
            // bucket. Same scope as `ReadApp` / `EditApp`.
            CredentialsAccess::ReadAppContent => (vec![apps_prefix.clone()], false),
            CredentialsAccess::EditAppContent => (vec![apps_prefix.clone()], true),
            CredentialsAccess::ReadAppDb => (vec![db_prefix.clone()], false),
            CredentialsAccess::EditAppDb => (vec![db_prefix.clone()], true),
            CredentialsAccess::EditUser => (vec![user_prefix.clone()], true),
            CredentialsAccess::ReadUser => (vec![user_prefix.clone()], false),
            // Run logs (`runs/{app_id}`) are written only by the server
            // executor (ServerExecute) and read only through the API
            // (ReadLogs); the desktop keeps its own local log store. The
            // client-facing invoke modes therefore never touch that prefix.
            CredentialsAccess::InvokeNone => (
                vec![user_prefix.clone(), temporary_user_prefix.clone()],
                true,
            ),
            CredentialsAccess::InvokeRead => (
                vec![
                    apps_prefix.clone(),
                    user_prefix.clone(),
                    temporary_user_prefix.clone(),
                    temporary_global_prefix.clone(),
                ],
                false,
            ),
            CredentialsAccess::InvokeWrite => (
                vec![
                    apps_prefix.clone(),
                    user_prefix.clone(),
                    temporary_user_prefix.clone(),
                    temporary_global_prefix.clone(),
                ],
                true,
            ),
            CredentialsAccess::ServerExecute => (
                vec![
                    apps_prefix.clone(),
                    user_prefix.clone(),
                    temporary_user_prefix.clone(),
                    temporary_global_prefix.clone(),
                    log_prefix.clone(),
                ],
                true,
            ),
            // Shadow/replay: same reachable prefixes as ServerExecute; the
            // access-boundary rules below drop writes on app/user content.
            CredentialsAccess::ShadowExecute => (
                vec![
                    apps_prefix.clone(),
                    user_prefix.clone(),
                    temporary_user_prefix.clone(),
                    temporary_global_prefix.clone(),
                    log_prefix.clone(),
                ],
                true,
            ),
            CredentialsAccess::ReadLogs => (vec![log_prefix.clone()], false),
        };

        // Generate a base access token, then downscope it with Credential Access Boundary
        let base_token = generate_access_token_standalone(&service_account_key).await?;
        let access_token = if matches!(mode, CredentialsAccess::ServerExecute) {
            downscope_token_for_rules(
                &base_token,
                &[
                    GcpAccessRule::new(
                        self.content_bucket.clone(),
                        vec![
                            apps_prefix.clone(),
                            user_prefix.clone(),
                            temporary_user_prefix.clone(),
                            temporary_global_prefix.clone(),
                        ],
                        true,
                    ),
                    // Run logs are append-only: the executor creates and
                    // appends Lance tables but never deletes; Lance's own
                    // auto-cleanup is best-effort and logs on denial.
                    GcpAccessRule::with_access(
                        self.logs_bucket.clone(),
                        vec![log_prefix.clone()],
                        GcpAccess::Append,
                    ),
                    server_execute_meta_rule(&self.meta_bucket, &apps_prefix),
                ],
            )
            .await?
        } else if matches!(mode, CredentialsAccess::ShadowExecute) {
            downscope_token_for_rules(
                &base_token,
                &[
                    // A shadow run reads live app/user content but may never
                    // mutate it; scratch stays read-write for request-file
                    // offloads and cache paths.
                    GcpAccessRule::with_access(
                        self.content_bucket.clone(),
                        vec![apps_prefix.clone(), user_prefix.clone()],
                        GcpAccess::Read,
                    ),
                    GcpAccessRule::new(
                        self.content_bucket.clone(),
                        vec![
                            temporary_user_prefix.clone(),
                            temporary_global_prefix.clone(),
                        ],
                        true,
                    ),
                    // Run logs stay append-only so the shadow run is recorded.
                    GcpAccessRule::with_access(
                        self.logs_bucket.clone(),
                        vec![log_prefix.clone()],
                        GcpAccess::Append,
                    ),
                    server_execute_meta_rule(&self.meta_bucket, &apps_prefix),
                ],
            )
            .await?
        } else if matches!(mode, CredentialsAccess::ReadLogs) {
            // Run logs live in the logs bucket (see `to_logs_db_builder`), not
            // the content bucket the other client modes are scoped to.
            downscope_token_for_rules(
                &base_token,
                &[GcpAccessRule::with_access(
                    self.logs_bucket.clone(),
                    vec![log_prefix.clone()],
                    GcpAccess::Read,
                )],
            )
            .await?
        } else {
            downscope_token(
                &base_token,
                &self.content_bucket,
                &allowed_prefixes,
                write_access,
            )
            .await?
        };
        let chrono_expiration = chrono::Utc::now() + chrono::Duration::hours(1);

        Ok(Self {
            service_account_key: None,
            access_token: Some(access_token),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            allowed_prefixes,
            write_access,
            expiration: Some(chrono_expiration),
            content_path_prefix,
            user_content_path_prefix,
        })
    }
}

#[cfg(feature = "gcp")]
fn scoped_content_path_prefixes(
    apps_prefix: &str,
    user_prefix: &str,
    mode: &CredentialsAccess,
) -> (Option<String>, Option<String>) {
    let app = matches!(
        mode,
        CredentialsAccess::EditApp
            | CredentialsAccess::ReadApp
            | CredentialsAccess::ReadAppContent
            | CredentialsAccess::EditAppContent
            | CredentialsAccess::ReadAppDb
            | CredentialsAccess::EditAppDb
            | CredentialsAccess::InvokeRead
            | CredentialsAccess::InvokeWrite
            | CredentialsAccess::ServerExecute
            | CredentialsAccess::ShadowExecute
    )
    .then(|| apps_prefix.to_string());

    let user = matches!(
        mode,
        CredentialsAccess::EditUser
            | CredentialsAccess::ReadUser
            | CredentialsAccess::InvokeNone
            | CredentialsAccess::InvokeRead
            | CredentialsAccess::InvokeWrite
            | CredentialsAccess::ServerExecute
            | CredentialsAccess::ShadowExecute
    )
    .then(|| user_prefix.to_string());

    (app, user)
}

/// Where the base OAuth2 token that the STS Credential Access Boundary exchange
/// downscopes is obtained from.
///
/// Only base-token acquisition branches here. Everything downstream — the
/// access boundary rules, the `CredentialsAccess` table, the prefix scoping —
/// is identical for both variants, because the STS exchange accepts any access
/// token regardless of how it was minted.
#[cfg(feature = "gcp")]
enum GcpBaseTokenSource {
    /// Cloud Run, GCE and GKE Workload Identity. No key material exists in the
    /// environment; the platform mints tokens for the service account bound to
    /// the workload.
    Metadata,
    /// Local development and non-GCP hosts, where an explicitly supplied
    /// service account key JSON is the only way to authenticate.
    ServiceAccountKey(String),
}

#[cfg(feature = "gcp")]
impl GcpBaseTokenSource {
    async fn access_token(&self, state: &State) -> Result<String> {
        match self {
            Self::Metadata => fetch_metadata_token().await,
            Self::ServiceAccountKey(key) => generate_access_token(key, state).await,
        }
    }
}

/// Read the master service account key from the environment, treating a
/// set-but-empty variable as absent.
///
/// `std::env::var` returns `Ok("")` for `GOOGLE_APPLICATION_CREDENTIALS_JSON=`,
/// which is the shape an optional Terraform variable renders to. Passing that
/// through as `Some("")` made "no key configured" and "key configured as blank"
/// indistinguishable downstream, where the difference decides whether the
/// process runs on the configured principal or the ambient one.
#[cfg(feature = "gcp")]
fn service_account_key_from_env() -> Option<String> {
    std::env::var("GOOGLE_APPLICATION_CREDENTIALS_JSON")
        .ok()
        .filter(|key| !key.trim().is_empty())
}

/// Metadata authorities to try, in order: hostname first, link-local IP second.
///
/// Mirrors object_store's `InstanceCredentialProvider` — including the
/// `GCE_METADATA_*` overrides — so the token this module mints and the token
/// object_store mints inside the same process come from the same server. A
/// deployment that redirected one and not the other would quietly run on two
/// different identities. The IP fallback exists because metadata access must
/// survive a pod with no working DNS; the hostname is unresolvable long before
/// the address is unreachable.
#[cfg(feature = "gcp")]
fn metadata_authorities() -> [String; 2] {
    let non_empty = |value: String| {
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    };

    let host = std::env::var("GCE_METADATA_HOST")
        .or_else(|_| std::env::var("GCE_METADATA_ROOT"))
        .ok()
        .and_then(non_empty)
        .unwrap_or_else(|| DEFAULT_METADATA_HOST.to_string());
    let ip = std::env::var("GCE_METADATA_IP")
        .ok()
        .and_then(non_empty)
        .unwrap_or_else(|| DEFAULT_METADATA_IP.to_string());

    [host, ip]
}

/// Fetch an OAuth2 access token for the workload's own service account from the
/// GCE/Cloud Run metadata server.
///
/// This is the keyless path. Under Workload Identity no service account key
/// JSON exists to sign a JWT assertion with, so the metadata server is the only
/// source of a base token — and without it every scoped-credential request on
/// Cloud Run failed outright.
///
/// The returned token never leaves this process: it is the *subject* token of
/// the STS exchange, and only the downscoped result reaches a client. That is
/// why no scope narrowing is requested here. The metadata server hands back the
/// workload's full token (`cloud-platform` on Cloud Run), which is what the
/// Credential Access Boundary exchange expects as its subject; the narrowing
/// that matters is expressed by the boundary itself, which GCP enforces
/// server-side on every object operation.
#[cfg(feature = "gcp")]
async fn fetch_metadata_token() -> Result<String> {
    let [host, ip] = metadata_authorities();

    let cache = METADATA_TOKENS.get_or_init(|| {
        moka::sync::Cache::builder()
            .max_capacity(4)
            .time_to_live(std::time::Duration::from_secs(
                METADATA_TOKEN_CACHE_TTL_SECONDS,
            ))
            .build()
    });

    // The moka TTL is a ceiling, not the truth — the metadata server decides the
    // real lifetime. Re-checking the advertised expiry keeps a nearly-dead token
    // out of the STS exchange. A freshly fetched short token is still used: the
    // metadata server refreshes on its own schedule and rejecting one here would
    // turn a narrow window into a hard outage.
    if let Some(cached) = cache.get(&host)
        && cached.expires_at - chrono::Utc::now()
            > chrono::Duration::seconds(METADATA_TOKEN_MIN_LIFETIME_SECONDS)
    {
        return Ok(cached.token);
    }

    // `no_proxy` is load-bearing, not tidiness: an ambient HTTP_PROXY would
    // route this request — and the service account token in its response —
    // through a third party. The metadata server is link-local and unroutable,
    // so there is never a legitimate proxy for it. HTTPS is likewise absent by
    // design; the endpoint is plain HTTP on an address only the hypervisor can
    // answer for, which is why `https_only` is not set here as it is elsewhere.
    //
    // Redirects are refused for the same reason: the only correct responder for
    // this request is the link-local address itself, so a 3xx pointing anywhere
    // else is either a misconfiguration or an attempt to feed this process a
    // token from a server it does not trust.
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(
            METADATA_CONNECT_TIMEOUT_SECONDS,
        ))
        .timeout(std::time::Duration::from_secs(
            METADATA_REQUEST_TIMEOUT_SECONDS,
        ))
        .build()
        .map_err(|error| anyhow!("failed to construct GCP metadata client: {error}"))?;

    #[derive(Deserialize)]
    struct MetadataTokenResponse {
        access_token: String,
        expires_in: i64,
    }

    let mut last_error: Option<flow_like_types::Error> = None;
    for authority in [host.as_str(), ip.as_str()] {
        // `Metadata-Flavor: Google` is the SSRF guard, not a formality: the
        // metadata server rejects any request that omits it, so a confused
        // proxy or a redirect-following client cannot be steered into reading
        // the token on an attacker's behalf. Requiring the header back on the
        // response closes the other direction — a hijacked
        // `metadata.google.internal` cannot feed this process a token it
        // controls, because a plain HTTP server does not echo the header.
        let response = client
            .get(format!("http://{}{}", authority, METADATA_TOKEN_PATH))
            .header(METADATA_FLAVOR_HEADER, METADATA_FLAVOR_VALUE)
            .send()
            .await;

        let response = match response {
            Ok(response) => response,
            Err(error) => {
                last_error = Some(anyhow!(
                    "GCP metadata token request to {authority} failed: {error}"
                ));
                continue;
            }
        };

        let status = response.status();
        let flavor_echoed = response
            .headers()
            .get(METADATA_FLAVOR_HEADER)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case(METADATA_FLAVOR_VALUE));

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            last_error = Some(anyhow!(
                "GCP metadata server at {authority} returned HTTP {status}: {body}"
            ));
            continue;
        }

        // Warned about, never enforced. The echo would be a useful signal that
        // the responder really is the metadata server, but Google documents the
        // header only as a *request* requirement: `google-auth` checks it in its
        // `ping()` environment probe and in neither `get()` nor
        // `get_service_account_token()`, object_store's own
        // `InstanceCredentialProvider` never checks it, and GKE has been
        // reported serving token responses without it. Requiring it would trade
        // a total credential outage for a narrow anti-spoofing check that the
        // platform does not promise to support.
        if !flavor_echoed {
            tracing::warn!(
                authority,
                "GCP metadata token response omitted {METADATA_FLAVOR_HEADER}: {METADATA_FLAVOR_VALUE}"
            );
        }

        // A malformed body falls through to the next authority rather than
        // aborting: the hostname answering with junk is exactly the broken-DNS
        // case the link-local address exists to survive.
        let token: MetadataTokenResponse = match response.json().await {
            Ok(token) => token,
            Err(error) => {
                last_error = Some(anyhow!(
                    "invalid GCP metadata token response from {authority}: {error}"
                ));
                continue;
            }
        };

        // Clamped because `chrono::Duration::seconds` panics on an absurd input
        // and this runs inside a request handler. Clamping low degrades to
        // "never serve from cache", which is correct rather than merely safe.
        let lifetime = chrono::Duration::seconds(token.expires_in.clamp(0, 24 * 60 * 60));
        cache.insert(
            host.clone(),
            CachedMetadataToken {
                token: token.access_token.clone(),
                expires_at: chrono::Utc::now() + lifetime,
            },
        );

        return Ok(token.access_token);
    }

    Err(last_error.unwrap_or_else(|| {
        anyhow!(
            "no GCP credentials available: the metadata server was unreachable at {host} and {ip}, \
             and no service account key is configured. On Cloud Run/GKE bind a runtime service \
             account to the workload; off-GCP set GOOGLE_APPLICATION_CREDENTIALS_JSON."
        )
    }))
}

/// Generate a short-lived OAuth2 access token using the service account key
#[cfg(feature = "gcp")]
async fn generate_access_token(service_account_key: &str, _state: &State) -> Result<String> {
    // Use reqwest directly since State's hyper client is lower-level
    generate_access_token_standalone(service_account_key).await
}

/// Standalone version for tests without State
#[cfg(feature = "gcp")]
async fn generate_access_token_standalone(service_account_key: &str) -> Result<String> {
    let jwt = create_jwt_assertion(service_account_key)?;
    let token_uri = get_token_uri(service_account_key)?;

    let client = reqwest::Client::new();
    let response = client
        .post(&token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", &jwt),
        ])
        .send()
        .await
        .map_err(|e| anyhow!("Failed to request access token: {}", e))?;

    parse_token_response(response).await
}

#[cfg(feature = "gcp")]
fn get_token_uri(service_account_key: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct ServiceAccountKey {
        token_uri: Option<String>,
    }

    let sa_key: ServiceAccountKey = flow_like_types::json::from_str(service_account_key)
        .map_err(|e| anyhow!("Failed to parse service account key: {}", e))?;

    Ok(sa_key
        .token_uri
        .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string()))
}

#[cfg(feature = "gcp")]
fn create_jwt_assertion(service_account_key: &str) -> Result<String> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    #[derive(Deserialize)]
    struct ServiceAccountKey {
        client_email: String,
        private_key: String,
        token_uri: Option<String>,
    }

    let sa_key: ServiceAccountKey = flow_like_types::json::from_str(service_account_key)
        .map_err(|e| anyhow!("Failed to parse service account key: {}", e))?;

    let token_uri = sa_key
        .token_uri
        .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string());

    let now = chrono::Utc::now().timestamp();
    let exp = now + 3600;

    let header = flow_like_types::json::json!({
        "alg": "RS256",
        "typ": "JWT"
    });

    let claims = flow_like_types::json::json!({
        "iss": sa_key.client_email,
        "sub": sa_key.client_email,
        "aud": token_uri,
        "iat": now,
        "exp": exp,
        "scope": "https://www.googleapis.com/auth/devstorage.read_write"
    });

    let header_b64 = URL_SAFE_NO_PAD.encode(header.to_string().as_bytes());
    let claims_b64 = URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
    let message = format!("{}.{}", header_b64, claims_b64);

    let signature = sign_rs256(&sa_key.private_key, message.as_bytes())?;
    let signature_b64 = URL_SAFE_NO_PAD.encode(&signature);

    Ok(format!("{}.{}", message, signature_b64))
}

#[cfg(feature = "gcp")]
async fn parse_token_response(response: reqwest::Response) -> Result<String> {
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Failed to get access token: {} - {}", status, body));
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
    }

    let token_response: TokenResponse = response
        .json()
        .await
        .map_err(|e| anyhow!("Failed to parse token response: {}", e))?;

    Ok(token_response.access_token)
}

/// Downscope an access token using Google's STS endpoint with Credential Access Boundaries.
/// This creates a new token that is restricted to the specified paths and permissions.
/// The token will only be able to access objects under the specified prefixes in the bucket.
///
/// The boundary is a whitelist, not a Cloud-Storage-only subtraction: a token
/// exchanged against a `cloud-platform` subject was verified to be refused
/// (HTTP 403) by Cloud Resource Manager, Secret Manager and Pub/Sub, and by
/// Cloud Storage operations outside the boundary, while the subject token
/// reached all of them. "Credential Access Boundaries are only available for
/// Cloud Storage" means the rules can only *name* Cloud Storage resources — not
/// that other APIs stay reachable. So it does not matter whether the subject
/// token came from a scoped key assertion or the metadata server's full-scope
/// workload token; do not add a `scope` parameter to narrow it, because STS
/// rejects the exchange outright with `invalid_request` when a scope accompanies
/// an access boundary.
#[cfg(feature = "gcp")]
async fn downscope_token(
    access_token: &str,
    bucket: &str,
    allowed_prefixes: &[String],
    write_access: bool,
) -> Result<String> {
    downscope_token_for_rules(
        access_token,
        &[GcpAccessRule::new(
            bucket.to_string(),
            allowed_prefixes.to_vec(),
            write_access,
        )],
    )
    .await
}

#[cfg(feature = "gcp")]
struct GcpAccessRule {
    bucket: String,
    prefixes: Vec<String>,
    access: GcpAccess,
}

/// What a Credential Access Boundary rule may do under its prefixes.
///
/// `Append` is create + read: GCS requires `storage.objects.delete` to
/// overwrite an existing object, so `objectCreator` alone can neither
/// overwrite nor delete — the shape Lance needs for run logs (fresh data
/// files, `.txn` files and conditional-put manifests, never a delete).
#[cfg(feature = "gcp")]
#[derive(Clone, Copy)]
enum GcpAccess {
    Read,
    Append,
    Write,
}

#[cfg(feature = "gcp")]
impl GcpAccess {
    fn roles(self) -> &'static [&'static str] {
        match self {
            GcpAccess::Read => &["inRole:roles/storage.objectViewer"],
            GcpAccess::Append => &[
                "inRole:roles/storage.objectCreator",
                "inRole:roles/storage.objectViewer",
            ],
            GcpAccess::Write => &["inRole:roles/storage.objectAdmin"],
        }
    }
}

#[cfg(feature = "gcp")]
impl GcpAccessRule {
    fn new(bucket: String, prefixes: Vec<String>, write_access: bool) -> Self {
        Self::with_access(
            bucket,
            prefixes,
            if write_access {
                GcpAccess::Write
            } else {
                GcpAccess::Read
            },
        )
    }

    fn with_access(bucket: String, prefixes: Vec<String>, access: GcpAccess) -> Self {
        // Every prefix names a directory. Without the trailing slash,
        // `startsWith('…/apps/app-1')` also matches `apps/app-10`.
        let prefixes = prefixes
            .into_iter()
            .map(|prefix| format!("{}/", prefix.trim_end_matches('/')))
            .collect();
        Self {
            bucket,
            prefixes,
            access,
        }
    }
}

/// Read-only meta rule for the executor: version artifacts and boards under
/// `apps/{app_id}/`, plus draft artifacts under `tmp/apps/{app_id}/` — the
/// drafts are written only by the API (the trust anchor for
/// `entry_authority_revision`) and reclaimed by lifecycle rules on `tmp/`.
#[cfg(feature = "gcp")]
fn server_execute_meta_rule(bucket: &str, apps_prefix: &str) -> GcpAccessRule {
    GcpAccessRule::new(
        bucket.to_string(),
        vec![apps_prefix.to_string(), format!("tmp/{apps_prefix}")],
        false,
    )
}

#[cfg(feature = "gcp")]
fn access_boundary_condition(rule: &GcpAccessRule) -> String {
    rule.prefixes
        .iter()
        .flat_map(|prefix| {
            [
                format!(
                    "resource.name.startsWith('projects/_/buckets/{}/objects/{}')",
                    rule.bucket, prefix
                ),
                format!(
                    "api.getAttribute('storage.googleapis.com/objectListPrefix', '').startsWith('{}')",
                    prefix
                ),
            ]
        })
        .collect::<Vec<_>>()
        .join(" || ")
}

#[cfg(feature = "gcp")]
async fn downscope_token_for_rules(access_token: &str, rules: &[GcpAccessRule]) -> Result<String> {
    use serde_json::json;

    let access_boundary_rules: Vec<serde_json::Value> = rules
        .iter()
        .map(|rule| {
            let permission_roles = rule.access.roles();

            json!({
                "availablePermissions": permission_roles,
                "availableResource": format!("//storage.googleapis.com/projects/_/buckets/{}", rule.bucket),
                "availabilityCondition": {
                    "expression": access_boundary_condition(rule)
                }
            })
        })
        .collect();

    let cab = json!({
        "accessBoundary": {
            "accessBoundaryRules": access_boundary_rules
        }
    });

    let cab_json = cab.to_string();

    let form = [
        (
            "grant_type",
            "urn:ietf:params:oauth:grant-type:token-exchange",
        ),
        (
            "subject_token_type",
            "urn:ietf:params:oauth:token-type:access_token",
        ),
        ("subject_token", access_token),
        (
            "requested_token_type",
            "urn:ietf:params:oauth:token-type:access_token",
        ),
        ("options", cab_json.as_str()),
    ];

    let client = reqwest::Client::new();
    let response = client
        .post("https://sts.googleapis.com/v1/token")
        .form(&form)
        .send()
        .await
        .map_err(|e| anyhow!("STS token exchange request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("STS token exchange failed: {} - {}", status, body));
    }

    #[derive(Deserialize)]
    struct StsResponse {
        access_token: String,
    }

    let sts_response: StsResponse = response
        .json()
        .await
        .map_err(|e| anyhow!("Failed to parse STS response: {}", e))?;

    Ok(sts_response.access_token)
}

/// Sign data with RS256 (RSA-SHA256)
#[cfg(feature = "gcp")]
fn sign_rs256(private_key_pem: &str, data: &[u8]) -> Result<Vec<u8>> {
    use rsa::{
        RsaPrivateKey, pkcs1v15::SigningKey, pkcs8::DecodePrivateKey, signature::SignatureEncoding,
        signature::Signer,
    };

    let private_key = RsaPrivateKey::from_pkcs8_pem(private_key_pem)
        .map_err(|e| anyhow!("Failed to parse private key: {}", e))?;

    let signing_key = SigningKey::<sha2::Sha256>::new(private_key);
    let signature = signing_key.sign(data);

    Ok(signature.to_bytes().to_vec())
}

#[cfg(feature = "gcp")]
#[async_trait]
impl RuntimeCredentialsTrait for GcpRuntimeCredentials {
    fn into_shared_credentials(&self) -> SharedCredentials {
        SharedCredentials::Gcp(GcpSharedCredentials {
            service_account_key: self.service_account_key.clone().unwrap_or_default(),
            access_token: self.access_token.clone(),
            meta_bucket: self.meta_bucket.clone(),
            content_bucket: self.content_bucket.clone(),
            logs_bucket: self.logs_bucket.clone(),
            allowed_prefixes: self.allowed_prefixes.clone(),
            write_access: self.write_access,
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
        name = "GcpRuntimeCredentials::to_state",
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

        let bucket = self.content_bucket.clone();

        // Narrowest credential first, matching `GcpSharedCredentials`. Branching
        // on the key first — as this did — let a present-but-blank key win over a
        // real downscoped token in the same struct, discarding the only
        // restriction the credential carried.
        let scoped_token = self
            .access_token
            .as_ref()
            .filter(|token| !token.trim().is_empty());
        let master_key = self
            .service_account_key
            .as_ref()
            .filter(|key| !key.trim().is_empty());

        if let Some(access_token) = scoped_token {
            config.register_build_logs_database(Arc::new(make_gcs_builder_with_token(
                bucket.clone(),
                access_token.clone(),
            )));
            config.register_build_project_database(Arc::new(make_gcs_builder_with_token(
                bucket.clone(),
                access_token.clone(),
            )));
            config.register_build_user_database(Arc::new(make_gcs_builder_with_token(
                bucket,
                access_token.clone(),
            )));
        } else if let Some(service_account_key) = master_key {
            config.register_build_logs_database(Arc::new(make_gcs_builder_with_key(
                bucket.clone(),
                service_account_key.clone(),
            )));
            config.register_build_project_database(Arc::new(make_gcs_builder_with_key(
                bucket.clone(),
                service_account_key.clone(),
            )));
            config.register_build_user_database(Arc::new(make_gcs_builder_with_key(
                bucket,
                service_account_key.clone(),
            )));
        } else {
            // Keyless master credentials: no key JSON and no downscoped token is
            // the normal shape on Cloud Run, so erroring here made the GCP
            // master path unusable under Workload Identity for the same reason
            // `scoped_credentials` was. Registering the builders without any
            // storage option lets object_store walk its own credential chain
            // down to the metadata server — the same identity
            // `fetch_metadata_token` uses, so the two paths cannot disagree.
            //
            // Master credentials only: a scoped credential that lands here has
            // lost its token, and the runtime identity it would pick up instead
            // enforces none of the prefixes it was scoped to.
            self.ensure_keyless_allowed()?;
            config.register_build_logs_database(Arc::new(make_gcs_builder_adc(bucket.clone())));
            config.register_build_project_database(Arc::new(make_gcs_builder_adc(bucket.clone())));
            config.register_build_user_database(Arc::new(make_gcs_builder_adc(bucket)));
        }

        let mut flow_like_state = FlowLikeState::new(config, http_client);

        flow_like_state.model_provider_config = state.provider.clone();
        flow_like_state.node_registry.write().await.node_registry = state.registry.clone();

        Ok(flow_like_state)
    }
}

/// Build a LanceDB connection factory over a GCS bucket.
///
/// `credential` is `None` only on the deliberate keyless path. A blank
/// credential is never normalised to `None` here: object_store rejects a
/// present-but-blank key, and that rejection is the outcome we want. Dropping
/// the option instead would resolve ADC and quietly substitute the unrestricted
/// runtime identity for the credential that was supposed to constrain the
/// caller.
#[cfg(feature = "gcp")]
fn make_gcs_builder(
    bucket: String,
    credential: Option<(&'static str, String)>,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder {
    move |path| {
        let url = format!("gs://{}/{}", bucket, path);
        let builder = connect(&url);
        match &credential {
            Some((option, value)) => builder.storage_option(option.to_string(), value.clone()),
            None => builder,
        }
    }
}

#[cfg(feature = "gcp")]
fn make_gcs_builder_with_key(
    bucket: String,
    service_account_key: String,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder {
    make_gcs_builder(
        bucket,
        Some((GCS_SERVICE_ACCOUNT_KEY_OPTION, service_account_key)),
    )
}

#[cfg(feature = "gcp")]
fn make_gcs_builder_with_token(
    bucket: String,
    access_token: String,
) -> impl Fn(object_store::path::Path) -> ConnectBuilder {
    make_gcs_builder(bucket, Some((GCS_STORAGE_TOKEN_OPTION, access_token)))
}

/// Application Default Credentials: no explicit credential, so object_store
/// resolves service-account key -> ADC file -> `InstanceCredentialProvider` on
/// the metadata server. This is the keyless equivalent of the two builders
/// above.
#[cfg(feature = "gcp")]
fn make_gcs_builder_adc(bucket: String) -> impl Fn(object_store::path::Path) -> ConnectBuilder {
    make_gcs_builder(bucket, None)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(all(test, feature = "gcp"))]
mod tests {
    use super::*;
    use crate::credentials::CredentialsAccess;
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

    #[tokio::test]
    #[ignore]
    async fn test_gcp_master_credentials_setup() {
        init_env();
        let creds = GcpRuntimeCredentials::from_env();
        assert!(
            creds.service_account_key.is_some(),
            "GOOGLE_APPLICATION_CREDENTIALS_JSON must be set"
        );
        assert!(!creds.meta_bucket.is_empty(), "GCP_META_BUCKET must be set");
        assert!(
            !creds.content_bucket.is_empty(),
            "GCP_CONTENT_BUCKET must be set"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_master_credentials_can_write() {
        init_env();
        let creds = GcpRuntimeCredentials::from_env().master_credentials().await;
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
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                s.put(&path, b"test content".to_vec().into())
                    .await
                    .expect("Master credentials should be able to write");
                s.delete(&path).await.ok();
            }
            _ => panic!("Expected GCP store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_master_credentials_can_read() {
        init_env();
        let creds = GcpRuntimeCredentials::from_env().master_credentials().await;
        let shared = creds.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from master credentials");

        let test_path = format!("test/master-read-test-{}.txt", flow_like_types::create_id());
        let path = Path::from(test_path.as_str());
        let content = b"read test content";

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                s.put(&path, content.to_vec().into())
                    .await
                    .expect("Setup: write should succeed");

                let result = s.get(&path).await.expect("Read should succeed");
                let bytes = result.bytes().await.expect("Should get bytes");
                assert_eq!(bytes.as_ref(), content);

                s.delete(&path).await.ok();
            }
            _ => panic!("Expected GCP store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_scoped_credentials_can_write_in_scope() {
        init_env();
        let master = GcpRuntimeCredentials::from_env().master_credentials().await;

        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::EditApp)
            .await
            .expect("Failed to generate scoped credentials");

        // Verify service account key is NOT included
        assert!(
            scoped.service_account_key.is_none(),
            "Scoped credentials should not include service account key"
        );
        assert!(
            scoped.access_token.is_some(),
            "Scoped credentials should include access token"
        );

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
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                s.put(&path, b"scoped test content".to_vec().into())
                    .await
                    .expect("Scoped credentials should be able to write in allowed path");
                s.delete(&path).await.ok();
            }
            _ => panic!("Expected GCP store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_scoped_credentials_invoke_write() {
        init_env();
        let master = GcpRuntimeCredentials::from_env().master_credentials().await;

        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::InvokeWrite)
            .await
            .expect("Failed to generate scoped credentials");

        assert!(scoped.write_access);
        assert_eq!(scoped.allowed_prefixes.len(), 5);

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
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                s.put(&path, b"invoke write test".to_vec().into())
                    .await
                    .expect("InvokeWrite credentials should be able to write");

                let result = s.get(&path).await.expect("Should be able to read");
                let bytes = result.bytes().await.expect("Should get bytes");
                assert_eq!(bytes.as_ref(), b"invoke write test");

                s.delete(&path).await.ok();
            }
            _ => panic!("Expected GCP store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_scoped_credentials_can_read_in_scope() {
        init_env();
        let master = GcpRuntimeCredentials::from_env().master_credentials().await;

        // First, write test data with master credentials
        let master_shared = master.clone().into_shared_credentials();
        let master_store = master_shared
            .to_store(false)
            .await
            .expect("Failed to create master store");

        let test_path = format!(
            "apps/{}/read-test-{}.txt",
            TEST_APP_ID,
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());
        let content = b"scoped read test content";

        match &master_store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                s.put(&path, content.to_vec().into())
                    .await
                    .expect("Setup: write should succeed");
            }
            _ => panic!("Expected GCP store"),
        }

        // Now read with scoped credentials
        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::ReadApp)
            .await
            .expect("Failed to generate scoped credentials");

        let shared = scoped.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from scoped credentials");

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                let result = s
                    .get(&path)
                    .await
                    .expect("Scoped credentials should be able to read in allowed path");
                let bytes = result.bytes().await.expect("Should get bytes");
                assert_eq!(bytes.as_ref(), content);
            }
            _ => panic!("Expected GCP store"),
        }

        // Cleanup with master
        if let flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) = &master_store
        {
            s.delete(&path).await.ok();
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_scoped_credentials_cannot_write_outside_scope() {
        init_env();
        let master = GcpRuntimeCredentials::from_env().master_credentials().await;

        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::InvokeWrite)
            .await
            .expect("Failed to generate scoped credentials");

        let shared = scoped.into_shared_credentials();
        let store = shared
            .to_store(false)
            .await
            .expect("Failed to create store from scoped credentials");

        // Try to write to a path outside the allowed prefixes (different user)
        let test_path = format!(
            "users/different-user/apps/{}/unauthorized-{}.txt",
            TEST_APP_ID,
            flow_like_types::create_id()
        );
        let path = Path::from(test_path.as_str());

        match &store {
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                let result = s.put(&path, b"should fail".to_vec().into()).await;

                if result.is_ok() {
                    s.delete(&path).await.ok();
                }

                assert!(
                    result.is_err(),
                    "Downscoped GCP credentials must not write outside their allowed prefixes"
                );
            }
            _ => panic!("Expected GCP store"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn test_gcp_scoped_credentials_read_only_cannot_write() {
        init_env();
        let master = GcpRuntimeCredentials::from_env().master_credentials().await;

        let scoped = master
            .scoped_credentials_for_test(TEST_SUB, TEST_APP_ID, CredentialsAccess::ReadApp)
            .await
            .expect("Failed to generate scoped credentials");

        // Verify write_access is false
        assert!(
            !scoped.write_access,
            "ReadApp credentials should have write_access=false"
        );

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
            flow_like::flow_like_storage::files::store::FlowLikeStore::Google(s) => {
                let result = s.put(&path, b"should fail".to_vec().into()).await;

                if result.is_ok() {
                    s.delete(&path).await.ok();
                }

                assert!(
                    result.is_err(),
                    "Read-only downscoped GCP credentials must not allow writes"
                );
            }
            _ => panic!("Expected GCP store"),
        }
    }

    #[test]
    fn test_gcp_runtime_credentials_serialization() {
        let creds = GcpRuntimeCredentials {
            service_account_key: Some(r#"{"type":"service_account"}"#.to_string()),
            access_token: Some("ya29.test-token".to_string()),
            meta_bucket: "meta".to_string(),
            content_bucket: "content".to_string(),
            logs_bucket: "logs".to_string(),
            allowed_prefixes: vec!["apps/test-app".to_string()],
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        };

        let json = to_string(&creds).expect("Failed to serialize");
        let deserialized: GcpRuntimeCredentials = from_str(&json).expect("Failed to deserialize");

        assert_eq!(creds.access_token, deserialized.access_token);
        assert_eq!(creds.meta_bucket, deserialized.meta_bucket);
        assert_eq!(creds.allowed_prefixes, deserialized.allowed_prefixes);
        assert_eq!(creds.write_access, deserialized.write_access);
    }

    #[test]
    fn test_gcp_scoped_credentials_do_not_include_service_account_key() {
        let creds = GcpRuntimeCredentials {
            service_account_key: None,
            access_token: Some("ya29.scoped-token".to_string()),
            meta_bucket: "meta".to_string(),
            content_bucket: "content".to_string(),
            logs_bucket: "logs".to_string(),
            allowed_prefixes: vec!["apps/test-app".to_string()],
            write_access: false,
            expiration: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
            content_path_prefix: None,
            user_content_path_prefix: None,
        };

        let json = to_string(&creds).expect("Failed to serialize");

        assert!(
            !json.contains("service_account") || json.contains("null"),
            "Scoped credentials should not expose service account key"
        );
        assert!(
            json.contains("ya29.scoped-token"),
            "Scoped credentials should include access token"
        );
    }

    #[test]
    fn test_gcp_server_execute_can_read_app_scoped_draft_artifacts() {
        let app_prefix = format!("apps/{TEST_APP_ID}");
        let rule = server_execute_meta_rule("meta", &app_prefix);
        let draft_artifact = flow_like::flow::compiled::draft_artifact_path(
            TEST_APP_ID,
            "board-1",
            "etag",
            &[7; 32],
        )
        .to_string();
        let other_app_artifact = flow_like::flow::compiled::draft_artifact_path(
            &format!("{TEST_APP_ID}-other"),
            "board-1",
            "etag",
            &[7; 32],
        )
        .to_string();

        assert_eq!(rule.bucket, "meta");
        assert_eq!(
            rule.prefixes,
            vec![format!("{app_prefix}/"), format!("tmp/{app_prefix}/")]
        );
        assert_eq!(rule.access.roles(), &["inRole:roles/storage.objectViewer"]);
        let condition = access_boundary_condition(&rule);
        assert!(condition.contains("resource.name.startsWith"));
        assert!(condition.contains("storage.googleapis.com/objectListPrefix"));
        assert!(condition.contains(&format!("objects/{app_prefix}/")));
        assert!(condition.contains(&format!("objects/tmp/{app_prefix}/")));
        assert!(draft_artifact.starts_with(&format!("tmp/{app_prefix}/")));
        assert!(!other_app_artifact.starts_with(&format!("tmp/{app_prefix}/")));
    }

    /// `startsWith` on an unanchored directory prefix leaks: `apps/app-1`
    /// matches `apps/app-10`. Every rule goes through `with_access`, which
    /// anchors each prefix with a trailing slash.
    #[test]
    fn test_gcp_access_rule_prefixes_never_match_a_sibling_app() {
        let rule = GcpAccessRule::new(
            "content".to_string(),
            vec![
                "apps/app-1".to_string(),
                "users/user-1/apps/app-1".to_string(),
                "tmp/global/apps/app-1".to_string(),
                "runs/app-1".to_string(),
            ],
            true,
        );

        assert!(
            "apps/app-10/board.board".starts_with("apps/app-1"),
            "raw prefix would leak into app-10"
        );
        for prefix in &rule.prefixes {
            assert!(prefix.ends_with('/'), "prefix must be anchored: {prefix}");
            assert!(!"apps/app-10/board.board".starts_with(prefix.as_str()));
            assert!(!"runs/app-10/log.lance".starts_with(prefix.as_str()));
        }
        let condition = access_boundary_condition(&rule);
        assert!(condition.contains("objects/apps/app-1/'"));
        assert!(condition.contains("startsWith('runs/app-1/')"));
    }

    #[test]
    fn test_credentials_access_modes() {
        let apps_prefix = format!("apps/{}", TEST_APP_ID);
        let user_prefix = format!("users/{}/apps/{}", TEST_SUB, TEST_APP_ID);
        let log_prefix = format!("runs/{}", TEST_APP_ID);
        let tmp_user_prefix = format!("tmp/user/{}/apps/{}", TEST_SUB, TEST_APP_ID);
        let tmp_global_prefix = format!("tmp/global/apps/{}", TEST_APP_ID);

        let creds = GcpRuntimeCredentials {
            service_account_key: None,
            access_token: Some("token".to_string()),
            meta_bucket: "meta".to_string(),
            content_bucket: "content".to_string(),
            logs_bucket: "logs".to_string(),
            allowed_prefixes: vec![apps_prefix.clone()],
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        };
        assert!(creds.write_access);
        assert_eq!(creds.allowed_prefixes, vec![apps_prefix.clone()]);

        let creds = GcpRuntimeCredentials {
            service_account_key: None,
            access_token: Some("token".to_string()),
            meta_bucket: "meta".to_string(),
            content_bucket: "content".to_string(),
            logs_bucket: "logs".to_string(),
            allowed_prefixes: vec![apps_prefix.clone()],
            write_access: false,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        };
        assert!(!creds.write_access);

        let creds = GcpRuntimeCredentials {
            service_account_key: None,
            access_token: Some("token".to_string()),
            meta_bucket: "meta".to_string(),
            content_bucket: "content".to_string(),
            logs_bucket: "logs".to_string(),
            allowed_prefixes: vec![
                apps_prefix.clone(),
                user_prefix.clone(),
                tmp_user_prefix.clone(),
                tmp_global_prefix.clone(),
                log_prefix.clone(),
            ],
            write_access: true,
            expiration: None,
            content_path_prefix: None,
            user_content_path_prefix: None,
        };
        assert!(creds.write_access);
        assert_eq!(creds.allowed_prefixes.len(), 5);
    }

    #[test]
    fn test_gcp_invoke_none_does_not_advertise_app_content_prefix() {
        let (app, user) = scoped_content_path_prefixes(
            &format!("apps/{}", TEST_APP_ID),
            &format!("users/{}/apps/{}", TEST_SUB, TEST_APP_ID),
            &CredentialsAccess::InvokeNone,
        );

        assert_eq!(app, None);
        assert_eq!(
            user,
            Some(format!("users/{}/apps/{}", TEST_SUB, TEST_APP_ID))
        );
    }

    #[test]
    fn test_gcp_app_content_modes_advertise_app_content_prefix() {
        for mode in [
            CredentialsAccess::InvokeRead,
            CredentialsAccess::InvokeWrite,
            CredentialsAccess::ServerExecute,
        ] {
            let (app, user) = scoped_content_path_prefixes(
                &format!("apps/{}", TEST_APP_ID),
                &format!("users/{}/apps/{}", TEST_SUB, TEST_APP_ID),
                &mode,
            );

            assert_eq!(app, Some(format!("apps/{}", TEST_APP_ID)));
            assert_eq!(
                user,
                Some(format!("users/{}/apps/{}", TEST_SUB, TEST_APP_ID))
            );
        }
    }
}

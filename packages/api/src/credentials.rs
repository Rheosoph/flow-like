use std::fmt::Display;

#[cfg(feature = "aws")]
use aws_credentials::AwsRuntimeCredentials;
#[cfg(feature = "azure")]
use azure_credentials::AzureRuntimeCredentials;
use flow_like::credentials::SharedCredentials;
use flow_like::flow::execution::ExecutionEnvironment;
use flow_like::flow_like_storage::files::store::FlowLikeStore;
use flow_like::state::FlowLikeState;
use flow_like_storage::lancedb::connection::ConnectBuilder;
use flow_like_types::Result;
use flow_like_types::async_trait;
#[cfg(feature = "gcp")]
use gcp_credentials::GcpRuntimeCredentials;
#[cfg(feature = "r2")]
use r2_credentials::R2RuntimeCredentials;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::state::AppState;
use crate::state::State;

#[cfg(feature = "aws")]
pub mod aws_credentials;
#[cfg(feature = "azure")]
pub mod azure_credentials;
#[cfg(feature = "gcp")]
pub mod gcp_credentials;
pub mod local_credentials;
pub mod mixed_credentials;
#[cfg(feature = "r2")]
pub mod r2_credentials;

/// Validates that a path component (sub, app_id) does not contain path traversal
/// or injection characters. Allows alphanumeric, hyphens, underscores, and dots.
pub fn validate_path_component(value: &str, name: &str) -> Result<()> {
    if value.contains("..")
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
        || value.contains('*')
    {
        return Err(flow_like_types::anyhow!(
            "Invalid {}: contains forbidden characters (path traversal or wildcards)",
            name
        ));
    }
    if !value
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '|' || c == ':')
    {
        return Err(flow_like_types::anyhow!(
            "Invalid {}: contains forbidden characters",
            name
        ));
    }
    Ok(())
}

/// Longest segment this function will ever emit.
const MAX_STORAGE_PATH_SEGMENT_CHARS: usize = 80;
/// Hex characters of the disambiguating digest appended to a lossy segment.
/// Twelve hex characters is 48 bits, which is far more than the number of
/// distinct subjects any deployment holds and short enough to stay readable.
const STORAGE_PATH_SEGMENT_DIGEST_CHARS: usize = 12;

/// Collapses an identifier into a single storage path segment.
///
/// The scratch directory `tmp/user/{sub}/apps/{app_id}` has five consumers that
/// must agree character for character, or the credential covers a directory
/// nobody writes to: the `/tmp` presign route, the HTTP-sink request offload,
/// the Azure directory SAS issuer, the AWS session policy and the GCP credential
/// access boundary. They share this function rather than each carrying a copy.
///
/// A segment that survives sanitisation unchanged is returned verbatim, which
/// keeps opaque IDs (`app_id`, and any subject the IdP already emits in this
/// alphabet) readable and stable. A segment that had to be rewritten — because
/// `validate_path_component` deliberately admits `|` and `:`, because it was
/// longer than the ceiling, or because it trimmed to nothing — carries a digest
/// of the *original* value. Without it `auth0|123`, `auth0:123` and `auth0_123`
/// would all collapse onto one directory, and a credential scoped to that
/// directory would reach another subject's scratch space.
pub fn storage_path_segment(value: &str, fallback: &str) -> String {
    let mut sanitized = String::with_capacity(value.len().min(MAX_STORAGE_PATH_SEGMENT_CHARS));
    let mut lossy = value.chars().count() > MAX_STORAGE_PATH_SEGMENT_CHARS;
    for ch in value.chars().take(MAX_STORAGE_PATH_SEGMENT_CHARS) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
            lossy = true;
        }
    }

    let trimmed = sanitized.trim_matches(|ch| ch == '.' || ch == '_');
    lossy |= trimmed.len() != sanitized.len();

    if !lossy {
        return if trimmed.is_empty() {
            fallback.to_string()
        } else {
            trimmed.to_string()
        };
    }

    let digest = blake3::hash(value.as_bytes()).to_hex();
    let digest = &digest[..STORAGE_PATH_SEGMENT_DIGEST_CHARS];
    let base = if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    };
    let base: String = base
        .chars()
        .take(MAX_STORAGE_PATH_SEGMENT_CHARS - STORAGE_PATH_SEGMENT_DIGEST_CHARS - 1)
        .collect();
    let base = base.trim_end_matches(|ch| ch == '.' || ch == '_');
    let base = if base.is_empty() { fallback } else { base };
    format!("{base}-{digest}")
}

/// The scratch prefixes every scoped credential must authorise, built from the
/// same segments the writers use. Returned as a pair so no caller can sanitise
/// one and forget the other.
pub fn temporary_prefixes(sub: &str, app_id: &str) -> (String, String) {
    let app_segment = storage_path_segment(app_id, "app");
    (
        format!(
            "tmp/user/{}/apps/{}",
            storage_path_segment(sub, "user"),
            app_segment
        ),
        format!("tmp/global/apps/{}", app_segment),
    )
}

#[async_trait]
pub trait RuntimeCredentialsTrait {
    async fn to_state(&self, state: AppState) -> Result<FlowLikeState>;
    async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder>;
    async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder>;
    fn into_shared_credentials(&self) -> SharedCredentials;
}

#[derive(Clone, Debug)]
pub enum CredentialsAccess {
    EditApp,
    ReadApp,
    /// Read-only access scoped to the **content** bucket of an app
    /// only (`apps/{app_id}/...` on the content store). Used by the
    /// fork-an-app flow: the desktop pulls user content (metadata/,
    /// upload/, storage/) directly via these credentials, while
    /// boards/events/widgets/templates/pages on the meta bucket
    /// always come back via API responses (after server-side
    /// secret-stripping). Granting `ReadApp` instead would let a
    /// misbehaving client GET raw `*.board` / `*.event` files
    /// containing source-app secrets.
    ReadAppContent,
    /// Read+write access scoped to the **content** bucket of an app
    /// only. Used wherever scoped credentials cross the trust
    /// boundary into a client (presign-data-access for uploads,
    /// fork-online-begin for desktop bundle uploads). Boards /
    /// events / widgets / templates / pages live in the meta bucket
    /// and *must* be written via the API so that authorization
    /// (RolePermissions::Write*) and validation (event schedule
    /// checks, page-event coupling, sink registration, etc.) run on
    /// every change. Granting `EditApp` to the client would let it
    /// drop arbitrary `.board` / `.event` files server-side,
    /// bypassing every guard.
    EditAppContent,
    /// Read-only access scoped to the project LanceDB at
    /// `apps/{app_id}/storage/db` on the **content** bucket only.
    /// Exists for app-to-app shared database access: a connected app
    /// may query the shared database directly, but must not see the
    /// rest of the app's content (e.g. `apps/{app_id}/upload/*`),
    /// which `ReadAppContent` would expose.
    ReadAppDb,
    /// Read+write access scoped to the project LanceDB at
    /// `apps/{app_id}/storage/db` on the **content** bucket only.
    /// Exists for app-to-app shared database access: a connected app
    /// may write to the shared database directly, but must not touch
    /// the rest of the app's content (e.g. `apps/{app_id}/upload/*`),
    /// which `EditAppContent` would expose.
    EditAppDb,
    EditUser,
    ReadUser,
    InvokeNone,
    InvokeRead,
    InvokeWrite,
    /// Server-side execution credentials. These are only sent to trusted
    /// executors, not returned by the client-facing invoke presign route.
    /// They include app content read/write for workflow storage and read-only
    /// app metadata so the executor can load the board/event definition.
    ServerExecute,
    ReadLogs,
}

impl Display for CredentialsAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CredentialsAccess::EditApp => write!(f, "edit_app"),
            CredentialsAccess::ReadApp => write!(f, "read_app"),
            CredentialsAccess::ReadAppContent => write!(f, "read_app_content"),
            CredentialsAccess::EditAppContent => write!(f, "edit_app_content"),
            CredentialsAccess::ReadAppDb => write!(f, "read_app_db"),
            CredentialsAccess::EditAppDb => write!(f, "edit_app_db"),
            CredentialsAccess::EditUser => write!(f, "edit_user"),
            CredentialsAccess::ReadUser => write!(f, "read_user"),
            CredentialsAccess::InvokeNone => write!(f, "invoke_none"),
            CredentialsAccess::InvokeRead => write!(f, "invoke_read"),
            CredentialsAccess::InvokeWrite => write!(f, "invoke_write"),
            CredentialsAccess::ServerExecute => write!(f, "server_execute"),
            CredentialsAccess::ReadLogs => write!(f, "read_logs"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RuntimeCredentials {
    #[cfg(feature = "aws")]
    Aws(AwsRuntimeCredentials),
    #[cfg(feature = "azure")]
    Azure(AzureRuntimeCredentials),
    #[cfg(feature = "gcp")]
    Gcp(GcpRuntimeCredentials),
    #[cfg(feature = "r2")]
    R2(R2RuntimeCredentials),
    Mixed(mixed_credentials::MixedRuntimeCredentials),
}

impl RuntimeCredentials {
    fn configured_provider_name() -> String {
        std::env::var("RUNTIME_CREDENTIALS_PROVIDER")
            .or_else(|_| std::env::var("STORAGE_PROVIDER"))
            .unwrap_or_else(|_| mixed_credentials::default_provider_name().to_string())
    }

    /// Whether the **content** store behind this credential is Azure Blob.
    ///
    /// Callers use this to decide whether a credential can mint signed URLs at
    /// all: an Azure SAS cannot sign a new URL, so those routes fall back to
    /// master credentials. A mixed deployment has to unwrap to its content
    /// credential to answer that — reporting `false` for a mixed set whose
    /// content bucket is Azure sends the caller down the scoped branch, where
    /// every signature then fails.
    pub fn is_azure(&self) -> bool {
        match self {
            #[cfg(feature = "azure")]
            RuntimeCredentials::Azure(_) => true,
            RuntimeCredentials::Mixed(mixed) => mixed.content.is_azure(),
            #[allow(unreachable_patterns)]
            _ => false,
        }
    }

    pub async fn scoped(
        sub: &str,
        app_id: &str,
        state: &State,
        mode: CredentialsAccess,
    ) -> Result<Self> {
        // Check for mixed-provider configuration first (runtime detection).
        // When per-bucket providers differ, scope each independently.
        if let Some(mixed) = mixed_credentials::MixedRuntimeCredentials::detect_from_env() {
            return Ok(RuntimeCredentials::Mixed(
                mixed.scoped_credentials(sub, app_id, state, mode).await?,
            ));
        }

        let provider = Self::configured_provider_name();

        #[cfg(feature = "r2")]
        if provider.eq_ignore_ascii_case("r2") && matches!(mode, CredentialsAccess::ServerExecute) {
            return Ok(RuntimeCredentials::Mixed(
                R2RuntimeCredentials::from_env()
                    .scoped_server_execute_credentials(sub, app_id)
                    .await?,
            ));
        }

        let credentials = mixed_credentials::provider_to_runtime_credentials(&provider)?;
        mixed_credentials::scope_inner(&credentials, sub, app_id, state, mode).await
    }

    pub async fn master_credentials() -> Result<Self> {
        // Check for mixed-provider configuration first.
        if let Some(mixed) = mixed_credentials::MixedRuntimeCredentials::detect_from_env() {
            return Ok(RuntimeCredentials::Mixed(mixed.master_credentials().await?));
        }

        let provider = Self::configured_provider_name();
        let credentials = mixed_credentials::provider_to_runtime_credentials(&provider)?;
        mixed_credentials::master_inner(&credentials).await
    }

    pub async fn to_store(&self, meta: bool) -> Result<FlowLikeStore> {
        self.to_store_type(if meta {
            flow_like::credentials::StoreType::Meta
        } else {
            flow_like::credentials::StoreType::Content
        })
        .await
    }

    pub async fn to_store_type(
        &self,
        store_type: flow_like::credentials::StoreType,
    ) -> Result<FlowLikeStore> {
        self.into_shared_credentials()
            .to_store_type(store_type)
            .await
    }

    pub async fn to_db(&self, app_id: &str) -> Result<ConnectBuilder> {
        match self {
            #[cfg(feature = "aws")]
            RuntimeCredentials::Aws(aws) => aws.to_db(app_id).await,
            #[cfg(feature = "azure")]
            RuntimeCredentials::Azure(azure) => azure.to_db(app_id).await,
            #[cfg(feature = "gcp")]
            RuntimeCredentials::Gcp(gcp) => gcp.to_db(app_id).await,
            #[cfg(feature = "r2")]
            RuntimeCredentials::R2(r2) => r2.to_db(app_id).await,
            RuntimeCredentials::Mixed(mixed) => mixed.to_db(app_id).await,
        }
    }

    pub async fn to_db_scoped(&self, sub: &str, app_id: &str) -> Result<ConnectBuilder> {
        match self {
            #[cfg(feature = "aws")]
            RuntimeCredentials::Aws(aws) => aws.to_db_scoped(sub, app_id).await,
            #[cfg(feature = "azure")]
            RuntimeCredentials::Azure(azure) => azure.to_db_scoped(sub, app_id).await,
            #[cfg(feature = "gcp")]
            RuntimeCredentials::Gcp(gcp) => gcp.to_db_scoped(sub, app_id).await,
            #[cfg(feature = "r2")]
            RuntimeCredentials::R2(r2) => r2.to_db_scoped(sub, app_id).await,
            RuntimeCredentials::Mixed(mixed) => mixed.to_db_scoped(sub, app_id).await,
        }
    }

    #[instrument(skip(self, state), level = "debug")]
    pub async fn to_state(&self, state: AppState) -> Result<FlowLikeState> {
        let package_widget_source = state.wasm_registry.clone();
        let mut flow_state = match self {
            #[cfg(feature = "aws")]
            RuntimeCredentials::Aws(aws) => aws.to_state(state).await,
            #[cfg(feature = "azure")]
            RuntimeCredentials::Azure(azure) => azure.to_state(state).await,
            #[cfg(feature = "gcp")]
            RuntimeCredentials::Gcp(gcp) => gcp.to_state(state).await,
            #[cfg(feature = "r2")]
            RuntimeCredentials::R2(r2) => r2.to_state(state).await,
            RuntimeCredentials::Mixed(mixed) => mixed.to_state(state).await,
        }?;

        // The API is a shared server: anything built on this state must not
        // fall back to the process's own cloud identity.
        flow_state.execution_environment = ExecutionEnvironment::Server;

        if let Some(package_widget_source) = package_widget_source {
            flow_state
                .register_package_widget_source(package_widget_source)
                .await;
        }

        Ok(flow_state)
    }

    #[instrument(skip(self), level = "debug")]
    pub fn into_shared_credentials(&self) -> SharedCredentials {
        match self {
            #[cfg(feature = "aws")]
            RuntimeCredentials::Aws(aws) => aws.into_shared_credentials(),
            #[cfg(feature = "azure")]
            RuntimeCredentials::Azure(azure) => azure.into_shared_credentials(),
            #[cfg(feature = "gcp")]
            RuntimeCredentials::Gcp(gcp) => gcp.into_shared_credentials(),
            #[cfg(feature = "r2")]
            RuntimeCredentials::R2(r2) => r2.into_shared_credentials(),
            RuntimeCredentials::Mixed(mixed) => mixed.into_shared_credentials(),
        }
    }
}

#[cfg(test)]
mod storage_path_segment_tests {
    use super::{storage_path_segment, temporary_prefixes, validate_path_component};

    /// The overwhelmingly common shape — an opaque ID — must survive untouched,
    /// or every existing scratch object moves the day this lands.
    #[test]
    fn already_safe_segments_are_returned_verbatim() {
        for value in ["app-1", "user_1", "01JABCDEF0123456789", "a.b-c_d"] {
            assert_eq!(storage_path_segment(value, "fallback"), value);
        }
    }

    /// `validate_path_component` admits `|` and `:` because IdP subjects use
    /// them, so the collapse they force is the normal case, not an edge case.
    #[test]
    fn subjects_that_collapse_stay_distinguishable() {
        for value in ["auth0|123", "auth0:123", "auth0_123"] {
            assert!(
                validate_path_component(value, "sub").is_ok(),
                "{value} should reach the sanitiser at all"
            );
        }
        let segments = ["auth0|123", "auth0:123", "auth0_123"]
            .map(|value| storage_path_segment(value, "user"))
            .to_vec();

        let mut unique = segments.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            segments.len(),
            "distinct subjects must not share a scratch directory: {segments:?}"
        );
        assert_eq!(storage_path_segment("auth0_123", "user"), "auth0_123");
    }

    #[test]
    fn the_digest_is_stable_across_calls() {
        assert_eq!(
            storage_path_segment("sink:abc", "user"),
            storage_path_segment("sink:abc", "user")
        );
    }

    #[test]
    fn empty_and_fully_stripped_values_fall_back_without_colliding() {
        assert_eq!(storage_path_segment("", "user"), "user");
        let dots = storage_path_segment("...", "user");
        let underscores = storage_path_segment("___", "user");
        assert!(dots.starts_with("user-"), "{dots}");
        assert!(underscores.starts_with("user-"), "{underscores}");
        assert_ne!(dots, underscores);
    }

    #[test]
    fn segments_never_exceed_the_ceiling_or_end_in_a_separator() {
        let long = "a".repeat(200);
        let segment = storage_path_segment(&long, "user");
        assert!(segment.len() <= 80, "{} chars", segment.len());
        assert!(!segment.ends_with('_') && !segment.ends_with('.'));
        // Truncation alone must not merge two different subjects.
        assert_ne!(segment, storage_path_segment(&format!("{long}b"), "user"));
    }

    /// The prefix a scoped credential authorises and the path the `/tmp` route
    /// and the HTTP-sink offload write must be built from the same segments.
    /// This is the invariant every cloud's prefix builder now depends on.
    #[test]
    fn temporary_prefixes_match_what_the_writers_build() {
        let (user_prefix, global_prefix) = temporary_prefixes("sink:abc", "app-1");
        assert_eq!(
            user_prefix,
            format!(
                "tmp/user/{}/apps/{}",
                storage_path_segment("sink:abc", "user"),
                storage_path_segment("app-1", "app")
            )
        );
        assert_eq!(global_prefix, "tmp/global/apps/app-1");
        assert!(!user_prefix.contains(':'), "{user_prefix}");
    }
}

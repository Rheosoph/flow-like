use std::fmt::Display;

#[cfg(feature = "aws")]
use aws_credentials::AwsRuntimeCredentials;
#[cfg(feature = "azure")]
use azure_credentials::AzureRuntimeCredentials;
use flow_like::credentials::SharedCredentials;
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
    pub fn is_azure(&self) -> bool {
        #[cfg(feature = "azure")]
        {
            matches!(self, RuntimeCredentials::Azure(_))
        }
        #[cfg(not(feature = "azure"))]
        {
            false
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

        #[cfg(feature = "r2")]
        if matches!(mode, CredentialsAccess::ServerExecute) {
            return Ok(RuntimeCredentials::Mixed(
                R2RuntimeCredentials::from_env()
                    .scoped_server_execute_credentials(sub, app_id)
                    .await?,
            ));
        }

        #[cfg(feature = "r2")]
        return Ok(RuntimeCredentials::R2(
            R2RuntimeCredentials::from_env()
                .scoped_credentials(sub, app_id, state, mode)
                .await?,
        ));

        #[cfg(all(feature = "aws", not(feature = "r2")))]
        return Ok(RuntimeCredentials::Aws(
            AwsRuntimeCredentials::from_env()
                .scoped_credentials(sub, app_id, state, mode)
                .await?,
        ));

        #[cfg(all(feature = "azure", not(feature = "aws"), not(feature = "r2")))]
        return Ok(RuntimeCredentials::Azure(
            AzureRuntimeCredentials::from_env()
                .scoped_credentials(sub, app_id, state, mode)
                .await?,
        ));

        #[cfg(all(
            feature = "gcp",
            not(feature = "aws"),
            not(feature = "azure"),
            not(feature = "r2")
        ))]
        return Ok(RuntimeCredentials::Gcp(
            GcpRuntimeCredentials::from_env()
                .scoped_credentials(sub, app_id, state, mode)
                .await?,
        ));

        #[cfg(not(any(feature = "aws", feature = "azure", feature = "gcp", feature = "r2")))]
        {
            let _ = (sub, app_id, state, mode);
            Err(flow_like_types::anyhow!(
                "No storage backend feature enabled"
            ))
        }
    }

    pub async fn master_credentials() -> Result<Self> {
        // Check for mixed-provider configuration first.
        if let Some(mixed) = mixed_credentials::MixedRuntimeCredentials::detect_from_env() {
            return Ok(RuntimeCredentials::Mixed(mixed.master_credentials().await?));
        }

        #[cfg(feature = "r2")]
        return Ok(RuntimeCredentials::R2(
            R2RuntimeCredentials::from_env().master_credentials().await,
        ));

        #[cfg(all(feature = "aws", not(feature = "r2")))]
        return Ok(RuntimeCredentials::Aws(
            AwsRuntimeCredentials::from_env().master_credentials().await,
        ));

        #[cfg(all(feature = "azure", not(feature = "aws"), not(feature = "r2")))]
        return Ok(RuntimeCredentials::Azure(
            AzureRuntimeCredentials::from_env()
                .master_credentials()
                .await,
        ));

        #[cfg(all(
            feature = "gcp",
            not(feature = "aws"),
            not(feature = "azure"),
            not(feature = "r2")
        ))]
        return Ok(RuntimeCredentials::Gcp(
            GcpRuntimeCredentials::from_env().master_credentials().await,
        ));

        #[cfg(not(any(feature = "aws", feature = "azure", feature = "gcp", feature = "r2")))]
        Err(flow_like_types::anyhow!(
            "No storage backend feature enabled"
        ))
    }

    pub async fn to_store(&self, meta: bool) -> Result<FlowLikeStore> {
        match self {
            #[cfg(feature = "aws")]
            RuntimeCredentials::Aws(aws) => aws.into_shared_credentials().to_store(meta).await,
            #[cfg(feature = "azure")]
            RuntimeCredentials::Azure(azure) => {
                azure.into_shared_credentials().to_store(meta).await
            }
            #[cfg(feature = "gcp")]
            RuntimeCredentials::Gcp(gcp) => gcp.into_shared_credentials().to_store(meta).await,
            #[cfg(feature = "r2")]
            RuntimeCredentials::R2(r2) => r2.into_shared_credentials().to_store(meta).await,
            RuntimeCredentials::Mixed(mixed) => {
                mixed.into_shared_credentials().to_store(meta).await
            }
        }
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
        match self {
            #[cfg(feature = "aws")]
            RuntimeCredentials::Aws(aws) => aws.to_state(state).await,
            #[cfg(feature = "azure")]
            RuntimeCredentials::Azure(azure) => azure.to_state(state).await,
            #[cfg(feature = "gcp")]
            RuntimeCredentials::Gcp(gcp) => gcp.to_state(state).await,
            #[cfg(feature = "r2")]
            RuntimeCredentials::R2(r2) => r2.to_state(state).await,
            RuntimeCredentials::Mixed(mixed) => mixed.to_state(state).await,
        }
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

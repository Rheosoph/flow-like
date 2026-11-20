//! Sources of a query request: which app database, whose copy of it, which columns the caller
//! may read, and the short-lived read-only credentials the service opens it with.

use flow_like::credentials::SharedCredentials;
use flow_like_query::{ReaderSpec, SourceLocation, SourceSpec};
use flow_like_storage::contracts::column_metadata::ColumnReader;

use super::QueryService;
use crate::credentials::{CredentialsAccess, validate_path_component};
use crate::error::ApiError;
use crate::middleware::jwt::{AppPermissionResponse, AppUser};
use crate::routes::app::db::{ScopeParams, column_reader};
use crate::state::State;

/// One app database the caller passed the permission check for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LanceRead {
    pub app_id: String,
    /// Whom project credentials are minted for: the person, the API key or the calling app.
    pub principal: String,
    /// The owner of a personal database; `None` reads the project database.
    pub owner: Option<String>,
    pub reader: ColumnReader,
}

impl LanceRead {
    /// The database `scope` names, with the columns `permission` may read.
    pub(crate) fn scoped(
        user: &AppUser,
        permission: &AppPermissionResponse,
        app_id: &str,
        scope: &ScopeParams,
    ) -> Result<Self, ApiError> {
        Ok(Self {
            app_id: app_id.to_string(),
            principal: permission.identifier(),
            owner: if scope.is_user_scoped() {
                Some(user.sub()?)
            } else {
                None
            },
            reader: column_reader(permission, scope),
        })
    }

    pub(crate) fn source(
        &self,
        schema: Option<String>,
        credentials: SharedCredentials,
    ) -> SourceSpec {
        SourceSpec {
            schema,
            reader: ReaderSpec::from(&self.reader),
            location: SourceLocation::Lance {
                app_id: self.app_id.clone(),
                owner: self.owner.clone(),
                credentials,
            },
        }
    }
}

impl QueryService {
    /// `read` as a request source under `schema`, with read-only credentials for exactly that
    /// database: `ReadAppDb` for the project database, the user-scope read grant for a personal one.
    pub(crate) async fn lance_source(
        &self,
        state: &State,
        read: &LanceRead,
        schema: Option<String>,
    ) -> Result<SourceSpec, ApiError> {
        Ok(read.source(schema, self.read_credentials(state, read).await?))
    }

    async fn read_credentials(
        &self,
        state: &State,
        read: &LanceRead,
    ) -> Result<SharedCredentials, ApiError> {
        validate_path_component(&read.app_id, "app_id")
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        let minted = match &read.owner {
            Some(owner) => {
                state
                    .scoped_credentials(owner, &read.app_id, CredentialsAccess::InvokeRead)
                    .await
            }
            None => {
                state
                    .scoped_credentials(&read.principal, &read.app_id, CredentialsAccess::ReadAppDb)
                    .await
            }
        };
        let error = match minted {
            Ok(credentials) => return Ok(credentials.into_shared_credentials()),
            Err(error) => error,
        };
        if !self.in_process().await {
            return Err(ApiError::service_unavailable(format!(
                "Minting read credentials for app '{}' failed: {error:#}",
                read.app_id
            )));
        }
        tracing::debug!(
            app_id = %read.app_id,
            error = %format!("{error:#}"),
            "No scoped read credentials; the in-process query service reads with the master credentials"
        );
        Ok(state.master_credentials().await?.into_shared_credentials())
    }
}

use crate::{
    credentials::CredentialsAccess,
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::{RolePermissions, has_role_permission},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::credentials::SharedCredentials;
use std::future::Future;

#[utoipa::path(
    get,
    path = "/apps/{app_id}/invoke/presign",
    tag = "execution",
    description = "Get shared credentials for runtime execution.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Shared credentials", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/invoke/presign", skip(state, user))]
pub async fn presign(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<SharedCredentials>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);

    let sub =
        crate::bot_caller::acting_owner(&state.db, &user, &crate::bot_caller::Names::default())
            .await?;
    let access = authorize_invoke_access(&permission.permissions, || {
        crate::routes::app::db::presign_db_access::ensure_no_hidden_columns(
            &state,
            &user,
            &app_id,
            &permission,
        )
    })
    .await?;

    let credentials = state
        .scoped_credentials(&sub, &app_id, access.clone())
        .await?;
    crate::audit::storage_access::authorize_credentials(
        &state,
        &user,
        &app_id,
        &sub,
        &access,
        "invoke",
        &credentials,
    )
    .await?;
    let credentials = credentials.into_shared_credentials();
    Ok(Json(credentials))
}

async fn authorize_invoke_access<F, Fut>(
    permissions: &RolePermissions,
    check_columns: F,
) -> Result<CredentialsAccess, ApiError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), ApiError>>,
{
    let access = invoke_access(permissions).ok_or(ApiError::FORBIDDEN)?;
    // These grants expose raw project database files, including hidden columns.
    // Recheck before returning a cached grant as well as before minting a new one.
    if matches!(
        access,
        CredentialsAccess::InvokeRead | CredentialsAccess::InvokeWrite
    ) {
        check_columns().await?;
    }
    Ok(access)
}

/// The credentials a run of this role obtains through presign; `None` when it may not run the project.
pub(crate) fn invoke_access(permissions: &RolePermissions) -> Option<CredentialsAccess> {
    if !has_role_permission(permissions, RolePermissions::ExecuteEvents) {
        return None;
    }
    Some(
        if has_role_permission(permissions, RolePermissions::WriteFiles) {
            CredentialsAccess::InvokeWrite
        } else if has_role_permission(permissions, RolePermissions::ReadFiles) {
            CredentialsAccess::InvokeRead
        } else {
            CredentialsAccess::InvokeNone
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn project_grants_propagate_column_denials_and_storage_failures() {
        for (files, expected_mode) in [
            (RolePermissions::ReadFiles, "invoke_read"),
            (RolePermissions::WriteFiles, "invoke_write"),
        ] {
            let permissions = RolePermissions::ExecuteEvents | files;
            let error = authorize_invoke_access(&permissions, || async {
                Err(ApiError::forbidden("A project column is hidden"))
            })
            .await
            .unwrap_err();
            assert_eq!(error.status(), StatusCode::FORBIDDEN);

            let error = authorize_invoke_access(&permissions, || async {
                Err(ApiError::service_unavailable(
                    "Column policies are unavailable",
                ))
            })
            .await
            .unwrap_err();
            assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);

            let granted = authorize_invoke_access(&permissions, || async { Ok(()) })
                .await
                .unwrap();
            assert_eq!(granted.to_string(), expected_mode);
        }
    }

    #[tokio::test]
    async fn user_only_grants_skip_project_database_checks() {
        let granted = authorize_invoke_access(&RolePermissions::ExecuteEvents, || async {
            panic!("A user-only grant must not open the project database")
        })
        .await
        .unwrap();
        assert!(matches!(granted, CredentialsAccess::InvokeNone));

        let error = authorize_invoke_access(&RolePermissions::ReadFiles, || async {
            panic!("A caller without execution permission must be rejected first")
        })
        .await
        .unwrap_err();
        assert_eq!(error.status(), StatusCode::FORBIDDEN);
    }
}

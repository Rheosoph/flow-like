use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::execution::UserExecutionContext;

#[utoipa::path(
    get,
    path = "/apps/{app_id}/invoke/context",
    tag = "execution",
    description = "Get your identity and role inside this app, as the runtime sees it. Runs started on your own device use this so a flow reports the same user, role and permissions it would in the cloud.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Execution identity", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/invoke/context", skip(state, user))]
pub async fn execution_context(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<UserExecutionContext>, ApiError> {
    // No permission gate beyond membership: this reports what the caller's role
    // already is, so gating it on a specific permission would hide exactly the
    // restricted roles a local run most needs to reproduce. Principals without a
    // role in the app are rejected by the resolver itself.
    let permission = user.execution_app_permission(&app_id, &state).await?;

    Ok(Json(permission.to_user_context()))
}

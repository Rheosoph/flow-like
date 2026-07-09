use crate::{
    app_connection_jwt::{
        self, AppConnectionJwtParams, MAX_APP_CONNECTION_CHAIN, MAX_APP_CONNECTION_TTL_SECONDS,
    },
    entity::{app_connection, sea_orm_active_enums::AppConnectionStatus},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct CreateAppConnectionTokenRequest {
    /// Token lifetime in seconds, clamped to [60, 900]. Defaults to 600.
    pub ttl_seconds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateAppConnectionTokenResponse {
    /// Short-lived JWT to call the target app. Bound to origin and target app.
    pub token: String,
    pub origin_app_id: String,
    pub target_app_id: String,
    /// Expiration as Unix timestamp (seconds)
    pub expires_at: i64,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/connections/{target_app_id}/token",
    tag = "team",
    description = "Mint a short-lived app-to-app token to call a connected app. The token pins both the origin and the target app and expires quickly.",
    params(
        ("app_id" = String, Path, description = "Application ID (the origin app)"),
        ("target_app_id" = String, Path, description = "The connected app to call")
    ),
    request_body = CreateAppConnectionTokenRequest,
    responses(
        (status = 200, description = "App connection token", body = CreateAppConnectionTokenResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "No active connection to the target app")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/connections/{target_app_id}/token",
    skip(state, user, payload)
)]
pub async fn create_app_connection_token(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, target_app_id)): Path<(String, String)>,
    payload: Option<Json<CreateAppConnectionTokenRequest>>,
) -> Result<Json<CreateAppConnectionTokenResponse>, ApiError> {
    // App-connection tokens must not be exchangeable for further
    // app-connection tokens; that would allow hopping across apps.
    if matches!(user, AppUser::ConnectedApp(_)) {
        return Err(ApiError::forbidden(
            "App connection tokens cannot mint further app connection tokens",
        ));
    }

    // Minting happens during flow execution (desktop: user token, cloud:
    // executor JWT, headless: API key) — any principal allowed to execute in
    // the origin app may act as the app.
    let permission = user.execution_app_permission(&app_id, &state).await?;
    if !permission.has_permission(RolePermissions::ExecuteBoards)
        && !permission.has_permission(RolePermissions::ExecuteEvents)
    {
        return Err(ApiError::FORBIDDEN);
    }

    let connection = app_connection::Entity::find()
        .filter(
            app_connection::Column::SourceAppId
                .eq(&app_id)
                .and(app_connection::Column::TargetAppId.eq(&target_app_id))
                .and(app_connection::Column::Status.eq(AppConnectionStatus::Active)),
        )
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("No active connection to the target app"))?;

    if connection.role_id.is_none() {
        return Err(ApiError::forbidden(
            "The connection has no role assigned yet",
        ));
    }

    // The original user is passed through the whole chain (A -> B -> C) so
    // downstream apps can attribute the call even if the user is not a member.
    let sub = permission.effective_user_id().ok();

    // Extend the app chain: if this run itself was triggered through an app
    // connection, its executor JWT carries the upstream chain.
    let (run_id, mut app_chain, correlation) = if let AppUser::Executor(executor) = &user {
        (
            Some(executor.run_id.clone()),
            executor.app_chain.clone().unwrap_or_default(),
            executor.correlation.clone(),
        )
    } else {
        (None, Vec::new(), None)
    };
    app_chain.push(app_id.clone());

    if app_chain.len() > MAX_APP_CONNECTION_CHAIN {
        return Err(ApiError::forbidden("The app connection chain is too deep"));
    }

    let ttl_seconds = payload
        .as_ref()
        .and_then(|p| p.ttl_seconds)
        .unwrap_or(600)
        .clamp(60, MAX_APP_CONNECTION_TTL_SECONDS);

    let token = app_connection_jwt::sign(AppConnectionJwtParams {
        sub,
        origin_app_id: app_id.clone(),
        target_app_id: target_app_id.clone(),
        app_chain,
        technical_user_id: permission.technical_user_id().map(|id| id.to_string()),
        run_id,
        correlation,
        ttl_seconds: Some(ttl_seconds),
    })
    .map_err(|err| {
        tracing::error!("Failed to sign app connection token: {}", err);
        ApiError::internal("Failed to sign app connection token")
    })?;

    Ok(Json(CreateAppConnectionTokenResponse {
        token,
        origin_app_id: app_id,
        target_app_id,
        expires_at: chrono::Utc::now().timestamp() + ttl_seconds,
    }))
}

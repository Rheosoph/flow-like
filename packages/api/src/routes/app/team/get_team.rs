use crate::{
    entity::membership, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::LanguageParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

#[utoipa::path(
    get,
    path = "/apps/{app_id}/team",
    tag = "team",
    description = "List team members for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("limit" = Option<u64>, Query, description = "Max results"),
        ("offset" = Option<u64>, Query, description = "Result offset")
    ),
    responses(
        (status = 200, description = "Team members", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/team", skip(state, user, params))]
pub async fn get_team(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(params): Query<LanguageParams>,
) -> Result<Json<Vec<membership::Model>>, ApiError> {
    let permission = user.execution_app_permission(&app_id, &state).await?;
    if !permission.has_permission(RolePermissions::ReadTeam) {
        if let Ok(user_id) = permission.sub() {
            state.invalidate_permission(&user_id, &app_id);
        }
        return Err(ApiError::FORBIDDEN);
    }

    let members = membership::Entity::find()
        .order_by_asc(membership::Column::CreatedAt)
        .filter(membership::Column::AppId.eq(app_id.clone()))
        .limit(params.limit)
        .offset(params.offset)
        .all(&state.db)
        .await?;

    Ok(Json(members))
}

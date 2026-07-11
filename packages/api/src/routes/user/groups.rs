use crate::{
    entity::{app_group, app_group_member, membership},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::app::groups::{GroupInfo, assemble_groups},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::State,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[utoipa::path(
    get,
    path = "/user/groups",
    tag = "user",
    description = "List suites (app groups) that any of the caller's apps own or belong to.",
    responses(
        (status = 200, description = "Groups across the user's apps", body = [GroupInfo]),
        (status = 401, description = "Unauthorized")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /user/groups", skip(state, user))]
pub async fn get_user_groups(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Vec<GroupInfo>>, ApiError> {
    let user_id = user.sub()?;

    let app_ids: Vec<String> = membership::Entity::find()
        .filter(membership::Column::UserId.eq(user_id))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|m| m.app_id)
        .collect();

    if app_ids.is_empty() {
        return Ok(Json(vec![]));
    }

    let owned_group_ids: Vec<String> = app_group::Entity::find()
        .filter(app_group::Column::OwnerAppId.is_in(app_ids.clone()))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|g| g.id)
        .collect();

    let member_group_ids: Vec<String> = app_group_member::Entity::find()
        .filter(app_group_member::Column::AppId.is_in(app_ids))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|m| m.group_id)
        .collect();

    let mut group_ids: Vec<String> = owned_group_ids;
    for group_id in member_group_ids {
        if !group_ids.contains(&group_id) {
            group_ids.push(group_id);
        }
    }

    if group_ids.is_empty() {
        return Ok(Json(vec![]));
    }

    let groups = app_group::Entity::find()
        .filter(app_group::Column::Id.is_in(group_ids.clone()))
        .all(&state.db)
        .await?;
    let members = app_group_member::Entity::find()
        .filter(app_group_member::Column::GroupId.is_in(group_ids))
        .all(&state.db)
        .await?;

    Ok(Json(assemble_groups(&state, groups, members).await?))
}

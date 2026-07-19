use crate::{
    entity::{app_group, app_group_member, membership, role},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::{RolePermissions, has_role_permission},
    routes::app::groups::{GroupInfo, assemble_groups},
    state::AppState,
};
use axum::{Extension, Json, extract::State};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::collections::HashSet;

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

    // Suites expose sibling apps' names, descriptions and artwork, so only
    // apps where the caller may actually see the team are considered. A bare
    // membership is not enough.
    let memberships = membership::Entity::find()
        .filter(membership::Column::UserId.eq(user_id))
        .all(&state.db)
        .await?;

    if memberships.is_empty() {
        return Ok(Json(vec![]));
    }

    let role_ids: Vec<String> = memberships.iter().map(|m| m.role_id.clone()).collect();
    let readable_roles: HashSet<String> = role::Entity::find()
        .filter(role::Column::Id.is_in(role_ids))
        .all(&state.db)
        .await?
        .into_iter()
        .filter(|r| {
            has_role_permission(
                &RolePermissions::from_bits_truncate(r.permissions),
                RolePermissions::ReadTeam,
            )
        })
        .map(|r| r.id)
        .collect();

    let app_ids: Vec<String> = memberships
        .into_iter()
        .filter(|m| readable_roles.contains(&m.role_id))
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

    let mut unique_group_ids: HashSet<String> = owned_group_ids.into_iter().collect();
    unique_group_ids.extend(member_group_ids);
    let group_ids: Vec<String> = unique_group_ids.into_iter().collect();

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

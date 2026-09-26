use crate::{
    entity::{app_group, app_group_member, membership, role},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::{RolePermissions, has_role_permission},
    routes::app::groups::{GroupInfo, assemble_groups},
    state::AppState,
};
use axum::{Extension, Json, extract::State};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, JoinType, QueryFilter, QuerySelect, QueryTrait,
    RelationTrait,
};

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
    let app_ids: Vec<String> = membership::Entity::find()
        .select_only()
        .column(membership::Column::AppId)
        .column(role::Column::Permissions)
        .join(JoinType::InnerJoin, membership::Relation::Role.def())
        .filter(membership::Column::UserId.eq(user_id))
        .into_tuple::<(String, i64)>()
        .all(&state.db)
        .await?
        .into_iter()
        .filter(|(_, permissions)| {
            has_role_permission(
                &RolePermissions::from_bits_truncate(*permissions),
                RolePermissions::ReadTeam,
            )
        })
        .map(|(app_id, _)| app_id)
        .collect();

    if app_ids.is_empty() {
        return Ok(Json(vec![]));
    }

    let member_group_ids = app_group_member::Entity::find()
        .select_only()
        .column(app_group_member::Column::GroupId)
        .filter(app_group_member::Column::AppId.is_in(app_ids.clone()))
        .into_query();

    let groups = app_group::Entity::find()
        .filter(
            Condition::any()
                .add(app_group::Column::OwnerAppId.is_in(app_ids))
                .add(app_group::Column::Id.in_subquery(member_group_ids)),
        )
        .all(&state.db)
        .await?;

    if groups.is_empty() {
        return Ok(Json(vec![]));
    }

    let members = app_group_member::Entity::find()
        .filter(app_group_member::Column::GroupId.is_in(groups.iter().map(|g| g.id.clone())))
        .all(&state.db)
        .await?;

    Ok(Json(assemble_groups(&state, groups, members).await?))
}

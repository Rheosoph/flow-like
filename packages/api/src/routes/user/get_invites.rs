use crate::{
    entity::{invitation, membership},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::LanguageParams,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

#[utoipa::path(
    get,
    path = "/user/invites",
    tag = "user",
    params(
        ("language" = Option<String>, Query, description = "Language code"),
        ("limit" = Option<u64>, Query, description = "Maximum number of results"),
        ("offset" = Option<u64>, Query, description = "Offset for pagination")
    ),
    responses(
        (status = 200, description = "List of pending invitations"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /user/invites", skip_all)]
pub async fn get_invites(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<LanguageParams>,
) -> Result<Json<Vec<invitation::Model>>, ApiError> {
    let sub = user.sub()?;

    let invitations = invitation::Entity::find()
        .order_by_desc(invitation::Column::CreatedAt)
        .filter(invitation::Column::UserId.eq(sub))
        .find_also_related(membership::Entity)
        .limit(query.limit)
        .offset(query.offset)
        .all(&state.db)
        .await?;

    let invitations = invitations
        .into_iter()
        .filter_map(|(mut invite, membership)| {
            membership.map(|m| {
                invite.by_member_id = m.user_id.clone();
                invite
            })
        })
        .collect::<Vec<_>>();

    Ok(Json(invitations))
}

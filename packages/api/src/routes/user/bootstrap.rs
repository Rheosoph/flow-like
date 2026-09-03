use crate::{
    entity::{app, invitation, membership, meta, notification, user},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use flow_like::{app::App, bit::Metadata};
use sea_orm::sea_query::ExprTrait;
use sea_orm::{
    ColumnTrait, EntityTrait, JoinType, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait,
};
use serde::{Deserialize, Serialize};
use utoipa::IntoParams;

use super::notifications::NotificationOverview;

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct BootstrapParams {
    pub language: Option<String>,
    pub apps_limit: Option<u64>,
    pub apps_offset: Option<u64>,
    pub invites_limit: Option<u64>,
    pub invites_offset: Option<u64>,
}

#[derive(Serialize)]
pub struct PaginatedApps {
    pub items: Vec<(App, Option<Metadata>)>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

#[derive(Debug, Serialize)]
pub struct PaginatedInvites {
    pub items: Vec<invitation::Model>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

#[derive(Serialize)]
pub struct BootstrapResponse {
    pub info: user::Model,
    pub notifications: NotificationOverview,
    pub apps: PaginatedApps,
    pub pending_invites: PaginatedInvites,
}

#[utoipa::path(
    get,
    path = "/user/bootstrap",
    tag = "user",
    params(BootstrapParams),
    responses(
        (status = 200, description = "Combined user bootstrap data", body = Object),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /user/bootstrap", skip_all)]
pub async fn bootstrap(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<BootstrapParams>,
) -> Result<Json<BootstrapResponse>, ApiError> {
    // 1. User info (reuse existing handler directly)
    let info = super::info::user_info(State(state.clone()), Extension(user.clone()))
        .await?
        .0;

    let sub = user.sub()?;

    // 2. Notification counts
    let invites_count = invitation::Entity::find()
        .filter(invitation::Column::UserId.eq(&sub))
        .count(&state.db)
        .await?;

    let notifications_count = notification::Entity::find()
        .filter(notification::Column::UserId.eq(&sub))
        .count(&state.db)
        .await?;

    let unread_count = notification::Entity::find()
        .filter(notification::Column::UserId.eq(&sub))
        .filter(notification::Column::Read.eq(false))
        .count(&state.db)
        .await?;

    let notifications = NotificationOverview {
        invites_count,
        notifications_count,
        unread_count,
    };

    // 3. Apps (paginated)
    let language = params.language.clone().unwrap_or_else(|| "en".to_string());
    let apps_limit = std::cmp::Ord::min(params.apps_limit.unwrap_or(50), 100);
    let apps_offset = params.apps_offset.unwrap_or(0);

    let apps_total = app::Entity::find()
        .join(JoinType::InnerJoin, app::Relation::Membership.def())
        .filter(membership::Column::UserId.eq(&sub))
        .count(&state.db)
        .await?;

    let apps_with_meta = app::Entity::find()
        .order_by_desc(app::Column::UpdatedAt)
        .join(JoinType::InnerJoin, app::Relation::Membership.def())
        .find_with_related(meta::Entity)
        .filter(
            meta::Column::Lang
                .eq(&language)
                .or(meta::Column::Lang.eq("en")),
        )
        .filter(membership::Column::UserId.eq(&sub))
        .limit(Some(apps_limit))
        .offset(Some(apps_offset))
        .all(&state.db)
        .await?;

    let master_store = state.master_credentials().await?;
    let store = master_store.to_store(false).await?;

    let mut apps_items = Vec::new();
    for (app_model, meta_models) in apps_with_meta {
        let metadata = if let Some(m) = meta_models
            .iter()
            .find(|m| m.lang == language)
            .or_else(|| meta_models.first())
        {
            let mut metadata = Metadata::from(m.clone());
            let prefix = flow_like_storage::Path::from("media")
                .child("apps")
                .child(app_model.id.clone());
            metadata.presign(prefix, &store).await;
            Some(metadata)
        } else {
            None
        };
        apps_items.push((App::from(app_model), metadata));
    }

    // 4. Pending invites (paginated)
    let invites_limit_val = std::cmp::Ord::min(params.invites_limit.unwrap_or(20), 100);
    let invites_offset_val = params.invites_offset.unwrap_or(0);

    let invites_total = invitation::Entity::find()
        .filter(invitation::Column::UserId.eq(&sub))
        .count(&state.db)
        .await?;

    let invitations = invitation::Entity::find()
        .order_by_desc(invitation::Column::CreatedAt)
        .filter(invitation::Column::UserId.eq(&sub))
        .find_also_related(membership::Entity)
        .limit(Some(invites_limit_val))
        .offset(Some(invites_offset_val))
        .all(&state.db)
        .await?;

    let invite_items: Vec<_> = invitations
        .into_iter()
        .filter_map(|(mut invite, membership)| {
            membership.map(|m| {
                invite.by_member_id = m.user_id.clone();
                invite
            })
        })
        .collect();

    Ok(Json(BootstrapResponse {
        info,
        notifications,
        apps: PaginatedApps {
            items: apps_items,
            total: apps_total,
            offset: apps_offset,
            limit: apps_limit,
        },
        pending_invites: PaginatedInvites {
            items: invite_items,
            total: invites_total,
            offset: invites_offset_val,
            limit: invites_limit_val,
        },
    }))
}

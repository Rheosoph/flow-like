use crate::{
    entity::{app, invitation, membership, meta, user},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use flow_like::{app::App, bit::Metadata};
use flow_like_types::tokio::try_join;
use sea_orm::{
    ColumnTrait, EntityTrait, JoinType, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::IntoParams;

use super::notifications::{NotificationOverview, notification_overview};

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
    let notifications = notification_overview(&state.db, &sub).await?;
    let invites_total = notifications.invites_count;

    // 3. Apps and pending invites (paginated)
    let language = params.language.clone().unwrap_or_else(|| "en".to_string());
    let apps_limit = std::cmp::Ord::min(params.apps_limit.unwrap_or(50), 100);
    let apps_offset = params.apps_offset.unwrap_or(0);
    let invites_limit_val = std::cmp::Ord::min(params.invites_limit.unwrap_or(20), 100);
    let invites_offset_val = params.invites_offset.unwrap_or(0);

    let apps_total_fut = app::Entity::find()
        .join(JoinType::InnerJoin, app::Relation::Membership.def())
        .filter(membership::Column::UserId.eq(&sub))
        .count(&state.db);
    let apps_page_fut = app::Entity::find()
        .order_by_desc(app::Column::UpdatedAt)
        .order_by_asc(app::Column::Id)
        .join(JoinType::InnerJoin, app::Relation::Membership.def())
        .filter(membership::Column::UserId.eq(&sub))
        .limit(Some(apps_limit))
        .offset(Some(apps_offset))
        .all(&state.db);
    let invitations_fut = invitation::Entity::find()
        .order_by_desc(invitation::Column::CreatedAt)
        .filter(invitation::Column::UserId.eq(&sub))
        .find_also_related(membership::Entity)
        .limit(Some(invites_limit_val))
        .offset(Some(invites_offset_val))
        .all(&state.db);

    let (apps_total, app_models, invitations) =
        try_join!(apps_total_fut, apps_page_fut, invitations_fut)?;

    let mut preferred_meta: HashMap<String, meta::Model> = HashMap::new();
    if !app_models.is_empty() {
        let metas = meta::Entity::find()
            .filter(meta::Column::AppId.is_in(app_models.iter().map(|a| a.id.clone())))
            .filter(meta::Column::Lang.is_in([language.as_str(), "en"]))
            .all(&state.db)
            .await?;
        for m in metas {
            let Some(app_id) = m.app_id.clone() else {
                continue;
            };
            let preferred = preferred_meta
                .get(&app_id)
                .is_none_or(|current| current.lang != language && m.lang == language);
            if preferred {
                preferred_meta.insert(app_id, m);
            }
        }
    }

    let master_store = state.master_credentials().await?;
    let store = master_store.to_store(false).await?;

    let mut apps_items = Vec::with_capacity(app_models.len());
    for app_model in app_models {
        let metadata = if let Some(m) = preferred_meta.remove(&app_model.id) {
            let mut metadata = Metadata::from(m);
            let prefix = flow_like_storage::Path::from("media")
                .join("apps")
                .join(app_model.id.clone());
            metadata.presign(prefix, &store).await;
            Some(metadata)
        } else {
            None
        };
        apps_items.push((App::from(app_model), metadata));
    }

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

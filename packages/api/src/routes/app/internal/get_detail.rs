use crate::{
    entity::{app, event, meta},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::app::ensure_app_publicly_visible,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::{app::App, bit::Metadata};
use flow_like_storage::Path as FlowPath;
use flow_like_types::tokio::try_join;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use utoipa::IntoParams;

const USER_FACING_EVENT_TYPES: &[&str] = &["simple_chat", "generic_form", "quick_action"];

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct DetailQuery {
    pub language: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AppPermissions {
    pub is_member: bool,
    pub role: Option<String>,
    pub can_use: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_href: Option<String>,
}

#[derive(Serialize)]
pub struct AppDetailResponse {
    pub app: App,
    pub meta: Option<Metadata>,
    pub permissions: AppPermissions,
}

fn compute_use_href(app_id: &str, active_events: &[event::Model]) -> Option<String> {
    // Check routes first: any active event with a route that has page_id or is user-facing
    let has_usable_route = active_events.iter().any(|e| {
        e.route.is_some()
            && (e.page_id.is_some() || USER_FACING_EVENT_TYPES.contains(&e.event_type.as_str()))
    });

    if has_usable_route {
        return Some(format!("/use?id={}", app_id));
    }

    // Fallback: any active event that is user-facing
    let fallback = active_events
        .iter()
        .find(|e| e.page_id.is_some() || USER_FACING_EVENT_TYPES.contains(&e.event_type.as_str()));

    fallback.map(|e| format!("/use?id={}&eventId={}", app_id, e.id))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/detail",
    tag = "apps",
    description = "Get combined app detail: app + metadata + permissions. Returns permission-aware data for members, or basic info for public apps.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("language" = Option<String>, Query, description = "Language code (default: en)")
    ),
    responses(
        (status = 200, description = "App detail with metadata and permissions", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/detail", skip(state, user, query))]
pub async fn get_detail(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<DetailQuery>,
) -> Result<Json<AppDetailResponse>, ApiError> {
    let language = query.language.clone().unwrap_or_else(|| "en".to_string());

    let (app_model, is_member, role_name) =
        if let Ok(perm) = user.app_permission(&app_id, &state).await {
            let app = app::Entity::find_by_id(&app_id)
                .one(&state.db)
                .await?
                .ok_or(ApiError::NOT_FOUND)?;
            (app, true, Some(perm.role.name.clone()))
        } else {
            if !state.platform_config.features.unauthorized_read {
                user.sub()?;
            }
            let app = ensure_app_publicly_visible(&app_id, &state).await?;
            (app, false, None)
        };

    let mut app: App = app_model.clone().into();

    // Load scoped data for members
    if is_member
        && let Ok(sub) = user.sub()
        && let Ok(scoped_app) = state.master_app(&sub, &app_id, &state).await
    {
        app.bits = scoped_app.bits;
        app.boards = scoped_app.boards;
        app.templates = scoped_app.templates;
        app.events = scoped_app.events;
    }

    // Load metadata (requested language, else English) and, for members, the
    // active events behind can_use / use_href
    let meta_rows = meta::Entity::find()
        .filter(meta::Column::AppId.eq(&app_id))
        .filter(meta::Column::Lang.is_in([language.as_str(), "en"]))
        .all(&state.db);
    let active_events = async {
        if !is_member {
            return Ok(Vec::new());
        }
        event::Entity::find()
            .filter(event::Column::AppId.eq(&app_id))
            .filter(event::Column::Active.eq(true))
            .order_by_desc(event::Column::Priority)
            .all(&state.db)
            .await
    };
    let (mut meta_rows, active_events) = try_join!(meta_rows, active_events)?;

    let existing_meta = meta_rows
        .iter()
        .position(|m| m.lang == language)
        .or_else(|| meta_rows.iter().position(|m| m.lang == "en"))
        .map(|index| meta_rows.swap_remove(index));

    let metadata = if let Some(meta_model) = existing_meta {
        let mut metadata = Metadata::from(meta_model);
        let master_store = state.master_credentials().await?;
        let store = master_store.to_store(false).await?;
        let prefix = FlowPath::from("media").join("apps").join(app_id.clone());
        metadata.presign(prefix, &store).await;
        Some(metadata)
    } else {
        None
    };

    // Compute can_use / use_href for members
    let (can_use, use_href) = if is_member {
        let href = compute_use_href(&app_id, &active_events);
        (href.is_some(), href)
    } else {
        (false, None)
    };

    Ok(Json(AppDetailResponse {
        app,
        meta: metadata,
        permissions: AppPermissions {
            is_member,
            role: role_name,
            can_use,
            use_href,
        },
    }))
}

use axum::{
    Router,
    routing::{get, post, put},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    entity::{app_connection, meta, role, sea_orm_active_enums::AppConnectionStatus},
    error::ApiError,
    state::AppState,
};

pub mod add_connection;
pub mod cases;
pub mod get_accessible;
pub mod get_connections;
pub mod graph;
pub mod manage_connection;
pub mod notes;
pub mod remote_events;
pub mod remote_tables;
pub mod remove_connection;
pub mod request_connection;
pub mod token;
pub mod update_connection;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            get(get_connections::get_connections).post(add_connection::add_connection),
        )
        .route("/request", put(request_connection::request_connection))
        .route(
            "/queue/{connection_id}",
            post(manage_connection::accept_connection_request)
                .delete(manage_connection::reject_connection_request),
        )
        .route("/accessible", get(get_accessible::get_accessible_apps))
        .route("/graph", get(graph::get_connection_graph))
        .route("/cases", get(cases::list_process_cases))
        .route("/cases/{case_id}", get(cases::get_process_case))
        .route("/notes", get(notes::list_notes).put(notes::create_note))
        .route(
            "/notes/{note_id}",
            put(notes::update_note).delete(notes::delete_note),
        )
        .route(
            "/{connection_id}",
            put(update_connection::update_connection).delete(remove_connection::remove_connection),
        )
        .route(
            "/{target_app_id}/tables",
            get(remote_tables::get_remote_tables),
        )
        .route(
            "/{target_app_id}/events",
            get(remote_events::get_remote_events),
        )
        .route(
            "/{target_app_id}/events/{event_id}/detail",
            get(remote_events::get_remote_event_detail),
        )
        .route(
            "/{target_app_id}/token",
            post(token::create_app_connection_token),
        )
}

/// A connection between two apps: the source app can act on the target app
/// with the permissions of the assigned role.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AppConnectionInfo {
    pub id: String,
    pub source_app_id: String,
    pub target_app_id: String,
    /// "PENDING" or "ACTIVE"
    pub status: String,
    pub role_id: Option<String>,
    pub role_name: Option<String>,
    /// Raw permission bits granted by the connection role.
    pub role_permissions: Option<i64>,
    pub comment: Option<String>,
    pub requested_by_user_id: Option<String>,
    pub approved_by_user_id: Option<String>,
    /// Name of the other app (source app for incoming, target app for outgoing)
    pub app_name: Option<String>,
    /// Description of the other app
    pub app_description: Option<String>,
    /// Presigned icon URL of the other app
    pub app_icon: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub(crate) fn status_to_string(status: &AppConnectionStatus) -> String {
    match status {
        AppConnectionStatus::Pending => "PENDING".to_string(),
        AppConnectionStatus::Active => "ACTIVE".to_string(),
    }
}

/// Preferred display metadata of an app (English, falling back to any locale).
#[derive(Debug, Clone)]
pub(crate) struct AppMetaPreview {
    pub name: String,
    pub description: Option<String>,
    /// Raw icon storage key (presign before returning to clients).
    pub icon: Option<String>,
    /// Raw banner/thumbnail storage key (presign before returning to clients).
    pub banner: Option<String>,
    pub website: Option<String>,
    pub docs_url: Option<String>,
    pub tags: Vec<String>,
}

/// Fetches the preferred (English, falling back to any) metadata for a list
/// of app ids.
pub(crate) async fn app_meta_lookup(
    state: &AppState,
    app_ids: &[String],
) -> Result<std::collections::HashMap<String, AppMetaPreview>, ApiError> {
    if app_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }

    let metas = meta::Entity::find()
        .filter(meta::Column::AppId.is_in(app_ids.iter().cloned()))
        .all(&state.db)
        .await?;

    let mut lookup: std::collections::HashMap<String, (AppMetaPreview, bool)> =
        std::collections::HashMap::new();

    for m in metas {
        let Some(meta_app_id) = m.app_id.clone() else {
            continue;
        };
        let is_english = m.lang == "en";
        match lookup.get(&meta_app_id) {
            Some((_, existing_is_english)) if *existing_is_english => {}
            _ => {
                lookup.insert(
                    meta_app_id,
                    (
                        AppMetaPreview {
                            name: m.name,
                            description: m.description,
                            icon: m.icon,
                            banner: m.thumbnail,
                            website: m.website,
                            docs_url: m.docs_url,
                            tags: m.tags.unwrap_or_default(),
                        },
                        is_english,
                    ),
                );
            }
        }
    }

    Ok(lookup
        .into_iter()
        .map(|(k, (preview, _))| (k, preview))
        .collect())
}

/// Loads the role names for the given role ids.
pub(crate) async fn role_name_lookup(
    state: &AppState,
    role_ids: &[String],
) -> Result<std::collections::HashMap<String, String>, ApiError> {
    if role_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let roles = role::Entity::find()
        .filter(role::Column::Id.is_in(role_ids.iter().cloned()))
        .all(&state.db)
        .await?;
    Ok(roles.into_iter().map(|r| (r.id, r.name)).collect())
}

/// Loads the raw permission bits granted by the given role ids.
pub(crate) async fn role_permission_lookup(
    state: &AppState,
    role_ids: &[String],
) -> Result<std::collections::HashMap<String, i64>, ApiError> {
    if role_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let roles = role::Entity::find()
        .filter(role::Column::Id.is_in(role_ids.iter().cloned()))
        .all(&state.db)
        .await?;
    Ok(roles.into_iter().map(|r| (r.id, r.permissions)).collect())
}

pub(crate) fn to_connection_info(
    model: app_connection::Model,
    role_names: &std::collections::HashMap<String, String>,
    role_permission_bits: &std::collections::HashMap<String, i64>,
    app_meta: &std::collections::HashMap<String, AppMetaPreview>,
    media: &std::collections::HashMap<String, (Option<String>, Option<String>)>,
    other_app_id: &str,
) -> AppConnectionInfo {
    let (app_name, app_description) = app_meta
        .get(other_app_id)
        .map(|preview| (Some(preview.name.clone()), preview.description.clone()))
        .unwrap_or((None, None));
    let app_icon = media.get(other_app_id).and_then(|(icon, _)| icon.clone());

    AppConnectionInfo {
        id: model.id,
        source_app_id: model.source_app_id,
        target_app_id: model.target_app_id,
        status: status_to_string(&model.status),
        role_name: model
            .role_id
            .as_ref()
            .and_then(|id| role_names.get(id).cloned()),
        role_permissions: model
            .role_id
            .as_ref()
            .and_then(|id| role_permission_bits.get(id).copied()),
        role_id: model.role_id,
        comment: model.comment,
        requested_by_user_id: model.requested_by_user_id,
        approved_by_user_id: model.approved_by_user_id,
        app_name,
        app_description,
        app_icon,
        created_at: model.created_at.and_utc().timestamp(),
        updated_at: model.updated_at.and_utc().timestamp(),
    }
}

/// Validates that a role belongs to the app and may be assigned to an app
/// connection. Owner and Admin roles can never be granted to another app:
/// Admin implies every permission, which would let the connected app manage
/// this app's team, roles, and connections (lateral movement across apps).
pub(crate) async fn validate_connection_role(
    state: &AppState,
    app_id: &str,
    role_id: &str,
) -> Result<role::Model, ApiError> {
    use crate::permission::role_permission::RolePermissions;

    let role_model = role::Entity::find_by_id(role_id)
        .filter(role::Column::AppId.eq(app_id))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("Role not found for this app"))?;

    let permissions = RolePermissions::from_bits(role_model.permissions)
        .ok_or_else(|| ApiError::internal("Invalid role permission bits"))?;

    if permissions.contains(RolePermissions::Owner) || permissions.contains(RolePermissions::Admin)
    {
        return Err(ApiError::forbidden(
            "Owner and admin roles cannot be assigned to an app connection",
        ));
    }

    Ok(role_model)
}

/// User ids of members whose role carries Admin or Owner — the people to
/// notify about connection lifecycle events. Capped to keep dispatch cheap.
pub(crate) async fn admin_user_ids(
    state: &AppState,
    app_id: &str,
) -> Result<Vec<String>, ApiError> {
    use crate::entity::membership;
    use crate::permission::role_permission::RolePermissions;

    let admin_role_ids: Vec<String> = role::Entity::find()
        .filter(role::Column::AppId.eq(app_id))
        .all(&state.db)
        .await?
        .into_iter()
        .filter(|role_model| {
            RolePermissions::from_bits(role_model.permissions)
                .map(|permissions| {
                    permissions.contains(RolePermissions::Admin)
                        || permissions.contains(RolePermissions::Owner)
                })
                .unwrap_or(false)
        })
        .map(|role_model| role_model.id)
        .collect();

    if admin_role_ids.is_empty() {
        return Ok(Vec::new());
    }

    use sea_orm::QuerySelect;
    let user_ids: Vec<String> = membership::Entity::find()
        .filter(membership::Column::AppId.eq(app_id))
        .filter(membership::Column::RoleId.is_in(admin_role_ids))
        .select_only()
        .column(membership::Column::UserId)
        .limit(25)
        .into_tuple()
        .all(&state.db)
        .await?;

    Ok(user_ids)
}

/// Notifies all admins/owners of an app about a connection lifecycle event.
/// Failures are logged, never surfaced — notifications must not fail the
/// underlying operation.
pub(crate) async fn notify_app_admins(
    state: &AppState,
    app_id: &str,
    title: String,
    description: String,
) {
    use crate::entity::sea_orm_active_enums::NotificationType;
    use crate::push_notifications::{DispatchNotificationInput, dispatch_notification};

    let user_ids = match admin_user_ids(state, app_id).await {
        Ok(user_ids) => user_ids,
        Err(error) => {
            tracing::warn!(error = %error, app_id = %app_id, "Failed to resolve admins for connection notification");
            return;
        }
    };

    for user_id in user_ids {
        if let Err(error) = dispatch_notification(
            state,
            DispatchNotificationInput {
                user_id,
                app_id: Some(app_id.to_string()),
                title: title.clone(),
                description: Some(description.clone()),
                icon: Some("blocks".to_string()),
                link: Some(format!("/library/config/team?id={}", app_id)),
                image: None,
                notification_type: NotificationType::System,
                source_run_id: None,
                source_node_id: None,
            },
        )
        .await
        {
            tracing::warn!(error = %error, "Failed to dispatch app connection notification");
        }
    }
}

/// Display name of an app for notifications, falling back to the id.
pub(crate) async fn app_display_name(state: &AppState, app_id: &str) -> String {
    app_meta_lookup(state, &[app_id.to_string()])
        .await
        .ok()
        .and_then(|meta| meta.get(app_id).map(|preview| preview.name.clone()))
        .unwrap_or_else(|| app_id.to_string())
}

/// App connections must be managed by people (or their API keys), never by
/// another app acting through a connection token.
pub(crate) fn deny_connected_app(user: &crate::middleware::jwt::AppUser) -> Result<(), ApiError> {
    if user.is_connected_app() {
        return Err(ApiError::forbidden(
            "Connected apps cannot manage app connections",
        ));
    }
    Ok(())
}

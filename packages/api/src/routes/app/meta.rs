use axum::{
    Router,
    routing::{delete, get, put},
};
use flow_like_storage::Path as FlowPath;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::{
    auth::AppUser,
    ensure_in_project, ensure_permission,
    entity::{
        app_group, app_group_member, meta,
        sea_orm_active_enums::{AppGroupMemberStatus, Visibility},
    },
    error::ApiError,
    middleware::jwt::AppPermissionResponse,
    permission::role_permission::RolePermissions,
    routes::app::connection::deny_connected_app,
    state::AppState,
};

pub mod push_media;
pub mod remove_media;

pub mod get_meta;
pub mod upsert_meta;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(get_meta::get_meta).put(upsert_meta::upsert_meta))
        .route("/media", put(push_media::push_media))
        .route("/media/{media_id}", delete(remove_media::remove_media))
}

#[derive(Deserialize, Debug, ToSchema)]
pub struct MetaQuery {
    pub language: Option<String>,
    pub template_id: Option<String>,
    pub course_id: Option<String>,
    pub widget_id: Option<String>,
    pub group_id: Option<String>,
}

#[derive(Deserialize, Debug, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MediaItem {
    Icon,
    Thumbnail,
    Preview,
}
#[derive(Deserialize, Debug, ToSchema)]
pub struct MediaQuery {
    pub language: Option<String>,
    pub template_id: Option<String>,
    pub course_id: Option<String>,
    pub widget_id: Option<String>,
    pub group_id: Option<String>,
    pub item: MediaItem,
    pub extension: String,
}

pub enum MetaMode {
    Template(String),
    App(String),
    Course(String),
    Widget(String),
    Group(String),
}

impl MetaMode {
    pub fn new(query: &MetaQuery, app_id: &str) -> Self {
        if let Some(template_id) = &query.template_id {
            MetaMode::Template(template_id.clone())
        } else if let Some(course_id) = &query.course_id {
            MetaMode::Course(course_id.clone())
        } else if let Some(widget_id) = &query.widget_id {
            MetaMode::Widget(widget_id.clone())
        } else if let Some(group_id) = &query.group_id {
            MetaMode::Group(group_id.clone())
        } else {
            MetaMode::App(app_id.to_string())
        }
    }

    pub fn from_media_query(query: &MediaQuery, app_id: &str) -> Self {
        if let Some(template_id) = &query.template_id {
            MetaMode::Template(template_id.clone())
        } else if let Some(course_id) = &query.course_id {
            MetaMode::Course(course_id.clone())
        } else if let Some(widget_id) = &query.widget_id {
            MetaMode::Widget(widget_id.clone())
        } else if let Some(group_id) = &query.group_id {
            MetaMode::Group(group_id.clone())
        } else {
            MetaMode::App(app_id.to_string())
        }
    }

    /// Object-store prefix holding this entity's media. Groups keep their own
    /// folder so forking an app (which copies `media/apps/{app_id}` wholesale)
    /// never drags suite artwork along.
    pub fn media_prefix(&self, app_id: &str) -> FlowPath {
        match self {
            MetaMode::Group(group_id) => FlowPath::from("media")
                .child("groups")
                .child(group_id.clone()),
            _ => FlowPath::from("media")
                .child("apps")
                .child(app_id.to_string()),
        }
    }

    pub async fn ensure_write_permission(
        &self,
        user: &AppUser,
        app_id: &str,
        state: &AppState,
    ) -> Result<AppPermissionResponse, ApiError> {
        match self {
            MetaMode::Template(_) => Ok(ensure_permission!(
                user,
                app_id,
                state,
                RolePermissions::WriteTemplates
            )),
            MetaMode::Course(_) => Ok(ensure_permission!(
                user,
                app_id,
                state,
                RolePermissions::WriteCourses
            )),
            MetaMode::Widget(_) => Ok(ensure_permission!(
                user,
                app_id,
                state,
                RolePermissions::WriteWidgets
            )),
            MetaMode::App(_) => Ok(ensure_permission!(
                user,
                app_id,
                state,
                RolePermissions::WriteMeta
            )),
            MetaMode::Group(group_id) => {
                deny_connected_app(user)?;
                let permission = ensure_permission!(user, app_id, state, RolePermissions::Admin);
                Self::ensure_group_anchor(group_id, app_id, state).await?;
                Ok(permission)
            }
        }
    }

    /// A group's branding is editable only through its owner (anchor) app, so
    /// admin rights on an unrelated app can never rewrite someone else's suite.
    async fn ensure_group_anchor(
        group_id: &str,
        app_id: &str,
        state: &AppState,
    ) -> Result<app_group::Model, ApiError> {
        let group = app_group::Entity::find_by_id(group_id)
            .one(&state.db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;
        if group.owner_app_id != app_id {
            return Err(ApiError::FORBIDDEN);
        }
        Ok(group)
    }

    /// Suites are readable by their anchor app and by any app with an active
    /// membership; anything else falls through to the public-visibility check.
    pub async fn is_publicly_visible_group(
        group_id: &str,
        state: &AppState,
    ) -> Result<bool, ApiError> {
        let group = app_group::Entity::find_by_id(group_id)
            .one(&state.db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;
        Ok(matches!(
            group.visibility,
            Visibility::Public | Visibility::PublicRequestAccess
        ))
    }

    pub async fn ensure_read_permission(
        &self,
        user: &AppUser,
        app_id: &str,
        state: &AppState,
    ) -> Result<AppPermissionResponse, ApiError> {
        match self {
            MetaMode::Template(_) => Ok(ensure_permission!(
                user,
                app_id,
                state,
                RolePermissions::ReadTemplates
            )),
            MetaMode::Course(_) => Ok(ensure_permission!(
                user,
                app_id,
                state,
                RolePermissions::ReadCourses
            )),
            MetaMode::Widget(_) => Ok(ensure_permission!(
                user,
                app_id,
                state,
                RolePermissions::ReadWidgets
            )),
            MetaMode::App(_) => Ok(ensure_in_project!(user, &app_id, &state)),
            MetaMode::Group(group_id) => {
                let permission = ensure_permission!(user, app_id, state, RolePermissions::ReadTeam);
                let group = app_group::Entity::find_by_id(group_id)
                    .one(&state.db)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)?;
                if group.owner_app_id == app_id {
                    return Ok(permission);
                }
                let is_member = app_group_member::Entity::find()
                    .filter(app_group_member::Column::GroupId.eq(group_id))
                    .filter(app_group_member::Column::AppId.eq(app_id))
                    .filter(app_group_member::Column::Status.eq(AppGroupMemberStatus::Active))
                    .one(&state.db)
                    .await?
                    .is_some();
                if !is_member {
                    return Err(ApiError::FORBIDDEN);
                }
                Ok(permission)
            }
        }
    }

    pub async fn find_existing_meta<C: sea_orm::ConnectionTrait>(
        &self,
        language: &str,
        db: &C,
    ) -> Result<Option<meta::Model>, sea_orm::DbErr> {
        match self {
            MetaMode::Template(id) => {
                meta::Entity::find()
                    .filter(meta::Column::TemplateId.eq(id))
                    .filter(meta::Column::Lang.eq(language))
                    .one(db)
                    .await
            }
            MetaMode::App(id) => {
                meta::Entity::find()
                    .filter(meta::Column::AppId.eq(id))
                    .filter(meta::Column::Lang.eq(language))
                    .one(db)
                    .await
            }
            MetaMode::Course(id) => {
                meta::Entity::find()
                    .filter(meta::Column::CourseId.eq(id))
                    .filter(meta::Column::Lang.eq(language))
                    .one(db)
                    .await
            }
            MetaMode::Widget(id) => {
                meta::Entity::find()
                    .filter(meta::Column::WidgetId.eq(id))
                    .filter(meta::Column::Lang.eq(language))
                    .one(db)
                    .await
            }
            MetaMode::Group(id) => {
                meta::Entity::find()
                    .filter(meta::Column::GroupId.eq(id))
                    .filter(meta::Column::Lang.eq(language))
                    .one(db)
                    .await
            }
        }
    }
}

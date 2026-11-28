use axum::{
    Router,
    routing::{delete, get, put},
};
use flow_like_storage::Path as FlowPath;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::{
    auth::AppUser,
    ensure_in_project, ensure_permission,
    entity::{
        app_group, app_group_member, meta,
        sea_orm_active_enums::{AppGroupMemberStatus, Visibility},
        template, widget,
    },
    error::ApiError,
    middleware::jwt::AppPermissionResponse,
    permission::{global_permission::GlobalPermission, role_permission::RolePermissions},
    routes::{
        app::connection::deny_connected_app,
        course::access::{ensure_course_exists, ensure_course_readable},
    },
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

/// Extension charset for every key the API mints from client input: ASCII
/// alphanumerics only, lower-cased, at most 16 bytes.
pub(crate) fn sanitize_ext(input: Option<&str>) -> Option<String> {
    let ext = input?.trim().trim_start_matches('.').to_ascii_lowercase();
    if ext.is_empty() || ext.len() > 16 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(ext)
}

/// `{item_id}.{ext}` for a media upload; an extension that could smuggle path
/// or percent characters into the object key is rejected.
pub(crate) fn media_item_name(item_id: &str, extension: &str) -> Result<String, ApiError> {
    let ext = sanitize_ext(Some(extension)).ok_or_else(|| {
        ApiError::bad_request(format!("unsupported media extension '{extension}'"))
    })?;
    Ok(format!("{item_id}.{ext}"))
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
                .join("groups")
                .join(group_id.clone()),
            _ => FlowPath::from("media")
                .join("apps")
                .join(app_id.to_string()),
        }
    }

    pub async fn ensure_write_permission(
        &self,
        user: &AppUser,
        app_id: &str,
        state: &AppState,
    ) -> Result<AppPermissionResponse, ApiError> {
        let permission = match self {
            MetaMode::Template(_) => {
                ensure_permission!(user, app_id, state, RolePermissions::WriteTemplates)
            }
            MetaMode::Course(course_id) => {
                let permission =
                    ensure_permission!(user, app_id, state, RolePermissions::WriteCourses);
                Self::ensure_course_writable(user, course_id, state).await?;
                permission
            }
            MetaMode::Widget(_) => {
                ensure_permission!(user, app_id, state, RolePermissions::WriteWidgets)
            }
            MetaMode::App(_) => ensure_permission!(user, app_id, state, RolePermissions::WriteMeta),
            MetaMode::Group(group_id) => {
                deny_connected_app(user)?;
                let permission = ensure_permission!(user, app_id, state, RolePermissions::Admin);
                Self::ensure_group_anchor(group_id, app_id, state).await?;
                permission
            }
        };
        self.ensure_app_target(app_id, &state.db).await?;
        Ok(permission)
    }

    /// Courses belong to the platform, not to an app, so no app role decides who may change one:
    /// the caller needs the same global permission the course routes ask for.
    async fn ensure_course_writable(
        user: &AppUser,
        course_id: &str,
        state: &AppState,
    ) -> Result<(), ApiError> {
        user.check_global_permission(state, GlobalPermission::WriteCourses)
            .await?;
        ensure_course_exists(state, course_id).await?;
        Ok(())
    }

    /// An app role authorizes only metadata belonging to that app.
    pub(crate) async fn ensure_app_target<C: sea_orm::ConnectionTrait>(
        &self,
        app_id: &str,
        db: &C,
    ) -> Result<(), ApiError> {
        let exists = match self {
            Self::Template(id) => template::Entity::find_by_id(id)
                .filter(template::Column::AppId.eq(app_id))
                .select_only()
                .column(template::Column::Id)
                .into_tuple::<String>()
                .one(db)
                .await?
                .is_some(),
            Self::Widget(id) => widget::Entity::find_by_id(id)
                .filter(widget::Column::AppId.eq(app_id))
                .select_only()
                .column(widget::Column::Id)
                .into_tuple::<String>()
                .one(db)
                .await?
                .is_some(),
            _ => true,
        };
        if !exists {
            return Err(ApiError::NOT_FOUND);
        }
        Ok(())
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
        let permission = match self {
            MetaMode::Template(_) => {
                ensure_permission!(user, app_id, state, RolePermissions::ReadTemplates)
            }
            MetaMode::Course(course_id) => {
                let permission =
                    ensure_permission!(user, app_id, state, RolePermissions::ReadCourses);
                ensure_course_readable(state, user, course_id).await?;
                permission
            }
            MetaMode::Widget(_) => {
                ensure_permission!(user, app_id, state, RolePermissions::ReadWidgets)
            }
            MetaMode::App(_) => ensure_in_project!(user, &app_id, &state),
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
                permission
            }
        };
        self.ensure_app_target(app_id, &state.db).await?;
        Ok(permission)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn metadata_targets_must_belong_to_the_authorized_app() {
        use sea_orm::{ConnectOptions, ConnectionTrait, Database};
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        for table in ["Template", "Widget"] {
            db.execute_unprepared(&format!(
                "CREATE TABLE public.\"{table}\" (id TEXT PRIMARY KEY, \"appId\" TEXT NOT NULL)"
            ))
            .await
            .unwrap();
            db.execute_unprepared(&format!(
                "INSERT INTO public.\"{table}\" VALUES ('owned', 'app-a'), ('foreign', 'app-b')"
            ))
            .await
            .unwrap();
        }
        for target in [MetaMode::Template, MetaMode::Widget] {
            assert!(
                target("owned".into())
                    .ensure_app_target("app-a", &db)
                    .await
                    .is_ok()
            );
            assert!(
                target("foreign".into())
                    .ensure_app_target("app-a", &db)
                    .await
                    .is_err()
            );
            assert!(
                target("missing".into())
                    .ensure_app_target("app-a", &db)
                    .await
                    .is_err()
            );
            assert!(
                target("foreign".into())
                    .ensure_app_target("app-b", &db)
                    .await
                    .is_ok()
            );
        }
    }

    /// A course has no app, so `ensure_app_target` cannot tie it to the caller's app; the global
    /// course permission is what stands between an app admin and every course on the platform.
    #[test]
    fn course_metadata_asks_for_the_platform_course_permission() {
        let source = include_str!("meta.rs");
        let body_of = |name: &str| {
            let body = source
                .split_once(&format!("\n    pub async fn {name}("))
                .unwrap_or_else(|| panic!("{name} exists"))
                .1;
            &body[..body.find("\n    }\n").unwrap()]
        };
        let write = body_of("ensure_write_permission");
        let course = write
            .split_once("MetaMode::Course(course_id) =>")
            .expect("the course arm of ensure_write_permission")
            .1;
        let course = &course[..course.find("MetaMode::Widget").unwrap()];
        assert!(course.contains("Self::ensure_course_writable(user, course_id, state).await?"));
        let read = body_of("ensure_read_permission");
        let course = read
            .split_once("MetaMode::Course(course_id) =>")
            .expect("the course arm of ensure_read_permission")
            .1;
        let course = &course[..course.find("MetaMode::Widget").unwrap()];
        assert!(course.contains("ensure_course_readable(state, user, course_id).await?"));
        let writable = source
            .split_once("\n    async fn ensure_course_writable(")
            .expect("ensure_course_writable exists")
            .1;
        assert!(
            writable[..writable.find("\n    }\n").unwrap()]
                .contains("check_global_permission(state, GlobalPermission::WriteCourses)")
        );
    }

    #[test]
    fn sanitize_ext_keeps_only_ascii_alphanumerics() {
        assert_eq!(sanitize_ext(Some(" .PNG ")).as_deref(), Some("png"));
        assert_eq!(sanitize_ext(Some("webp")).as_deref(), Some("webp"));
        assert_eq!(sanitize_ext(Some("pn%g")), None);
        assert_eq!(sanitize_ext(Some("Übersicht")), None);
        assert_eq!(sanitize_ext(Some("png/../x")), None);
        assert_eq!(sanitize_ext(Some("")), None);
        assert_eq!(sanitize_ext(Some("averyverylongextension")), None);
        assert_eq!(sanitize_ext(None), None);
    }

    #[test]
    fn media_item_name_rejects_unsafe_extensions() {
        assert_eq!(media_item_name("id-1", "JPG").unwrap(), "id-1.jpg");
        assert!(media_item_name("id-1", "pn%g").is_err());
        assert!(media_item_name("id-1", "Übersicht").is_err());
        assert!(media_item_name("id-1", "").is_err());
    }
}

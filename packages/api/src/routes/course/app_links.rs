use crate::{
    entity::{course_app_link, sea_orm_active_enums::CourseAppPurpose},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::{global_permission::GlobalPermission, role_permission::RolePermissions},
    routes::course::access::{
        ensure_app_link_in_course, ensure_course_exists, ensure_course_readable,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[utoipa::path(
    get,
    path = "/courses/{course_id}/app-links",
    tag = "courses",
    params(("course_id" = String, Path, description = "Course identifier")),
    responses(
        (status = 200, description = "Lists every app link configured on a course", body = Vec<Object>)
    )
)]
#[tracing::instrument(name = "GET /courses/{course_id}/app-links", skip(state, user))]
pub async fn list_app_links(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(course_id): Path<String>,
) -> Result<Json<Vec<course_app_link::Model>>, ApiError> {
    ensure_course_readable(&state, &user, &course_id).await?;
    let rows = course_app_link::Entity::find()
        .filter(course_app_link::Column::CourseId.eq(course_id))
        .all(&state.db)
        .await?;
    Ok(Json(rows))
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct AppLinkUpsertBody {
    pub app_id: String,
    pub purpose: Option<String>,
    pub alias: Option<String>,
}

pub fn parse_purpose(s: &str) -> CourseAppPurpose {
    match s.to_uppercase().as_str() {
        "REFERENCE" => CourseAppPurpose::Reference,
        "PLAYGROUND" => CourseAppPurpose::Playground,
        _ => CourseAppPurpose::SharedTemplate,
    }
}

#[utoipa::path(
    put,
    path = "/courses/{course_id}/app-links/{link_id}",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("link_id" = String, Path, description = "App link identifier")
    ),
    request_body = AppLinkUpsertBody,
    responses(
        (status = 200, description = "App link created or updated", body = Object),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "PUT /courses/{course_id}/app-links/{link_id}",
    skip(state, user, body)
)]
pub async fn upsert_app_link(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, link_id)): Path<(String, String)>,
    Json(body): Json<AppLinkUpsertBody>,
) -> Result<Json<course_app_link::Model>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;
    let app_permission = user.app_permission(&body.app_id, &state).await?;
    if !app_permission.has_permission(RolePermissions::Admin) {
        return Err(ApiError::FORBIDDEN);
    }

    let now = chrono::Utc::now().fixed_offset();
    let purpose = parse_purpose(body.purpose.as_deref().unwrap_or("SHARED_TEMPLATE"));
    let existing = course_app_link::Entity::find_by_id(&link_id)
        .one(&state.db)
        .await?;

    let saved = if let Some(m) = existing {
        ensure_app_link_in_course(&state, &course_id, &link_id).await?;
        let mut active = m.into_active_model();
        active.app_id = Set(body.app_id);
        active.purpose = Set(purpose);
        active.alias = Set(body.alias);
        active.updated_at = Set(now);
        active.update(&state.db).await?
    } else {
        ensure_course_exists(&state, &course_id).await?;
        let active = course_app_link::ActiveModel {
            id: Set(link_id),
            course_id: Set(course_id),
            app_id: Set(body.app_id),
            purpose: Set(purpose),
            alias: Set(body.alias),
            created_at: Set(now),
            updated_at: Set(now),
        };
        active.insert(&state.db).await?
    };

    Ok(Json(saved))
}

#[utoipa::path(
    delete,
    path = "/courses/{course_id}/app-links/{link_id}",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("link_id" = String, Path, description = "App link identifier")
    ),
    responses(
        (status = 200, description = "App link deleted"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "DELETE /courses/{course_id}/app-links/{link_id}",
    skip(state, user)
)]
pub async fn delete_app_link(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, link_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WriteCourses)
        .await?;
    ensure_app_link_in_course(&state, &course_id, &link_id).await?;
    course_app_link::Entity::delete_by_id(link_id)
        .exec(&state.db)
        .await?;
    Ok(Json(()))
}

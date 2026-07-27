use crate::{
    entity::user_course_enrollment, error::ApiError, middleware::jwt::AppUser,
    routes::course::access::ensure_course_readable, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct TranslateQuery {
    pub alias: Option<String>,
    pub board: Option<String>,
    pub node: Option<String>,
    pub event: Option<String>,
    pub page: Option<String>,
    pub layer: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct TranslateResponse {
    pub course_id: String,
    pub alias: Option<String>,
    pub app_id: Option<String>,
    pub board: Option<String>,
    pub node: Option<String>,
    pub event: Option<String>,
    pub page: Option<String>,
    pub layer: Option<String>,
    /// True iff every requested ID was present in the user's id_map.
    pub fully_resolved: bool,
}

fn lookup<'a>(map: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    map.get(key).and_then(|v| v.as_str())
}

#[utoipa::path(
    get,
    path = "/courses/{course_id}/translate",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("alias" = Option<String>, Query, description = "App link alias to resolve against"),
        ("board" = Option<String>, Query, description = "Source board ID to translate"),
        ("node" = Option<String>, Query, description = "Source node ID to translate"),
        ("event" = Option<String>, Query, description = "Source event ID to translate"),
        ("page" = Option<String>, Query, description = "Source page ID to translate"),
        ("layer" = Option<String>, Query, description = "Source layer ID to translate")
    ),
    responses(
        (status = 200, description = "Translates source-app IDs to the user's forked-copy IDs using the enrollment id-map", body = TranslateResponse),
        (status = 404, description = "No enrollment / map for this user and course")
    )
)]
#[tracing::instrument(name = "GET /courses/{course_id}/translate", skip(state, user, q))]
pub async fn translate(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(course_id): Path<String>,
    Query(q): Query<TranslateQuery>,
) -> Result<Json<TranslateResponse>, ApiError> {
    let sub = user.sub()?;
    ensure_course_readable(&state, &user, &course_id).await?;
    let enrollment = user_course_enrollment::Entity::find()
        .filter(user_course_enrollment::Column::UserId.eq(&sub))
        .filter(user_course_enrollment::Column::CourseId.eq(&course_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let alias = q.alias.clone();
    let mut response = TranslateResponse {
        course_id: course_id.clone(),
        alias: alias.clone(),
        ..Default::default()
    };
    response.fully_resolved = true;

    // No alias → return originals so callers can still chain
    let Some(alias) = alias else {
        response.board = q.board;
        response.node = q.node;
        response.event = q.event;
        response.page = q.page;
        response.layer = q.layer;
        response.fully_resolved = false;
        return Ok(Json(response));
    };

    response.app_id = enrollment
        .linked_app_ids
        .as_object()
        .and_then(|m| m.get(&alias))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let Some(map_for_alias) = enrollment.id_maps.as_object().and_then(|m| m.get(&alias)) else {
        response.board = q.board;
        response.node = q.node;
        response.event = q.event;
        response.page = q.page;
        response.layer = q.layer;
        response.fully_resolved = false;
        return Ok(Json(response));
    };

    let boards = map_for_alias.get("boards").cloned().unwrap_or_default();
    let nodes = map_for_alias.get("nodes").cloned().unwrap_or_default();
    let events = map_for_alias.get("events").cloned().unwrap_or_default();
    let pages = map_for_alias.get("pages").cloned().unwrap_or_default();
    let layers = map_for_alias.get("layers").cloned().unwrap_or_default();

    let mut resolved = true;
    let translate_one = |table: &serde_json::Value,
                         requested: &Option<String>,
                         resolved: &mut bool|
     -> Option<String> {
        match requested {
            Some(src) => match lookup(table, src) {
                Some(dst) => Some(dst.to_string()),
                None => {
                    *resolved = false;
                    Some(src.clone())
                }
            },
            None => None,
        }
    };

    response.board = translate_one(&boards, &q.board, &mut resolved);
    response.node = translate_one(&nodes, &q.node, &mut resolved);
    response.event = translate_one(&events, &q.event, &mut resolved);
    response.page = translate_one(&pages, &q.page, &mut resolved);
    response.layer = translate_one(&layers, &q.layer, &mut resolved);
    response.fully_resolved = resolved;

    Ok(Json(response))
}

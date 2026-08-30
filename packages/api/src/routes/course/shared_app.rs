use crate::{
    entity::{course_app_link, sea_orm_active_enums::CourseAppPurpose, user_course_enrollment},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::course::access::ensure_course_readable,
    state::AppState,
    utils::fork::{ForkOptions, ForkTarget, fork_with_options},
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, Default)]
pub struct OpenSharedAppQuery {
    /// Force creating a fresh fork even if one is already linked.
    pub refork: Option<bool>,
    /// Language code for newly created metadata when forking. Default: en.
    pub language: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct OpenSharedAppResponse {
    pub course_id: String,
    pub alias: String,
    pub app_id: String,
    /// Source (template) app id from the course's app link.
    pub source_app_id: String,
    /// True when this enrollment had not previously linked this alias.
    pub linked_now: bool,
    /// True when a fresh fork was created during this call.
    pub forked_now: bool,
}

#[utoipa::path(
    post,
    path = "/courses/{course_id}/links/{alias}/open",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("alias" = String, Path, description = "Logical alias defined by the course's app link"),
        ("refork" = Option<bool>, Query, description = "Force a fresh fork even if one is already linked"),
        ("language" = Option<String>, Query, description = "Language code for new metadata (default en)")
    ),
    responses(
        (status = 200, description = "Returns the user-linked app id for this alias. Forks the shared template into a user-owned copy on first encounter; subsequent calls reuse the existing fork unless ?refork=true", body = OpenSharedAppResponse),
        (status = 404, description = "No app link with this alias is configured")
    )
)]
#[tracing::instrument(
    name = "POST /courses/{course_id}/links/{alias}/open",
    skip(state, user, q)
)]
pub async fn open_shared_app(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, alias)): Path<(String, String)>,
    Query(q): Query<OpenSharedAppQuery>,
) -> Result<Json<OpenSharedAppResponse>, ApiError> {
    let sub = user.sub()?;
    let now = chrono::Utc::now().naive_utc();
    let language = q.language.clone().unwrap_or_else(|| "en".to_string());
    let refork = q.refork.unwrap_or(false);
    ensure_course_readable(&state, &user, &course_id).await?;

    let link = course_app_link::Entity::find()
        .filter(course_app_link::Column::CourseId.eq(&course_id))
        .filter(course_app_link::Column::Alias.eq(&alias))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let enrollment = user_course_enrollment::Entity::find()
        .filter(user_course_enrollment::Column::UserId.eq(&sub))
        .filter(user_course_enrollment::Column::CourseId.eq(&course_id))
        .one(&state.db)
        .await?;

    let existing_link = enrollment
        .as_ref()
        .and_then(|e| e.linked_app_ids.as_object())
        .and_then(|m| m.get(&alias))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Decide whether we need to fork:
    //   - REFERENCE links never fork (the user gets a direct link to the
    //     shared app, e.g. for read-only demos).
    //   - SHARED_TEMPLATE / PLAYGROUND fork on first use, or when ?refork=true.
    let should_fork = matches!(
        link.purpose,
        CourseAppPurpose::SharedTemplate | CourseAppPurpose::Playground
    ) && (existing_link.is_none() || refork);

    let (target_app_id, fork_map, forked_now) = if should_fork {
        // Course apps used as templates are typically `Private`. This flow
        // never calls `check_can_fork`, so the user-facing `allow_forking`
        // flag is deliberately not consulted. The source owner's fork
        // policy still applies — it is loaded inside the engine.
        let options = ForkOptions {
            source_app_id: &link.app_id,
            target_user_sub: Some(&sub),
            target_mode: ForkTarget::OnlineSameStore,
            language: &language,
            remote_event_token: None,
            requested_visibility: None,
        };
        let (new_app_id, report) = fork_with_options(&state, options).await?;
        // The learner never sees a fork dialog here, so anything the engine
        // could not carry would vanish silently. Only the operator can act on
        // it, so it goes to the log rather than the response.
        if !report.skipped.is_empty() || !report.warnings.is_empty() {
            tracing::warn!(
                source_app_id = %link.app_id,
                new_app_id = %new_app_id,
                skipped = ?report.skipped,
                warnings = ?report.warnings,
                "course app fork did not carry everything"
            );
        }
        (new_app_id, Some(report.id_map), true)
    } else {
        (
            existing_link.clone().unwrap_or_else(|| link.app_id.clone()),
            None,
            false,
        )
    };

    let target_app_id_str: String = target_app_id;
    let new_id_map_value = fork_map.as_ref().and_then(|m| serde_json::to_value(m).ok());

    let (saved, was_new_link) = if let Some(e) = enrollment {
        let mut linked: serde_json::Map<String, serde_json::Value> =
            e.linked_app_ids.as_object().cloned().unwrap_or_default();
        let existed = linked.contains_key(&alias);
        linked.insert(alias.clone(), json!(target_app_id_str.clone()));

        let mut id_maps: serde_json::Map<String, serde_json::Value> =
            e.id_maps.as_object().cloned().unwrap_or_default();
        if let Some(map_value) = new_id_map_value.clone() {
            id_maps.insert(alias.clone(), map_value);
        }

        let mut active = e.into_active_model();
        active.linked_app_ids = Set(serde_json::Value::Object(linked));
        active.id_maps = Set(serde_json::Value::Object(id_maps));
        active.last_seen_at = Set(now);
        let updated = active.update(&state.db).await?;
        (updated, !existed)
    } else {
        let mut linked = serde_json::Map::new();
        linked.insert(alias.clone(), json!(target_app_id_str.clone()));
        let mut id_maps = serde_json::Map::new();
        if let Some(map_value) = new_id_map_value.clone() {
            id_maps.insert(alias.clone(), map_value);
        }
        let active = user_course_enrollment::ActiveModel {
            id: Set(create_id()),
            user_id: Set(sub),
            course_id: Set(course_id.clone()),
            linked_app_ids: Set(serde_json::Value::Object(linked)),
            id_maps: Set(serde_json::Value::Object(id_maps)),
            started_at: Set(now),
            last_seen_at: Set(now),
            completed_at: Set(None),
        };
        (active.insert(&state.db).await?, true)
    };

    let app_id = saved
        .linked_app_ids
        .as_object()
        .and_then(|m| m.get(&alias))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or(target_app_id_str);

    Ok(Json(OpenSharedAppResponse {
        course_id,
        alias,
        app_id,
        source_app_id: link.app_id,
        linked_now: was_new_link,
        forked_now,
    }))
}

use crate::{
    db::{DEFAULT_WRITE_CHUNK, insert_in_chunks},
    entity::{
        app_package, course_app_link, membership, sea_orm_active_enums::CourseAppPurpose,
        user_course_enrollment,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::course::access::ensure_course_readable,
    state::AppState,
    utils::fork::{
        ForkOptions, ForkTarget, fork_with_options,
        packages::{self, BlockedPackage, RepinnedPackage},
    },
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
    sea_query::OnConflict,
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

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct SharedAppPackagesResponse {
    /// The app the learner works in for this alias.
    pub app_id: String,
    /// Packages the course's app uses that the learner's copy still lacks,
    /// each with what stands in the way. Only some can be bought or requested.
    pub blocked_packages: Vec<BlockedPackage>,
    /// Packages this call added to the copy, because the learner has come to
    /// hold them since the copy was made.
    pub added_packages: Vec<String>,
}

impl SharedAppPackagesResponse {
    fn unchanged(app_id: &str) -> Self {
        Self {
            app_id: app_id.to_string(),
            blocked_packages: Vec::new(),
            added_packages: Vec::new(),
        }
    }
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
    let now = chrono::Utc::now().fixed_offset();
    let language = q.language.clone().unwrap_or_else(|| "en".to_string());
    let refork = q.refork.unwrap_or(false);
    ensure_course_readable(&state, &user, &course_id).await?;

    let (link, enrollment) = link_and_enrollment(&state, &sub, &course_id, &alias).await?;
    let existing_link = linked_app_id(enrollment.as_ref(), &alias);

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
        // The learner never sees a fork dialog here. Packages the copy lacks
        // are reported by `sync_shared_app_packages`, where the learner can
        // act on them; everything else the engine could not carry only the
        // operator can act on, so it goes to the log.
        if !report.skipped.is_empty() || !report.warnings.is_empty() {
            tracing::warn!(
                source_app_id = %link.app_id,
                new_app_id = %new_app_id,
                skipped = ?report.skipped,
                warnings = ?report.warnings,
                "course app fork did not carry everything"
            );
        }
        // Node / pin / layer maps run to thousands of pairs and are derivable
        // from the top-level ids; only those are persisted on the enrollment.
        (new_app_id, Some(report.id_map.top_level()), true)
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

#[utoipa::path(
    post,
    path = "/courses/{course_id}/links/{alias}/packages",
    tag = "courses",
    params(
        ("course_id" = String, Path, description = "Course identifier"),
        ("alias" = String, Path, description = "Logical alias defined by the course's app link")
    ),
    responses(
        (status = 200, description = "Brings your copy of a course app up to date with the packages the course app uses: adds the ones you have access to and lists the ones your copy still lacks, each with what stands in the way. Nothing is listed when you work in the course app itself", body = SharedAppPackagesResponse),
        (status = 404, description = "No app link with this alias is configured, or you have not opened it yet")
    )
)]
#[tracing::instrument(
    name = "POST /courses/{course_id}/links/{alias}/packages",
    skip(state, user)
)]
pub async fn sync_shared_app_packages(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((course_id, alias)): Path<(String, String)>,
) -> Result<Json<SharedAppPackagesResponse>, ApiError> {
    let sub = user.sub()?;
    ensure_course_readable(&state, &user, &course_id).await?;

    let (link, enrollment) = link_and_enrollment(&state, &sub, &course_id, &alias).await?;
    let app_id = linked_app_id(enrollment.as_ref(), &alias).ok_or(ApiError::NOT_FOUND)?;

    // Only the copy this enrollment forked from the template follows the
    // template's packages. The course's own app has nothing to catch up on,
    // and neither has the copy of an app the alias pointed to earlier.
    let forked_from_template = enrollment
        .as_ref()
        .is_some_and(|enrollment| is_fork_of(&enrollment.id_maps, &alias, &link.app_id, &app_id));
    if !forked_from_template {
        return Ok(Json(SharedAppPackagesResponse::unchanged(&app_id)));
    }

    Ok(Json(
        sync_copy_packages(&state, &user, &sub, &link.app_id, &app_id).await?,
    ))
}

/// The course's app link for `alias`, and the caller's enrollment if they
/// have one.
async fn link_and_enrollment(
    state: &AppState,
    sub: &str,
    course_id: &str,
    alias: &str,
) -> Result<
    (
        course_app_link::Model,
        Option<user_course_enrollment::Model>,
    ),
    ApiError,
> {
    let link = course_app_link::Entity::find()
        .filter(course_app_link::Column::CourseId.eq(course_id))
        .filter(course_app_link::Column::Alias.eq(alias))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let enrollment = user_course_enrollment::Entity::find()
        .filter(user_course_enrollment::Column::UserId.eq(sub))
        .filter(user_course_enrollment::Column::CourseId.eq(course_id))
        .one(&state.db)
        .await?;

    Ok((link, enrollment))
}

/// The app an enrollment works in for `alias`.
fn linked_app_id(
    enrollment: Option<&user_course_enrollment::Model>,
    alias: &str,
) -> Option<String> {
    enrollment?
        .linked_app_ids
        .as_object()?
        .get(alias)?
        .as_str()
        .map(str::to_string)
}

/// Whether the fork an enrollment recorded for `alias` made `copy_app_id` out
/// of `template_app_id`.
fn is_fork_of(
    id_maps: &serde_json::Value,
    alias: &str,
    template_app_id: &str,
    copy_app_id: &str,
) -> bool {
    let Some(fork) = id_maps.get(alias) else {
        return false;
    };
    let recorded = |field: &str| fork.get(field).and_then(serde_json::Value::as_str);
    recorded("source_app_id") == Some(template_app_id) && recorded("app_id") == Some(copy_app_id)
}

/// Whether the caller administers `app_id`. Having no role there is an answer;
/// a lookup that failed is not.
async fn manages_app(state: &AppState, user: &AppUser, app_id: &str) -> Result<bool, ApiError> {
    match user.app_permission(app_id, state).await {
        Ok(permission) => Ok(permission.has_permission(RolePermissions::Admin)),
        Err(error) if error.status() == StatusCode::FORBIDDEN => Ok(false),
        Err(error) => Err(error),
    }
}

/// The template's pins the copy does not have.
async fn missing_template_pins(
    state: &AppState,
    template_app_id: &str,
    copy_app_id: &str,
) -> Result<Vec<app_package::Model>, ApiError> {
    let pins = app_package::Entity::find()
        .filter(app_package::Column::AppId.is_in([template_app_id, copy_app_id]))
        .all(&state.db)
        .await?;
    let (template, copy): (Vec<_>, Vec<_>) = pins
        .into_iter()
        .partition(|pin| pin.app_id == template_app_id);
    Ok(packages::missing_pins(&template, &copy))
}

/// Like a fork's other shortfalls, a substituted version is the course
/// author's to fix, so it goes to the log.
fn log_repinned(template_app_id: &str, copy_app_id: &str, repinned: &[RepinnedPackage]) {
    if repinned.is_empty() {
        return;
    }
    tracing::warn!(
        source_app_id = %template_app_id,
        app_id = %copy_app_id,
        repinned = ?repinned,
        "course app copy pins another package version than its template"
    );
}

/// Keeps the learner's copy on the template's packages: pins every template
/// package the copy lacks and the learner holds (bought, or granted by its
/// author, since the copy was made) and reports the ones they don't hold.
async fn sync_copy_packages(
    state: &AppState,
    user: &AppUser,
    sub: &str,
    template_app_id: &str,
    copy_app_id: &str,
) -> Result<SharedAppPackagesResponse, ApiError> {
    let unchanged = SharedAppPackagesResponse::unchanged(copy_app_id);
    // Getting a package is only offered to someone who can pin it on the copy.
    if !manages_app(state, user, copy_app_id).await? {
        return Ok(unchanged);
    }
    let missing = missing_template_pins(state, template_app_id, copy_app_id).await?;
    if missing.is_empty() {
        return Ok(unchanged);
    }
    let split = packages::split_pins(&state.db, Some(sub), &missing).await?;
    log_repinned(template_app_id, copy_app_id, &split.repinned);
    let added_packages = add_held_pins(state, sub, copy_app_id, &split.held).await?;
    Ok(SharedAppPackagesResponse {
        blocked_packages: split.blocked,
        added_packages,
        ..unchanged
    })
}

/// Pins the packages the learner holds on their copy, with the learner as the
/// licence holder. Returns the ids it pinned.
async fn add_held_pins(
    state: &AppState,
    sub: &str,
    copy_app_id: &str,
    held: &[app_package::Model],
) -> Result<Vec<String>, ApiError> {
    if held.is_empty() {
        return Ok(Vec::new());
    }
    let Some(member) = membership::Entity::find()
        .filter(membership::Column::AppId.eq(copy_app_id))
        .filter(membership::Column::UserId.eq(sub))
        .one(&state.db)
        .await?
    else {
        return Ok(Vec::new());
    };
    let now = chrono::Utc::now().fixed_offset();
    let rows: Vec<app_package::ActiveModel> = held
        .iter()
        .map(|pin| app_package::ActiveModel {
            id: Set(create_id()),
            app_id: Set(copy_app_id.to_string()),
            membership_id: Set(Some(member.id.clone())),
            package_id: Set(pin.package_id.clone()),
            version: Set(pin.version.clone()),
            added_at: Set(now),
            auto_update: Set(pin.auto_update),
            stale: Set(false),
            stale_since: Set(None),
        })
        .collect();
    insert_in_chunks(
        &state.db,
        state.db_dialect,
        rows,
        DEFAULT_WRITE_CHUNK,
        Some(OnConflict::new().do_nothing().to_owned()),
    )
    .await?;
    Ok(held.iter().map(|pin| pin.package_id.clone()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id_maps() -> serde_json::Value {
        json!({
            "crm": { "source_app_id": "template", "app_id": "copy", "boards": {} },
            "legacy": { "boards": {} },
        })
    }

    #[test]
    fn a_copy_is_the_app_the_enrollment_forked_from_the_template() {
        assert!(is_fork_of(&id_maps(), "crm", "template", "copy"));
    }

    #[test]
    fn a_copy_of_an_app_the_alias_pointed_to_earlier_is_not_one() {
        assert!(!is_fork_of(&id_maps(), "crm", "new-template", "copy"));
    }

    #[test]
    fn an_app_the_fork_record_does_not_name_is_not_a_copy() {
        assert!(!is_fork_of(&id_maps(), "crm", "template", "template"));
        assert!(!is_fork_of(&id_maps(), "crm", "template", "another-app"));
    }

    #[test]
    fn an_alias_without_a_fork_record_has_no_copy() {
        assert!(!is_fork_of(&id_maps(), "billing", "template", "copy"));
        assert!(!is_fork_of(&id_maps(), "legacy", "template", "copy"));
        assert!(!is_fork_of(&json!(null), "crm", "template", "copy"));
    }
}

use crate::{
    audit_branch, ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::{board::VersionType, event::Event};
use serde::Deserialize;
use std::collections::HashMap;
use utoipa::ToSchema;

use super::db::{sync_event_with_sink_tokens, validate_event_schedule};
#[derive(Deserialize, ToSchema)]
pub struct EventUpsertBody {
    #[schema(value_type = Object)]
    event: Event,
    #[schema(value_type = Option<String>)]
    version_type: Option<VersionType>,
    /// Optional PAT to store with the sink (enables model/file access in triggered flows)
    #[serde(default)]
    pat: Option<String>,
    /// Optional OAuth tokens to store with the sink (provider-specific access)
    #[serde(default)]
    oauth_tokens: Option<HashMap<String, serde_json::Value>>,
    /// Optional profile ID to use for the sink (the user's currently active profile)
    #[serde(default)]
    profile_id: Option<String>,
    /// False saves a deployable event without starting a trigger or publishing endpoints on the
    /// hub (device-only). True on a stored device-only event is an explicit clear: the marker is
    /// removed and the hub trigger is registered like for any other event. Omitted keeps the
    /// stored state; a device-only event stays device-only.
    #[serde(default)]
    register_source: Option<bool>,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/events/{event_id}",
    tag = "events",
    description = "Create or update an event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = EventUpsertBody,
    responses(
        (status = 200, description = "Event saved", body = String, content_type = "application/json"),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/events/{event_id}",
    skip(state, user, params)
)]
pub async fn upsert_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(params): Json<EventUpsertBody>,
) -> Result<Json<Event>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;
    let user_context = permission.to_user_context();

    let mut event = params.event;
    event.id = event_id.clone();
    let saved_event = super::db::get_event_from_db_opt(&state.db, &event_id, &app_id).await?;
    apply_register_source(&mut event, params.register_source, saved_event.as_ref())?;
    crate::teams::management::validate_event_type(&state, saved_event.as_ref(), &event).await?;
    if event.event_type == "ontology_action"
        || saved_event
            .as_ref()
            .is_some_and(|saved| saved.event_type == "ontology_action")
    {
        return Err(ApiError::forbidden(
            "Ontology action events are managed through Data Studio actions",
        ));
    }

    let preserve_id = event.is_device_source().then_some(true);
    if preserve_id.is_some() && saved_event.is_none() {
        ensure_client_event_id_available(&state, &event_id).await?;
    }

    // Secret overrides are blanked on read, so the client sends them back empty.
    // Carry the stored values across or saving the event would wipe them.
    if let Some(saved) = saved_event.as_ref() {
        super::db::preserve_event_secrets(&mut event, saved);
    }

    // For rest/mcp events we need to know whether this upsert is a fresh
    // create (in which case a failed setup should roll back the whole
    // thing, leaving no half-broken event behind) or an update to an
    // already-working event (in which case inbound traffic keeps
    // routing to the prior `last_setup_version`).
    let existed_before =
        matches!(event.event_type.as_str(), "rest" | "mcp") && saved_event.is_some();

    validate_event_schedule(&state, &event)
        .await
        .map_err(|error| match error {
            flow_like_sinks::SchedulerError::InvalidCronExpression(message) => {
                ApiError::bad_request(message)
            }
            other => ApiError::service_unavailable(format!(
                "Failed to validate cron schedule: {}",
                other
            )),
        })?;

    let mut app = super::editable_event_app(&state, &sub, &app_id).await?;

    // Upsert to bucket (handles versioning)
    let event = app
        .upsert_event(event, params.version_type, preserve_id)
        .await?;
    let register_source = !event.is_device_source();
    app.save().await?;

    // Fetch the updater's profile for the sink (so triggers can use their bits/hubs)
    // Persisted in the sink row — never embed decrypted custom-bit secrets.
    let profile_json = crate::execution::fetch_profile_for_dispatch(
        &state,
        &sub,
        params.profile_id.as_deref(),
        &app_id,
        false,
    )
    .await;

    // Sync to database for fast lookups (also creates/updates sink and external scheduler)
    // Pass optional PAT and OAuth tokens for sink storage
    sync_event_with_sink_tokens(
        &state.db,
        &state,
        &app_id,
        &event,
        params.pat.as_deref(),
        params.oauth_tokens.as_ref(),
        profile_json,
    )
    .await?;

    audit_branch!(state, user, app_id, "event.upsert", "Event", event_id);

    // Run remote setup synchronously for event types that publish
    // registrations (REST endpoints, MCP servers). The user-facing
    // contract is "if the upsert returns 200, the event is live", so we
    // can't defer this. On failure for a fresh create we roll back the
    // whole event so we don't leave a dead /r/{slug} behind.
    if register_source && matches!(event.event_type.as_str(), "rest" | "mcp") {
        let setup_result = super::setup_event::run_event_setup(
            state.clone(),
            sub.clone(),
            app_id.clone(),
            event.id.clone(),
            super::setup_event::SetupEventRequest {
                payload: None,
                profile_id: params.profile_id.clone(),
                timeout_seconds: None,
                force: false,
                // Auto-setup always (re)builds the stable surface; a variant
                // setup is only ever an explicit POST /setup.
                variant: None,
            },
            user_context,
        )
        .await;

        match setup_result {
            Ok(resp) if resp.status == "ok" => {
                tracing::info!(
                    app_id = %app_id,
                    event_id = %event.id,
                    event_type = %event.event_type,
                    registrations = resp.registrations_written,
                    auths = resp.auths_written,
                    "auto-setup completed for event upsert"
                );
            }
            Ok(resp) => {
                let err_msg = resp
                    .error
                    .unwrap_or_else(|| "setup failed without error detail".to_string());
                rollback_failed_setup(&state, &mut app, &app_id, &event.id, existed_before).await;
                return Err(ApiError::bad_request(err_msg));
            }
            Err(err) => {
                let err_msg = err.to_string();
                rollback_failed_setup(&state, &mut app, &app_id, &event.id, existed_before).await;
                return Err(ApiError::bad_request(format!(
                    "event setup failed: {err_msg}"
                )));
            }
        }
    }

    prune_versions_after_save(&state, &app_id, &app, saved_event.as_ref(), &event).await;

    Ok(Json(event))
}

fn apply_register_source(
    event: &mut Event,
    register_source: Option<bool>,
    saved_event: Option<&Event>,
) -> Result<(), ApiError> {
    let stored_is_device = saved_event.is_some_and(Event::is_device_source);
    let outcome = match register_source {
        Some(false) => event.set_device_source(),
        Some(true) if stored_is_device => event.request_default_source(),
        _ if stored_is_device => event.ensure_source_settings(),
        _ => Ok(()),
    };
    outcome.map_err(|error| ApiError::bad_request(error.to_string()))
}

async fn ensure_client_event_id_available(
    state: &AppState,
    event_id: &str,
) -> Result<(), ApiError> {
    validate_client_event_id(event_id)?;
    if super::db::event_row_app_id(&state.db, event_id)
        .await?
        .is_some()
    {
        return Err(ApiError::conflict(
            "This event ID is already used by another event. Choose a different ID.",
        ));
    }
    Ok(())
}

const MAX_CLIENT_EVENT_ID_LEN: usize = 128;

/// A client-chosen id outlives this handler: device exports, run history and regression
/// suites all require this alphabet.
fn validate_client_event_id(event_id: &str) -> Result<(), ApiError> {
    let valid = !event_id.is_empty()
        && event_id.len() <= MAX_CLIENT_EVENT_ID_LEN
        && event_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid {
        return Ok(());
    }
    Err(ApiError::bad_request(format!(
        "Event IDs must be 1 to {MAX_CLIENT_EVENT_ID_LEN} characters of letters, digits, '-' and '_'"
    )))
}

/// Retention gate shared with the restore endpoint: only a save that actually
/// cut an archive can push the count over the cap, so identical re-saves and
/// metadata edits skip the LIST entirely.
pub(super) async fn prune_versions_after_save(
    state: &AppState,
    app_id: &str,
    app: &flow_like::app::App,
    event_before_save: Option<&Event>,
    event: &Event,
) {
    let version_was_cut =
        event_before_save.is_some_and(|before| before.event_version != event.event_version);
    if version_was_cut {
        prune_archived_versions(state, app_id, app, event).await;
    }
}

/// Retention cap for archived event versions, applied per save on the cloud
/// path only — desktop-local history lives on the user's disk and is never
/// pruned.
const MAX_EVENT_VERSIONS: usize = 200;

/// Best-effort: a failed prune must never fail the save that triggered it.
/// The version `last_setup_version` names is always protected — inbound
/// REST/MCP may still be serving its registration set.
pub(super) async fn prune_archived_versions(
    state: &AppState,
    app_id: &str,
    app: &flow_like::app::App,
    event: &flow_like::flow::event::Event,
) {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let last_setup_version = match crate::entity::event::Entity::find_by_id(&event.id)
        .filter(crate::entity::event::Column::AppId.eq(app_id))
        .one(&state.db)
        .await
    {
        Ok(row) => row.and_then(|row| row.last_setup_version),
        Err(error) => {
            tracing::warn!(
                event_id = %event.id,
                %error,
                "skipping version prune: could not read last_setup_version"
            );
            return;
        }
    };
    let protect = match last_setup_version.as_deref() {
        None => Vec::new(),
        Some(raw) => match super::parse_version_tuple(raw) {
            Some(version) => vec![version],
            // A value we cannot parse must skip the prune, not run it with an
            // empty protect set — that would delete the very version the row
            // still advertises as the last successful setup.
            None => {
                tracing::warn!(
                    event_id = %event.id,
                    last_setup_version = %raw,
                    "skipping version prune: unparseable last_setup_version"
                );
                return;
            }
        },
    };

    match event
        .prune_versions(app, MAX_EVENT_VERSIONS, &protect)
        .await
    {
        Ok(0) => {}
        Ok(deleted) => tracing::debug!(
            event_id = %event.id,
            deleted,
            "pruned archived event versions beyond the retention cap"
        ),
        Err(error) => tracing::warn!(
            event_id = %event.id,
            %error,
            "failed to prune archived event versions"
        ),
    }
}

/// Best-effort rollback when remote setup fails for a fresh rest/mcp
/// event. For an update we keep the event in place because inbound
/// traffic still routes to the prior `last_setup_version`; the failed
/// setup is recorded on the row (`setup_status = "error"`) and the
/// caller already sees the error in the API response.
async fn rollback_failed_setup(
    state: &AppState,
    app: &mut flow_like::app::App,
    app_id: &str,
    event_id: &str,
    existed_before: bool,
) {
    if existed_before {
        return;
    }
    if let Err(e) = app.delete_event(event_id).await {
        tracing::warn!(
            app_id = %app_id,
            event_id = %event_id,
            error = %e,
            "rollback: failed to delete event from bucket"
        );
    }
    if let Err(e) = app.save().await {
        tracing::warn!(
            app_id = %app_id,
            event_id = %event_id,
            error = %e,
            "rollback: failed to persist bucket cleanup"
        );
    }
    if let Err(e) = super::db::delete_event_with_sink(&state.db, state, event_id).await {
        tracing::warn!(
            app_id = %app_id,
            event_id = %event_id,
            error = %e,
            "rollback: failed to delete event from db/sink"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::FromProto;

    fn event(config: &[u8]) -> Event {
        let mut event = Event::from_proto(flow_like_types::proto::Event::default());
        event.config = config.to_vec();
        event
    }

    fn device_event() -> Event {
        let mut event = event(br#"{"expression":"0 9 * * *"}"#);
        event.set_device_source().unwrap();
        event
    }

    fn source_of(event: &Event) -> Option<serde_json::Value> {
        serde_json::from_slice::<serde_json::Value>(&event.config)
            .ok()?
            .get("__flow_like_source")
            .cloned()
    }

    #[test]
    fn register_source_false_marks_the_event_device_only() {
        let mut incoming = event(b"");
        apply_register_source(&mut incoming, Some(false), None).unwrap();
        assert!(incoming.is_device_source());
    }

    #[test]
    fn register_source_true_on_a_device_only_event_requests_an_explicit_clear() {
        let stored = device_event();
        let mut incoming = event(br#"{"expression":"0 10 * * *"}"#);
        apply_register_source(&mut incoming, Some(true), Some(&stored)).unwrap();
        assert!(!incoming.is_device_source());
        assert_eq!(source_of(&incoming), Some(serde_json::json!("default")));
    }

    #[test]
    fn register_source_true_on_an_ordinary_event_changes_nothing() {
        let stored = event(br#"{"expression":"0 9 * * *"}"#);
        let mut incoming = event(br#"{"expression":"0 10 * * *"}"#);
        let before = incoming.config.clone();
        apply_register_source(&mut incoming, Some(true), Some(&stored)).unwrap();
        assert_eq!(incoming.config, before);
    }

    #[test]
    fn omitted_register_source_leaves_the_incoming_config_alone() {
        let stored = device_event();
        let mut incoming = event(br#"{"expression":"0 10 * * *"}"#);
        let before = incoming.config.clone();
        apply_register_source(&mut incoming, None, Some(&stored)).unwrap();
        assert_eq!(incoming.config, before);
    }

    #[test]
    fn non_object_settings_for_a_device_only_event_are_a_bad_request() {
        let stored = device_event();
        for register_source in [None, Some(true)] {
            let mut incoming = event(b"[1,2]");
            let error =
                apply_register_source(&mut incoming, register_source, Some(&stored)).unwrap_err();
            assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
        }
    }

    #[test]
    fn client_event_ids_are_limited_to_the_safe_alphabet_and_length() {
        assert!(validate_client_event_id("my-event_1").is_ok());
        assert!(validate_client_event_id(&"a".repeat(128)).is_ok());
        assert!(validate_client_event_id("").is_err());
        assert!(validate_client_event_id(&"a".repeat(129)).is_err());
        assert!(validate_client_event_id("my event:1").is_err());
        assert!(validate_client_event_id("a.b").is_err());
        assert!(validate_client_event_id("ünï").is_err());
        assert_eq!(
            validate_client_event_id("a b").unwrap_err().status(),
            axum::http::StatusCode::BAD_REQUEST
        );
    }
}

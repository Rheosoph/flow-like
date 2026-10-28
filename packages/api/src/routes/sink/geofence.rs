//! Geofence door: a device that monitors a Remote location region reports each crossing here,
//! and the event runs on the hub as the person whose device saw it.

use super::trigger::{
    IdempotencyClaim, OAuthTokens, ServiceTriggerResponse, SinkUse, TriggerEventInput,
    TriggerResponse, claim_trigger_idempotency, is_inactive_project_refusal,
    publish_trigger_result, release_trigger_claim, sink_dispatch_error, trigger_event,
};
use crate::{
    ensure_fresh_permission,
    entity::event_sink,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::{app::events::db::get_event_from_db_opt, oauth::authorize_incoming_oauth_tokens},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use flow_like::flow::event::{Event, EventExecutionMode};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use utoipa::ToSchema;

const GEOLOCATION_EVENT_TYPE: &str = "geolocation";
const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";
const IDEMPOTENCY_KEY_MAX_BYTES: usize = 200;
/// The event or its trigger is switched off. The device declines the region.
const EVENT_PAUSED: &str = "EVENT_PAUSED";
/// The event's project is not active, which pauses nothing. The device keeps the region and
/// delivers the crossing again.
const PROJECT_INACTIVE: &str = "PROJECT_INACTIVE";
/// Another request is still starting the run for this crossing. The device delivers it again.
const IDEMPOTENCY_KEY_IN_USE: &str = "IDEMPOTENCY_KEY_IN_USE";
const NOT_A_REMOTE_GEOFENCE: &str = "This event is not a Remote location event";
const TRIGGERED: &str = "Event triggered successfully";

/// A crossing of a Remote location region, as the device recorded it.
#[derive(Debug, Deserialize, ToSchema)]
pub struct GeofenceTriggerRequest {
    /// The crossing; the run receives it as its payload.
    pub transition: serde_json::Value,
    /// The caller's own OAuth sign-ins, keyed by provider id, in the form an event
    /// invocation takes them: `access_token`, and `refresh_token` as a connection handle
    /// (`fl_oauth_…`) the caller owns or a raw refresh token, `expires_at` and `token_type`.
    /// The run uses each for its provider and the sign-ins saved with the event for every
    /// other provider.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub oauth_tokens: Option<OAuthTokens>,
}

/// POST /sink/trigger/geofence/{event_id}
///
/// Starts a Remote location event for a crossing a device saw. The caller needs ExecuteEvents in
/// the event's app, and the run executes as the caller: their rights, profile and private Bits,
/// never the profile saved with the event. It signs in with the OAuth tokens the device sent, and
/// with the event's saved sign-ins for any other provider. A repeat with the same
/// `Idempotency-Key` gets the first answer instead of a second run.
#[utoipa::path(
    post,
    path = "/sink/trigger/geofence/{event_id}",
    tag = "sink",
    description = "Start a Remote location event for a crossing a device saw. The caller needs ExecuteEvents in the event's app, and the event runs as the caller, with their own profile and private models, never the profile saved with the event. It signs in to services with the OAuth sign-ins the request sends and, for any other service, with the sign-ins saved with the event. A pause because the person the event runs as lost access does not stop crossings; switching the event or its trigger off does. A repeat with the same Idempotency-Key within about 15 minutes gets the first answer instead of a second run.",
    params(
        ("event_id" = String, Path, description = "The Remote location event whose region was crossed"),
        ("Idempotency-Key" = String, Header, description = "The crossing's id: 1 to 200 bytes without control characters")
    ),
    request_body = GeofenceTriggerRequest,
    responses(
        (status = 200, description = "The crossing started a run", body = TriggerResponse),
        (status = 400, description = "Missing or invalid Idempotency-Key, a transition that is not a JSON object, or an OAuth sign-in that must be connected again (oauth_reauthorization_required)"),
        (status = 401, description = "Missing, invalid, expired or revoked credential"),
        (status = 402, description = "The billing account cannot start this run now; retry later"),
        (status = 403, description = "The caller is not a person or lacks ExecuteEvents in the event's app"),
        (status = 404, description = "No Remote location event has this id"),
        (status = 409, description = "EVENT_PAUSED: the event or its trigger is switched off. PROJECT_INACTIVE: the event's project is not active; send the crossing again later. IDEMPOTENCY_KEY_IN_USE: another request with this key is still starting the run; retry shortly"),
        (status = 500, description = "Internal server error"),
        (status = 503, description = "SINK_RUN_AS_UNAVAILABLE: the event or its trigger changed while the crossing started its run. Also when the run could not be dispatched; retry later")
    ),
    security(
        ("bearer_auth" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /sink/trigger/geofence/{event_id}",
    skip(state, user, headers, request)
)]
pub async fn trigger_geofence(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(event_id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<GeofenceTriggerRequest>,
) -> Result<Json<TriggerResponse>, ApiError> {
    let caller = user.sub()?;
    let crossing = crossing_key(&headers)?;
    if !request.transition.is_object() {
        return Err(ApiError::bad_request(
            "The transition must be a JSON object",
        ));
    }
    let sink = geofence_sink(&state, &event_id).await?;
    ensure_fresh_permission!(user, &sink.app_id, &state, RolePermissions::ExecuteEvents);
    let event = remote_geofence(&state, &sink).await?;

    let key = geofence_idempotency_key(&caller, &sink.app_id, &sink.event_id, crossing);
    if let Some(cached) = state.trigger_idempotency.get(&key) {
        return replayed(cached).map(Json);
    }
    let claim = claim_trigger_idempotency(&state, &key, &sink.event_id).await;
    if let IdempotencyClaim::Replay(response) = claim {
        return replayed(response).map(Json);
    }

    tracing::info!(
        event_id = %sink.event_id,
        app_id = %sink.app_id,
        "Geofence crossing: triggering event"
    );
    let input = TriggerEventInput {
        event_id: sink.event_id.clone(),
        payload: Some(request.transition),
        idempotency_key: Some(key.clone()),
        actor: Some(caller),
        oauth_tokens: request.oauth_tokens,
    };
    match start_crossing(&state, &user, sink, event, input).await {
        Ok(run_id) => {
            remember(&state, &claim, &key, &run_id).await;
            Ok(Json(triggered(run_id)))
        }
        Err(error) => {
            release_trigger_claim(&claim, Some(key.as_str())).await;
            Err(error)
        }
    }
}

/// Starts a crossing's run once the sign-ins the caller sent with it resolve as theirs: a
/// connection handle they own, or a refresh token adopted for them.
async fn start_crossing(
    state: &AppState,
    user: &AppUser,
    sink: event_sink::Model,
    event: Event,
    mut input: TriggerEventInput,
) -> Result<String, ApiError> {
    authorize_incoming_oauth_tokens(state, user, &mut input.oauth_tokens).await?;
    started(trigger_event(state, sink, event, input).await)
}

/// The crossing's id from `Idempotency-Key`: 1 to 200 bytes of UTF-8 without control characters.
fn crossing_key(headers: &HeaderMap) -> Result<&str, ApiError> {
    let value = headers.get(IDEMPOTENCY_KEY_HEADER).ok_or_else(|| {
        ApiError::bad_request("An Idempotency-Key header with the crossing's id is required")
    })?;
    std::str::from_utf8(value.as_bytes())
        .ok()
        .filter(|key| {
            !key.is_empty()
                && key.len() <= IDEMPOTENCY_KEY_MAX_BYTES
                && !key.chars().any(char::is_control)
        })
        .ok_or_else(|| {
            ApiError::bad_request(
                "The Idempotency-Key must be 1 to 200 bytes of text without control characters",
            )
        })
}

/// The event's sink, on or off so that a paused event answers 409 rather than 404. Nothing about
/// the event is read until the caller's permission in the sink's app is checked.
async fn geofence_sink(state: &AppState, event_id: &str) -> Result<event_sink::Model, ApiError> {
    event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(event_id))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found(NOT_A_REMOTE_GEOFENCE))
}

/// The sink's event, once [`geofence_target`] accepts it and the sink. A crossing runs as its
/// person, so a sink a release paused still takes it.
async fn remote_geofence(state: &AppState, sink: &event_sink::Model) -> Result<Event, ApiError> {
    let event = get_event_from_db_opt(&state.db, &sink.event_id, &sink.app_id)
        .await?
        .ok_or_else(|| ApiError::not_found(NOT_A_REMOTE_GEOFENCE))?;
    geofence_target(&event, SinkUse::Person.admits(sink))?;
    Ok(event)
}

/// Only a Remote location event is this door's to run, and only while it is on and its trigger
/// takes runs a person triggered.
fn geofence_target(event: &Event, sink_takes_crossings: bool) -> Result<(), ApiError> {
    if event.event_type != GEOLOCATION_EVENT_TYPE
        || event.execution_mode != EventExecutionMode::Remote
    {
        return Err(ApiError::not_found(NOT_A_REMOTE_GEOFENCE));
    }
    if !(event.active && sink_takes_crossings) {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            EVENT_PAUSED,
            "This event is paused",
        ));
    }
    Ok(())
}

/// One crossing as one caller reported it. Scoped like the service door's keys, with the
/// caller in place of the service credential, so nobody else's identical header replays it.
fn geofence_idempotency_key(caller: &str, app_id: &str, event_id: &str, crossing: &str) -> String {
    let scope = serde_json::json!(["geofence", caller, app_id, event_id, crossing]);
    format!(
        "geofence:{}",
        blake3::hash(scope.to_string().as_bytes()).to_hex()
    )
}

/// The run id of a crossing that started a run. A billing refusal keeps its answer, so the device
/// backs off. An inactive project, the one refusal a crossing run as its person meets, answers
/// 409 `PROJECT_INACTIVE`, and the device keeps the region. A change of the event while the run
/// started keeps its retryable answer, and any other failure asks the device to deliver the
/// crossing again.
fn started(outcome: flow_like_types::Result<TriggerResponse>) -> Result<String, ApiError> {
    match outcome {
        Ok(TriggerResponse {
            triggered: true,
            run_id: Some(run_id),
            ..
        }) => Ok(run_id),
        Ok(response) => Err(ApiError::service_unavailable(response.message)),
        Err(error) => match error.downcast_ref::<ApiError>() {
            Some(billing) if billing.status() == StatusCode::PAYMENT_REQUIRED => {
                Err(billing.clone())
            }
            Some(refusal) if is_inactive_project_refusal(refusal) => Err(ApiError::coded(
                StatusCode::CONFLICT,
                PROJECT_INACTIVE,
                "This event's project is not active; send the crossing again later",
            )),
            _ => Err(sink_dispatch_error(error)),
        },
    }
}

/// The answer to a repeat of a crossing that already started a run, or that another request is
/// still starting.
fn replayed(response: ServiceTriggerResponse) -> Result<TriggerResponse, ApiError> {
    match response {
        ServiceTriggerResponse {
            success: true,
            run_id: Some(run_id),
            ..
        } => Ok(triggered(run_id)),
        _ => Err(ApiError::coded(
            StatusCode::CONFLICT,
            IDEMPOTENCY_KEY_IN_USE,
            "This crossing is still starting its run; send it again shortly",
        )),
    }
}

fn triggered(run_id: String) -> TriggerResponse {
    TriggerResponse {
        triggered: true,
        run_id: Some(run_id),
        message: TRIGGERED.to_owned(),
    }
}

/// Keeps a started run for repeats of the same crossing, on this replica and on the others.
async fn remember(state: &AppState, claim: &IdempotencyClaim, key: &str, run_id: &str) {
    let response = ServiceTriggerResponse {
        success: true,
        run_id: Some(run_id.to_owned()),
        error: None,
        duplicate: false,
        skipped: None,
    };
    state
        .trigger_idempotency
        .insert(key.to_owned(), response.clone());
    if let IdempotencyClaim::Owned(cache) = claim {
        publish_trigger_result(cache, key, &response).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::sink::authority::RunAsDenial;
    use crate::routes::sink::trigger::{SINK_RUN_AS_UNAVAILABLE, run_as_refused};
    use axum::http::HeaderValue;

    fn event(event_type: &str, execution_mode: EventExecutionMode, active: bool) -> Event {
        Event {
            id: "event-1".to_string(),
            name: "Arrive home".to_string(),
            description: String::new(),
            board_id: "board-1".to_string(),
            board_version: None,
            node_id: "node-1".to_string(),
            variables: Default::default(),
            config: Vec::new(),
            active,
            canary: None,
            variants: Vec::new(),
            priority: 0,
            event_type: event_type.to_string(),
            notes: None,
            event_version: (1, 0, 0),
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
            default_page_id: None,
            inputs: Vec::new(),
            route: None,
            is_default: false,
            execution_mode,
            exposure: Default::default(),
            correlation_mappings: None,
        }
    }

    fn answer(error: &ApiError) -> (StatusCode, &str) {
        (error.status(), error.public_code())
    }

    #[test]
    fn only_a_remote_location_event_that_is_on_takes_crossings() {
        use EventExecutionMode::{Local, Remote};
        let not_found = Some((StatusCode::NOT_FOUND, "NOT_FOUND"));
        let paused = Some((StatusCode::CONFLICT, EVENT_PAUSED));
        let cases = [
            ("geolocation", Remote, true, true, None),
            ("geolocation", Remote, true, false, paused),
            ("geolocation", Remote, false, true, paused),
            ("geolocation", Remote, false, false, paused),
            ("geolocation", Local, true, true, not_found),
            ("geolocation", Local, false, false, not_found),
            ("http", Remote, true, true, not_found),
            ("webhook", Remote, true, true, not_found),
            ("cron", Remote, true, true, not_found),
            ("deeplink", Remote, true, true, not_found),
            ("Geolocation", Remote, true, true, not_found),
            ("", Remote, true, true, not_found),
        ];
        for (event_type, mode, event_active, sink_active, expected) in cases {
            let target = geofence_target(&event(event_type, mode, event_active), sink_active);
            assert_eq!(
                target.as_ref().err().map(answer),
                expected,
                "{event_type} {mode:?} event_active={event_active} sink_active={sink_active}"
            );
        }
    }

    #[test]
    fn a_crossing_replays_only_for_the_same_caller_app_event_and_key() {
        let key = geofence_idempotency_key("ada", "app-a", "event-a", "crossing-1");
        let cache = moka::sync::Cache::new(10);
        cache.insert(key.clone(), "run-a");
        assert_eq!(
            cache.get(&geofence_idempotency_key(
                "ada",
                "app-a",
                "event-a",
                "crossing-1"
            )),
            Some("run-a")
        );
        for (caller, app, event, crossing) in [
            ("grace", "app-a", "event-a", "crossing-1"),
            ("ada", "app-b", "event-a", "crossing-1"),
            ("ada", "app-a", "event-b", "crossing-1"),
            ("ada", "app-a", "event-a", "crossing-2"),
            ("", "app-a", "event-a", "crossing-1"),
        ] {
            assert!(
                cache
                    .get(&geofence_idempotency_key(caller, app, event, crossing))
                    .is_none(),
                "{caller} {app} {event} {crossing}"
            );
        }
        assert_ne!(
            geofence_idempotency_key("ada\",\"app-a", "event-a", "crossing-1", ""),
            geofence_idempotency_key("ada", "app-a\",\"event-a", "crossing-1", ""),
            "parts cannot run into each other"
        );
        assert!(key.starts_with("geofence:"));
    }

    #[test]
    fn the_idempotency_key_is_required_short_and_printable() {
        let mut headers = HeaderMap::new();
        assert_eq!(
            crossing_key(&headers).err().as_ref().map(answer),
            Some((StatusCode::BAD_REQUEST, "BAD_REQUEST"))
        );
        let uuid = "6F9619FF-8B86-D011-B42D-00C04FC964FF";
        let longest = "k".repeat(IDEMPOTENCY_KEY_MAX_BYTES);
        let unicode = "ü".repeat(IDEMPOTENCY_KEY_MAX_BYTES / 2);
        for accepted in [uuid, longest.as_str(), unicode.as_str()] {
            headers.insert(
                IDEMPOTENCY_KEY_HEADER,
                HeaderValue::from_bytes(accepted.as_bytes()).unwrap(),
            );
            assert_eq!(crossing_key(&headers).ok(), Some(accepted));
        }
        let too_long = "k".repeat(IDEMPOTENCY_KEY_MAX_BYTES + 1);
        let too_long_unicode = "ü".repeat(IDEMPOTENCY_KEY_MAX_BYTES / 2 + 1);
        let refusals: [&[u8]; 6] = [
            b"",
            too_long.as_bytes(),
            too_long_unicode.as_bytes(),
            b"crossing\t1",
            "crossing\u{85}1".as_bytes(),
            b"\xff\xfe",
        ];
        for refused in refusals {
            headers.insert(
                IDEMPOTENCY_KEY_HEADER,
                HeaderValue::from_bytes(refused).unwrap(),
            );
            assert_eq!(
                crossing_key(&headers).err().as_ref().map(answer),
                Some((StatusCode::BAD_REQUEST, "BAD_REQUEST")),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn a_crossing_without_a_run_keeps_its_answer_or_asks_for_a_retry() {
        let started_run = started(Ok(TriggerResponse {
            triggered: true,
            run_id: Some("run-1".into()),
            message: TRIGGERED.into(),
        }));
        assert_eq!(started_run.ok().as_deref(), Some("run-1"));

        let not_dispatched = started(Ok(TriggerResponse {
            triggered: false,
            run_id: Some("run-2".into()),
            message: "Dispatch failed: queue down".into(),
        }));
        assert_eq!(
            not_dispatched.err().as_ref().map(answer),
            Some((StatusCode::SERVICE_UNAVAILABLE, "SERVICE_UNAVAILABLE"))
        );

        let overtaken = ApiError::coded(
            StatusCode::SERVICE_UNAVAILABLE,
            SINK_RUN_AS_UNAVAILABLE,
            "Try again later",
        );
        let billing = ApiError::payment_required("No runs left this month");
        for kept in [overtaken, billing] {
            let outcome = started(Err(flow_like_types::Error::from(kept.clone())));
            assert_eq!(outcome.err().as_ref().map(answer), Some(answer(&kept)));
        }

        for internal in [
            flow_like_types::anyhow!("database is gone"),
            flow_like_types::Error::from(ApiError::forbidden("not a run-as answer")),
            flow_like_types::Error::from(ApiError::bad_request("not a run-as answer")),
        ] {
            assert_eq!(
                started(Err(internal)).err().map(|error| error.status()),
                Some(StatusCode::INTERNAL_SERVER_ERROR)
            );
        }
    }

    /// A crossing runs as its person, so nothing about the event's run-as user refuses it. The
    /// project still has to be active; that refusal paused nothing, so the device keeps the
    /// region and delivers the crossing again.
    #[test]
    fn an_inactive_project_keeps_the_region() {
        let refusal = run_as_refused(RunAsDenial::AppInactive);
        let outcome = started(Err(flow_like_types::Error::from(refusal)));
        assert_eq!(
            outcome.err().as_ref().map(answer),
            Some((StatusCode::CONFLICT, PROJECT_INACTIVE))
        );
    }

    #[test]
    fn a_repeat_gets_the_first_run_or_waits_for_it() {
        let done = ServiceTriggerResponse {
            success: true,
            run_id: Some("run-1".into()),
            error: None,
            duplicate: false,
            skipped: None,
        };
        let replay = replayed(done).expect("a started run replays");
        assert!(replay.triggered);
        assert_eq!(replay.run_id.as_deref(), Some("run-1"));
        assert_eq!(replay.message, TRIGGERED);

        let in_flight = ServiceTriggerResponse {
            success: true,
            run_id: None,
            error: None,
            duplicate: true,
            skipped: None,
        };
        let failed = ServiceTriggerResponse {
            success: false,
            run_id: Some("run-2".into()),
            error: Some("Dispatch failed".into()),
            duplicate: false,
            skipped: None,
        };
        for pending in [in_flight, failed] {
            assert_eq!(
                replayed(pending).err().as_ref().map(answer),
                Some((StatusCode::CONFLICT, IDEMPOTENCY_KEY_IN_USE))
            );
        }
    }

    /// Axum panics on an overlapping route or method while the router is built, so a second
    /// POST at the door's path panics only when the sink router already routes crossings there.
    #[test]
    fn the_sink_router_builds_and_routes_crossings_by_post() {
        const DOOR: &str = "/trigger/geofence/{event_id}";
        let _ = crate::routes::sink::routes(1024);
        let second_door = std::panic::catch_unwind(|| {
            crate::routes::sink::routes(1024).route(DOOR, axum::routing::post(|| async {}))
        });
        let Err(overlap) = second_door else {
            panic!("the sink router must route POST {DOOR}");
        };
        let message = overlap
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| overlap.downcast_ref::<&str>().copied())
            .unwrap_or_default();
        assert!(message.contains(&format!("POST {DOOR}")), "{message}");
    }

    fn body<'a>(source: &'a str, signature: &str) -> &'a str {
        let body = source
            .split_once(signature)
            .unwrap_or_else(|| panic!("{signature} exists"))
            .1;
        body.split_once("\n}\n").map_or(body, |(body, _)| body)
    }

    /// The caller is a person with a valid key and may trigger the events of the sink's app
    /// before anything about the event is read, so a non-member cannot tell whether an event
    /// is a Remote location event or whether it is on. Then the event must be a Remote
    /// location event that is on, all before a replay or a run. Only then is the crossing
    /// claimed and the run started as that person, so the event's run-as user is never
    /// checked and its trigger never paused for a crossing.
    #[test]
    fn every_check_runs_before_a_replay_or_a_run() {
        let source = include_str!("geofence.rs");
        let handler = body(source, "\npub async fn trigger_geofence(");
        let needles = [
            "let caller = user.sub()?;",
            "crossing_key(&headers)?",
            "request.transition.is_object()",
            "geofence_sink(&state, &event_id).await?",
            "ensure_fresh_permission!(user, &sink.app_id, &state, RolePermissions::ExecuteEvents)",
            "remote_geofence(&state, &sink).await?",
            "state.trigger_idempotency.get(&key)",
            "claim_trigger_idempotency(&state, &key, &sink.event_id)",
            "actor: Some(caller),",
            "oauth_tokens: request.oauth_tokens,",
            "start_crossing(&state, &user, sink, event, input)",
            "remember(&state, &claim, &key, &run_id)",
            "release_trigger_claim(&claim, Some(key.as_str()))",
        ];
        let positions = needles
            .iter()
            .map(|needle| {
                handler
                    .find(needle)
                    .unwrap_or_else(|| panic!("trigger_geofence calls {needle}"))
            })
            .collect::<Vec<_>>();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "trigger_geofence must run {needles:?} in this order"
        );
        assert!(!handler.contains("resolve_sink_run_as"));
        assert!(!handler.contains("trigger_event("));
        let start = body(source, "\nasync fn start_crossing(");
        let resolved = start
            .find("authorize_incoming_oauth_tokens(state, user, &mut input.oauth_tokens)")
            .expect("the caller's sign-ins resolve as theirs");
        let fired = start
            .find("trigger_event(state, sink, event, input)")
            .expect("the crossing fires");
        assert!(resolved < fired, "sign-ins resolve before the run starts");

        let sink = body(source, "\nasync fn geofence_sink(");
        assert!(sink.contains("event_sink::Entity::find()"));
        for event_read in ["get_event_from_db_opt(", "geofence_target(", ".active"] {
            assert!(
                !sink.contains(event_read),
                "the sink lookup reads {event_read} before the permission check"
            );
        }
        assert!(
            !sink.contains("Column::Active"),
            "a paused sink is a 409, not a 404"
        );

        let event = body(source, "\nasync fn remote_geofence(");
        let read = event.find("get_event_from_db_opt(").unwrap();
        let target = event
            .find("geofence_target(&event, SinkUse::Person.admits(sink))?")
            .unwrap();
        assert!(read < target);
    }

    fn sink(active: bool, released: bool) -> event_sink::Model {
        let now = chrono::Utc::now().fixed_offset();
        event_sink::Model {
            id: "sink-1".into(),
            event_id: "event-1".into(),
            app_id: "app-1".into(),
            sink_type: GEOLOCATION_EVENT_TYPE.into(),
            active,
            path: None,
            auth_token: None,
            webhook_secret: None,
            cron_expression: None,
            cron_timezone: None,
            pat_encrypted: None,
            oauth_tokens_encrypted: None,
            profile_json: None,
            created_at: now,
            updated_at: now,
            method: None,
            run_as_user_id: (!released).then(|| "run-as-1".into()),
            run_as_released_at: released.then_some(now),
        }
    }

    /// A crossing runs as the person whose device saw it, so a pause because the user the
    /// event runs as lost their standing does not stop it. A trigger a person switched off
    /// does, and so does an event that is off, whatever paused its trigger.
    #[test]
    fn only_a_switch_off_stops_crossings_and_a_release_does_not() {
        let paused = Some((StatusCode::CONFLICT, EVENT_PAUSED));
        for (case, event_active, sink, expected) in [
            ("on", true, sink(true, false), None),
            ("released", true, sink(false, true), None),
            ("switched off", true, sink(false, false), paused),
            (
                "event off, trigger released",
                false,
                sink(false, true),
                paused,
            ),
            ("event off, trigger on", false, sink(true, false), paused),
        ] {
            let crossing = event("geolocation", EventExecutionMode::Remote, event_active);
            let target = geofence_target(&crossing, SinkUse::Person.admits(&sink));
            assert_eq!(target.as_ref().err().map(answer), expected, "{case}");
        }
    }

    /// A device may send its own sign-ins with a crossing, or none. The order test above
    /// checks that they reach the fire only once they resolved as the caller's.
    #[test]
    fn a_crossing_may_carry_the_callers_sign_ins() {
        let request: GeofenceTriggerRequest = serde_json::from_value(serde_json::json!({
            "transition": {"kind": "enter"},
            "oauth_tokens": {"google": {"access_token": "a", "refresh_token": "fl_oauth_x"}},
        }))
        .unwrap();
        let tokens = request.oauth_tokens.expect("the sign-ins are read");
        assert_eq!(tokens["google"]["refresh_token"], "fl_oauth_x");
        let bare: GeofenceTriggerRequest =
            serde_json::from_value(serde_json::json!({"transition": {}})).unwrap();
        assert!(bare.oauth_tokens.is_none());
    }
}

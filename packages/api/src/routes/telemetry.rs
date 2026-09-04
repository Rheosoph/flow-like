//! Anonymous product telemetry ingest.
//!
//! PRIVACY INVARIANT: handlers in this module are anonymous by construction.
//! They must never extract `Extension(AppUser)` and never store user identity
//! or IP addresses — only the random, client-generated `anon_id`.

use axum::{
    Json, Router,
    extract::State,
    http::HeaderMap,
    routing::{get, post},
};
use flow_like_types::Value;
use sea_orm::{EntityTrait, Set};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{entity::telemetry_event, error::ApiError, state::AppState, telemetry::sink_from_env};

pub mod config;
pub mod errors;
pub mod fingerprint;
pub mod llm;
pub mod performance;
pub mod sessions;
pub mod spans;

const MAX_EVENTS_PER_BATCH: usize = 50;
const MAX_PROPS_BYTES: usize = 8192;
const MAX_ANON_ID_LEN: usize = 64;
/// Synthetic anon id used for server-side events; clients must never claim it.
const RESERVED_ANON_ID: &str = "backend";

const COUNTRY_HEADERS: [&str; 4] = [
    "cloudfront-viewer-country",
    "cf-ipcountry",
    "x-vercel-ip-country",
    "x-country-code",
];

/// Keys whose values are replaced before storage. Credentials keep secrets out
/// of the telemetry store; the identity keys make the anonymity guarantee hold
/// even against a buggy or hostile client that puts identity into its own batch.
const REDACTED_KEYS: [&str; 16] = [
    "password",
    "passwd",
    "pwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "access_token",
    "refresh_token",
    "sub",
    "user_id",
    "userid",
    "email",
    "username",
    "ip",
    "ip_address",
];

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/events", post(ingest_events))
        .route("/errors", post(errors::ingest_errors))
        .route("/sessions", post(sessions::ingest_sessions))
        .route("/spans", post(spans::ingest_spans))
        .route("/performance", post(performance::ingest_performance))
        .route("/llm", post(llm::ingest_llm_calls))
        .route("/config", get(config::telemetry_config))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetryEventPayload {
    /// Event name, must match `^[a-z0-9_.:-]{1,128}$`.
    pub name: String,
    #[serde(default)]
    pub props: Option<Value>,
    /// Client-side timestamp (RFC 3339). Invalid values are stored as null.
    #[serde(default)]
    pub client_ts: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetryIngestPayload {
    /// Random client-generated identifier, 1-64 characters. Never a user id.
    pub anon_id: String,
    /// Origin of the batch: "desktop", "web", "desktop_core" or "backend".
    pub source: String,
    #[serde(default)]
    pub app_version: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    /// Up to 50 events per batch.
    pub events: Vec<TelemetryEventPayload>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TelemetryIngestResponse {
    pub accepted: usize,
}

fn validate_anon_id(anon_id: &str) -> Result<(), ApiError> {
    if anon_id.is_empty() || anon_id.len() > MAX_ANON_ID_LEN {
        return Err(ApiError::bad_request(format!(
            "anon_id must be between 1 and {} characters",
            MAX_ANON_ID_LEN
        )));
    }
    if anon_id == RESERVED_ANON_ID {
        return Err(ApiError::bad_request(format!(
            "anon_id '{}' is reserved for server-side events",
            RESERVED_ANON_ID
        )));
    }
    Ok(())
}

fn is_valid_event_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | ':' | '-')
        })
}

fn sanitize_props(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, entry) in map.iter_mut() {
                if REDACTED_KEYS.contains(&key.to_ascii_lowercase().as_str()) {
                    *entry = Value::String("[REDACTED]".to_string());
                } else {
                    sanitize_props(entry);
                }
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                sanitize_props(item);
            }
        }
        _ => {}
    }
}

struct ValidatedEvent {
    name: String,
    props: Option<Value>,
    client_ts: Option<chrono::DateTime<chrono::FixedOffset>>,
}

/// Derives an ISO 3166-1 alpha-2 country code from proxy geolocation headers,
/// in priority order. Placeholder values ("XX", "T1") and anything that is not
/// exactly two ASCII letters are skipped. The client IP is never read.
fn country_from_headers(headers: &HeaderMap) -> Option<String> {
    COUNTRY_HEADERS.iter().find_map(|name| {
        let raw = headers.get(*name)?.to_str().ok()?.trim();
        if raw.len() != 2 || !raw.chars().all(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        let code = raw.to_ascii_uppercase();
        if code == "XX" || code == "T1" {
            return None;
        }
        Some(code)
    })
}

/// Normalised to UTC so the client's offset never leaks into the stored value —
/// a later `date_naive()` on it must read the UTC calendar day, not theirs.
fn parse_client_ts(raw: Option<&str>) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(raw?.trim())
        .map(|dt| dt.to_utc().fixed_offset())
        .ok()
}

fn validate_events(events: Vec<TelemetryEventPayload>) -> Vec<ValidatedEvent> {
    events
        .into_iter()
        .filter_map(|event| {
            if !is_valid_event_name(&event.name) {
                return None;
            }
            let props = match event.props {
                Some(mut props) => {
                    sanitize_props(&mut props);
                    let serialized = serde_json::to_vec(&props)
                        .map(|bytes| bytes.len())
                        .unwrap_or(usize::MAX);
                    if serialized > MAX_PROPS_BYTES {
                        return None;
                    }
                    Some(props)
                }
                None => None,
            };
            Some(ValidatedEvent {
                name: event.name,
                props,
                client_ts: parse_client_ts(event.client_ts.as_deref()),
            })
        })
        .collect()
}

/// Anonymous by construction: this handler intentionally never extracts
/// `Extension(AppUser)` and never persists user identity or IP addresses.
/// The stored country is derived exclusively from proxy geolocation headers
/// (CloudFront/Cloudflare/Vercel); the client IP is never read or stored.
#[utoipa::path(
    post,
    path = "/telemetry/events",
    tag = "telemetry",
    request_body = TelemetryIngestPayload,
    responses(
        (status = 200, description = "Number of telemetry events that were accepted", body = TelemetryIngestResponse),
        (status = 400, description = "Invalid batch"),
        (status = 404, description = "Telemetry is disabled on this platform")
    ),
    description = "Submit a batch of anonymous, opt-in product telemetry events. No account, user identity or IP address is ever stored — only a random client-generated identifier."
)]
#[tracing::instrument(name = "POST /telemetry/events", skip(state, headers, payload))]
pub async fn ingest_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<TelemetryIngestPayload>,
) -> Result<Json<TelemetryIngestResponse>, ApiError> {
    if !state.platform_config.features.telemetry {
        return Err(ApiError::NOT_FOUND);
    }

    validate_anon_id(&payload.anon_id)?;

    if !matches!(
        payload.source.as_str(),
        "desktop" | "web" | "desktop_core" | "backend"
    ) {
        return Err(ApiError::bad_request(format!(
            "Unknown telemetry source '{}'",
            payload.source
        )));
    }

    if payload.events.len() > MAX_EVENTS_PER_BATCH {
        return Err(ApiError::bad_request(format!(
            "A telemetry batch may contain at most {} events",
            MAX_EVENTS_PER_BATCH
        )));
    }

    let validated = validate_events(payload.events);
    if validated.is_empty() {
        return Ok(Json(TelemetryIngestResponse { accepted: 0 }));
    }

    let sink = sink_from_env();

    if sink == "none" {
        return Ok(Json(TelemetryIngestResponse {
            accepted: validated.len(),
        }));
    }

    if sink == "log" {
        tracing::info!(
            source = %payload.source,
            anon_id = %payload.anon_id,
            app_version = payload.app_version.as_deref().unwrap_or(""),
            platform = payload.platform.as_deref().unwrap_or(""),
            events = validated.len(),
            names = ?validated.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            "telemetry batch"
        );
        return Ok(Json(TelemetryIngestResponse {
            accepted: validated.len(),
        }));
    }

    let now = chrono::Utc::now().fixed_offset();
    let accepted = validated.len();
    let country = country_from_headers(&headers);
    let models: Vec<telemetry_event::ActiveModel> = validated
        .into_iter()
        .map(|event| telemetry_event::ActiveModel {
            id: Set(flow_like_types::create_id()),
            name: Set(event.name),
            source: Set(payload.source.clone()),
            anon_id: Set(payload.anon_id.clone()),
            props: Set(event.props),
            app_version: Set(payload.app_version.clone()),
            platform: Set(payload.platform.clone()),
            country: Set(country.clone()),
            client_ts: Set(event.client_ts),
            created_at: Set(now),
        })
        .collect();

    if let Err(e) = telemetry_event::Entity::insert_many(models)
        .exec(&state.db)
        .await
    {
        tracing::error!("Failed to persist telemetry batch: {}", e);
        return Ok(Json(TelemetryIngestResponse { accepted: 0 }));
    }

    Ok(Json(TelemetryIngestResponse { accepted }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_valid_event_names() {
        for name in ["page_view", "app_started", "flow:run.finished-2", "a"] {
            assert!(is_valid_event_name(name), "expected '{}' to be valid", name);
        }
    }

    #[test]
    fn rejects_invalid_event_names() {
        let too_long = "a".repeat(129);
        for name in ["", "Page_View", "has space", "emoji💥", "semi;colon"] {
            assert!(
                !is_valid_event_name(name),
                "expected '{}' to be invalid",
                name
            );
        }
        assert!(!is_valid_event_name(&too_long));
        assert!(is_valid_event_name(&"a".repeat(128)));
    }

    #[test]
    fn rejects_reserved_backend_anon_id() {
        assert!(validate_anon_id(RESERVED_ANON_ID).is_err());
        assert!(validate_anon_id("").is_err());
        assert!(validate_anon_id(&"a".repeat(MAX_ANON_ID_LEN + 1)).is_err());
        assert!(validate_anon_id("backend2").is_ok());
        assert!(validate_anon_id("a1b2c3").is_ok());
    }

    #[test]
    fn redacts_secret_keys_at_any_depth() {
        let mut props = json!({
            "path": "/library",
            "password": "hunter2",
            "nested": {
                "API_KEY": "abc",
                "list": [{ "refresh_token": "xyz", "keep": 1 }]
            }
        });
        sanitize_props(&mut props);
        assert_eq!(props["password"], "[REDACTED]");
        assert_eq!(props["nested"]["API_KEY"], "[REDACTED]");
        assert_eq!(props["nested"]["list"][0]["refresh_token"], "[REDACTED]");
        assert_eq!(props["path"], "/library");
        assert_eq!(props["nested"]["list"][0]["keep"], 1);
    }

    #[test]
    fn redacts_identity_keys_at_any_depth() {
        let mut props = json!({
            "sub": "auth0|42",
            "USER_ID": "u-1",
            "userId": "u-2",
            "email": "a@b.c",
            "username": "felix",
            "ip": "203.0.113.7",
            "ip_address": "203.0.113.7",
            "nested": { "list": [{ "Email": "x@y.z", "kept": "library" }] }
        });
        sanitize_props(&mut props);
        for key in [
            "sub",
            "USER_ID",
            "userId",
            "email",
            "username",
            "ip",
            "ip_address",
        ] {
            assert_eq!(
                props[key], "[REDACTED]",
                "expected '{}' to be redacted",
                key
            );
        }
        assert_eq!(props["nested"]["list"][0]["Email"], "[REDACTED]");
        assert_eq!(props["nested"]["list"][0]["kept"], "library");
    }

    #[test]
    fn every_redacted_key_is_lowercase_so_matching_is_case_insensitive() {
        for key in REDACTED_KEYS {
            assert_eq!(key, key.to_ascii_lowercase(), "'{}' must be lowercase", key);
        }
    }

    #[test]
    fn drops_events_with_oversized_props() {
        let events = vec![
            TelemetryEventPayload {
                name: "small".to_string(),
                props: Some(json!({ "ok": true })),
                client_ts: None,
            },
            TelemetryEventPayload {
                name: "huge".to_string(),
                props: Some(json!({ "blob": "x".repeat(MAX_PROPS_BYTES + 1) })),
                client_ts: None,
            },
            TelemetryEventPayload {
                name: "Invalid Name".to_string(),
                props: None,
                client_ts: None,
            },
        ];
        let validated = validate_events(events);
        assert_eq!(validated.len(), 1);
        assert_eq!(validated[0].name, "small");
    }

    #[test]
    fn parses_client_ts_leniently() {
        assert!(parse_client_ts(Some("2026-07-26T10:00:00Z")).is_some());
        assert!(parse_client_ts(Some(" 2026-07-26T10:00:00+02:00 ")).is_some());
        assert!(parse_client_ts(Some("not-a-date")).is_none());
        assert!(parse_client_ts(None).is_none());
    }

    fn headers_from(entries: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in entries {
            headers.insert(*name, value.parse().unwrap());
        }
        headers
    }

    #[test]
    fn extracts_country_from_each_geo_header() {
        for name in COUNTRY_HEADERS {
            let headers = headers_from(&[(name, "DE")]);
            assert_eq!(
                country_from_headers(&headers).as_deref(),
                Some("DE"),
                "expected header '{}' to yield a country",
                name
            );
        }
    }

    #[test]
    fn country_headers_are_checked_in_priority_order() {
        let headers = headers_from(&[
            ("x-vercel-ip-country", "US"),
            ("cloudfront-viewer-country", "DE"),
            ("cf-ipcountry", "FR"),
        ]);
        assert_eq!(country_from_headers(&headers).as_deref(), Some("DE"));
    }

    #[test]
    fn country_is_uppercased_and_trimmed() {
        let headers = headers_from(&[("cf-ipcountry", " de ")]);
        assert_eq!(country_from_headers(&headers).as_deref(), Some("DE"));
    }

    #[test]
    fn invalid_country_values_are_skipped() {
        for value in ["USA", "D", "1A", "", "D-"] {
            let headers = headers_from(&[("cf-ipcountry", value)]);
            assert_eq!(
                country_from_headers(&headers),
                None,
                "expected '{}' to be rejected",
                value
            );
        }
    }

    #[test]
    fn placeholder_countries_are_rejected() {
        for value in ["XX", "xx", "T1"] {
            let headers = headers_from(&[("cloudfront-viewer-country", value)]);
            assert_eq!(country_from_headers(&headers), None);
        }
        let headers = headers_from(&[("cloudfront-viewer-country", "XX"), ("cf-ipcountry", "AT")]);
        assert_eq!(country_from_headers(&headers).as_deref(), Some("AT"));
    }

    #[test]
    fn missing_geo_headers_yield_no_country() {
        assert_eq!(country_from_headers(&HeaderMap::new()), None);
    }
}

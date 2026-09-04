//! Anonymous web-vitals and app performance metric ingest.
//!
//! PRIVACY INVARIANT: like the event ingest, this handler is anonymous by
//! construction. It must never extract `Extension(AppUser)` and never store
//! user identity or IP addresses — only the random, client-generated `anon_id`,
//! a coarse country derived from proxy headers and an anonymized route path.

use axum::{Json, extract::State, http::HeaderMap};
use sea_orm::{ConnectionTrait, DbErr, EntityTrait, Set};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::errors::{MAX_RELEASE_LEN, MAX_SHORT_STRING_LEN, optional_string, validate_source};
use super::{country_from_headers, parse_client_ts, validate_anon_id};
use crate::{
    entity::telemetry_perf_metric, error::ApiError, middleware::trace_context::is_static_segment,
    state::AppState, telemetry::sink_from_env,
};

const MAX_METRICS_PER_BATCH: usize = 50;
/// Mirrors `MAX_TELEMETRY_PATH_LENGTH` in `packages/ui/lib/telemetry/page-view.ts`.
const MAX_PATH_LEN: usize = 256;

/// Metric vocabulary. Values are milliseconds except `cls`, which is unitless.
const PERF_METRICS: [&str; 7] = [
    "lcp",
    "inp",
    "cls",
    "ttfb",
    "fcp",
    "app_start",
    "screen_load",
];

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetryPerfMetricPayload {
    /// One of "lcp", "inp", "cls", "ttfb", "fcp", "app_start", "screen_load".
    pub metric: String,
    /// Milliseconds, except for "cls" which is unitless.
    pub value: f64,
    /// Anonymized route path. Query strings and fragments are rejected.
    #[serde(default)]
    pub path: Option<String>,
    /// Client-side timestamp (RFC 3339). Invalid values are stored as null.
    #[serde(default)]
    pub client_ts: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TelemetryPerfIngestPayload {
    /// Random client-generated identifier, 1-64 characters. Never a user id.
    pub anon_id: String,
    /// Origin of the batch: "desktop", "desktop_core", "desktop_native", "web" or "backend".
    pub source: String,
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    /// Up to 50 metrics per batch.
    pub metrics: Vec<TelemetryPerfMetricPayload>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TelemetryPerfIngestResponse {
    pub accepted: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct ValidatedMetric {
    metric: String,
    value: f64,
    path: Option<String>,
    client_ts: Option<chrono::DateTime<chrono::FixedOffset>>,
}

/// Server-side equivalent of `sanitizeTelemetryPath`, hardened to the same rule
/// the trace-context middleware uses: a segment survives only when it is
/// unambiguously static, so anything caller-controlled — opaque ids, auth
/// subjects like `auth0|1234`, e-mail addresses — collapses to `:id` instead of
/// being persisted. Unlike the client helper this rejects instead of trimming a
/// query string or fragment: a client that sends one has skipped sanitization,
/// so its path is not trustworthy.
fn sanitize_path(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.contains(['?', '#']) {
        return None;
    }
    let sanitized = trimmed
        .split('/')
        .map(|segment| {
            if is_static_segment(segment) {
                segment
            } else {
                ":id"
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    Some(optional_string(Some(sanitized), MAX_PATH_LEN).unwrap_or_else(|| "/".to_string()))
}

fn validate_metric(metric: TelemetryPerfMetricPayload) -> Option<ValidatedMetric> {
    let name = metric.metric.trim().to_ascii_lowercase();
    if !PERF_METRICS.contains(&name.as_str()) {
        return None;
    }
    if !metric.value.is_finite() || metric.value < 0.0 {
        return None;
    }
    let path = match metric.path.as_deref() {
        Some(raw) if !raw.trim().is_empty() => Some(sanitize_path(raw)?),
        _ => None,
    };
    Some(ValidatedMetric {
        metric: name,
        value: metric.value,
        path,
        client_ts: parse_client_ts(metric.client_ts.as_deref()),
    })
}

fn validate_metrics(metrics: Vec<TelemetryPerfMetricPayload>) -> Vec<ValidatedMetric> {
    metrics.into_iter().filter_map(validate_metric).collect()
}

fn ensure_batch_size(len: usize) -> Result<(), ApiError> {
    if len > MAX_METRICS_PER_BATCH {
        return Err(ApiError::bad_request(format!(
            "A telemetry performance batch may contain at most {} metrics",
            MAX_METRICS_PER_BATCH
        )));
    }
    Ok(())
}

async fn persist_metrics<C: ConnectionTrait>(
    db: &C,
    payload: &TelemetryPerfIngestPayload,
    metrics: Vec<ValidatedMetric>,
    country: Option<String>,
) -> Result<usize, DbErr> {
    let now = chrono::Utc::now().fixed_offset();
    let accepted = metrics.len();
    let models: Vec<telemetry_perf_metric::ActiveModel> = metrics
        .into_iter()
        .map(|metric| telemetry_perf_metric::ActiveModel {
            id: Set(flow_like_types::create_id()),
            anon_id: Set(payload.anon_id.clone()),
            source: Set(payload.source.clone()),
            platform: Set(payload.platform.clone()),
            release: Set(payload.release.clone()),
            metric: Set(metric.metric),
            value: Set(metric.value),
            path: Set(metric.path),
            country: Set(country.clone()),
            client_ts: Set(metric.client_ts),
            created_at: Set(now),
        })
        .collect();

    telemetry_perf_metric::Entity::insert_many(models)
        .exec_without_returning(db)
        .await?;

    Ok(accepted)
}

/// Anonymous by construction: this handler intentionally never extracts
/// `Extension(AppUser)` and never persists user identity or IP addresses.
/// The stored country is derived exclusively from proxy geolocation headers;
/// the client IP is never read.
#[utoipa::path(
    post,
    path = "/telemetry/performance",
    tag = "telemetry",
    request_body = TelemetryPerfIngestPayload,
    responses(
        (status = 200, description = "Number of performance metrics that were accepted", body = TelemetryPerfIngestResponse),
        (status = 400, description = "Invalid batch"),
        (status = 404, description = "Telemetry is disabled on this platform")
    ),
    description = "Submit a batch of anonymous performance measurements (Core Web Vitals, app start and screen load timings). No account, user identity or IP address is ever stored — only a random client-generated identifier."
)]
#[tracing::instrument(name = "POST /telemetry/performance", skip(state, headers, payload))]
pub async fn ingest_performance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut payload): Json<TelemetryPerfIngestPayload>,
) -> Result<Json<TelemetryPerfIngestResponse>, ApiError> {
    if !state.platform_config.features.telemetry {
        return Err(ApiError::NOT_FOUND);
    }

    validate_anon_id(&payload.anon_id)?;
    validate_source(&payload.source)?;
    ensure_batch_size(payload.metrics.len())?;

    payload.release = optional_string(payload.release.take(), MAX_RELEASE_LEN);
    payload.platform = optional_string(payload.platform.take(), MAX_SHORT_STRING_LEN);

    let validated = validate_metrics(std::mem::take(&mut payload.metrics));
    if validated.is_empty() {
        return Ok(Json(TelemetryPerfIngestResponse { accepted: 0 }));
    }

    let sink = sink_from_env();

    if sink == "none" {
        return Ok(Json(TelemetryPerfIngestResponse {
            accepted: validated.len(),
        }));
    }

    if sink == "log" {
        tracing::info!(
            source = %payload.source,
            anon_id = %payload.anon_id,
            release = payload.release.as_deref().unwrap_or(""),
            platform = payload.platform.as_deref().unwrap_or(""),
            metrics = validated.len(),
            names = ?validated.iter().map(|metric| metric.metric.as_str()).collect::<Vec<_>>(),
            "telemetry performance batch"
        );
        return Ok(Json(TelemetryPerfIngestResponse {
            accepted: validated.len(),
        }));
    }

    let country = country_from_headers(&headers);
    match persist_metrics(&state.db, &payload, validated, country).await {
        Ok(accepted) => Ok(Json(TelemetryPerfIngestResponse { accepted })),
        Err(e) => {
            tracing::error!("Failed to persist telemetry performance batch: {}", e);
            Ok(Json(TelemetryPerfIngestResponse { accepted: 0 }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(name: &str, value: f64) -> TelemetryPerfMetricPayload {
        TelemetryPerfMetricPayload {
            metric: name.to_string(),
            value,
            path: None,
            client_ts: None,
        }
    }

    fn with_path(name: &str, path: &str) -> TelemetryPerfMetricPayload {
        TelemetryPerfMetricPayload {
            path: Some(path.to_string()),
            ..metric(name, 1200.0)
        }
    }

    #[test]
    fn accepts_every_documented_metric() {
        for name in PERF_METRICS {
            let validated = validate_metric(metric(name, 0.05)).unwrap();
            assert_eq!(validated.metric, name);
        }
    }

    #[test]
    fn normalizes_metric_case_and_whitespace() {
        assert_eq!(
            validate_metric(metric(" LCP ", 10.0)).unwrap().metric,
            "lcp"
        );
        assert_eq!(
            validate_metric(metric("App_Start", 10.0)).unwrap().metric,
            "app_start"
        );
    }

    #[test]
    fn rejects_metrics_outside_the_vocabulary() {
        for name in ["fid", "", "lcp2", "screen load", "cpu"] {
            assert!(
                validate_metric(metric(name, 10.0)).is_none(),
                "expected '{}' to be rejected",
                name
            );
        }
    }

    #[test]
    fn rejects_negative_and_non_finite_values() {
        assert!(validate_metric(metric("lcp", -1.0)).is_none());
        assert!(validate_metric(metric("lcp", f64::NAN)).is_none());
        assert!(validate_metric(metric("lcp", f64::INFINITY)).is_none());
        assert_eq!(validate_metric(metric("cls", 0.0)).unwrap().value, 0.0);
    }

    #[test]
    fn rejects_paths_with_a_query_string_or_fragment() {
        assert!(validate_metric(with_path("lcp", "/library?tab=apps")).is_none());
        assert!(validate_metric(with_path("lcp", "/library#section")).is_none());
        assert!(sanitize_path("/library?tab=apps").is_none());
        assert!(sanitize_path("/library#section").is_none());
    }

    #[test]
    fn collapses_id_like_path_segments() {
        assert_eq!(
            sanitize_path("/apps/12345/flows").as_deref(),
            Some("/apps/:id/flows")
        );
        assert_eq!(
            sanitize_path("/apps/0f1e2d3c4b5a69788796/board").as_deref(),
            Some("/apps/:id/board")
        );
        assert_eq!(
            sanitize_path("/apps/f47ac10b-58cc-4372-a567-0e02b2c3d479").as_deref(),
            Some("/apps/:id")
        );
        assert_eq!(
            sanitize_path("/library/apps").as_deref(),
            Some("/library/apps")
        );
    }

    /// Regression: these segments carry identity but are not purely
    /// alphanumeric, so the previous predicate let them through verbatim into
    /// `TelemetryPerfMetric.path`, which the admin performance view renders.
    #[test]
    fn collapses_identity_bearing_segments_that_are_not_alphanumeric() {
        assert_eq!(
            sanitize_path("/user/auth0|1234").as_deref(),
            Some("/user/:id")
        );
        assert_eq!(
            sanitize_path("/user/auth0%7C1234").as_deref(),
            Some("/user/:id")
        );
        assert_eq!(
            sanitize_path("/user/someone@example.com").as_deref(),
            Some("/user/:id")
        );
        assert_eq!(
            sanitize_path("/search/felix schultz").as_deref(),
            Some("/search/:id")
        );
        assert_eq!(
            sanitize_path("/user/Felix.Schultz").as_deref(),
            Some("/user/:id")
        );
    }

    #[test]
    fn caps_and_normalizes_empty_paths() {
        let long = sanitize_path(&"/abc-def".repeat(MAX_PATH_LEN)).unwrap();
        assert_eq!(long.len(), MAX_PATH_LEN);
        assert_eq!(sanitize_path("/").as_deref(), Some("/"));
        assert_eq!(sanitize_path("   ").as_deref(), Some("/"));
        assert_eq!(validate_metric(with_path("lcp", "   ")).unwrap().path, None);
        assert_eq!(validate_metric(metric("lcp", 1.0)).unwrap().path, None);
    }

    #[test]
    fn enforces_the_batch_cap() {
        assert!(ensure_batch_size(0).is_ok());
        assert!(ensure_batch_size(MAX_METRICS_PER_BATCH).is_ok());
        assert!(ensure_batch_size(MAX_METRICS_PER_BATCH + 1).is_err());
    }

    #[test]
    fn invalid_metrics_do_not_drop_the_rest_of_the_batch() {
        let validated = validate_metrics(vec![
            metric("lcp", 2400.0),
            metric("fid", 10.0),
            with_path("inp", "/apps/12345"),
        ]);
        assert_eq!(validated.len(), 2);
        assert_eq!(validated[0].metric, "lcp");
        assert_eq!(validated[1].path.as_deref(), Some("/apps/:id"));
    }

    #[test]
    fn parses_client_timestamps_leniently() {
        let parsed = validate_metric(TelemetryPerfMetricPayload {
            client_ts: Some(" 2026-07-26T10:00:00+02:00 ".to_string()),
            ..metric("ttfb", 120.0)
        })
        .unwrap();
        assert!(parsed.client_ts.is_some());

        let invalid = validate_metric(TelemetryPerfMetricPayload {
            client_ts: Some("yesterday".to_string()),
            ..metric("ttfb", 120.0)
        })
        .unwrap();
        assert_eq!(invalid.client_ts, None);
    }

    #[test]
    fn rejects_the_reserved_backend_anon_id() {
        assert!(validate_anon_id("backend").is_err());
        assert!(validate_anon_id("a1b2c3").is_ok());
    }
}

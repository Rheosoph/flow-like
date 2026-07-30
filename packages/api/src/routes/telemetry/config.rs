//! Public client configuration for anonymous product telemetry.
//!
//! PRIVACY INVARIANT: like the ingest handlers, this endpoint is anonymous by
//! construction. It never extracts `Extension(AppUser)`, reads no request body
//! and returns the same payload to every caller, so fetching it cannot identify
//! or profile an install.
//!
//! Clients pull this once per session to learn how aggressively to sample their
//! own captures. `page_view` fires on every route change, which dominates event
//! volume, so it is sampled down by default while explicit product events are
//! kept in full.

use axum::{Json, extract::State};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{error::ApiError, state::AppState, telemetry::sink_from_env};

const DEFAULT_PAGE_VIEW_SAMPLE_RATE: f64 = 0.25;
const DEFAULT_EVENT_SAMPLE_RATE: f64 = 1.0;

/// Per-event-class capture rates in `0.0..=1.0`, where 1.0 keeps everything.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetrySamplingConfig {
    /// Share of `page_view` captures the client should keep.
    pub page_view: f64,
    /// Share of all other product events the client should keep.
    pub event: f64,
}

impl Default for TelemetrySamplingConfig {
    fn default() -> Self {
        Self {
            page_view: DEFAULT_PAGE_VIEW_SAMPLE_RATE,
            event: DEFAULT_EVENT_SAMPLE_RATE,
        }
    }
}

impl TelemetrySamplingConfig {
    /// Build the sampling rates from environment variables.
    /// - `FLOW_LIKE_TELEMETRY_PAGE_VIEW_SAMPLE_RATE` (default 0.25)
    /// - `FLOW_LIKE_TELEMETRY_EVENT_SAMPLE_RATE` (default 1.0)
    ///
    /// Values outside `0.0..=1.0` are clamped; unparsable or non-finite values
    /// fall back to the default rate.
    pub fn from_env() -> Self {
        Self {
            page_view: parse_rate(
                std::env::var("FLOW_LIKE_TELEMETRY_PAGE_VIEW_SAMPLE_RATE")
                    .ok()
                    .as_deref(),
                DEFAULT_PAGE_VIEW_SAMPLE_RATE,
            ),
            event: parse_rate(
                std::env::var("FLOW_LIKE_TELEMETRY_EVENT_SAMPLE_RATE")
                    .ok()
                    .as_deref(),
                DEFAULT_EVENT_SAMPLE_RATE,
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryConfigResponse {
    pub sampling: TelemetrySamplingConfig,
    /// False when the deployment discards every batch (`FLOW_LIKE_TELEMETRY_SINK=none`),
    /// letting clients stop sending usage events entirely. Crash reports,
    /// sessions, traces and performance samples are unaffected.
    pub enabled: bool,
}

fn parse_rate(raw: Option<&str>, default: f64) -> f64 {
    raw.and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(0.0, 1.0))
        .unwrap_or(default)
}

/// Anonymous by construction: this handler intentionally never extracts
/// `Extension(AppUser)` and reads nothing about the caller.
#[utoipa::path(
    get,
    path = "/telemetry/config",
    tag = "telemetry",
    responses(
        (status = 200, description = "Client-side telemetry sampling configuration", body = TelemetryConfigResponse),
        (status = 404, description = "Telemetry is disabled on this platform")
    ),
    description = "Fetch the sampling rates this platform wants clients to apply to anonymous product telemetry. Public and anonymous — no account is required and nothing about the caller is recorded."
)]
#[tracing::instrument(name = "GET /telemetry/config", skip(state))]
pub async fn telemetry_config(
    State(state): State<AppState>,
) -> Result<Json<TelemetryConfigResponse>, ApiError> {
    if !state.platform_config.features.telemetry {
        return Err(ApiError::NOT_FOUND);
    }

    Ok(Json(TelemetryConfigResponse {
        sampling: TelemetrySamplingConfig::from_env(),
        enabled: sink_from_env() != "none",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_fall_back_to_the_default() {
        assert_eq!(parse_rate(None, 0.25), 0.25);
        assert_eq!(parse_rate(Some(""), 0.25), 0.25);
        assert_eq!(parse_rate(Some("half"), 1.0), 1.0);
        assert_eq!(parse_rate(Some("nan"), 0.25), 0.25);
        assert_eq!(parse_rate(Some("inf"), 0.25), 0.25);
    }

    #[test]
    fn rates_are_clamped_to_the_unit_interval() {
        assert_eq!(parse_rate(Some("0"), 0.25), 0.0);
        assert_eq!(parse_rate(Some("1"), 0.25), 1.0);
        assert_eq!(parse_rate(Some(" 0.5 "), 0.25), 0.5);
        assert_eq!(parse_rate(Some("7"), 0.25), 1.0);
        assert_eq!(parse_rate(Some("-3"), 0.25), 0.0);
    }

    #[test]
    fn default_config_matches_the_documented_rates() {
        let config = TelemetrySamplingConfig::default();
        assert_eq!(config.page_view, 0.25);
        assert_eq!(config.event, 1.0);
    }

    #[test]
    fn response_serializes_camel_case_rates() {
        let body = serde_json::to_value(TelemetryConfigResponse {
            sampling: TelemetrySamplingConfig::default(),
            enabled: true,
        })
        .unwrap();
        assert_eq!(body["sampling"]["pageView"], 0.25);
        assert_eq!(body["sampling"]["event"], 1.0);
        assert_eq!(body["enabled"], true);
    }
}

//! Reusable server-side telemetry capture helper.

pub mod alerts;
pub mod llm;
pub mod notify;
pub mod rollup;
pub mod spans;
pub mod sweeper;

use flow_like_types::Value;
use sea_orm::{EntityTrait, Set};

pub use crate::middleware::trace_context::{TraceContext, trace_context_middleware};
pub use alerts::{
    AlertEvaluationResult, TelemetryAlertConfig, evaluate_once, spawn_telemetry_alert_evaluator,
};
pub use rollup::{
    TelemetryRollupConfig, TelemetryRollupResult, rollup_once, spawn_telemetry_rollup,
};
pub use spans::{
    SpanExportConfig, TelemetrySpanExporter, TelemetrySpanLayer, telemetry_span_layer,
};
pub use sweeper::{TelemetrySweepResult, TelemetrySweeperConfig, spawn_telemetry_sweeper};

use crate::{entity::telemetry_event, state::AppState};

const DEFAULT_TRACE_SAMPLE_RATE: f64 = 0.05;

pub(crate) fn sink_from_env() -> String {
    std::env::var("FLOW_LIKE_TELEMETRY_SINK")
        .unwrap_or_else(|_| "db".to_string())
        .to_ascii_lowercase()
}

/// Head-based trace sampling rate, read from `FLOW_LIKE_TRACE_SAMPLE_RATE`.
///
/// Values outside `0.0..=1.0` are clamped; unparsable or non-finite values
/// fall back to the default rate.
pub fn trace_sample_rate_from_env() -> f64 {
    parse_sample_rate(std::env::var("FLOW_LIKE_TRACE_SAMPLE_RATE").ok().as_deref())
}

pub(crate) fn parse_sample_rate(raw: Option<&str>) -> f64 {
    raw.and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(0.0, 1.0))
        .unwrap_or(DEFAULT_TRACE_SAMPLE_RATE)
}

/// Records a platform-internal telemetry event from the backend itself.
///
/// Best-effort and fire-and-forget: respects the `telemetry` feature flag and
/// the `FLOW_LIKE_TELEMETRY_SINK` env var, then spawns a background insert
/// with `source` and `anon_id` fixed to "backend". Never stores user identity.
pub fn record_backend_event(state: &AppState, name: impl Into<String>, props: Option<Value>) {
    if !state.platform_config.features.telemetry {
        return;
    }

    let sink = sink_from_env();
    if sink == "none" {
        return;
    }

    let name = name.into();

    if sink == "log" {
        tracing::info!(source = "backend", event = %name, "telemetry event");
        return;
    }

    let db = state.db.clone();
    flow_like_types::tokio::spawn(async move {
        let model = telemetry_event::ActiveModel {
            id: Set(flow_like_types::create_id()),
            name: Set(name),
            source: Set("backend".to_string()),
            anon_id: Set("backend".to_string()),
            props: Set(props),
            app_version: Set(None),
            platform: Set(None),
            country: Set(None),
            client_ts: Set(None),
            created_at: Set(chrono::Utc::now().naive_utc()),
        };
        if let Err(e) = telemetry_event::Entity::insert(model).exec(&db).await {
            tracing::error!("Failed to persist backend telemetry event: {}", e);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_rate_defaults_when_unset_or_invalid() {
        assert_eq!(parse_sample_rate(None), DEFAULT_TRACE_SAMPLE_RATE);
        assert_eq!(parse_sample_rate(Some("")), DEFAULT_TRACE_SAMPLE_RATE);
        assert_eq!(parse_sample_rate(Some("abc")), DEFAULT_TRACE_SAMPLE_RATE);
        assert_eq!(parse_sample_rate(Some("nan")), DEFAULT_TRACE_SAMPLE_RATE);
        assert_eq!(parse_sample_rate(Some("inf")), DEFAULT_TRACE_SAMPLE_RATE);
    }

    #[test]
    fn sample_rate_is_clamped_to_unit_interval() {
        assert_eq!(parse_sample_rate(Some("0")), 0.0);
        assert_eq!(parse_sample_rate(Some("1")), 1.0);
        assert_eq!(parse_sample_rate(Some(" 0.25 ")), 0.25);
        assert_eq!(parse_sample_rate(Some("2.5")), 1.0);
        assert_eq!(parse_sample_rate(Some("-0.5")), 0.0);
    }
}

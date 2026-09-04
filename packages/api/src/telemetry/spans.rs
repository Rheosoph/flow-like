//! Backend span emission: closed `tracing` spans → `TelemetrySpan` rows.
//!
//! Deployments opt in by building the pair with [`telemetry_span_layer`],
//! installing [`TelemetrySpanLayer`] in their `tracing_subscriber` registry and
//! handing [`TelemetrySpanExporter`] a database connection once the application
//! state exists (the subscriber is initialized long before the pool is).
//!
//! The hot path never awaits, never touches the database and never grows
//! without bound: a span close performs one bounded `try_send` and drops the
//! span when the queue is full, warning at most once per minute. The background
//! exporter batches spans and flushes on a full batch or a fixed interval.
//!
//! Sampling is head-based — the decision is made once per trace root (or
//! inherited from an inbound `traceparent`) and propagated to every child span,
//! so a trace is either complete or absent, never partial.
//!
//! # Attribute invariant
//!
//! A span field is persisted **only** if its name appears in
//! `ALLOWED_ATTRIBUTE_KEYS`. Everything else is dropped outright — not
//! truncated, not redacted, not stored under a placeholder. This is an
//! allowlist by necessity: `#[tracing::instrument]` records every handler
//! argument that is not listed in `skip(...)`, so the field names arriving here
//! are defined by every instrumented call site in the workspace and grow with
//! every new handler. A name denylist cannot be kept correct against that; an
//! allowlist fails closed. Stored values pass a second, value-level guard
//! (`is_sensitive_value`) that drops credential and identity shapes even under
//! an allowlisted name.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use flow_like_types::Value;
use flow_like_types::tokio::{self, sync::mpsc, task::JoinHandle};
use rand::RngCore;
use sea_orm::{DatabaseConnection, EntityTrait, Set};
use serde_json::Map;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

use super::{parse_sample_rate, sink_from_env, trace_sample_rate_from_env};
use crate::db::DEFAULT_WRITE_CHUNK;
use crate::entity::telemetry_span;
use crate::state::AppState;

/// Span field carrying the trace id of an inbound `traceparent`.
pub const FIELD_TRACE_ID: &str = "telemetry.trace_id";
/// Span field carrying the remote parent span id of an inbound `traceparent`.
pub const FIELD_PARENT_SPAN_ID: &str = "telemetry.parent_span_id";
/// Span field carrying the upstream sampling decision.
pub const FIELD_SAMPLED: &str = "telemetry.sampled";
/// Span field overriding the stored status ("ok" | "error").
pub const FIELD_STATUS: &str = "telemetry.status";
/// Span field overriding the stored kind.
pub const FIELD_KIND: &str = "otel.kind";

const DEFAULT_QUEUE_CAPACITY: usize = 2048;
const DEFAULT_MAX_BATCH: usize = 512;
const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_secs(5);
const DROP_WARN_INTERVAL: Duration = Duration::from_secs(60);
const DEFAULT_TARGET_PREFIX: &str = "flow_like";
const DEFAULT_SOURCE: &str = "backend";

const MAX_SPAN_NAME_LEN: usize = 256;
const MAX_ATTRIBUTES: usize = 32;
const MAX_ATTRIBUTE_VALUE_LEN: usize = 512;
const MAX_ATTRIBUTES_BYTES: usize = 8192;
const MIN_HEX_SECRET_LEN: usize = 32;
const MIN_BASE64_SECRET_LEN: usize = 24;

const STATUS_OK: &str = "ok";
const STATUS_ERROR: &str = "error";
const KIND_INTERNAL: &str = "internal";
const SPAN_KINDS: [&str; 5] = ["server", "client", "internal", "producer", "consumer"];

/// The only span field names that may be persisted as attributes.
///
/// Every entry is operational and low cardinality: it describes what the server
/// did, never who it did it for. Adding a name here makes it storable in a
/// table documented as anonymous and rendered in the admin trace viewer, so an
/// entry needs a justification that holds for *every* value the field can take:
///
/// - `http.method` — the HTTP verb; a fixed, tiny vocabulary.
/// - `http.route` — the matched route *template* (`/user/lookup/{sub}`).
///   [`crate::middleware::trace_context`] guarantees path parameters are never
///   substituted into it, so it carries the endpoint, not the argument.
/// - `http.status_code` — the numeric response status.
/// - `db.operation` — the statement verb (`select`, `insert`, …).
/// - `db.table` — the table name; a closed set fixed by the schema.
/// - `error.type` — the error variant or type name. The free-form `error`
///   message is deliberately *not* allowlisted: messages interpolate runtime
///   values (ids, paths, user input), and the `status` column already records
///   that a span failed.
/// - `retry_count` — number of retries; a small integer.
///
/// `otel.kind` and the `telemetry.*` fields are reserved rather than
/// allowlisted: [`SpanVisitor`] routes them into dedicated columns, so they
/// never reach the attribute object at all.
const ALLOWED_ATTRIBUTE_KEYS: [&str; 7] = [
    "http.method",
    "http.route",
    "http.status_code",
    "db.operation",
    "db.table",
    "error.type",
    "retry_count",
];

/// A closed span ready to be persisted.
#[derive(Clone, Debug, PartialEq)]
pub struct FinishedSpan {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: String,
    pub started_at: chrono::DateTime<chrono::FixedOffset>,
    pub duration_ms: i32,
    pub status: String,
    pub attributes: Option<Value>,
}

/// Configuration of the span emission pipeline.
#[derive(Clone, Debug, PartialEq)]
pub struct SpanExportConfig {
    /// Head sampling rate applied once per trace root, `0.0..=1.0`.
    pub sample_rate: f64,
    /// Stored `source` column, "backend" for the API itself.
    pub source: String,
    pub release: Option<String>,
    pub platform: Option<String>,
    /// Only spans whose `target` starts with this prefix are recorded. It is
    /// mandatory: recording every dependency's spans would persist the database
    /// driver's own spans, which the exporter itself produces — a feedback loop.
    /// An empty value falls back to the default prefix.
    pub target_prefix: String,
    pub queue_capacity: usize,
    /// Spans per `INSERT`; clamped to [`DEFAULT_WRITE_CHUNK`] so one flush
    /// always fits a single bounded transaction.
    pub max_batch: usize,
    pub flush_interval: Duration,
}

impl Default for SpanExportConfig {
    fn default() -> Self {
        Self {
            sample_rate: parse_sample_rate(None),
            source: DEFAULT_SOURCE.to_string(),
            release: None,
            platform: Some(std::env::consts::OS.to_string()),
            target_prefix: DEFAULT_TARGET_PREFIX.to_string(),
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            max_batch: DEFAULT_MAX_BATCH,
            flush_interval: DEFAULT_FLUSH_INTERVAL,
        }
    }
}

impl SpanExportConfig {
    /// Build config from environment variables.
    /// - `FLOW_LIKE_TRACE_SAMPLE_RATE`: head sampling rate (default 0.05)
    /// - `FLOW_LIKE_RELEASE`: release stamped on every emitted span
    pub fn from_env() -> Self {
        Self {
            sample_rate: trace_sample_rate_from_env(),
            release: std::env::var("FLOW_LIKE_RELEASE")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            ..Self::default()
        }
    }
}

/// Bounded, non-blocking handoff from the tracing hot path to the exporter.
#[derive(Clone)]
struct SpanSink {
    tx: mpsc::Sender<FinishedSpan>,
    enabled: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
}

impl SpanSink {
    fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    fn disable(&self) {
        self.enabled.store(false, Ordering::Relaxed);
    }

    fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Never blocks: a full or closed queue costs the span, not the request.
    /// Drops are only counted here — the warning is emitted by the exporter,
    /// because events raised inside a subscriber callback are swallowed by
    /// tracing's re-entrancy guard.
    fn submit(&self, span: FinishedSpan) {
        if !self.is_enabled() {
            return;
        }
        if self.tx.try_send(span).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Warns about a saturated queue at most once per [`DROP_WARN_INTERVAL`],
/// from the exporter task where the event actually reaches the subscriber.
struct DropReporter {
    reported: u64,
    last_report: Instant,
}

impl DropReporter {
    fn new() -> Self {
        Self {
            reported: 0,
            last_report: Instant::now(),
        }
    }

    fn pending(&self, dropped: u64, since_report: Duration) -> Option<u64> {
        if dropped <= self.reported || since_report < DROP_WARN_INTERVAL {
            return None;
        }
        Some(dropped - self.reported)
    }

    fn report(&mut self, dropped: u64) {
        let Some(since_last) = self.pending(dropped, self.last_report.elapsed()) else {
            return;
        };
        tracing::warn!(
            dropped_total = dropped,
            dropped_since_last_report = since_last,
            "Telemetry span queue is saturated; spans were dropped instead of blocking request handling"
        );
        self.reported = dropped;
        self.last_report = Instant::now();
    }
}

/// `tracing_subscriber` layer converting closed spans into telemetry rows.
pub struct TelemetrySpanLayer {
    sink: SpanSink,
    sample_rate: f64,
    target_prefix: String,
}

impl TelemetrySpanLayer {
    /// Number of spans dropped because the queue was saturated.
    pub fn dropped_spans(&self) -> u64 {
        self.sink.dropped()
    }

    fn records_target(&self, target: &str) -> bool {
        target.starts_with(self.target_prefix.as_str())
    }
}

/// Per-span state kept in the registry until the span closes.
struct SpanState {
    trace_id: Arc<str>,
    span_id: String,
    parent_span_id: Option<String>,
    sampled: bool,
    kind: String,
    status: String,
    started: Instant,
    started_at: chrono::DateTime<chrono::FixedOffset>,
    attributes: Map<String, Value>,
}

impl<S> Layer<S> for TelemetrySpanLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if !self.sink.is_enabled() {
            return;
        }
        if !self.records_target(attrs.metadata().target()) {
            return;
        }
        let Some(span) = ctx.span(id) else {
            return;
        };

        let mut visitor = SpanVisitor::default();
        attrs.record(&mut visitor);

        let inherited = span.scope().skip(1).find_map(|parent| {
            let extensions = parent.extensions();
            extensions
                .get::<SpanState>()
                .map(|state| (state.trace_id.clone(), state.span_id.clone(), state.sampled))
        });

        let (trace_id, parent_span_id, sampled) = match inherited {
            Some((trace_id, parent_span_id, sampled)) => (trace_id, Some(parent_span_id), sampled),
            None => match visitor.trace_id.take() {
                Some(remote) => {
                    let sampled = visitor
                        .sampled
                        .unwrap_or_else(|| should_sample(&remote, self.sample_rate));
                    (
                        Arc::<str>::from(remote.as_str()),
                        visitor.parent_span_id.take(),
                        sampled,
                    )
                }
                None => {
                    let trace_id = new_trace_id();
                    let sampled = should_sample(&trace_id, self.sample_rate);
                    (Arc::<str>::from(trace_id.as_str()), None, sampled)
                }
            },
        };

        span.extensions_mut().insert(SpanState {
            trace_id,
            span_id: new_span_id(),
            parent_span_id,
            sampled,
            kind: visitor
                .kind
                .take()
                .unwrap_or_else(|| KIND_INTERNAL.to_string()),
            status: visitor
                .status
                .take()
                .unwrap_or_else(|| STATUS_OK.to_string()),
            started: Instant::now(),
            started_at: chrono::Utc::now().fixed_offset(),
            attributes: if sampled {
                std::mem::take(&mut visitor.attributes)
            } else {
                Map::new()
            },
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut extensions = span.extensions_mut();
        let Some(state) = extensions.get_mut::<SpanState>() else {
            return;
        };

        let mut visitor = SpanVisitor::default();
        values.record(&mut visitor);

        if let Some(status) = visitor.status.take() {
            state.status = status;
        }
        if let Some(kind) = visitor.kind.take() {
            state.kind = kind;
        }
        if !state.sampled {
            return;
        }
        for (key, value) in visitor.attributes {
            if state.attributes.len() >= MAX_ATTRIBUTES && !state.attributes.contains_key(&key) {
                break;
            }
            state.attributes.insert(key, value);
        }
    }

    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        if *event.metadata().level() != Level::ERROR {
            return;
        }
        let Some(span) = ctx.event_span(event) else {
            return;
        };
        let mut extensions = span.extensions_mut();
        if let Some(state) = extensions.get_mut::<SpanState>() {
            state.status = STATUS_ERROR.to_string();
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else {
            return;
        };
        let state = span.extensions_mut().remove::<SpanState>();
        let Some(state) = state else {
            return;
        };
        if !state.sampled {
            return;
        }

        let duration_ms = state.started.elapsed().as_millis().min(i32::MAX as u128) as i32;
        self.sink.submit(FinishedSpan {
            trace_id: state.trace_id.to_string(),
            span_id: state.span_id,
            parent_span_id: state.parent_span_id,
            name: truncate(span.name(), MAX_SPAN_NAME_LEN),
            kind: state.kind,
            started_at: state.started_at,
            duration_ms,
            status: state.status,
            attributes: finalize_attributes(state.attributes),
        });
    }
}

/// Background task draining the queue into `TelemetrySpan` rows.
pub struct TelemetrySpanExporter {
    rx: mpsc::Receiver<FinishedSpan>,
    sink: SpanSink,
    config: SpanExportConfig,
}

impl TelemetrySpanExporter {
    /// Start the exporter once a database connection exists.
    ///
    /// Returns `None` — and permanently disarms the layer, so span closes cost
    /// nothing — when telemetry is disabled for the platform or
    /// `FLOW_LIKE_TELEMETRY_SINK=none`. With `FLOW_LIKE_TELEMETRY_SINK=log`
    /// batches are logged instead of written.
    pub fn spawn(self, db: DatabaseConnection, telemetry_enabled: bool) -> Option<JoinHandle<()>> {
        let sink_kind = sink_from_env();
        if !telemetry_enabled || sink_kind == "none" {
            self.sink.disable();
            tracing::info!(
                telemetry_enabled,
                sink = %sink_kind,
                "Backend span emission disabled"
            );
            return None;
        }

        tracing::info!(
            sample_rate = self.config.sample_rate,
            max_batch = self.config.max_batch,
            flush_interval_secs = self.config.flush_interval.as_secs(),
            sink = %sink_kind,
            "Spawning backend telemetry span exporter"
        );

        let log_only = sink_kind == "log";
        Some(tokio::spawn(self.run(db, log_only)))
    }

    /// Convenience wiring for deployments that already built the app state.
    pub fn spawn_for_state(self, state: &AppState) -> Option<JoinHandle<()>> {
        let enabled = state.platform_config.features.telemetry;
        self.spawn(state.db.clone(), enabled)
    }

    async fn run(mut self, db: DatabaseConnection, log_only: bool) {
        let mut batch: Vec<FinishedSpan> = Vec::with_capacity(self.config.max_batch);
        let mut last_flush = Instant::now();
        let mut drops = DropReporter::new();

        loop {
            let budget = self
                .config
                .flush_interval
                .saturating_sub(last_flush.elapsed());
            match tokio::time::timeout(budget, self.rx.recv()).await {
                Ok(Some(span)) => {
                    batch.push(span);
                    if should_flush(
                        batch.len(),
                        self.config.max_batch,
                        last_flush.elapsed(),
                        self.config.flush_interval,
                    ) {
                        self.flush(&db, &mut batch, log_only).await;
                        last_flush = Instant::now();
                    }
                }
                Ok(None) => {
                    self.flush(&db, &mut batch, log_only).await;
                    break;
                }
                Err(_) => {
                    self.flush(&db, &mut batch, log_only).await;
                    last_flush = Instant::now();
                    drops.report(self.sink.dropped());
                }
            }
        }
    }

    async fn flush(&self, db: &DatabaseConnection, batch: &mut Vec<FinishedSpan>, log_only: bool) {
        if batch.is_empty() {
            return;
        }
        let spans = std::mem::replace(batch, Vec::with_capacity(self.config.max_batch));
        let count = spans.len();

        if log_only {
            tracing::info!(
                source = %self.config.source,
                spans = count,
                dropped = self.sink.dropped(),
                "backend telemetry span batch"
            );
            return;
        }

        let now = chrono::Utc::now().fixed_offset();
        let models: Vec<telemetry_span::ActiveModel> = spans
            .into_iter()
            .map(|span| telemetry_span::ActiveModel {
                id: Set(flow_like_types::create_id()),
                trace_id: Set(span.trace_id),
                span_id: Set(span.span_id),
                parent_span_id: Set(span.parent_span_id),
                name: Set(span.name),
                kind: Set(span.kind),
                source: Set(self.config.source.clone()),
                anon_id: Set(None),
                release: Set(self.config.release.clone()),
                platform: Set(self.config.platform.clone()),
                started_at: Set(span.started_at),
                duration_ms: Set(span.duration_ms),
                status: Set(span.status),
                attributes: Set(span.attributes),
                created_at: Set(now),
            })
            .collect();

        if let Err(e) = telemetry_span::Entity::insert_many(models)
            .exec_without_returning(db)
            .await
        {
            tracing::error!(error = %e, spans = count, "Failed to persist backend telemetry spans");
        }
    }

    #[cfg(test)]
    fn drain(&mut self) -> Vec<FinishedSpan> {
        let mut spans = Vec::new();
        while let Ok(span) = self.rx.try_recv() {
            spans.push(span);
        }
        spans
    }
}

/// Build the layer and its exporter. Install the layer in the subscriber
/// registry at startup, then call [`TelemetrySpanExporter::spawn`] once the
/// database connection is available.
pub fn telemetry_span_layer(
    mut config: SpanExportConfig,
) -> (TelemetrySpanLayer, TelemetrySpanExporter) {
    config.max_batch = config.max_batch.clamp(1, DEFAULT_WRITE_CHUNK);
    let (tx, rx) = mpsc::channel(config.queue_capacity.max(1));
    let sink = SpanSink {
        tx,
        enabled: Arc::new(AtomicBool::new(true)),
        dropped: Arc::new(AtomicU64::new(0)),
    };
    let target_prefix = match config.target_prefix.trim() {
        "" => DEFAULT_TARGET_PREFIX.to_string(),
        prefix => prefix.to_string(),
    };
    let layer = TelemetrySpanLayer {
        sink: sink.clone(),
        sample_rate: config.sample_rate,
        target_prefix,
    };
    let exporter = TelemetrySpanExporter { rx, sink, config };
    (layer, exporter)
}

/// Head sampling decision, derived from the trace id so that every service and
/// every retry of the same trace agrees without coordination.
pub fn should_sample(trace_id: &str, rate: f64) -> bool {
    if !rate.is_finite() || rate <= 0.0 {
        return false;
    }
    if rate >= 1.0 {
        return true;
    }
    let digest = blake3::hash(trace_id.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest.as_bytes()[..8]);
    let bucket = (u64::from_le_bytes(bytes) >> 11) as f64 / (1u64 << 53) as f64;
    bucket < rate
}

/// W3C-shaped trace id (32 lowercase hex characters).
pub fn new_trace_id() -> String {
    random_hex(16)
}

/// W3C-shaped span id (16 lowercase hex characters).
pub fn new_span_id() -> String {
    random_hex(8)
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buffer);
    hex::encode(buffer)
}

fn should_flush(len: usize, max_batch: usize, since_flush: Duration, interval: Duration) -> bool {
    len >= max_batch || since_flush >= interval
}

pub(crate) fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

/// The allowlist gate: only these names may be stored, everything else is
/// dropped. See `ALLOWED_ATTRIBUTE_KEYS` for the invariant and the
/// justification of each entry.
fn is_allowed_key(key: &str) -> bool {
    ALLOWED_ATTRIBUTE_KEYS.contains(&key)
}

fn is_persistable(key: &str, value: &Value) -> bool {
    if !is_allowed_key(key) {
        return false;
    }
    match value {
        Value::String(text) => !is_sensitive_value(text),
        _ => true,
    }
}

/// Value-level guard applied to everything that survives the allowlist. It is
/// defence in depth: an allowlisted name must still not carry a credential or
/// an identity, whether through a mis-set field or a future call site.
fn is_sensitive_value(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    has_credential_scheme(value)
        || looks_like_jwt(value)
        || carries_query_string(value)
        || value
            .split(is_token_separator)
            .any(|token| is_email_like(token) || is_secret_blob(token))
}

fn is_token_separator(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            ',' | ';' | '<' | '>' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}'
        )
}

fn has_credential_scheme(value: &str) -> bool {
    let lowercase = value.to_ascii_lowercase();
    ["bearer ", "basic ", "token "]
        .iter()
        .any(|scheme| lowercase.starts_with(scheme))
}

/// A JWT header is base64url of `{"…`, so every JWT starts with `eyJ`. The
/// prefix alone is the test: `MAX_ATTRIBUTE_VALUE_LEN` may already have cut the
/// token apart, leaving fewer than the three canonical segments.
fn looks_like_jwt(value: &str) -> bool {
    value
        .split(|character: char| character.is_whitespace() || character == '.')
        .any(|segment| segment.starts_with("eyJ"))
}

fn carries_query_string(value: &str) -> bool {
    let Some((base, query)) = value.split_once('?') else {
        return false;
    };
    !query.is_empty() && (base.contains("://") || base.starts_with('/'))
}

fn is_email_like(token: &str) -> bool {
    let token = token.trim_matches(|character: char| matches!(character, ':' | '=' | '.'));
    let mut parts = token.split('@');
    let (Some(local), Some(domain)) = (parts.next(), parts.next()) else {
        return false;
    };
    if parts.next().is_some() || local.is_empty() || !domain.contains('.') {
        return false;
    }
    domain
        .rsplit('.')
        .next()
        .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()))
}

/// Long opaque runs of hex or base64 are the shape of API keys, session ids and
/// signatures. No allowlisted value looks like this: routes and templates carry
/// separators, methods and verbs are short.
fn is_secret_blob(token: &str) -> bool {
    let token = token.trim_matches(|character: char| matches!(character, '`' | ':' | '='));
    if token.len() >= MIN_HEX_SECRET_LEN && token.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    token.len() >= MIN_BASE64_SECRET_LEN
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '_' | '-' | '='))
        && token.chars().any(|c| c.is_ascii_uppercase())
        && token.chars().any(|c| c.is_ascii_lowercase())
        && token.chars().any(|c| c.is_ascii_digit())
}

fn normalize_vocab(value: &str, vocab: &[&str]) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    vocab.contains(&normalized.as_str()).then_some(normalized)
}

fn finalize_attributes(attributes: Map<String, Value>) -> Option<Value> {
    if attributes.is_empty() {
        return None;
    }
    let value = Value::Object(attributes);
    let bytes = serde_json::to_vec(&value)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    if bytes > MAX_ATTRIBUTES_BYTES {
        return None;
    }
    Some(value)
}

/// Splits the reserved propagation fields out of a span's fields. Of the rest,
/// only allowlisted names with a non-sensitive value become attributes; every
/// other field is discarded.
#[derive(Default)]
struct SpanVisitor {
    trace_id: Option<String>,
    parent_span_id: Option<String>,
    sampled: Option<bool>,
    status: Option<String>,
    kind: Option<String>,
    attributes: Map<String, Value>,
}

/// Cheap gate applied before a value is rendered: a field that is neither
/// reserved nor allowlisted can never be stored, so its value is not even
/// formatted. Handler arguments are usually whole request structs — keeping
/// their `Debug` output out of the process entirely is both cheaper and safer
/// than building it and throwing it away.
fn is_recorded_field(field: &Field) -> bool {
    matches!(
        field.name(),
        FIELD_TRACE_ID | FIELD_PARENT_SPAN_ID | FIELD_SAMPLED | FIELD_STATUS | FIELD_KIND
    ) || is_allowed_key(field.name())
}

impl SpanVisitor {
    fn record_value(&mut self, field: &Field, value: Value) {
        match field.name() {
            FIELD_TRACE_ID => self.trace_id = as_non_empty_string(value),
            FIELD_PARENT_SPAN_ID => self.parent_span_id = as_non_empty_string(value),
            FIELD_SAMPLED => {
                self.sampled = match value {
                    Value::Bool(flag) => Some(flag),
                    Value::String(raw) => raw.parse::<bool>().ok(),
                    _ => None,
                }
            }
            FIELD_STATUS => {
                self.status = as_non_empty_string(value)
                    .and_then(|raw| normalize_vocab(&raw, &[STATUS_OK, STATUS_ERROR]))
            }
            FIELD_KIND => {
                self.kind =
                    as_non_empty_string(value).and_then(|raw| normalize_vocab(&raw, &SPAN_KINDS))
            }
            name => {
                if !is_persistable(name, &value) {
                    return;
                }
                if self.attributes.len() < MAX_ATTRIBUTES {
                    self.attributes.insert(name.to_string(), value);
                }
            }
        }
    }
}

impl Visit for SpanVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if !is_recorded_field(field) {
            return;
        }
        self.record_value(
            field,
            Value::String(truncate(value, MAX_ATTRIBUTE_VALUE_LEN)),
        );
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record_value(field, Value::Bool(value));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_value(field, Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_value(field, Value::from(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        match serde_json::Number::from_f64(value) {
            Some(number) => self.record_value(field, Value::Number(number)),
            None => self.record_value(field, Value::Null),
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if !is_recorded_field(field) {
            return;
        }
        self.record_value(
            field,
            Value::String(truncate(&format!("{value:?}"), MAX_ATTRIBUTE_VALUE_LEN)),
        );
    }
}

fn as_non_empty_string(value: Value) -> Option<String> {
    let raw = match value {
        Value::String(raw) => raw,
        Value::Null => return None,
        other => other.to_string(),
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::trace_context::{TraceContext, server_span};
    use tracing::subscriber::with_default;
    use tracing_subscriber::layer::SubscriberExt;

    fn config(sample_rate: f64) -> SpanExportConfig {
        SpanExportConfig {
            sample_rate,
            ..SpanExportConfig::default()
        }
    }

    fn finished(
        config: SpanExportConfig,
        body: impl FnOnce(),
    ) -> (Vec<FinishedSpan>, TelemetrySpanExporter) {
        let (layer, mut exporter) = telemetry_span_layer(config);
        let subscriber = tracing_subscriber::registry().with(layer);
        with_default(subscriber, body);
        let spans = exporter.drain();
        (spans, exporter)
    }

    fn by_name<'a>(spans: &'a [FinishedSpan], name: &str) -> &'a FinishedSpan {
        spans
            .iter()
            .find(|span| span.name == name)
            .unwrap_or_else(|| panic!("expected a span named '{name}' in {spans:?}"))
    }

    #[test]
    fn sampling_is_deterministic_for_a_trace_id() {
        let trace_id = "4bf92f3577b34da6a3ce929d0e0e4736";
        let first = should_sample(trace_id, 0.5);
        for _ in 0..64 {
            assert_eq!(should_sample(trace_id, 0.5), first);
        }
    }

    #[test]
    fn sampling_honours_the_rate_bounds() {
        let trace_id = new_trace_id();
        assert!(!should_sample(&trace_id, 0.0));
        assert!(!should_sample(&trace_id, -1.0));
        assert!(!should_sample(&trace_id, f64::NAN));
        assert!(should_sample(&trace_id, 1.0));
        assert!(should_sample(&trace_id, 2.0));
    }

    #[test]
    fn sampling_approximates_the_configured_rate() {
        let sampled = (0..10_000)
            .filter(|index| should_sample(&format!("trace-{index}"), 0.1))
            .count();
        assert!(
            (700..1300).contains(&sampled),
            "expected ~1000 sampled traces, got {sampled}"
        );
    }

    #[test]
    fn generated_ids_are_w3c_shaped() {
        let trace_id = new_trace_id();
        let span_id = new_span_id();
        assert_eq!(trace_id.len(), 32);
        assert_eq!(span_id.len(), 16);
        assert!(
            trace_id
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
        assert!(
            span_id
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
        assert_ne!(trace_id, new_trace_id());
    }

    #[test]
    fn a_saturated_queue_drops_spans_instead_of_blocking() {
        let (layer, mut exporter) = telemetry_span_layer(SpanExportConfig {
            queue_capacity: 2,
            ..config(1.0)
        });
        let subscriber = tracing_subscriber::registry().with(layer);

        with_default(subscriber, || {
            for index in 0..5 {
                let span = tracing::info_span!("overflow", index);
                let _entered = span.enter();
            }
        });

        assert_eq!(exporter.drain().len(), 2);
        assert_eq!(exporter.sink.dropped(), 3);
    }

    #[test]
    fn a_disabled_sink_never_queues_spans() {
        let (layer, mut exporter) = telemetry_span_layer(config(1.0));
        exporter.sink.disable();
        let subscriber = tracing_subscriber::registry().with(layer);

        with_default(subscriber, || {
            let span = tracing::info_span!("disabled");
            let _entered = span.enter();
        });

        assert!(exporter.drain().is_empty());
        assert_eq!(exporter.sink.dropped(), 0);
    }

    #[test]
    fn nested_spans_share_a_trace_and_link_to_their_parent() {
        let (spans, _exporter) = finished(config(1.0), || {
            let root = tracing::info_span!("root");
            let _root = root.enter();
            let child = tracing::info_span!("child");
            let _child = child.enter();
        });

        assert_eq!(spans.len(), 2);
        let root = by_name(&spans, "root");
        let child = by_name(&spans, "child");
        assert_eq!(root.trace_id, child.trace_id);
        assert_eq!(root.parent_span_id, None);
        assert_eq!(child.parent_span_id.as_deref(), Some(root.span_id.as_str()));
        assert_eq!(root.kind, KIND_INTERNAL);
        assert_eq!(root.status, STATUS_OK);
    }

    #[test]
    fn a_zero_sample_rate_emits_nothing() {
        let (spans, _exporter) = finished(config(0.0), || {
            let root = tracing::info_span!("root");
            let _root = root.enter();
            let child = tracing::info_span!("child");
            let _child = child.enter();
        });
        assert!(spans.is_empty());
    }

    #[test]
    fn an_inbound_trace_context_is_continued() {
        let context = TraceContext {
            trace_id: "4bf92f3577b34da6a3ce929d0e0e4736".to_string(),
            parent_span_id: "00f067aa0ba902b7".to_string(),
            sampled: true,
        };

        let (spans, _exporter) = finished(config(0.0), || {
            let span = server_span(Some(&context), "GET", "/api/v1/apps");
            let _entered = span.enter();
            let child = tracing::info_span!("db.query");
            let _child = child.enter();
        });

        assert_eq!(spans.len(), 2);
        let root = by_name(&spans, "http.request");
        let child = by_name(&spans, "db.query");
        assert_eq!(root.trace_id, context.trace_id);
        assert_eq!(root.parent_span_id.as_deref(), Some("00f067aa0ba902b7"));
        assert_eq!(root.kind, "server");
        assert_eq!(child.trace_id, context.trace_id);
        assert_eq!(child.parent_span_id.as_deref(), Some(root.span_id.as_str()));
    }

    #[test]
    fn an_unsampled_inbound_trace_is_not_recorded() {
        let context = TraceContext {
            trace_id: "4bf92f3577b34da6a3ce929d0e0e4736".to_string(),
            parent_span_id: "00f067aa0ba902b7".to_string(),
            sampled: false,
        };

        let (spans, _exporter) = finished(config(1.0), || {
            let span = server_span(Some(&context), "GET", "/api/v1/apps");
            let _entered = span.enter();
        });
        assert!(spans.is_empty());
    }

    #[test]
    fn error_events_mark_the_enclosing_span_as_failed() {
        let (spans, _exporter) = finished(config(1.0), || {
            let span = tracing::info_span!("failing");
            let _entered = span.enter();
            tracing::error!("boom");
        });
        assert_eq!(by_name(&spans, "failing").status, STATUS_ERROR);
    }

    #[test]
    fn allowlisted_fields_become_attributes() {
        let (spans, _exporter) = finished(config(1.0), || {
            let span = tracing::info_span!(
                "http.request",
                http.method = "GET",
                http.route = "/api/v1/apps",
                http.status_code = 200_u64,
                token = "super-secret",
                telemetry.status = tracing::field::Empty
            );
            let _entered = span.enter();
            span.record("telemetry.status", "error");
        });

        let span = by_name(&spans, "http.request");
        let attributes = span.attributes.as_ref().expect("attributes");
        assert_eq!(attributes["http.method"], "GET");
        assert_eq!(attributes["http.route"], "/api/v1/apps");
        assert_eq!(attributes["http.status_code"], 200);
        assert!(attributes.get("token").is_none());
        assert!(attributes.get(FIELD_STATUS).is_none());
        assert_eq!(span.status, STATUS_ERROR);
    }

    #[test]
    fn instrumented_handler_arguments_never_reach_the_span_table() {
        let (spans, _exporter) = finished(config(1.0), || {
            let span = tracing::info_span!(
                "handler",
                request = "TokenRefreshRequest { refresh_token: \"rt-live\" }",
                req = "Request { uri: /auth/token?code=abc }",
                body = "{\"prompt\":\"my private note\"}",
                payload = "payload",
                sub = "auth0|1234",
                password = "hunter2",
                user_sub = "auth0|1234",
                key = "tmp/user/auth0-1234/2026/07/27/file.bin",
                app_id = "app-1",
                query = "someone@example.com"
            );
            let _entered = span.enter();
        });

        let span = by_name(&spans, "handler");
        assert_eq!(
            span.attributes, None,
            "no non-allowlisted field may be persisted, not even redacted"
        );
    }

    #[test]
    fn fields_recorded_after_creation_are_allowlisted_too() {
        let (spans, _exporter) = finished(config(1.0), || {
            let span = tracing::info_span!(
                "http.request",
                http.status_code = tracing::field::Empty,
                sub = tracing::field::Empty
            );
            let _entered = span.enter();
            span.record("http.status_code", 404_u64);
            span.record("sub", "auth0|1234");
        });

        let attributes = by_name(&spans, "http.request")
            .attributes
            .as_ref()
            .expect("attributes");
        assert_eq!(attributes["http.status_code"], 404);
        assert!(attributes.get("sub").is_none());
    }

    #[test]
    fn the_value_guard_drops_credentials_under_an_allowlisted_name() {
        let jwt = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJhdXRoMHwxMjM0In0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let (spans, _exporter) = finished(config(1.0), || {
            let span = tracing::info_span!(
                "http.request",
                http.route = "Bearer sk-live-abcdefghijklmnop",
                db.table = jwt,
                db.operation = "someone@example.com",
                "error.type" = "/auth/callback?code=4%2F0AY0e&state=xyz",
                retry_count = "0f1e2d3c4b5a697887960f1e2d3c4b5a",
                http.method = "GET"
            );
            let _entered = span.enter();
        });

        let attributes = by_name(&spans, "http.request")
            .attributes
            .as_ref()
            .expect("attributes");
        assert!(
            attributes.get("http.route").is_none(),
            "bearer token stored"
        );
        assert!(attributes.get("db.table").is_none(), "jwt stored");
        assert!(attributes.get("db.operation").is_none(), "email stored");
        assert!(
            attributes.get("error.type").is_none(),
            "query string stored"
        );
        assert!(attributes.get("retry_count").is_none(), "hex secret stored");
        assert_eq!(attributes["http.method"], "GET");
    }

    #[test]
    fn the_value_guard_recognizes_credential_and_identity_shapes() {
        for sensitive in [
            "Bearer abc123",
            "bearer abc123",
            "Basic dXNlcjpwYXNz",
            "token abc123",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.sig",
            "authorization: Bearer eyJhbGciOiJIUzI1NiJ9",
            "someone@example.com",
            "contact <someone@example.com>",
            "https://idp.example.com/authorize?code=abc",
            "/auth/callback?code=abc&state=xyz",
            "0f1e2d3c4b5a697887960f1e2d3c4b5a",
            "sk9Xk2LpQz7RvT4mNb8Wc1Yh",
        ] {
            assert!(
                is_sensitive_value(sensitive),
                "expected '{sensitive}' to be treated as sensitive"
            );
        }

        for operational in [
            "GET",
            "/api/v1/apps",
            "/user/lookup/{sub}",
            "select",
            "telemetry_span",
            "InternalError",
            "",
            "  ",
            "/api/v1/apps?",
        ] {
            assert!(
                !is_sensitive_value(operational),
                "expected '{operational}' to be storable"
            );
        }
    }

    #[test]
    fn only_allowlisted_names_pass_the_key_gate() {
        for key in ALLOWED_ATTRIBUTE_KEYS {
            assert!(is_allowed_key(key));
        }
        for key in [
            "request",
            "req",
            "body",
            "payload",
            "sub",
            "password",
            "app_id",
            "error",
            "HTTP.ROUTE",
            "http_route",
        ] {
            assert!(!is_allowed_key(key), "'{key}' must not be persistable");
        }
    }

    #[test]
    fn spans_from_foreign_targets_are_ignored() {
        let (spans, _exporter) = finished(config(1.0), || {
            let span = tracing::info_span!(target: "hyper::client", "connect");
            let _entered = span.enter();
        });
        assert!(spans.is_empty());
    }

    #[test]
    fn oversized_and_empty_attribute_objects_collapse_to_null() {
        assert_eq!(finalize_attributes(Map::new()), None);

        let mut oversized = Map::new();
        oversized.insert(
            "blob".to_string(),
            Value::String("x".repeat(MAX_ATTRIBUTES_BYTES + 1)),
        );
        assert_eq!(finalize_attributes(oversized), None);

        let mut small = Map::new();
        small.insert("ok".to_string(), Value::Bool(true));
        assert!(finalize_attributes(small).is_some());
    }

    #[test]
    fn drops_are_reported_at_most_once_per_interval() {
        let reporter = DropReporter {
            reported: 10,
            last_report: Instant::now(),
        };
        assert_eq!(reporter.pending(10, Duration::from_secs(120)), None);
        assert_eq!(reporter.pending(9, Duration::from_secs(120)), None);
        assert_eq!(reporter.pending(42, Duration::from_secs(30)), None);
        assert_eq!(reporter.pending(42, DROP_WARN_INTERVAL), Some(32));
    }

    #[test]
    fn flushes_on_a_full_batch_or_an_elapsed_interval() {
        let interval = Duration::from_secs(5);
        assert!(!should_flush(1, 512, Duration::from_secs(1), interval));
        assert!(should_flush(512, 512, Duration::from_secs(1), interval));
        assert!(should_flush(1, 512, Duration::from_secs(5), interval));
        assert!(should_flush(1, 512, Duration::from_secs(9), interval));
    }

    #[test]
    fn long_names_and_values_are_capped() {
        assert_eq!(truncate(&"n".repeat(300), MAX_SPAN_NAME_LEN).len(), 256);
        assert_eq!(truncate("äöü", 3), "ä");
        assert_eq!(truncate("short", 256), "short");
    }

    #[test]
    fn env_config_keeps_the_documented_defaults() {
        let config = SpanExportConfig::default();
        assert_eq!(config.max_batch, 512);
        assert_eq!(config.flush_interval, Duration::from_secs(5));
        assert_eq!(config.source, "backend");
        assert_eq!(config.sample_rate, 0.05);
        assert_eq!(config.target_prefix, "flow_like");
    }

    #[test]
    fn batches_never_exceed_the_write_chunk() {
        let (_layer, exporter) = telemetry_span_layer(SpanExportConfig {
            max_batch: DEFAULT_WRITE_CHUNK * 4,
            ..config(1.0)
        });
        assert_eq!(exporter.config.max_batch, DEFAULT_WRITE_CHUNK);

        let (_layer, exporter) = telemetry_span_layer(SpanExportConfig {
            max_batch: 0,
            ..config(1.0)
        });
        assert_eq!(exporter.config.max_batch, 1);
    }

    #[test]
    fn an_empty_target_prefix_falls_back_to_the_default() {
        let (layer, _exporter) = telemetry_span_layer(SpanExportConfig {
            target_prefix: "  ".to_string(),
            ..config(1.0)
        });
        assert!(layer.records_target("flow_like_api::routes"));
        assert!(!layer.records_target("hyper::client"));
    }
}

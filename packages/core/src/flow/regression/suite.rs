//! The regression-suite data model: suite config, promoted fixtures, the
//! two case sources, payload redaction and the bucket layout.
//!
//! Storage authority split (do not add dual writes): suite config, fixtures
//! and their payloads live in the bucket under `apps/{app_id}/regression/` on
//! every deployment, so desktop shares one code path. Cloud additionally
//! keeps a `RegressionSuite` *projection* row in Postgres (written by the
//! same PUT) and stores suite runs in `RegressionSuiteRun` +
//! `RegressionCaseResult` only; desktop archives runs as JSON in the bucket,
//! newest [`DESKTOP_RUN_ARCHIVE_CAP`].

use super::discover::discover_board_tests;
use super::grade::TestVerdict;
use crate::flow::board::Board;
use crate::{app::App, state::FlowLikeState};
use flow_like_storage::Path;
use flow_like_types::Value;
use futures::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};

use crate::utils::compression::{compress_to_file_json, from_compressed_json};

/// Upper bound on cases per suite run. Keeps the replay-exclusion set bounded
/// (50 suite runs × 100 case run ids) and a suite run's wall clock plannable.
pub const SUITE_CASE_CAP: usize = 100;

/// Authored `test*` events per suite run — mirrors the copilot
/// `run_board_tests` tool's cap so the two surfaces never disagree about
/// which tests a board has.
pub const AUTHORED_TESTS_CAP: usize = 20;

/// Desktop keeps the newest N suite-run archives in the bucket. Cloud stores
/// runs in Postgres and writes no bucket archive.
pub const DESKTOP_RUN_ARCHIVE_CAP: usize = 50;

/// Stamped when the recorded run's board `log_level` was above `Info`:
/// `ASSERT_OK` markers were discarded, so the baseline (and any replay graded
/// under the same board) cannot justify a green verdict.
pub const CAVEAT_GRADING_BLIND: &str = "grading_blind";

/// Stamped when the recorded run carried caller OAuth tokens. The tokens are
/// per-caller and not part of the fixture, so a replay diverges for reasons
/// unrelated to the board — a suite containing such a fixture cannot be
/// scheduled and must surface a check issue instead.
pub const CAVEAT_CALLER_OAUTH_TOKENS: &str = "caller_oauth_tokens";

/// What a failing gate verdict does to publish/promote. Enforcement's
/// consumer is `promote_canary`: `Block` refuses with the suite run id in the
/// error, `Warn` adds a response field, `Off` is a no-op.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GateMode {
    #[default]
    Off,
    Warn,
    Block,
}

/// Suite configuration — the bucket object. `event_id: None` models a
/// board-keyed suite (for the FlowBoard tests panel later); the initial UI
/// only creates event-keyed suites. Page events and `ontology_action` events
/// are excluded from suites entirely.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RegressionSuite {
    pub id: String,
    pub board_id: String,
    #[serde(default)]
    pub event_id: Option<String>,
    /// The suite's default start node: the event's handler node for
    /// event-keyed suites. Recorded fixtures override it with their own
    /// `source_node_id`.
    pub node_id: String,
    #[serde(default)]
    pub trigger_on_publish: bool,
    /// Cron expression; scheduling is cloud-only (desktop suites run
    /// client-side, unscheduled).
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub gate_mode: GateMode,
    /// Must be explicitly acknowledged before the suite's first run: shadow
    /// isolation only guards storage writes and WASM — outbound HTTP from
    /// native nodes is not enforceable. Runners refuse to run without it.
    #[serde(default)]
    pub allow_live_side_effects: bool,
    /// Unix micros.
    pub created_at: u64,
    /// Unix micros.
    pub updated_at: u64,
}

/// The verdict recorded at promotion — what the case outcome compares
/// against. Never an output golden: verdict-vs-baseline only.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FixtureBaseline {
    pub verdict: TestVerdict,
    /// Error class of a failing baseline (see `compare::error_class_of`).
    #[serde(default)]
    pub error_class: Option<String>,
    #[serde(default)]
    pub visited_node_ids: Vec<String>,
    /// Unix micros — the recorded run's start.
    pub recorded_at: u64,
}

impl FixtureBaseline {
    /// The synthetic baseline of an authored `test*` case: its expectation is
    /// always `Pass`, straight from the assert/error predicate.
    pub fn pass_expectation(recorded_at: u64) -> Self {
        FixtureBaseline {
            verdict: TestVerdict::Pass,
            error_class: None,
            visited_node_ids: Vec::new(),
            recorded_at,
        }
    }
}

/// A promoted recorded run. The payload is stored post-redaction (see
/// [`prepare_fixture_payload`]); `source_node_id` comes from the Lance
/// summary because REST/MCP rows carry the registration's handler node —
/// replaying into the suite's default node would run the wrong handler.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RegressionFixture {
    pub id: String,
    pub payload: Value,
    pub source_node_id: String,
    pub source_board_id: String,
    pub baseline: FixtureBaseline,
    /// Sub of the user who promoted the run.
    pub promoted_by: String,
    /// Well-known values: [`CAVEAT_GRADING_BLIND`],
    /// [`CAVEAT_CALLER_OAUTH_TOKENS`]. Kept as strings for forward
    /// compatibility between desktop and cloud versions.
    #[serde(default)]
    pub caveats: Vec<String>,
}

/// One replayable case. Both variants dispatch identically — board-invoke
/// shape, pinned candidate version, the case's own start node, shadow-signed
/// claims — and grade with the same `grade_run`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SuiteCase {
    RecordedFixture {
        fixture_id: String,
        payload: Value,
        source_node_id: String,
        baseline: FixtureBaseline,
    },
    AuthoredTest {
        node_id: String,
        alias: String,
    },
}

/// The case list for one suite run against one candidate board version, plus
/// what could not run and why.
#[derive(Debug, Clone, PartialEq)]
pub struct SuiteCasePlan {
    pub cases: Vec<SuiteCase>,
    /// Fixture ids skipped because the candidate version no longer contains
    /// their `source_node_id`.
    pub skipped_missing_node: Vec<String>,
    /// Cases dropped by [`SUITE_CASE_CAP`].
    pub truncated: usize,
}

/// Assemble a suite run's cases: every fixture whose `source_node_id` exists
/// on the candidate board, then the board's authored `test*` events
/// (discovered via `discover_board_tests`, optional case-insensitive substring
/// filter on the alias, capped at [`AUTHORED_TESTS_CAP`] — the copilot tool's
/// semantics).
pub fn plan_suite_cases(
    fixtures: &[RegressionFixture],
    candidate: &Board,
    authored_filter: Option<&str>,
) -> SuiteCasePlan {
    let mut cases = Vec::new();
    let mut skipped_missing_node = Vec::new();
    for fixture in fixtures {
        if candidate.nodes.contains_key(&fixture.source_node_id) {
            cases.push(SuiteCase::RecordedFixture {
                fixture_id: fixture.id.clone(),
                payload: fixture.payload.clone(),
                source_node_id: fixture.source_node_id.clone(),
                baseline: fixture.baseline.clone(),
            });
        } else {
            skipped_missing_node.push(fixture.id.clone());
        }
    }
    cases.extend(
        discover_board_tests(candidate)
            .into_iter()
            .filter(|test| {
                authored_filter
                    .is_none_or(|needle| test.alias.to_lowercase().contains(&needle.to_lowercase()))
            })
            .take(AUTHORED_TESTS_CAP)
            .map(|test| SuiteCase::AuthoredTest {
                node_id: test.node_id,
                alias: test.alias,
            }),
    );
    let truncated = cases.len().saturating_sub(SUITE_CASE_CAP);
    cases.truncate(SUITE_CASE_CAP);
    SuiteCasePlan {
        cases,
        skipped_missing_node,
        truncated,
    }
}

// ---------------------------------------------------------------------------
// Redaction — redact FIRST, truncate SECOND, on both corpus routes (the
// preview listing and promotion). The corpus is the first cloud surface to
// return recorded plaintext payloads at all, and an inbound REST payload
// carries the entire lowercased header map plus the body three times over.
// ---------------------------------------------------------------------------

pub const REDACTED_PLACEHOLDER: &str = "[redacted]";

/// Inbound payloads store the raw body a fourth time as bytes; dropped
/// outright at promotion (never truncated, never redacted-in-place).
pub const BODY_BYTES_KEY: &str = "body_bytes";
/// The raw request body's STRING duplicate — same bytes as `body_bytes`, same
/// leak channel: a flat string that key-name redaction cannot see inside. Both
/// duplicates are dropped on every payload-serving surface; the parsed `body`
/// object stays as the readable, redactable form. Replay caveat: a flow that
/// reads `body_text` (rather than `body`) sees it absent in a replayed
/// fixture.
pub const BODY_TEXT_KEY: &str = "body_text";

/// Preview strings (corpus listings) are capped at 2 KiB.
pub const PAYLOAD_PREVIEW_CAP_BYTES: usize = 2 * 1024;

/// A fixture payload may be at most 256 KiB after redaction; promotion of a
/// larger payload is refused, not truncated.
pub const FIXTURE_PAYLOAD_CAP_BYTES: usize = 256 * 1024;

const REDACT_EXACT_KEYS: [&str; 4] = ["authorization", "cookie", "set-cookie", "x-api-key"];
const REDACT_SUBSTRINGS: [&str; 3] = ["token", "secret", "password"];

/// Case-insensitive: the exact header names, plus any key containing
/// `token`, `secret` or `password`.
pub fn key_requires_redaction(key: &str) -> bool {
    let key = key.to_lowercase();
    REDACT_EXACT_KEYS.contains(&key.as_str())
        || REDACT_SUBSTRINGS.iter().any(|needle| key.contains(needle))
}

/// Replace the value of every matching key — by leaf key name, across the
/// whole document, arrays included — with [`REDACTED_PLACEHOLDER`]. A matched
/// key's entire subtree is replaced wholesale. Pointer-based redaction over
/// the inbound REST shape is a false assurance; key-name matching is the
/// contract. Returns how many values were redacted.
pub fn redact_by_key_name(value: &mut Value) -> usize {
    match value {
        Value::Object(map) => {
            let mut redacted = 0;
            for (key, entry) in map.iter_mut() {
                if key_requires_redaction(key) {
                    *entry = Value::String(REDACTED_PLACEHOLDER.to_string());
                    redacted += 1;
                } else {
                    redacted += redact_by_key_name(entry);
                }
            }
            redacted
        }
        Value::Array(items) => items.iter_mut().map(redact_by_key_name).sum(),
        _ => 0,
    }
}

/// Remove every `body_bytes` entry anywhere in the document. Returns how many
/// were dropped.
pub fn drop_raw_body_duplicates(value: &mut Value) -> usize {
    match value {
        Value::Object(map) => {
            let mut dropped = usize::from(map.remove(BODY_BYTES_KEY).is_some());
            dropped += usize::from(map.remove(BODY_TEXT_KEY).is_some());
            for entry in map.values_mut() {
                dropped += drop_raw_body_duplicates(entry);
            }
            dropped
        }
        Value::Array(items) => items.iter_mut().map(drop_raw_body_duplicates).sum(),
        _ => 0,
    }
}

/// The one promotion entry point: redact by key name, drop `body_bytes`,
/// then enforce [`FIXTURE_PAYLOAD_CAP_BYTES`] on the redacted serialization.
/// Oversized payloads are refused — a truncated fixture would replay a
/// payload that was never received.
pub fn prepare_fixture_payload(mut payload: Value) -> flow_like_types::Result<Value> {
    redact_by_key_name(&mut payload);
    drop_raw_body_duplicates(&mut payload);
    let size = flow_like_types::json::to_vec(&payload)?.len();
    if size > FIXTURE_PAYLOAD_CAP_BYTES {
        return Err(flow_like_types::anyhow!(
            "fixture payload is {size} bytes after redaction; the cap is {FIXTURE_PAYLOAD_CAP_BYTES} bytes — this run cannot be promoted"
        ));
    }
    Ok(payload)
}

/// Redacted, size-capped preview string for corpus listings. Redacts a clone
/// internally so no call site can serve an unredacted preview; truncation to
/// [`PAYLOAD_PREVIEW_CAP_BYTES`] happens after redaction, on a char boundary.
pub fn payload_preview(payload: &Value) -> String {
    let mut clone = payload.clone();
    redact_by_key_name(&mut clone);
    // The raw-body duplicates defeat key-name redaction (flat string / byte
    // array); the preview shows the parsed, redacted `body` instead.
    drop_raw_body_duplicates(&mut clone);
    let serialized = clone.to_string();
    if serialized.len() <= PAYLOAD_PREVIEW_CAP_BYTES {
        return serialized;
    }
    // '…' is three UTF-8 bytes; keep the total within the cap.
    let mut cut = PAYLOAD_PREVIEW_CAP_BYTES.saturating_sub(3);
    while cut > 0 && !serialized.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut preview = serialized[..cut].to_string();
    preview.push('…');
    preview
}

// ---------------------------------------------------------------------------
// Bucket layout + IO — the layout literal exists exactly once, Event-style.
// ---------------------------------------------------------------------------

impl RegressionSuite {
    /// `apps/{app_id}/regression` — the storage root every regression object
    /// lives under.
    fn storage_root(app_id: &str) -> Path {
        Path::from("apps").child(app_id).child("regression")
    }

    fn suite_path(app_id: &str, suite_id: &str) -> Path {
        Self::storage_root(app_id).child(format!("{suite_id}.suite"))
    }

    fn fixtures_root(app_id: &str, suite_id: &str) -> Path {
        Self::storage_root(app_id).child("fixtures").child(suite_id)
    }

    fn fixture_path(app_id: &str, suite_id: &str, fixture_id: &str) -> Path {
        Self::fixtures_root(app_id, suite_id).child(format!("{fixture_id}.fixture"))
    }

    /// Desktop-only run archive root; cloud stores runs in Postgres and
    /// writes nothing here.
    pub fn runs_root(app_id: &str, suite_id: &str) -> Path {
        Self::storage_root(app_id).child("runs").child(suite_id)
    }

    pub fn run_archive_path(app_id: &str, suite_id: &str, suite_run_id: &str) -> Path {
        Self::runs_root(app_id, suite_id).child(format!("{suite_run_id}.run"))
    }

    async fn store(
        app: &App,
    ) -> flow_like_types::Result<std::sync::Arc<dyn flow_like_storage::object_store::ObjectStore>>
    {
        let state = app
            .app_state
            .clone()
            .ok_or(flow_like_types::anyhow!("App state not found"))?;
        Ok(FlowLikeState::project_meta_store(&state)
            .await?
            .as_generic())
    }

    pub async fn load(app: &App, suite_id: &str) -> flow_like_types::Result<RegressionSuite> {
        let store = Self::store(app).await?;
        from_compressed_json(store, Self::suite_path(&app.id, suite_id)).await
    }

    pub async fn save(&self, app: &App) -> flow_like_types::Result<()> {
        let store = Self::store(app).await?;
        compress_to_file_json(store, Self::suite_path(&app.id, &self.id), self).await
    }

    /// Delete the suite config plus every fixture and archived run under it.
    pub async fn delete(&self, app: &App) -> flow_like_types::Result<()> {
        let store = Self::store(app).await?;
        store.delete(&Self::suite_path(&app.id, &self.id)).await?;
        for root in [
            Self::fixtures_root(&app.id, &self.id),
            Self::runs_root(&app.id, &self.id),
        ] {
            let locations = store.list(Some(&root)).map_ok(|meta| meta.location).boxed();
            store
                .delete_stream(locations)
                .try_collect::<Vec<Path>>()
                .await?;
        }
        Ok(())
    }

    /// Ids of every suite stored for the app, sorted. One LIST, no GETs —
    /// callers load the configs they actually need.
    pub async fn list_suite_ids(app: &App) -> flow_like_types::Result<Vec<String>> {
        let store = Self::store(app).await?;
        let root = Self::storage_root(&app.id);
        let mut locations = store.list(Some(&root)).map_ok(|meta| meta.location).boxed();
        let mut ids = Vec::new();
        while let Some(Ok(location)) = locations.next().await {
            if let Some(id) = location
                .filename()
                .and_then(|name| name.strip_suffix(".suite"))
            {
                ids.push(id.to_string());
            }
        }
        ids.sort_unstable();
        Ok(ids)
    }

    /// Write (or overwrite — the desktop runner persists progress per case)
    /// one suite-run archive, then prune the archive set to
    /// [`DESKTOP_RUN_ARCHIVE_CAP`] newest by store modification time.
    /// Desktop-only storage: cloud keeps suite runs in Postgres and never
    /// writes here.
    pub async fn archive_run<T>(
        &self,
        app: &App,
        suite_run_id: &str,
        archive: &T,
    ) -> flow_like_types::Result<()>
    where
        T: serde::Serialize + serde::Deserialize<'static>,
    {
        let store = Self::store(app).await?;
        compress_to_file_json(
            store.clone(),
            Self::run_archive_path(&app.id, &self.id, suite_run_id),
            archive,
        )
        .await?;
        Self::prune_run_archives(&store, &app.id, &self.id, DESKTOP_RUN_ARCHIVE_CAP).await
    }

    pub async fn load_run_archive<T>(
        app: &App,
        suite_id: &str,
        suite_run_id: &str,
    ) -> flow_like_types::Result<T>
    where
        T: serde::Serialize + serde::de::DeserializeOwned,
    {
        let store = Self::store(app).await?;
        from_compressed_json(
            store,
            Self::run_archive_path(&app.id, suite_id, suite_run_id),
        )
        .await
    }

    /// Suite-run archive ids, newest first by the store's modification time.
    /// Bounded by [`DESKTOP_RUN_ARCHIVE_CAP`] because [`Self::archive_run`]
    /// prunes on every write.
    pub async fn list_run_archive_ids(
        app: &App,
        suite_id: &str,
    ) -> flow_like_types::Result<Vec<String>> {
        let store = Self::store(app).await?;
        let mut metas = Self::run_archive_metas(&store, &app.id, suite_id).await?;
        metas.sort_by(|a, b| {
            b.last_modified
                .cmp(&a.last_modified)
                .then_with(|| b.location.to_string().cmp(&a.location.to_string()))
        });
        Ok(metas
            .into_iter()
            .filter_map(|meta| {
                meta.location
                    .filename()
                    .and_then(|name| name.strip_suffix(".run"))
                    .map(str::to_string)
            })
            .collect())
    }

    async fn run_archive_metas(
        store: &std::sync::Arc<dyn flow_like_storage::object_store::ObjectStore>,
        app_id: &str,
        suite_id: &str,
    ) -> flow_like_types::Result<Vec<flow_like_storage::object_store::ObjectMeta>> {
        let root = Self::runs_root(app_id, suite_id);
        let mut listing = store.list(Some(&root)).boxed();
        let mut metas = Vec::new();
        while let Some(meta) = listing.next().await {
            let meta = meta.map_err(|error| {
                flow_like_types::anyhow!("Failed to list suite-run archives under {root}: {error}")
            })?;
            if meta
                .location
                .filename()
                .is_some_and(|name| name.ends_with(".run"))
            {
                metas.push(meta);
            }
        }
        Ok(metas)
    }

    /// Keep the newest `cap` archives (by store modification time), delete the
    /// rest.
    async fn prune_run_archives(
        store: &std::sync::Arc<dyn flow_like_storage::object_store::ObjectStore>,
        app_id: &str,
        suite_id: &str,
        cap: usize,
    ) -> flow_like_types::Result<()> {
        let mut metas = Self::run_archive_metas(store, app_id, suite_id).await?;
        if metas.len() <= cap {
            return Ok(());
        }
        metas.sort_by(|a, b| {
            b.last_modified
                .cmp(&a.last_modified)
                .then_with(|| b.location.to_string().cmp(&a.location.to_string()))
        });
        for meta in metas.into_iter().skip(cap) {
            store.delete(&meta.location).await?;
        }
        Ok(())
    }

    pub async fn load_fixture(
        &self,
        app: &App,
        fixture_id: &str,
    ) -> flow_like_types::Result<RegressionFixture> {
        let store = Self::store(app).await?;
        from_compressed_json(store, Self::fixture_path(&app.id, &self.id, fixture_id)).await
    }

    pub async fn save_fixture(
        &self,
        app: &App,
        fixture: &RegressionFixture,
    ) -> flow_like_types::Result<()> {
        let store = Self::store(app).await?;
        compress_to_file_json(
            store,
            Self::fixture_path(&app.id, &self.id, &fixture.id),
            fixture,
        )
        .await
    }

    pub async fn delete_fixture(&self, app: &App, fixture_id: &str) -> flow_like_types::Result<()> {
        let store = Self::store(app).await?;
        store
            .delete(&Self::fixture_path(&app.id, &self.id, fixture_id))
            .await?;
        Ok(())
    }

    /// Load every fixture of the suite, sorted by id for deterministic case
    /// ordering. Fixture counts are bounded by [`SUITE_CASE_CAP`].
    pub async fn list_fixtures(
        &self,
        app: &App,
    ) -> flow_like_types::Result<Vec<RegressionFixture>> {
        let store = Self::store(app).await?;
        let root = Self::fixtures_root(&app.id, &self.id);
        let mut locations = store.list(Some(&root)).map_ok(|meta| meta.location).boxed();
        let mut paths = Vec::new();
        while let Some(Ok(location)) = locations.next().await {
            if location
                .filename()
                .is_some_and(|name| name.ends_with(".fixture"))
            {
                paths.push(location);
            }
        }
        let mut fixtures = Vec::with_capacity(paths.len());
        for path in paths {
            fixtures.push(from_compressed_json::<RegressionFixture>(store.clone(), path).await?);
        }
        fixtures.sort_unstable_by(|a, b| a.id.cmp(&b.id));
        Ok(fixtures)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    #[test]
    fn redaction_matches_exact_headers_and_substrings_case_insensitively() {
        for key in [
            "authorization",
            "Authorization",
            "cookie",
            "Set-Cookie",
            "X-API-Key",
            "access_token",
            "REFRESH_TOKEN",
            "client_secret",
            "userPassword",
        ] {
            assert!(key_requires_redaction(key), "{key} must redact");
        }
        for key in ["user", "body", "args", "toke", "authorized_scopes"] {
            assert!(!key_requires_redaction(key), "{key} must not redact");
        }
    }

    #[test]
    fn redact_by_key_name_walks_the_whole_document() {
        let mut payload = json!({
            "headers": {
                "authorization": "Bearer abc",
                "cookie": "sid=1",
                "x-api-key": "k",
                "accept": "application/json"
            },
            "body": {
                "user": { "password": "hunter2" },
                "tokens": { "a": "1", "b": "2" }
            },
            "items": [ { "secret_value": 42 }, { "plain": true } ]
        });
        let redacted = redact_by_key_name(&mut payload);
        assert_eq!(redacted, 6);
        assert_eq!(
            payload["headers"]["authorization"],
            json!(REDACTED_PLACEHOLDER)
        );
        assert_eq!(payload["headers"]["accept"], json!("application/json"));
        assert_eq!(
            payload["body"]["user"]["password"],
            json!(REDACTED_PLACEHOLDER)
        );
        // A matched key's whole subtree is replaced wholesale.
        assert_eq!(payload["body"]["tokens"], json!(REDACTED_PLACEHOLDER));
        assert_eq!(
            payload["items"][0]["secret_value"],
            json!(REDACTED_PLACEHOLDER)
        );
        assert_eq!(payload["items"][1]["plain"], json!(true));
    }

    #[test]
    fn drop_raw_body_duplicates_removes_every_occurrence() {
        let mut payload = json!({
            "body_bytes": [1, 2, 3],
            "body_text": "{\"password\":\"hunter2\"}",
            "nested": { "body_bytes": "AAAA", "body_text": "raw", "keep": 1 },
            "list": [ { "body_bytes": {} } ]
        });
        assert_eq!(drop_raw_body_duplicates(&mut payload), 5);
        assert_eq!(payload, json!({ "nested": { "keep": 1 }, "list": [ {} ] }));
        // The whole point: a body-borne secret inside the flat string duplicate
        // is unreachable for key-name redaction, so the duplicate must go.
        assert!(!payload.to_string().contains("hunter2"));
    }

    #[test]
    fn payload_preview_never_serves_raw_body_duplicates() {
        let payload = json!({
            "body": { "password": "hunter2" },
            "body_text": "{\"password\":\"hunter2\"}"
        });
        let preview = payload_preview(&payload);
        assert!(!preview.contains("hunter2"));
        assert!(!preview.contains("body_text"));
        assert!(preview.contains(REDACTED_PLACEHOLDER));
    }

    #[test]
    fn prepare_fixture_payload_redacts_drops_then_caps() {
        let payload = json!({
            "headers": { "authorization": "Bearer abc" },
            "body_bytes": "x".repeat(FIXTURE_PAYLOAD_CAP_BYTES),
            "body": { "ok": true }
        });
        // The oversized body_bytes is dropped before the cap is measured.
        let prepared = prepare_fixture_payload(payload).unwrap();
        assert_eq!(
            prepared["headers"]["authorization"],
            json!(REDACTED_PLACEHOLDER)
        );
        assert!(prepared.get("body_bytes").is_none());

        let oversized = json!({ "body": "y".repeat(FIXTURE_PAYLOAD_CAP_BYTES + 1) });
        let error = prepare_fixture_payload(oversized).unwrap_err();
        assert!(
            error.to_string().contains("cannot be promoted"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn payload_preview_redacts_before_truncating() {
        let payload = json!({
            "authorization": "Bearer super-secret-value",
            "filler": "z".repeat(4 * 1024)
        });
        let preview = payload_preview(&payload);
        assert!(preview.len() <= PAYLOAD_PREVIEW_CAP_BYTES);
        assert!(preview.contains(REDACTED_PLACEHOLDER));
        assert!(!preview.contains("super-secret-value"));
        assert!(preview.ends_with('…'));

        let small = json!({ "ok": true });
        assert_eq!(payload_preview(&small), small.to_string());
    }

    #[test]
    fn plan_prefers_fixtures_and_skips_missing_source_nodes() {
        use crate::flow::node::Node;

        let mut handler = Node::new("events_simple", "Handler", "", "events");
        handler.id = "handler-node".to_string();
        handler.start = Some(true);
        let mut authored = Node::new("events_simple", "Test Checkout", "", "events");
        authored.id = "test-node".to_string();
        authored.start = Some(true);
        let mut other = Node::new("events_simple", "Test Other", "", "events");
        other.id = "other-test".to_string();
        other.start = Some(true);
        let board = test_board(vec![handler, authored, other]);

        let fixtures = vec![fixture("f1", "handler-node"), fixture("f2", "gone-node")];

        let plan = plan_suite_cases(&fixtures, &board, None);
        assert_eq!(plan.skipped_missing_node, vec!["f2"]);
        assert_eq!(plan.truncated, 0);
        assert_eq!(plan.cases.len(), 3);
        assert!(matches!(
            &plan.cases[0],
            SuiteCase::RecordedFixture { fixture_id, source_node_id, .. }
                if fixture_id == "f1" && source_node_id == "handler-node"
        ));
        let aliases: Vec<&str> = plan
            .cases
            .iter()
            .filter_map(|case| match case {
                SuiteCase::AuthoredTest { alias, .. } => Some(alias.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(aliases, vec!["testCheckout", "testOther"]);

        let filtered = plan_suite_cases(&fixtures, &board, Some("checkout"));
        let filtered_aliases: Vec<&str> = filtered
            .cases
            .iter()
            .filter_map(|case| match case {
                SuiteCase::AuthoredTest { alias, .. } => Some(alias.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(filtered_aliases, vec!["testCheckout"]);
    }

    fn fixture(id: &str, source_node_id: &str) -> RegressionFixture {
        RegressionFixture {
            id: id.to_string(),
            payload: json!({ "k": 1 }),
            source_node_id: source_node_id.to_string(),
            source_board_id: "board".to_string(),
            baseline: FixtureBaseline::pass_expectation(0),
            promoted_by: "user".to_string(),
            caveats: vec![],
        }
    }

    fn test_board(nodes: Vec<crate::flow::node::Node>) -> Board {
        use crate::flow::board::{ExecutionMode, ExecutionStage};
        use crate::flow::execution::LogLevel;
        use std::collections::HashMap;
        use std::time::SystemTime;

        Board {
            id: "board".to_string(),
            name: "Board".to_string(),
            description: String::new(),
            nodes: nodes
                .into_iter()
                .map(|node| (node.id.clone(), node))
                .collect(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            viewport: (0.0, 0.0, 1.0),
            version: (0, 0, 1),
            stage: ExecutionStage::Dev,
            log_level: LogLevel::Info,
            execution_mode: ExecutionMode::Hybrid,
            refs: HashMap::new(),
            internal_refs: HashMap::new(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            parent: None,
            board_dir: Path::from("/test"),
            logic_nodes: HashMap::new(),
            app_state: None,
            pin_index: None,
        }
    }

    mod storage {
        use super::*;
        use crate::state::{FlowLikeConfig, FlowLikeState};
        use flow_like_storage::files::store::FlowLikeStore;
        use flow_like_storage::object_store;
        use flow_like_types::tokio;
        use std::sync::Arc;

        async fn test_app() -> App {
            let mut config = FlowLikeConfig::new();
            config.register_app_meta_store(FlowLikeStore::Other(Arc::new(
                object_store::memory::InMemory::new(),
            )));
            config.register_app_storage_store(FlowLikeStore::Other(Arc::new(
                object_store::memory::InMemory::new(),
            )));
            let state = Arc::new(FlowLikeState::new(
                config,
                crate::utils::http::HTTPClient::new_without_refetch(),
            ));
            App::new(None, crate::bit::Metadata::default(), vec![], state)
                .await
                .unwrap()
        }

        fn test_suite(id: &str) -> RegressionSuite {
            RegressionSuite {
                id: id.to_string(),
                board_id: "board".to_string(),
                event_id: Some("event".to_string()),
                node_id: "node".to_string(),
                trigger_on_publish: true,
                schedule: None,
                gate_mode: GateMode::Warn,
                allow_live_side_effects: false,
                created_at: 1,
                updated_at: 2,
            }
        }

        #[tokio::test]
        async fn suite_and_fixtures_round_trip_through_the_bucket() {
            let app = test_app().await;
            let suite = test_suite("suite-1");
            suite.save(&app).await.unwrap();
            test_suite("suite-2").save(&app).await.unwrap();

            let loaded = RegressionSuite::load(&app, "suite-1").await.unwrap();
            assert_eq!(loaded, suite);
            assert_eq!(
                RegressionSuite::list_suite_ids(&app).await.unwrap(),
                vec!["suite-1", "suite-2"]
            );

            suite
                .save_fixture(&app, &fixture("f2", "node-b"))
                .await
                .unwrap();
            suite
                .save_fixture(&app, &fixture("f1", "node-a"))
                .await
                .unwrap();
            let fixtures = suite.list_fixtures(&app).await.unwrap();
            assert_eq!(fixtures.len(), 2);
            assert_eq!(fixtures[0].id, "f1");
            assert_eq!(
                suite.load_fixture(&app, "f2").await.unwrap().source_node_id,
                "node-b"
            );

            suite.delete_fixture(&app, "f1").await.unwrap();
            assert_eq!(suite.list_fixtures(&app).await.unwrap().len(), 1);

            // Fixture objects never leak into the suite listing.
            assert_eq!(
                RegressionSuite::list_suite_ids(&app).await.unwrap(),
                vec!["suite-1", "suite-2"]
            );

            suite.delete(&app).await.unwrap();
            assert!(RegressionSuite::load(&app, "suite-1").await.is_err());
            assert!(suite.list_fixtures(&app).await.unwrap().is_empty());
            assert_eq!(
                RegressionSuite::list_suite_ids(&app).await.unwrap(),
                vec!["suite-2"]
            );
        }

        #[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
        struct TestArchive {
            id: String,
            status: String,
        }

        #[tokio::test]
        async fn run_archives_round_trip_and_list_newest_first() {
            let app = test_app().await;
            let suite = test_suite("suite-runs");
            suite.save(&app).await.unwrap();

            for id in ["run-a", "run-b"] {
                suite
                    .archive_run(
                        &app,
                        id,
                        &TestArchive {
                            id: id.to_string(),
                            status: "completed".to_string(),
                        },
                    )
                    .await
                    .unwrap();
                flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }

            let loaded: TestArchive = RegressionSuite::load_run_archive(&app, &suite.id, "run-a")
                .await
                .unwrap();
            assert_eq!(loaded.id, "run-a");

            // The runner overwrites an archive per case — the rewrite must
            // land on the same path, not mint a sibling.
            suite
                .archive_run(
                    &app,
                    "run-a",
                    &TestArchive {
                        id: "run-a".to_string(),
                        status: "errored".to_string(),
                    },
                )
                .await
                .unwrap();
            let rewritten: TestArchive =
                RegressionSuite::load_run_archive(&app, &suite.id, "run-a")
                    .await
                    .unwrap();
            assert_eq!(rewritten.status, "errored");

            let ids = RegressionSuite::list_run_archive_ids(&app, &suite.id)
                .await
                .unwrap();
            assert_eq!(ids.len(), 2);
            assert_eq!(ids[0], "run-a", "the rewritten archive is the newest");

            // Archives never leak into the suite or fixture listings.
            assert!(suite.list_fixtures(&app).await.unwrap().is_empty());
            assert_eq!(
                RegressionSuite::list_suite_ids(&app).await.unwrap(),
                vec!["suite-runs"]
            );
        }

        #[tokio::test]
        async fn prune_keeps_the_newest_archives() {
            let app = test_app().await;
            let suite = test_suite("suite-prune");
            suite.save(&app).await.unwrap();
            let store = RegressionSuite::store(&app).await.unwrap();

            for id in ["run-1", "run-2", "run-3"] {
                compress_to_file_json(
                    store.clone(),
                    RegressionSuite::run_archive_path(&app.id, &suite.id, id),
                    &TestArchive {
                        id: id.to_string(),
                        status: "completed".to_string(),
                    },
                )
                .await
                .unwrap();
                flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }

            RegressionSuite::prune_run_archives(&store, &app.id, &suite.id, 2)
                .await
                .unwrap();
            let ids = RegressionSuite::list_run_archive_ids(&app, &suite.id)
                .await
                .unwrap();
            assert_eq!(ids, vec!["run-3", "run-2"]);
        }
    }
}

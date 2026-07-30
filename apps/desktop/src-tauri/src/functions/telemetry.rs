use crate::functions::TauriFunctionError;
use crate::settings::{Settings, TelemetrySettings};
use crate::state::TauriSettingsState;
use flow_like_types::create_id;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tauri::AppHandle;

const MAX_BUFFERED_EVENTS: i64 = 5000;
const MAX_BUFFERED_ERRORS: i64 = 5000;
const DEFAULT_DRAIN_LIMIT: u32 = 100;
const DEFAULT_ERROR_DRAIN_LIMIT: u32 = 20;
const MAX_EVENT_NAME_LEN: usize = 128;
const MAX_ERROR_VALUE_LEN: usize = 8192;
const MAX_STACKTRACE_FRAMES: usize = 100;
const ACK_CHUNK_SIZE: usize = 500;
/// Upper bound for the settings file the early crash-reporting bootstrap reads.
/// Anything larger is not a settings file we can trust, so it is skipped.
const MAX_SETTINGS_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Resolved once during startup so the panic hook can reach the crash buffer
/// without a Tauri state lookup or an async runtime.
static CRASH_DB_PATH: OnceLock<PathBuf> = OnceLock::new();
/// Mirrors `TelemetrySettings::crash_reports_enabled` for the same reason.
/// Primed straight from the settings file by `init_crash_reporting_from_disk`
/// before the panic hook is installed, then confirmed by `init_crash_capture`.
static CRASH_REPORTS_ENABLED: AtomicBool = AtomicBool::new(false);
/// Reference point for the `app_start` performance metric. Set at the very top
/// of the desktop entrypoint so the elapsed time covers native init plus the
/// webview boot up to the frontend's first render.
static PROCESS_START: OnceLock<Instant> = OnceLock::new();

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct QueuedTelemetryEvent {
    pub id: i64,
    pub name: String,
    pub props: Option<serde_json::Value>,
    #[serde(rename = "client_ts")]
    pub client_ts: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct QueuedTelemetryError {
    pub id: i64,
    pub kind: String,
    pub value: String,
    pub level: String,
    pub stacktrace: Option<serde_json::Value>,
    pub context: Option<serde_json::Value>,
    #[serde(rename = "client_ts")]
    pub client_ts: String,
}

#[derive(Serialize, Debug, Default)]
struct BacktraceFrame {
    #[serde(skip_serializing_if = "Option::is_none")]
    function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lineno: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    colno: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    in_app: Option<bool>,
}

fn buffer_db_path_from_project_dir(project_dir: &Path) -> Result<PathBuf, TauriFunctionError> {
    let base = project_dir.parent().ok_or_else(|| {
        TauriFunctionError::new("Telemetry db dir not found: project_dir has no parent")
    })?;
    std::fs::create_dir_all(base)
        .map_err(|e| TauriFunctionError::new(&format!("Failed to create telemetry db dir: {e}")))?;
    Ok(base.join("telemetry.db"))
}

fn buffer_db_path(settings: &Settings) -> Result<PathBuf, TauriFunctionError> {
    buffer_db_path_from_project_dir(&settings.project_dir)
}

async fn enabled_buffer_path(
    app_handle: &AppHandle,
) -> Result<Option<PathBuf>, TauriFunctionError> {
    let settings = TauriSettingsState::construct(app_handle)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Settings state not found: {e}")))?;
    let settings = settings.lock().await;
    if !settings.telemetry.usage_enabled() {
        return Ok(None);
    }
    buffer_db_path(&settings).map(Some)
}

async fn crash_buffer_path(app_handle: &AppHandle) -> Result<Option<PathBuf>, TauriFunctionError> {
    let settings = TauriSettingsState::construct(app_handle)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Settings state not found: {e}")))?;
    let settings = settings.lock().await;
    if !settings.telemetry.crash_reports_enabled() {
        return Ok(None);
    }
    buffer_db_path(&settings).map(Some)
}

fn open_and_init(db_path: &PathBuf) -> Result<Connection, TauriFunctionError> {
    let conn = Connection::open(db_path)
        .map_err(|e| TauriFunctionError::new(&format!("Failed to open telemetry db: {e}")))?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS telemetry_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            props TEXT,
            client_ts TEXT NOT NULL
        )",
        [],
    )
    .map_err(|e| TauriFunctionError::new(&format!("Failed to init telemetry schema: {e}")))?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS telemetry_errors (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            value TEXT NOT NULL,
            level TEXT NOT NULL,
            stacktrace TEXT,
            context TEXT,
            client_ts TEXT NOT NULL
        )",
        [],
    )
    .map_err(|e| TauriFunctionError::new(&format!("Failed to init crash schema: {e}")))?;
    Ok(conn)
}

fn insert_event(
    db_path: &PathBuf,
    name: &str,
    props: Option<&serde_json::Value>,
) -> Result<(), TauriFunctionError> {
    let conn = open_and_init(db_path)?;
    let props = props.map(|p| p.to_string());
    let client_ts = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO telemetry_events (name, props, client_ts) VALUES (?1, ?2, ?3)",
        params![name, props, client_ts],
    )
    .map_err(|e| TauriFunctionError::new(&format!("Failed to buffer telemetry event: {e}")))?;
    conn.execute(
        "DELETE FROM telemetry_events WHERE id NOT IN (
            SELECT id FROM telemetry_events ORDER BY id DESC LIMIT ?1
        )",
        params![MAX_BUFFERED_EVENTS],
    )
    .map_err(|e| TauriFunctionError::new(&format!("Failed to cap telemetry buffer: {e}")))?;
    Ok(())
}

/// Reads the oldest events without removing them; rows stay buffered until
/// they are acknowledged via `ack_events` after a successful upload.
fn drain_events(
    db_path: &PathBuf,
    limit: u32,
) -> Result<Vec<QueuedTelemetryEvent>, TauriFunctionError> {
    let conn = open_and_init(db_path)?;
    let mut stmt = conn
        .prepare("SELECT id, name, props, client_ts FROM telemetry_events ORDER BY id ASC LIMIT ?1")
        .map_err(|e| {
            TauriFunctionError::new(&format!("Failed to prepare telemetry drain query: {e}"))
        })?;
    let rows = stmt
        .query_map(params![limit], |row| {
            let props: Option<String> = row.get(2)?;
            Ok(QueuedTelemetryEvent {
                id: row.get(0)?,
                name: row.get(1)?,
                props: props.and_then(|p| serde_json::from_str(&p).ok()),
                client_ts: row.get(3)?,
            })
        })
        .map_err(|e| TauriFunctionError::new(&format!("Failed to query telemetry events: {e}")))?;
    let mut events = Vec::new();
    for row in rows {
        events.push(
            row.map_err(|e| {
                TauriFunctionError::new(&format!("Failed to read telemetry row: {e}"))
            })?,
        );
    }
    Ok(events)
}

fn ack_events(db_path: &PathBuf, ids: &[i64]) -> Result<(), TauriFunctionError> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut conn = open_and_init(db_path)?;
    let tx = conn.transaction().map_err(|e| {
        TauriFunctionError::new(&format!("Failed to start telemetry ack transaction: {e}"))
    })?;
    for chunk in ids.chunks(ACK_CHUNK_SIZE) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        tx.execute(
            &format!("DELETE FROM telemetry_events WHERE id IN ({placeholders})"),
            rusqlite::params_from_iter(chunk.iter()),
        )
        .map_err(|e| {
            TauriFunctionError::new(&format!("Failed to delete acked telemetry events: {e}"))
        })?;
    }
    tx.commit()
        .map_err(|e| TauriFunctionError::new(&format!("Failed to commit telemetry ack: {e}")))?;
    Ok(())
}

fn purge_events(db_path: &PathBuf) -> Result<(), TauriFunctionError> {
    if !db_path.exists() {
        return Ok(());
    }
    let conn = open_and_init(db_path)?;
    conn.execute("DELETE FROM telemetry_events", [])
        .map_err(|e| TauriFunctionError::new(&format!("Failed to purge telemetry buffer: {e}")))?;
    Ok(())
}

fn truncate_value(value: &str) -> String {
    if value.len() <= MAX_ERROR_VALUE_LEN {
        return value.to_string();
    }
    let mut end = MAX_ERROR_VALUE_LEN;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn insert_error(
    db_path: &PathBuf,
    kind: &str,
    value: &str,
    level: &str,
    stacktrace: Option<&serde_json::Value>,
    context: Option<&serde_json::Value>,
) -> Result<(), TauriFunctionError> {
    let conn = open_and_init(db_path)?;
    let stacktrace = stacktrace.map(|s| s.to_string());
    let context = context.map(|c| c.to_string());
    let client_ts = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO telemetry_errors (kind, value, level, stacktrace, context, client_ts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            kind,
            truncate_value(value),
            level,
            stacktrace,
            context,
            client_ts
        ],
    )
    .map_err(|e| TauriFunctionError::new(&format!("Failed to buffer telemetry error: {e}")))?;
    conn.execute(
        "DELETE FROM telemetry_errors WHERE id NOT IN (
            SELECT id FROM telemetry_errors ORDER BY id DESC LIMIT ?1
        )",
        params![MAX_BUFFERED_ERRORS],
    )
    .map_err(|e| TauriFunctionError::new(&format!("Failed to cap crash buffer: {e}")))?;
    Ok(())
}

/// Reads the oldest crashes without removing them; rows stay buffered until
/// they are acknowledged via `ack_errors` after a successful upload.
fn drain_errors(
    db_path: &PathBuf,
    limit: u32,
) -> Result<Vec<QueuedTelemetryError>, TauriFunctionError> {
    let conn = open_and_init(db_path)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, value, level, stacktrace, context, client_ts
             FROM telemetry_errors ORDER BY id ASC LIMIT ?1",
        )
        .map_err(|e| {
            TauriFunctionError::new(&format!("Failed to prepare crash drain query: {e}"))
        })?;
    let rows = stmt
        .query_map(params![limit], |row| {
            let stacktrace: Option<String> = row.get(4)?;
            let context: Option<String> = row.get(5)?;
            Ok(QueuedTelemetryError {
                id: row.get(0)?,
                kind: row.get(1)?,
                value: row.get(2)?,
                level: row.get(3)?,
                stacktrace: stacktrace.and_then(|s| serde_json::from_str(&s).ok()),
                context: context.and_then(|c| serde_json::from_str(&c).ok()),
                client_ts: row.get(6)?,
            })
        })
        .map_err(|e| TauriFunctionError::new(&format!("Failed to query telemetry errors: {e}")))?;
    let mut errors = Vec::new();
    for row in rows {
        errors.push(
            row.map_err(|e| TauriFunctionError::new(&format!("Failed to read crash row: {e}")))?,
        );
    }
    Ok(errors)
}

fn ack_errors(db_path: &PathBuf, ids: &[i64]) -> Result<(), TauriFunctionError> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut conn = open_and_init(db_path)?;
    let tx = conn.transaction().map_err(|e| {
        TauriFunctionError::new(&format!("Failed to start crash ack transaction: {e}"))
    })?;
    for chunk in ids.chunks(ACK_CHUNK_SIZE) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        tx.execute(
            &format!("DELETE FROM telemetry_errors WHERE id IN ({placeholders})"),
            rusqlite::params_from_iter(chunk.iter()),
        )
        .map_err(|e| {
            TauriFunctionError::new(&format!("Failed to delete acked telemetry errors: {e}"))
        })?;
    }
    tx.commit()
        .map_err(|e| TauriFunctionError::new(&format!("Failed to commit crash ack: {e}")))?;
    Ok(())
}

fn purge_errors(db_path: &PathBuf) -> Result<(), TauriFunctionError> {
    if !db_path.exists() {
        return Ok(());
    }
    let conn = open_and_init(db_path)?;
    conn.execute("DELETE FROM telemetry_errors", [])
        .map_err(|e| TauriFunctionError::new(&format!("Failed to purge crash buffer: {e}")))?;
    Ok(())
}

/// The install id backs both usage telemetry and crash reporting, so it exists
/// while either consent is active and is dropped once both are off. Returns
/// whether the identity changed and the settings need to be persisted.
fn sync_anon_id(telemetry: &mut TelemetrySettings) -> bool {
    let needed = telemetry.usage_enabled() || telemetry.crash_reports_enabled();
    match (needed, telemetry.anon_id.is_some()) {
        (true, false) => {
            telemetry.anon_id = Some(create_id());
            true
        }
        (false, true) => {
            telemetry.anon_id = None;
            true
        }
        _ => false,
    }
}

/// The slice of the persisted settings file the panic hook needs before the
/// full `Settings` graph exists. Every field is optional so an older, partial
/// or hand-edited file degrades to a no-op instead of a hard failure.
#[derive(Deserialize)]
struct DiskCrashSettings {
    #[serde(default)]
    project_dir: Option<PathBuf>,
    #[serde(default)]
    telemetry: Option<TelemetrySettings>,
}

/// Reads the persisted project dir and the crash-report consent out of raw
/// settings bytes. A missing telemetry block means the user never answered the
/// prompt, and crash reporting defaults on — matching `crash_reports_enabled`.
fn parse_disk_crash_settings(bytes: &[u8]) -> Option<(Option<PathBuf>, bool)> {
    let parsed = serde_json::from_slice::<DiskCrashSettings>(bytes).ok()?;
    let crash_enabled = parsed
        .telemetry
        .as_ref()
        .is_none_or(TelemetrySettings::crash_reports_enabled);
    Some((parsed.project_dir, crash_enabled))
}

fn load_disk_crash_settings(path: &Path) -> Option<(Option<PathBuf>, bool)> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_SETTINGS_FILE_BYTES {
        return None;
    }
    parse_disk_crash_settings(&std::fs::read(path).ok()?)
}

/// Primes the crash handles directly from the settings file on disk, before the
/// panic hook is installed and long before the Tauri builder runs, so panics
/// during early startup are buffered instead of lost. Deliberately
/// dependency-free: no async runtime, no Tauri state, no logging. Every failure
/// mode — missing file, unreadable file, malformed JSON, absent project dir,
/// unwritable buffer directory — degrades to a silent no-op, and the flag is
/// only raised after the buffer path is in place. `init_crash_capture` stays
/// the authoritative update once the real settings have loaded.
pub fn init_crash_reporting_from_disk() {
    let sources = [
        crate::settings::settings_store_path(),
        crate::settings::legacy_settings_store_path(),
    ];
    let Some((project_dir, crash_enabled)) = sources
        .iter()
        .find_map(|path| load_disk_crash_settings(path))
    else {
        return;
    };
    if !crash_enabled {
        return;
    }
    let Some(project_dir) = crate::settings::resolve_project_dir(project_dir) else {
        return;
    };
    let Ok(db_path) = buffer_db_path_from_project_dir(&project_dir) else {
        return;
    };
    let _ = CRASH_DB_PATH.set(db_path);
    CRASH_REPORTS_ENABLED.store(true, Ordering::Relaxed);
}

/// Wires the process-global crash handles used by `track_error_blocking` and
/// mints the install id when either consent is active. Called once from the
/// desktop startup path before the Tauri builder runs.
pub fn init_crash_capture(settings: &mut Settings) {
    if sync_anon_id(&mut settings.telemetry) {
        settings.serialize();
    }
    CRASH_REPORTS_ENABLED.store(
        settings.telemetry.crash_reports_enabled(),
        Ordering::Relaxed,
    );
    match buffer_db_path(settings) {
        Ok(db_path) => {
            let _ = CRASH_DB_PATH.set(db_path);
        }
        Err(error) => tracing::debug!("Crash buffer path unavailable: {error}"),
    }
}

/// Records the process start reference for `app_start`. Idempotent: only the
/// first call wins, so re-entrant startup paths cannot skew the measurement.
pub fn mark_process_start() {
    let _ = PROCESS_START.set(Instant::now());
}

/// Milliseconds elapsed since process start, or `None` when the entrypoint did
/// not mark it (tests, embedded runs). Anonymous by construction: a duration
/// carries no identity, so no consent gate is needed here — the frontend only
/// forwards it when usage telemetry is granted.
#[tauri::command(async)]
pub async fn app_start_elapsed_ms() -> Result<Option<u64>, TauriFunctionError> {
    Ok(PROCESS_START
        .get()
        .map(|start| u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)))
}

fn frame_is_in_app(file: &str) -> bool {
    !(file.contains("/rustc/")
        || file.contains("\\rustc\\")
        || file.contains(".cargo")
        || file.contains("/library/std/")
        || file.contains("/library/core/")
        || file.contains("/library/alloc/"))
}

fn frame_function(line: &str) -> Option<&str> {
    let (index, function) = line.trim_start().split_once(": ")?;
    if index.is_empty() || !index.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let function = function.trim();
    if function.is_empty() {
        return None;
    }
    Some(function)
}

fn frame_location(line: &str) -> Option<(&str, Option<u32>, Option<u32>)> {
    let mut file = line.trim_start().strip_prefix("at ")?.trim();
    if file.is_empty() {
        return None;
    }
    let mut lineno = None;
    let mut colno = None;
    if let Some((head, tail)) = file.rsplit_once(':')
        && let Ok(number) = tail.parse::<u32>()
    {
        colno = Some(number);
        file = head;
    }
    if let Some((head, tail)) = file.rsplit_once(':')
        && let Ok(number) = tail.parse::<u32>()
    {
        lineno = Some(number);
        file = head;
    }
    if lineno.is_none() {
        lineno = colno.take();
    }
    Some((file, lineno, colno))
}

/// Turns a rendered `std::backtrace::Backtrace` into ingest stack frames
/// (innermost first). Returns `None` when nothing could be parsed, so callers
/// can fall back to the raw string.
pub fn parse_backtrace_frames(backtrace: &str) -> Option<serde_json::Value> {
    let mut frames: Vec<BacktraceFrame> = Vec::new();
    for line in backtrace.lines() {
        if let Some((file, lineno, colno)) = frame_location(line) {
            if let Some(frame) = frames.last_mut()
                && frame.file.is_none()
            {
                frame.in_app = Some(frame_is_in_app(file));
                frame.file = Some(file.to_string());
                frame.lineno = lineno;
                frame.colno = colno;
            }
            continue;
        }
        if frames.len() >= MAX_STACKTRACE_FRAMES {
            break;
        }
        if let Some(function) = frame_function(line) {
            frames.push(BacktraceFrame {
                function: Some(function.to_string()),
                ..BacktraceFrame::default()
            });
        }
    }
    if frames.is_empty() {
        return None;
    }
    serde_json::to_value(frames).ok()
}

/// Reusable desktop-Rust telemetry capture. Buffers the event in the local
/// durable queue when the user has opted in (`enabled == Some(true)`);
/// otherwise a silent no-op. Never fails the caller — buffer errors are only
/// logged at debug level.
pub async fn track(app_handle: &AppHandle, name: &str, props: Option<serde_json::Value>) {
    match enabled_buffer_path(app_handle).await {
        Ok(Some(db_path)) => {
            if let Err(error) = insert_event(&db_path, name, props.as_ref()) {
                tracing::debug!("Failed to buffer telemetry event '{name}': {error}");
            }
        }
        Ok(None) => {}
        Err(error) => tracing::debug!("Telemetry buffer unavailable: {error}"),
    }
}

/// Synchronous crash writer for contexts that cannot await or take locks —
/// primarily the panic hook. Resolves everything from process globals wired by
/// `init_crash_capture` and no-ops when crash reporting is off or startup has
/// not wired them yet. Every path is fallible-by-construction (no unwrap, no
/// indexing, no logging): a nested panic inside a panic hook aborts the process
/// before `catch_unwind` could intervene, so this must not panic at all.
pub fn track_error_blocking(
    kind: &str,
    value: &str,
    level: &str,
    stacktrace: Option<serde_json::Value>,
    context: Option<serde_json::Value>,
) {
    if !CRASH_REPORTS_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let Some(db_path) = CRASH_DB_PATH.get() else {
        return;
    };
    let _ = insert_error(
        db_path,
        kind,
        value,
        level,
        stacktrace.as_ref(),
        context.as_ref(),
    );
}

#[tauri::command(async)]
pub async fn get_telemetry_settings(
    app_handle: AppHandle,
) -> Result<TelemetrySettings, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Settings state not found: {e}")))?;
    let settings = settings.lock().await;
    Ok(settings.telemetry.clone())
}

#[tauri::command(async)]
pub async fn set_telemetry_enabled(
    app_handle: AppHandle,
    enabled: bool,
) -> Result<TelemetrySettings, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Settings state not found: {e}")))?;
    let (telemetry, purge_path) = {
        let mut settings = settings.lock().await;
        settings.telemetry.enabled = Some(enabled);
        sync_anon_id(&mut settings.telemetry);
        Settings::serialize(&mut settings);
        (settings.telemetry.clone(), buffer_db_path(&settings))
    };
    // Purge on both transitions: disable drops the buffer, enable clears any
    // straggler rows from a previous identity before the fresh anon id is used.
    // Settings are already persisted, so purge failures must not fail the command.
    match purge_path {
        Ok(db_path) => {
            if let Err(error) = purge_events(&db_path) {
                tracing::warn!("Failed to purge telemetry buffer on settings change: {error}");
            }
            // The install id is shared with crash reporting. Once both consents
            // are off the identity is gone, so buffered crashes go with it.
            if !telemetry.crash_reports_enabled()
                && let Err(error) = purge_errors(&db_path)
            {
                tracing::warn!("Failed to purge crash buffer on settings change: {error}");
            }
        }
        Err(error) => {
            tracing::warn!("Telemetry buffer path unavailable, skipping purge: {error}");
        }
    }
    Ok(telemetry)
}

#[tauri::command(async)]
pub async fn set_crash_reports_enabled(
    app_handle: AppHandle,
    enabled: bool,
) -> Result<TelemetrySettings, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Settings state not found: {e}")))?;
    let (telemetry, purge_path, purge_buffer) = {
        let mut settings = settings.lock().await;
        let was_enabled = settings.telemetry.crash_reports_enabled();
        settings.telemetry.crash_reports = Some(enabled);
        sync_anon_id(&mut settings.telemetry);
        Settings::serialize(&mut settings);
        (
            settings.telemetry.clone(),
            buffer_db_path(&settings),
            !enabled || !was_enabled,
        )
    };
    CRASH_REPORTS_ENABLED.store(enabled, Ordering::Relaxed);
    // Disabling drops the buffered crashes, re-enabling clears stragglers from a
    // previous install id. Re-affirming an already active consent keeps the
    // pending buffer, since crash reporting is on by default.
    // Settings are already persisted, so purge failures must not fail the command.
    if purge_buffer {
        match purge_path {
            Ok(db_path) => {
                if let Err(error) = purge_errors(&db_path) {
                    tracing::warn!("Failed to purge crash buffer on settings change: {error}");
                }
            }
            Err(error) => {
                tracing::warn!("Crash buffer path unavailable, skipping purge: {error}");
            }
        }
    }
    Ok(telemetry)
}

#[tauri::command(async)]
pub async fn queue_telemetry_event(
    app_handle: AppHandle,
    name: String,
    props: Option<serde_json::Value>,
) -> Result<(), TauriFunctionError> {
    if name.is_empty() || name.len() > MAX_EVENT_NAME_LEN {
        return Err(TauriFunctionError::new(&format!(
            "Invalid telemetry event name length: {} (must be 1..={MAX_EVENT_NAME_LEN})",
            name.len()
        )));
    }
    let Some(db_path) = enabled_buffer_path(&app_handle).await? else {
        return Ok(());
    };
    insert_event(&db_path, &name, props.as_ref())
}

#[tauri::command(async)]
pub async fn drain_telemetry_events(
    app_handle: AppHandle,
    limit: Option<u32>,
) -> Result<Vec<QueuedTelemetryEvent>, TauriFunctionError> {
    let Some(db_path) = enabled_buffer_path(&app_handle).await? else {
        return Ok(Vec::new());
    };
    drain_events(&db_path, limit.unwrap_or(DEFAULT_DRAIN_LIMIT))
}

#[tauri::command(async)]
pub async fn ack_telemetry_events(
    app_handle: AppHandle,
    ids: Vec<i64>,
) -> Result<(), TauriFunctionError> {
    let Some(db_path) = enabled_buffer_path(&app_handle).await? else {
        return Ok(());
    };
    ack_events(&db_path, &ids)
}

#[tauri::command(async)]
pub async fn drain_telemetry_errors(
    app_handle: AppHandle,
    limit: Option<u32>,
) -> Result<Vec<QueuedTelemetryError>, TauriFunctionError> {
    let Some(db_path) = crash_buffer_path(&app_handle).await? else {
        return Ok(Vec::new());
    };
    drain_errors(&db_path, limit.unwrap_or(DEFAULT_ERROR_DRAIN_LIMIT))
}

#[tauri::command(async)]
pub async fn ack_telemetry_errors(
    app_handle: AppHandle,
    ids: Vec<i64>,
) -> Result<(), TauriFunctionError> {
    let Some(db_path) = crash_buffer_path(&app_handle).await? else {
        return Ok(());
    };
    ack_errors(&db_path, &ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "flow-like-telemetry-{tag}-{}.db",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn temp_settings(tag: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "flow-like-settings-{tag}-{}.json",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn drain_keeps_rows_until_acked() {
        let db = temp_db("drain");
        insert_event(&db, "first", None).unwrap();
        insert_event(
            &db,
            "second",
            Some(&serde_json::json!({"path": "/library"})),
        )
        .unwrap();
        insert_event(&db, "third", None).unwrap();

        let drained = drain_events(&db, 2).unwrap();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].name, "first");
        assert_eq!(drained[1].name, "second");
        assert_eq!(
            drained[1].props,
            Some(serde_json::json!({"path": "/library"}))
        );
        assert!(!drained[0].client_ts.is_empty());

        let again = drain_events(&db, 2).unwrap();
        assert_eq!(
            again.iter().map(|e| e.id).collect::<Vec<_>>(),
            drained.iter().map(|e| e.id).collect::<Vec<_>>()
        );

        ack_events(&db, &[drained[0].id, drained[1].id]).unwrap();
        let rest = drain_events(&db, 10).unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].name, "third");

        ack_events(&db, &[rest[0].id]).unwrap();
        assert!(drain_events(&db, 10).unwrap().is_empty());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn ack_removes_only_the_acked_ids() {
        let db = temp_db("ack");
        insert_event(&db, "a", None).unwrap();
        insert_event(&db, "b", None).unwrap();
        insert_event(&db, "c", None).unwrap();

        let drained = drain_events(&db, 10).unwrap();
        assert_eq!(drained.len(), 3);
        ack_events(&db, &[drained[1].id]).unwrap();
        ack_events(&db, &[]).unwrap();

        let rest = drain_events(&db, 10).unwrap();
        assert_eq!(
            rest.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["a", "c"]
        );
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn purge_empties_the_buffer() {
        let db = temp_db("purge");
        insert_event(&db, "event", None).unwrap();
        purge_events(&db).unwrap();
        assert!(drain_events(&db, 10).unwrap().is_empty());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn purge_of_missing_buffer_is_a_noop() {
        let db = temp_db("missing");
        purge_events(&db).unwrap();
        assert!(!db.exists());
    }

    #[test]
    fn cap_drops_oldest_rows() {
        let db = temp_db("cap");
        for i in 0..(MAX_BUFFERED_EVENTS + 5) {
            insert_event(&db, &format!("event_{i}"), None).unwrap();
        }
        let drained = drain_events(&db, (MAX_BUFFERED_EVENTS + 10) as u32).unwrap();
        assert_eq!(drained.len(), MAX_BUFFERED_EVENTS as usize);
        assert_eq!(drained[0].name, "event_5");
        let _ = std::fs::remove_file(&db);
    }

    const SAMPLE_BACKTRACE: &str = concat!(
        "   0: flow_like_desktop::functions::telemetry::boom\n",
        "             at /home/dev/flow-like/apps/desktop/src-tauri/src/functions/telemetry.rs:42:9\n",
        "   1: core::panicking::panic_fmt\n",
        "             at /rustc/abc123/library/core/src/panicking.rs:72:14\n",
        "   2: <unknown>\n",
    );

    fn insert_test_error(db: &PathBuf, value: &str) {
        insert_error(db, "panic", value, "fatal", None, None).unwrap();
    }

    #[test]
    fn error_drain_keeps_rows_until_acked() {
        let db = temp_db("error-drain");
        insert_test_error(&db, "first");
        insert_error(
            &db,
            "Error",
            "second",
            "error",
            Some(&serde_json::json!([{ "function": "run", "in_app": true }])),
            Some(&serde_json::json!({ "route": "/library" })),
        )
        .unwrap();
        insert_test_error(&db, "third");

        let drained = drain_errors(&db, 2).unwrap();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].value, "first");
        assert_eq!(drained[0].kind, "panic");
        assert_eq!(drained[0].level, "fatal");
        assert!(drained[0].stacktrace.is_none());
        assert!(!drained[0].client_ts.is_empty());
        assert_eq!(
            drained[1].stacktrace,
            Some(serde_json::json!([{ "function": "run", "in_app": true }]))
        );
        assert_eq!(
            drained[1].context,
            Some(serde_json::json!({ "route": "/library" }))
        );

        let again = drain_errors(&db, 2).unwrap();
        assert_eq!(
            again.iter().map(|e| e.id).collect::<Vec<_>>(),
            drained.iter().map(|e| e.id).collect::<Vec<_>>()
        );

        ack_errors(&db, &[drained[0].id, drained[1].id]).unwrap();
        let rest = drain_errors(&db, 10).unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].value, "third");

        ack_errors(&db, &[rest[0].id]).unwrap();
        assert!(drain_errors(&db, 10).unwrap().is_empty());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn error_ack_removes_only_the_acked_ids() {
        let db = temp_db("error-ack");
        insert_test_error(&db, "a");
        insert_test_error(&db, "b");
        insert_test_error(&db, "c");

        let drained = drain_errors(&db, 10).unwrap();
        assert_eq!(drained.len(), 3);
        ack_errors(&db, &[drained[1].id]).unwrap();
        ack_errors(&db, &[]).unwrap();

        let rest = drain_errors(&db, 10).unwrap();
        assert_eq!(
            rest.iter().map(|e| e.value.as_str()).collect::<Vec<_>>(),
            vec!["a", "c"]
        );
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn purge_errors_leaves_the_event_buffer_intact() {
        let db = temp_db("error-purge");
        insert_event(&db, "event", None).unwrap();
        insert_test_error(&db, "crash");

        purge_errors(&db).unwrap();
        assert!(drain_errors(&db, 10).unwrap().is_empty());
        assert_eq!(drain_events(&db, 10).unwrap().len(), 1);

        purge_events(&db).unwrap();
        assert!(drain_events(&db, 10).unwrap().is_empty());
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn purge_of_missing_error_buffer_is_a_noop() {
        let db = temp_db("error-missing");
        purge_errors(&db).unwrap();
        assert!(!db.exists());
    }

    #[test]
    fn error_cap_drops_oldest_rows() {
        let db = temp_db("error-cap");
        for i in 0..(MAX_BUFFERED_ERRORS + 5) {
            insert_test_error(&db, &format!("crash_{i}"));
        }
        let drained = drain_errors(&db, (MAX_BUFFERED_ERRORS + 10) as u32).unwrap();
        assert_eq!(drained.len(), MAX_BUFFERED_ERRORS as usize);
        assert_eq!(drained[0].value, "crash_5");
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn oversized_error_values_are_truncated() {
        let db = temp_db("error-truncate");
        let value = "ü".repeat(MAX_ERROR_VALUE_LEN);
        insert_test_error(&db, &value);
        let drained = drain_errors(&db, 1).unwrap();
        assert!(drained[0].value.len() <= MAX_ERROR_VALUE_LEN);
        assert!(drained[0].value.starts_with('ü'));
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn track_error_blocking_without_wiring_is_a_noop() {
        track_error_blocking("panic", "unwired", "fatal", None, None);
        assert!(!CRASH_REPORTS_ENABLED.load(Ordering::Relaxed));
    }

    #[test]
    fn crash_reports_default_to_enabled() {
        let mut telemetry = TelemetrySettings::default();
        assert!(telemetry.crash_reports_enabled());
        assert!(!telemetry.usage_enabled());

        telemetry.crash_reports = Some(true);
        assert!(telemetry.crash_reports_enabled());
        telemetry.crash_reports = Some(false);
        assert!(!telemetry.crash_reports_enabled());

        telemetry.enabled = Some(false);
        assert!(!telemetry.usage_enabled());
        telemetry.enabled = Some(true);
        assert!(telemetry.usage_enabled());
    }

    #[test]
    fn anon_id_lives_while_either_consent_is_active() {
        let mut telemetry = TelemetrySettings::default();
        assert!(sync_anon_id(&mut telemetry));
        let minted = telemetry.anon_id.clone().unwrap();
        assert!(!sync_anon_id(&mut telemetry));

        telemetry.enabled = Some(false);
        assert!(!sync_anon_id(&mut telemetry));
        assert_eq!(telemetry.anon_id.as_deref(), Some(minted.as_str()));

        telemetry.crash_reports = Some(false);
        assert!(sync_anon_id(&mut telemetry));
        assert!(telemetry.anon_id.is_none());

        telemetry.enabled = Some(true);
        assert!(sync_anon_id(&mut telemetry));
        assert_ne!(telemetry.anon_id.clone().unwrap(), minted);
    }

    #[test]
    fn backtrace_frames_carry_locations_and_in_app() {
        let frames = parse_backtrace_frames(SAMPLE_BACKTRACE).unwrap();
        let frames = frames.as_array().unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(
            frames[0]["function"],
            "flow_like_desktop::functions::telemetry::boom"
        );
        assert_eq!(frames[0]["lineno"], 42);
        assert_eq!(frames[0]["colno"], 9);
        assert_eq!(frames[0]["in_app"], true);
        assert_eq!(frames[1]["function"], "core::panicking::panic_fmt");
        assert_eq!(frames[1]["in_app"], false);
        assert_eq!(frames[2]["function"], "<unknown>");
        assert!(frames[2].get("file").is_none());
    }

    #[test]
    fn process_start_is_marked_once() {
        assert!(PROCESS_START.get().is_none());
        mark_process_start();
        let first = *PROCESS_START.get().unwrap();
        mark_process_start();
        assert_eq!(*PROCESS_START.get().unwrap(), first);
    }

    #[test]
    fn backtrace_without_frames_is_none() {
        assert!(parse_backtrace_frames("note: backtrace unavailable").is_none());
        assert!(parse_backtrace_frames("").is_none());
    }

    #[test]
    fn disk_settings_yield_project_dir_and_consent() {
        let root = std::env::temp_dir().join(format!(
            "flow-like-early-crash-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project_dir = root.join("projects");
        let contents = serde_json::json!({
            "project_dir": project_dir,
            "logs_dir": root.join("logs"),
            "telemetry": { "enabled": true, "crashReports": true, "anonId": "abc" }
        })
        .to_string();
        let path = temp_settings("valid", &contents);

        let (parsed_dir, crash_enabled) = load_disk_crash_settings(&path).unwrap();
        assert_eq!(parsed_dir.as_ref(), Some(&project_dir));
        assert!(crash_enabled);
        assert_eq!(
            buffer_db_path_from_project_dir(&project_dir).unwrap(),
            root.join("telemetry.db")
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn disk_settings_of_missing_file_are_none() {
        let path = std::env::temp_dir().join("flow-like-settings-does-not-exist.json");
        let _ = std::fs::remove_file(&path);
        assert!(load_disk_crash_settings(&path).is_none());
    }

    #[test]
    fn malformed_disk_settings_are_none() {
        let path = temp_settings("malformed", r#"{"project_dir": "a", "telemetry":"#);
        assert!(load_disk_crash_settings(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn disk_settings_without_telemetry_block_default_to_enabled() {
        let path = temp_settings("no-telemetry", r#"{"project_dir":"flow-like/projects"}"#);
        let (project_dir, crash_enabled) = load_disk_crash_settings(&path).unwrap();
        assert_eq!(project_dir, Some(PathBuf::from("flow-like/projects")));
        assert!(crash_enabled);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn disk_settings_honour_an_explicit_crash_opt_out() {
        let path = temp_settings(
            "opt-out",
            r#"{"project_dir":"flow-like/projects","telemetry":{"crashReports":false}}"#,
        );
        let (_, crash_enabled) = load_disk_crash_settings(&path).unwrap();
        assert!(!crash_enabled);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn disk_settings_without_project_dir_carry_consent_only() {
        let path = temp_settings("no-project-dir", r#"{"telemetry":{"crashReports":true}}"#);
        let (project_dir, crash_enabled) = load_disk_crash_settings(&path).unwrap();
        assert!(project_dir.is_none());
        assert!(crash_enabled);
        let _ = std::fs::remove_file(&path);
    }
}

use flow_like::flow::execution::ExecutionEnvironment;
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

static LAST_EMIT: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

pub fn local_execution_environment() -> ExecutionEnvironment {
    ExecutionEnvironment::from_env().unwrap_or({
        if cfg!(any(target_os = "android", target_os = "ios")) {
            ExecutionEnvironment::Mobile
        } else {
            ExecutionEnvironment::Desktop
        }
    })
}

#[derive(Clone, Debug)]
pub enum UiEmitTarget {
    Main,
    Label(String),
    All,
}

fn should_emit(key: &str, min_interval: Duration) -> bool {
    let now = Instant::now();
    let map = LAST_EMIT.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut m) = map.try_lock() {
        let ok = m
            .get(key)
            .map(|last| now.duration_since(*last) >= min_interval)
            .unwrap_or(true);
        if ok {
            m.insert(key.to_string(), now);
        }
        ok
    } else {
        false
    }
}

/// Schedule a frontend emission on Tauri's main thread and return without waiting for it.
///
/// `Emitter::emit` holds Tauri's process-wide `webviews_lock` for the whole emission while
/// `Webview::eval` blocks on the main run loop. The main thread takes that same lock from
/// `AppManager::get_webview` while servicing a WebKit URL-scheme task — which is how every IPC
/// message arrives. Emitting from a Tokio worker therefore deadlocks the desktop process: the
/// worker holds the lock and waits for the main loop, the main loop waits for the lock. Emitting
/// on the main thread instead serializes the emission with those URL-scheme handlers.
///
/// `run_on_main_thread` executes the job inline when the caller already is the main thread, so
/// this is the correct call from every thread and never blocks the caller.
fn emit_on_main_thread<F>(app: &AppHandle, event: &str, emit: F)
where
    F: FnOnce(&AppHandle) + Send + 'static,
{
    let app_handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || emit(&app_handle)) {
        tracing::warn!("Failed to schedule the UI event '{event}' on the main thread: {error}");
    }
}

/// Emit an event to every frontend target from any thread.
///
/// This is the only supported way to reach the frontend from backend code; calling
/// `Emitter::emit` directly off the main thread deadlocks the process. See
/// [`emit_on_main_thread`] for the mechanism.
pub fn emit_to_ui<T>(app: &AppHandle, event: &str, payload: T)
where
    T: Serialize + Clone + Send + 'static,
{
    let event_name = event.to_string();
    emit_on_main_thread(app, event, move |app| {
        if let Err(error) = app.emit(&event_name, payload) {
            tracing::warn!("Failed to emit the UI event '{event_name}': {error}");
        }
    });
}

pub fn emit_throttled<T>(
    app: &AppHandle,
    target: UiEmitTarget,
    event: &str,
    payload: T,
    min_interval: Duration,
) where
    T: Serialize + Clone + Send + 'static,
{
    let throttle_key = match &target {
        UiEmitTarget::Main => format!("{event}::main"),
        UiEmitTarget::Label(label) => format!("{event}::label::{label}"),
        UiEmitTarget::All => format!("{event}::all"),
    };
    if !should_emit(&throttle_key, min_interval) {
        return;
    }

    let event_name = event.to_string();
    emit_on_main_thread(app, event, move |app| match target {
        UiEmitTarget::Main => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.emit(&event_name, payload);
            } else {
                let _ = app.emit_to("main", &event_name, payload);
            }
        }
        UiEmitTarget::Label(label) => {
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.emit(&event_name, payload);
            }
        }
        UiEmitTarget::All => {
            for (_, window) in app.webview_windows() {
                let _ = window.emit(&event_name, &payload);
            }
        }
    });
}

pub fn emit_to_main_throttled<T>(app: &AppHandle, event: &str, payload: T, min_interval: Duration)
where
    T: Serialize + Clone + Send + 'static,
{
    emit_throttled(app, UiEmitTarget::Main, event, payload, min_interval);
}

struct PendingEventBatch {
    events: Vec<flow_like_types::intercom::InterComEvent>,
    target: UiEmitTarget,
    event_name: String,
    last_emit: Option<Instant>,
    trailing_scheduled: bool,
}

static PENDING_BATCHES: OnceLock<Mutex<HashMap<String, PendingEventBatch>>> = OnceLock::new();

fn pending_batches() -> &'static Mutex<HashMap<String, PendingEventBatch>> {
    PENDING_BATCHES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Throttled emitter for intercom event batches. Unlike [`emit_throttled`],
/// which is for latest-state-wins payloads and DISCARDS anything arriving
/// inside the throttle window, this never drops content: a throttled batch is
/// appended to a pending buffer and delivered by a trailing flush once the
/// window elapses. Event streams rebuilt by accumulating deltas (chat tokens)
/// must go through this.
pub fn emit_event_batch_throttled(
    app: &AppHandle,
    target: UiEmitTarget,
    event: &str,
    events: Vec<flow_like_types::intercom::InterComEvent>,
    min_interval: Duration,
) {
    if events.is_empty() {
        return;
    }
    let key = match &target {
        UiEmitTarget::Main => format!("{event}::main"),
        UiEmitTarget::Label(label) => format!("{event}::label::{label}"),
        UiEmitTarget::All => format!("{event}::all"),
    };

    let emit_now = {
        let mut map = pending_batches()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = map.entry(key.clone()).or_insert_with(|| PendingEventBatch {
            events: Vec::new(),
            target: target.clone(),
            event_name: event.to_string(),
            last_emit: None,
            trailing_scheduled: false,
        });
        entry.events.extend(events);
        entry.target = target;
        if entry.trailing_scheduled {
            // The already-scheduled flush will carry these events.
            false
        } else if entry
            .last_emit
            .is_none_or(|last| last.elapsed() >= min_interval)
        {
            true
        } else {
            entry.trailing_scheduled = true;
            let delay = min_interval
                .saturating_sub(entry.last_emit.map(|last| last.elapsed()).unwrap_or_default());
            let app = app.clone();
            let trailing_key = key.clone();
            tauri::async_runtime::spawn(async move {
                flow_like_types::tokio::time::sleep(delay).await;
                flush_pending_event_batch(&app, &trailing_key);
            });
            false
        }
    };

    if emit_now {
        flush_pending_event_batch(app, &key);
    }
}

fn flush_pending_event_batch(app: &AppHandle, key: &str) {
    let flushed = {
        let mut map = pending_batches()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        map.get_mut(key).and_then(|entry| {
            entry.trailing_scheduled = false;
            if entry.events.is_empty() {
                return None;
            }
            entry.last_emit = Some(Instant::now());
            Some((
                entry.target.clone(),
                entry.event_name.clone(),
                std::mem::take(&mut entry.events),
            ))
        })
    };
    let Some((target, event_name, events)) = flushed else {
        return;
    };

    emit_on_main_thread(app, &event_name.clone(), move |app| match target {
        UiEmitTarget::Main => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.emit(&event_name, events);
            } else {
                let _ = app.emit_to("main", &event_name, events);
            }
        }
        UiEmitTarget::Label(label) => {
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.emit(&event_name, events);
            }
        }
        UiEmitTarget::All => {
            for (_, window) in app.webview_windows() {
                let _ = window.emit(&event_name, &events);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// Files allowed to call `Emitter::emit` directly, because every emission in them already
    /// runs inside a `run_on_main_thread` job: this module, and the FlowPilot bridge, which owns
    /// its own bounded main-thread dispatch.
    const MAIN_THREAD_EMIT_OWNERS: [&str; 2] = ["utils.rs", "functions/ai/frontend_tool_bridge.rs"];

    const EMIT_CALLS: [&str; 4] = [".emit(", ".emit_to(", ".emit_filter(", ".emit_str("];

    fn collect_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rust_sources(&path, sources);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push(path);
            }
        }
    }

    #[test]
    fn emitter_emit_is_confined_to_the_main_thread_helpers() {
        let source_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut sources = Vec::new();
        collect_rust_sources(&source_root, &mut sources);
        assert!(
            !sources.is_empty(),
            "no Rust sources found under {}",
            source_root.display()
        );

        let mut offenders = Vec::new();
        for path in sources {
            let relative = path
                .strip_prefix(&source_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if MAIN_THREAD_EMIT_OWNERS.contains(&relative.as_str()) {
                continue;
            }
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            for (index, line) in contents.lines().enumerate() {
                let code = line.split_once("//").map_or(line, |(code, _)| code);
                if EMIT_CALLS.iter().any(|call| code.contains(call)) {
                    offenders.push(format!("{relative}:{}", index + 1));
                }
            }
        }

        assert!(
            offenders.is_empty(),
            "Emitter::emit must never run on a Tokio worker: it holds Tauri's webviews_lock while \
             Webview::eval waits for the main run loop, deadlocking against the WebKit URL-scheme \
             handler that takes the same lock. Use crate::utils::emit_to_ui instead. Offenders: {offenders:?}"
        );
    }
}

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

/// Schedule a frontend emission on Tauri's main thread.
///
/// `Emitter::emit` holds Tauri's `webviews_lock` while delivering events. With Tauri's
/// `tracing` feature, `Webview::eval` waits for the main run loop. An IPC handler on that
/// loop can be waiting for the same lock, deadlocking both threads. Main-thread emission
/// serializes these operations and also works when tracing is enabled for diagnostics.
///
/// Worker callers return after queuing the job. Calls already on the main thread run inline.
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
/// `Emitter::emit` directly off the main thread can deadlock with tracing enabled. See
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
            let delay = min_interval.saturating_sub(
                entry
                    .last_emit
                    .map(|last| last.elapsed())
                    .unwrap_or_default(),
            );
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

    const EMIT_CALLS: [&str; 4] = [".emit(", ".emit_to(", ".emit_filter(", ".emit_str("];

    fn direct_emit_lines(relative: &str, contents: &str) -> Vec<usize> {
        if relative == "utils.rs" {
            return Vec::new();
        }

        let mut in_main_thread_helper = false;
        let mut offenders = Vec::new();
        for (index, line) in contents.lines().enumerate() {
            // The bridge has a bounded dispatch with its own result channel. Only its
            // module-level helper runs exclusively inside those main-thread jobs.
            if relative == "functions/ai/frontend_tool_bridge.rs"
                && line.starts_with("fn emit_to_owner<")
            {
                in_main_thread_helper = true;
            }
            if in_main_thread_helper {
                if line == "}" {
                    in_main_thread_helper = false;
                }
                continue;
            }
            let code = line.split_once("//").map_or(line, |(code, _)| code);
            if EMIT_CALLS.iter().any(|call| code.contains(call)) {
                offenders.push(index + 1);
            }
        }
        offenders
    }

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
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in direct_emit_lines(&relative, &contents) {
                offenders.push(format!("{relative}:{line}"));
            }
        }

        assert!(
            offenders.is_empty(),
            "Emitter::emit must run on the main thread: with tracing enabled, it holds Tauri's \
             webviews_lock while Webview::eval waits for that thread, which may need the same lock \
             to handle IPC. Use crate::utils::emit_to_ui instead. Offenders: {offenders:?}"
        );
    }

    #[test]
    fn bridge_exemption_does_not_cover_background_notifications() {
        let source = concat!(
            "fn emit_to_owner<S>(app: &AppHandle, payload: S) {\n",
            "    app.emit(\"request\", payload);\n",
            "}\n",
            "async fn notify(app: &AppHandle) {\n",
            "    app.emit(\"completed\", ());\n",
            "}\n",
        );
        assert_eq!(
            direct_emit_lines("functions/ai/frontend_tool_bridge.rs", source),
            vec![5],
        );
        assert_eq!(direct_emit_lines("native_auth.rs", source), vec![2, 5]);
    }
}

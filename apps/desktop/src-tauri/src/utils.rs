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
    ExecutionEnvironment::from_env().unwrap_or_else(|| {
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

pub fn emit_throttled<T>(
    app: &AppHandle,
    target: UiEmitTarget,
    event: &str,
    payload: T,
    min_interval: Duration,
) where
    T: Serialize + Send + 'static + Clone,
{
    let throttle_key = match &target {
        UiEmitTarget::Main => format!("{event}::main"),
        UiEmitTarget::Label(label) => format!("{event}::label::{label}"),
        UiEmitTarget::All => format!("{event}::all"),
    };
    if !should_emit(&throttle_key, min_interval) {
        return;
    }

    let app_cloned = app.clone();
    let target_cloned = target.clone();
    let event_str = event.to_string();
    let payload_cloned = payload.clone();

    tauri::async_runtime::spawn(async move {
        let app_for_method = app_cloned.clone();
        let app_for_closure = app_cloned.clone();
        let event_inner = event_str.clone();
        let target_inner = target_cloned.clone();
        let payload_inner = payload_cloned.clone();

        let _ = app_for_method.run_on_main_thread(move || match target_inner {
            UiEmitTarget::Main => {
                if let Some(win) = app_for_closure.get_webview_window("main") {
                    let _ = win.emit(event_inner.as_str(), &payload_inner);
                } else {
                    let _ = app_for_closure.emit_to("main", event_inner.as_str(), &payload_inner);
                }
            }
            UiEmitTarget::Label(label) => {
                if let Some(win) = app_for_closure.get_webview_window(&label) {
                    let _ = win.emit(event_inner.as_str(), &payload_inner);
                }
            }
            UiEmitTarget::All => {
                for (_, win) in app_for_closure.webview_windows() {
                    let _ = win.emit(event_inner.as_str(), &payload_inner);
                }
            }
        });
    });
}

pub fn emit_to_main_throttled<T>(app: &AppHandle, event: &str, payload: T, min_interval: Duration)
where
    T: Serialize + Send + 'static + Clone,
{
    emit_throttled(app, UiEmitTarget::Main, event, payload, min_interval);
}

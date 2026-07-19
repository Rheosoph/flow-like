mod deeplink;
mod event_bus;
mod event_sink;
mod functions;
mod profile;
mod settings;
mod state;
#[cfg(desktop)]
mod tray;
pub mod utils;

// Stub for tray_update_state on non-desktop platforms
#[cfg(not(desktop))]
#[tauri::command]
async fn tray_update_state() -> Result<(), String> {
    Ok(())
}

use flow_like::{
    flow_like_storage::{
        Path,
        files::store::{FlowLikeStore, local_store::LocalObjectStore},
        lancedb,
    },
    state::{FlowLikeConfig, FlowLikeState},
    utils::http::HTTPClient,
};
use flow_like_catalog::{get_catalog, initialize as initialize_catalog};
use flow_like_types::{sync::Mutex, tokio::time::interval};
use settings::Settings;
use state::TauriFlowLikeState;
use std::{sync::Arc, time::Duration};
use tauri::{AppHandle, Manager};
use tauri_plugin_deep_link::DeepLinkExt;
#[cfg(desktop)]
use tauri_plugin_updater::UpdaterExt;

#[cfg(not(debug_assertions))]
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{deeplink::handle_deep_link, event_bus::EventBus};

#[cfg(target_os = "macos")]
fn disable_app_nap() {
    macos_app_nap::prevent();
}

#[cfg(not(target_os = "macos"))]
fn disable_app_nap() {}

/// Disable the WKWebView scroll view's automatic content inset adjustment.
/// Without this, iOS adds a system-level content inset on top of our CSS
/// `env(safe-area-inset-*)` padding, causing double safe-area offsets.
///
/// IMPORTANT: Must NEVER be called synchronously during `setup` — the
/// WKWebView is not fully initialised at that point and `with_webview`
/// will crash. Only call from the delayed retry loop (≥100 ms).
#[cfg(target_os = "ios")]
fn harden_ios_webview_scroll(window: &tauri::WebviewWindow) {
    if let Err(err) = window.with_webview(|webview| {
        unsafe {
            use objc2::runtime::AnyObject;

            let wk_webview: *mut AnyObject = webview.inner().cast();
            if wk_webview.is_null() {
                return;
            }

            let scroll_view: *mut AnyObject = objc2::msg_send![wk_webview, scrollView];
            if scroll_view.is_null() {
                return;
            }

            // Disable rubber-band overscroll and force no automatic inset adjustment.
            let _: () = objc2::msg_send![scroll_view, setBounces: false];
            let _: () = objc2::msg_send![scroll_view, setAlwaysBounceVertical: false];
            let _: () = objc2::msg_send![scroll_view, setAlwaysBounceHorizontal: false];
            // UIScrollViewContentInsetAdjustmentNever = 2
            let _: () = objc2::msg_send![scroll_view, setContentInsetAdjustmentBehavior: 2isize];

            // Disable link previews (3D Touch peek/pop) — avoids accidental long-press pauses
            let _: () = objc2::msg_send![wk_webview, setAllowsLinkPreview: false];

            // Disable data detectors (phone numbers, addresses, etc.) — reduces layout cost
            // on pages with lots of text content.
            let configuration: *mut AnyObject = objc2::msg_send![wk_webview, configuration];
            if !configuration.is_null() {
                // WKDataDetectorTypeNone = 0
                let _: () = objc2::msg_send![configuration, setDataDetectorTypes: 0u64];
            }
        }
    }) {
        tracing::warn!("Failed to apply iOS webview scroll hardening: {}", err);
    }
}

/// Tune the macOS WKWebView for performance.
/// Must be called after the webview is fully initialised (delayed from setup).
#[cfg(target_os = "macos")]
fn tune_macos_webview(window: &tauri::WebviewWindow) {
    if let Err(err) = window.with_webview(|webview| {
        unsafe {
            use objc2::runtime::AnyObject;

            let wk_webview: *mut AnyObject = webview.inner().cast();
            if wk_webview.is_null() {
                return;
            }

            // Disable back-forward navigation gestures — we're a SPA
            let _: () = objc2::msg_send![wk_webview, setAllowsBackForwardNavigationGestures: false];
        }
    }) {
        tracing::warn!("Failed to tune macOS webview: {}", err);
    }
}

/// Tune the Windows WebView2 for performance.
#[cfg(windows)]
fn tune_windows_webview(window: &tauri::WebviewWindow) {
    if let Err(err) = window.with_webview(|webview| {
        unsafe {
            use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings6;
            use windows::core::Interface;

            let controller = webview.controller();
            let core = controller.CoreWebView2().unwrap();

            // Disable swipe navigation (back-forward gesture) — we're a SPA
            if let Ok(settings) = core.Settings() {
                if let Ok(settings6) = settings.cast::<ICoreWebView2Settings6>() {
                    let _ = settings6.SetIsSwipeNavigationEnabled(false);
                }
            }

            // Use COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW when minimized
            // (the app regains normal budget when re-focused via our event handler)
            // Not setting this proactively; will be used via visibility change events in JS.
        }
    }) {
        tracing::warn!("Failed to tune Windows webview: {}", err);
    }
}

/// JS snippet that probes CSS env(safe-area-inset-*) and applies the values
/// as CSS custom properties. Also re-syncs `--fl-mobile-vvh` so the body
/// height is correct after `harden_ios_webview_scroll` changes the scroll
/// view's content inset adjustment (which affects `visualViewport.height`).
#[cfg(target_os = "ios")]
const IOS_SAFE_AREA_JS: &str = concat!(
    "(function(){",
    "var d=document.documentElement;",
    "var p=document.createElement('div');",
    "p.style.cssText='position:fixed;left:-9999px;top:0;width:1px;height:1px;",
    "padding-top:env(safe-area-inset-top,0px);",
    "padding-bottom:env(safe-area-inset-bottom,0px);",
    "visibility:hidden;pointer-events:none';",
    "(document.body||d).appendChild(p);",
    "var cs=getComputedStyle(p);",
    "var t=Math.round(parseFloat(cs.paddingTop)||0);",
    "var b=Math.round(parseFloat(cs.paddingBottom)||0);",
    "p.remove();",
    "t=Math.max(t,window.__FL_NATIVE_SAFE_TOP||0);",
    "b=Math.max(b,window.__FL_NATIVE_SAFE_BOTTOM||0);",
    "if(t>0||b>0){",
    "d.style.setProperty('--fl-native-safe-top',t+'px');",
    "d.style.setProperty('--fl-native-safe-bottom',b+'px');",
    "window.__FL_NATIVE_SAFE_TOP=t;",
    "window.__FL_NATIVE_SAFE_BOTTOM=b;",
    "}",
    "var vvh=Math.round((window.visualViewport?window.visualViewport.height:0)||window.innerHeight);",
    "if(vvh>0)d.style.setProperty('--fl-mobile-vvh',vvh+'px');",
    "})();"
);

// --- iOS Release logging -----------------------------------------------------
#[cfg(all(target_os = "ios", not(debug_assertions)))]
mod ios_release_logging {
    use tracing_subscriber::{
        EnvFilter, filter::LevelFilter, layer::SubscriberExt, util::SubscriberInitExt,
    };

    pub fn init() {
        use std::sync::OnceLock;
        use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
        static INIT_GUARD: OnceLock<()> = OnceLock::new();

        // If we've already run (or someone else set a global subscriber), bail quietly.
        if INIT_GUARD.set(()).is_err() {
            return;
        }

        // Prefer Apple unified logging so you can see everything in Console.app
        let oslog = tracing_oslog::OsLogger::new("com.flow-like.app", "default");

        // Keep third-party noise down; raise your own crate(s). Never panic on parse errors.
        let builder = EnvFilter::builder().with_default_directive(LevelFilter::INFO.into());
        let mut filter = builder.from_env_lossy();
        for d in [
            "tao=warn",
            "wry=warn",
            "tauri=info",
            "flow_like=info",
            "flow_like_types=info",
        ] {
            if let Ok(dir) = d.parse() {
                filter = filter.add_directive(dir);
            }
        }

        // Don't panic if a global subscriber is already installed.
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(oslog)
            .try_init(); // <- returns Err if someone else initialized first; we ignore it.
    }
}

// On iOS Release, map println!/eprintln! to tracing so we never hit stdio.
#[cfg(all(target_os = "ios", not(debug_assertions)))]
macro_rules! println { ($($t:tt)*) => { tracing::info!($($t)*); } }
#[cfg(all(target_os = "ios", not(debug_assertions)))]
macro_rules! eprintln { ($($t:tt)*) => { tracing::error!($($t)*); } }
// ---------------------------------------------------------------------------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub fn run() {
    // Ensure panics are logged with backtraces in release too.
    std::panic::set_hook(Box::new(|info| {
        // Use write! instead of eprintln! to avoid a double-panic when stderr
        // is a broken pipe (e.g. the parent process that captured output has gone away).
        let _ = std::io::Write::write_fmt(
            &mut std::io::stderr(),
            format_args!(
                "PANIC: {info}\n{}\n",
                std::backtrace::Backtrace::force_capture()
            ),
        );
        // The debug devtools plugin installs a dynamic tracing layer. If that layer itself panics
        // during initialization/event dispatch, re-entering `tracing` from the panic hook can
        // trigger a second panic and abort the process before the original report is preserved.
        // Debug builds already wrote the panic/backtrace directly to stderr above; retain tracing
        // and Sentry integration for release builds where the subscriber is stable.
        #[cfg(not(debug_assertions))]
        if let Some(location) = info.location() {
            tracing::error!(
                target: "panic",
                message = %info,
                file = location.file(),
                line = location.line(),
                "Application panic"
            );
        } else {
            tracing::error!(target: "panic", message = %info, "Application panic (no location)");
        }

        {
            let _ = sentry::capture_message(&format!("panic: {info}"), sentry::Level::Fatal);
        }
    }));
    #[cfg(all(target_os = "ios", not(debug_assertions)))]
    ios_release_logging::init();
    disable_app_nap();

    // On Android, HOME & CACHE_DIR are set by MainActivity.kt before the
    // native runtime starts.  The block below acts as a safety net in case
    // Kotlin's Os.setenv did not execute (e.g. running tests without the Activity).
    #[cfg(target_os = "android")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let home_path = std::path::PathBuf::from(&home);
            println!("Android HOME: {:?}", home_path);

            // Only set CACHE_DIR if Kotlin didn't already set it.
            if std::env::var_os("CACHE_DIR").is_none() {
                let cache_parent = home_path.join(".cache");
                let _ = std::fs::create_dir_all(&cache_parent);
                // SAFETY: called once at startup before any threads access CACHE_DIR.
                unsafe {
                    std::env::set_var("CACHE_DIR", cache_parent.to_string_lossy().as_ref());
                }
            }

            // Ensure TMPDIR lives under filesDir so rename() across temp→data
            // stays on the same filesystem / SELinux context (required by LanceDB).
            if std::env::var_os("TMPDIR").is_none() {
                let tmp_dir = home_path.join("tmp");
                let _ = std::fs::create_dir_all(&tmp_dir);
                // SAFETY: called once at startup before any threads access TMPDIR.
                unsafe {
                    std::env::set_var("TMPDIR", tmp_dir.to_string_lossy().as_ref());
                }
            }
        } else {
            eprintln!("Android: HOME is NOT set — storage paths will likely fail");
        }

        println!("Android CACHE_DIR: {:?}", std::env::var_os("CACHE_DIR"));
        println!("Android TMPDIR: {:?}", std::env::var_os("TMPDIR"));
        println!(
            "Android LANCE_CACHE_DIR: {:?}",
            std::env::var_os("LANCE_CACHE_DIR")
        );
        let root = settings::mobile_storage_root();
        println!("Android mobile_storage_root: {:?}", root);
        let _ = settings::ensure_app_dirs();
    }

    // Initialize catalog runtime (ONNX execution providers, etc.)
    initialize_catalog();

    let mut settings_state = Settings::new();
    let project_dir = settings_state.project_dir.clone();
    let logs_dir = settings_state.logs_dir.clone();
    let temporary_dir = settings_state.temporary_dir.clone();

    let mut config: FlowLikeConfig = FlowLikeConfig::new();

    // Helper to build a store with a safe fallback to in-memory on failure (prevents startup crashes on mobile)
    let build_store = |path: std::path::PathBuf| -> FlowLikeStore {
        println!("build_store: attempting path {:?}", path);
        match LocalObjectStore::new(path.clone()) {
            Ok(store) => {
                println!("build_store: success for {:?}", path);
                FlowLikeStore::Local(Arc::new(store))
            }
            Err(e) => {
                eprintln!(
                    "Failed to init LocalObjectStore at {:?}: {:?}. Attempting to create dir...",
                    path, e
                );
                let _ = std::fs::create_dir_all(&path);
                match LocalObjectStore::new(path.clone()) {
                    Ok(store) => {
                        println!("build_store: success on retry for {:?}", path);
                        FlowLikeStore::Local(Arc::new(store))
                    }
                    Err(err) => {
                        eprintln!(
                            "Re-initialization failed for {:?}: {:?}. Falling back to in-memory store.",
                            path, err
                        );
                        FlowLikeStore::Memory(Arc::new(
                            flow_like::flow_like_storage::object_store::memory::InMemory::new(),
                        ))
                    }
                }
            }
        }
    };

    config.register_bits_store(build_store(settings_state.bit_dir.clone()));

    let user_dir = settings_state.user_dir.clone();
    let blob_dir = settings_state.user_dir.join("blob_store");
    config.register_user_store(build_store(settings_state.user_dir.clone()));

    config.register_app_storage_store(build_store(project_dir.clone()));

    config.register_app_meta_store(build_store(project_dir.clone()));

    config.register_log_store(build_store(logs_dir.clone()));

    config.register_temporary_store(build_store(temporary_dir.clone()));

    config.register_build_project_database(Arc::new(move |path: Path| {
        let directory = project_dir.join(path.to_string());
        let _ = std::fs::create_dir_all(&directory);
        lancedb::connect(directory.to_string_lossy().as_ref())
    }));

    config.register_build_user_database(Arc::new(move |path: Path| {
        let directory = user_dir.join(path.to_string());
        let _ = std::fs::create_dir_all(&directory);
        lancedb::connect(directory.to_string_lossy().as_ref())
    }));

    config.register_build_logs_database(Arc::new(move |path: Path| {
        let directory = logs_dir.join(path.to_string());
        let _ = std::fs::create_dir_all(&directory);
        lancedb::connect(directory.to_string_lossy().as_ref())
    }));

    // On Android, use a custom ObjectStore wrapper to avoid hard_link() which fails on Android SELinux
    #[cfg(target_os = "android")]
    config.register_lance_write_options(
        flow_like::flow_like_storage::android_store::android_write_options(),
    );

    settings_state.set_config(&config);
    let settings_state = Arc::new(Mutex::new(settings_state));
    let (http_client, refetch_rx) = HTTPClient::new();
    let state = FlowLikeState::new(config, http_client);
    let state_ref = Arc::new(state);

    let initialized_state = state_ref.clone();
    tauri::async_runtime::spawn(async move {
        #[cfg(any(target_os = "ios", target_os = "android"))]
        flow_like_types::tokio::time::sleep(Duration::from_millis(800)).await;

        let weak_ref = Arc::downgrade(&initialized_state);
        let catalog = get_catalog();
        let registry_guard = initialized_state.node_registry.clone();
        let mut registry = registry_guard.write().await;
        registry.initialize(weak_ref);
        registry.push_nodes(catalog);
        println!("Catalog Initialized");
    });

    let sentry_endpoint = std::option_env!("PUBLIC_SENTRY_ENDPOINT");
    // Defer Sentry init on mobile to improve startup; init immediately elsewhere.
    let mut _sentry_guard: Option<sentry::ClientInitGuard> = None;
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        _sentry_guard = sentry_endpoint.map(|endpoint| {
            sentry::init((
                endpoint,
                sentry::ClientOptions {
                    release: sentry::release_name!(),
                    auto_session_tracking: true,
                    traces_sample_rate: 0.1,
                    ..Default::default()
                },
            ))
        });
    }
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        tauri::async_runtime::spawn(async move {
            flow_like_types::tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if let Some(endpoint) = std::option_env!("PUBLIC_SENTRY_ENDPOINT") {
                let _ = sentry::init((
                    endpoint,
                    sentry::ClientOptions {
                        release: sentry::release_name!(),
                        auto_session_tracking: true,
                        traces_sample_rate: 0.1,
                        ..Default::default()
                    },
                ));
                tracing::info!("Sentry Tracing Layer Initialized (deferred)");
            }
        });
    }

    #[cfg(not(debug_assertions))]
    {
        #[cfg(all(target_os = "ios"))]
        { /* iOS Release: oslog is already set up above; Sentry init is deferred. */ }

        #[cfg(target_os = "android")]
        {
            // Android Release: Sentry is deferred; logcat captures stdout/stderr natively.
            match _sentry_guard {
                Some(_) => {
                    tracing_subscriber::registry()
                        .with(tracing_subscriber::fmt::layer())
                        .with(sentry_tracing::layer())
                        .init();
                    tracing::info!("Sentry Tracing Layer Initialized");
                }
                None => {
                    tracing_subscriber::registry()
                        .with(tracing_subscriber::fmt::layer())
                        .init();
                    tracing::info!("Sentry Tracing Layer Not Initialized");
                }
            }
        }

        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        {
            // Non-iOS Release (macOS/Windows/Linux): stdio fmt layer is OK
            match _sentry_guard {
                Some(_) => {
                    tracing_subscriber::registry()
                        .with(tracing_subscriber::fmt::layer())
                        .with(sentry_tracing::layer())
                        .init();
                    tracing::info!("Sentry Tracing Layer Initialized");
                }
                None => {
                    tracing_subscriber::registry()
                        .with(tracing_subscriber::fmt::layer())
                        .init();
                    tracing::info!("Sentry Tracing Layer Not Initialized");
                }
            }
        }
    }

    let settings_state_for_sink = settings_state.clone();
    let shared_wasm_engine =
        state::TauriWasmEngineState::create_shared().expect("Failed to create shared WasmEngine");
    let mut builder = tauri::Builder::default()
        .manage(state::TauriSettingsState(settings_state.clone()))
        .manage(state::TauriFlowLikeState(state_ref.clone()))
        .manage(state::TauriRegistryState(Arc::new(Mutex::new(None))))
        .manage(state::TauriWasmEngineState(shared_wasm_engine))
        .manage(state::TauriRecordingState::new())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_remote_push::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            let storage_cleanup_handle = app.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                flow_like_types::tokio::time::sleep(Duration::from_secs(2)).await;
                if let Err(error) = functions::storage_management::run_configured_log_cleanup(
                    &storage_cleanup_handle,
                )
                .await
                {
                    tracing::warn!(error = %error, "Automatic local log cleanup failed");
                }
            });

            // Start the WasmEngine epoch ticker inside the async runtime
            if let Some(wasm_state) = app.try_state::<state::TauriWasmEngineState>() {
                let engine = wasm_state.0.clone();
                tauri::async_runtime::spawn(async move {
                    engine.start_epoch_ticker();
                });
            }

            #[cfg(desktop)]
            if let Err(e) = app
                .handle()
                .plugin(tauri_plugin_updater::Builder::new().build())
            {
                eprintln!("Failed to register updater plugin: {}", e);
            }

            // Initialize EventBus and register as managed state
            let (event_bus, mut event_receiver) = EventBus::new(app.app_handle().clone());
            app.manage(state::TauriEventBusState(event_bus));

            // Initialize Event Sink Manager synchronously to ensure it's ready before accepting commands
            let settings_clone = settings_state_for_sink.clone();
            let manager_init_handle = app.app_handle().clone();

            // Block on initialization to ensure EventSinkManager is ready
            tauri::async_runtime::spawn(async move {
                let event_sink_db_path = settings_clone
                    .lock()
                    .await
                    .project_dir
                    .parent()
                    .unwrap()
                    .join("event_sinks.db")
                    .to_string_lossy()
                    .to_string();

                match event_sink::EventSinkManager::new(&event_sink_db_path) {
                    Ok(manager) => {
                        tracing::info!("Event Sink Manager initialized successfully");
                        manager_init_handle.manage(state::TauriEventSinkManagerState(Arc::new(
                            Mutex::new(manager),
                        )));

                        // Load existing registrations from database
                        if let Some(manager_state) =
                            manager_init_handle.try_state::<state::TauriEventSinkManagerState>()
                        {
                            let manager = manager_state.0.lock().await;
                            if let Err(e) = manager.init_from_storage(&manager_init_handle).await {
                                tracing::error!(
                                    "Failed to restore event sink registrations: {}",
                                    e
                                );
                            } else {
                                tracing::info!("Event sink registrations restored from database");
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to initialize Event Sink Manager: {}", e);
                    }
                }
            });

            let relay_handle = app.app_handle().clone();
            let gc_handle = relay_handle.clone();
            let refetch_handle = relay_handle.clone();
            let deep_link_handle = relay_handle.clone();
            let event_bus_handle = relay_handle.clone();

            // macOS: tune WKWebView after it's fully initialised
            #[cfg(target_os = "macos")]
            {
                let macos_handle = app.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    flow_like_types::tokio::time::sleep(Duration::from_millis(200)).await;
                    if let Some(main) = macos_handle.get_webview_window("main") {
                        tune_macos_webview(&main);
                    }
                });
            }

            // Windows: tune WebView2 after it's fully initialised
            #[cfg(windows)]
            {
                let win_handle = app.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    flow_like_types::tokio::time::sleep(Duration::from_millis(200)).await;
                    if let Some(main) = win_handle.get_webview_window("main") {
                        tune_windows_webview(&main);
                    }
                });
            }

            #[cfg(target_os = "ios")]
            {
                let ios_handle = app.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    // Wait for the WKWebView to be fully initialised before
                    // touching ObjC properties — calling too early crashes.
                    for delay_ms in [100, 300, 700, 1500, 3000] {
                        flow_like_types::tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        if let Some(main) = ios_handle.get_webview_window("main") {
                            harden_ios_webview_scroll(&main);
                            let _ = main.eval(IOS_SAFE_AREA_JS);
                        }
                    }
                });
            }

            // Android: retry safe-area CSS var application after page load.
            // The Kotlin MainActivity exposes insets via a JavascriptInterface
            // (FlowLikeInsets) that the page can read synchronously.  This retry
            // also reads from that bridge so it works even if the globals set by
            // evaluateJavascript were wiped during the about:blank → app page transition.
            #[cfg(target_os = "android")]
            {
                let android_handle = app.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    for delay_ms in [50, 150, 300, 700, 1500, 3000] {
                        flow_like_types::tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        if let Some(main) = android_handle.get_webview_window("main") {
                            let _ = main.eval(concat!(
                                "(function(){",
                                "var t=0,b=0;",
                                "if(typeof FlowLikeInsets!=='undefined'){",
                                "try{var dpr=window.devicePixelRatio||1;",
                                "t=Math.ceil(FlowLikeInsets.getTopPx()/dpr);",
                                "b=Math.ceil(FlowLikeInsets.getBottomPx()/dpr);",
                                "}catch(e){}",
                                "}",
                                "t=Math.max(t,window.__FL_NATIVE_SAFE_TOP||0);",
                                "b=Math.max(b,window.__FL_NATIVE_SAFE_BOTTOM||0);",
                                "if(t>0||b>0){",
                                "var d=document.documentElement;",
                                "d.style.setProperty('--fl-native-safe-top',t+'px');",
                                "d.style.setProperty('--fl-native-safe-bottom',b+'px');",
                                "window.__FL_NATIVE_SAFE_TOP=t;",
                                "window.__FL_NATIVE_SAFE_BOTTOM=b;",
                                "}",
                                "})();"
                            ));
                        }
                    }
                });
            }

            #[cfg(desktop)]
            {
                // Manage TauriTrayState for desktop platforms
                app.manage(state::TauriTrayState(Arc::new(Mutex::new(
                    tray::TrayRuntimeState::default(),
                ))));

                if let Err(err) = tray::init_tray(&relay_handle) {
                    eprintln!("Failed to initialize tray: {}", err);
                } else {
                    tray::spawn_tray_refresh(relay_handle.clone());
                }
            }

            #[cfg(desktop)]
            {
                use tauri_plugin_window_state::StateFlags;

                if let Err(e) = app.handle().plugin(
                    tauri_plugin_window_state::Builder::default()
                        .with_state_flags(StateFlags::all())
                        .build(),
                ) {
                    eprintln!("Failed to register window state plugin: {}", e);
                } else {
                    println!("Window state plugin registered successfully");
                }
            }

            #[cfg(any(target_os = "linux", windows))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                app.deep_link().register_all()?;
            }

            let start_urls = app.deep_link().get_current();
            if let Ok(Some(urls)) = start_urls {
                tracing::info!("deep link URLs for start: {:?}", urls);
                handle_deep_link(&deep_link_handle, &urls);
            }

            app.deep_link().on_open_url(move |event| {
                let deep_link_handle = deep_link_handle.clone();
                handle_deep_link(&deep_link_handle, &event.urls());
            });

            tauri::async_runtime::spawn(async move {
                #[cfg(any(target_os = "ios", target_os = "android"))]
                flow_like_types::tokio::time::sleep(Duration::from_millis(1200)).await;

                let handle = gc_handle;

                // Load developer project WASM nodes into catalog
                functions::developer::load_all_developer_nodes(&handle).await;

                // Initialize the WASM package registry so installed packages
                // appear in the catalog immediately (without waiting for the
                // user to visit the store page).
                if let Err(e) = functions::registry::registry_init(handle.clone(), None).await {
                    tracing::warn!("Failed to initialize WASM registry at startup: {:?}", e);
                }

                let model_factory = {
                    println!("Starting GC");
                    let flow_like_state = match TauriFlowLikeState::construct(&handle).await {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("GC init failed: {:?}", e);
                            return;
                        }
                    };

                    flow_like_state.model_factory.clone()
                };
                println!("GC Started");

                let mut interval = interval(Duration::from_secs(1));

                loop {
                    interval.tick().await;

                    {
                        let state = model_factory.try_lock();
                        if let Ok(mut state) = state {
                            state.gc();
                        }
                    }
                }
            });

            tauri::async_runtime::spawn(async move {
                #[cfg(any(target_os = "ios", target_os = "android"))]
                flow_like_types::tokio::time::sleep(Duration::from_millis(1200)).await;

                let mut receiver = refetch_rx;
                let handle = refetch_handle;

                let http_client = {
                    println!("Starting Refetch Handler");
                    let flow_like_state = match TauriFlowLikeState::construct(&handle).await {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("Refetch handler init failed: {:?}", e);
                            return;
                        }
                    };
                    flow_like_state.http_client.clone()
                };

                let client = http_client.client();

                println!("Refetch Handler Started");
                while let Some(event) = receiver.recv().await {
                    let request = event;
                    let request_hash = http_client.quick_hash(&request);
                    let response = match client.execute(request).await {
                        Ok(response) => response,
                        Err(e) => {
                            eprintln!("Error fetching request: {:?}", e);
                            continue;
                        }
                    };

                    let value = match response.json::<serde_json::Value>().await {
                        Ok(value) => value,
                        Err(e) => {
                            eprintln!("Error parsing response: {:?}", e);
                            continue;
                        }
                    };

                    match http_client.put(&request_hash, &value) {
                        Ok(result) => result,
                        Err(e) => {
                            eprintln!("Error putting value in cache: {:?}", e);
                            continue;
                        }
                    };
                }
            });

            // EventBus event processing sink
            tauri::async_runtime::spawn(async move {
                #[cfg(any(target_os = "ios", target_os = "android"))]
                flow_like_types::tokio::time::sleep(Duration::from_millis(1200)).await;

                let handle = event_bus_handle;

                println!("Starting EventBus Sink");

                let flow_like_state = match state::TauriFlowLikeState::construct(&handle).await {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("EventBus sink init failed: {:?}", e);
                        return;
                    }
                };

                println!("EventBus Sink Started");

                while let Some(event) = event_receiver.recv().await {
                    // Spawn each event execution as a separate task for parallel processing
                    let handle_clone = handle.clone();
                    let flow_like_state_clone = flow_like_state.clone();

                    tokio::spawn(async move {
                        match event.execute(&handle_clone, flow_like_state_clone).await {
                            Ok(meta) => _ = meta,
                            Err(e) => {
                                eprintln!("Error executing event: {:?}", e);
                            }
                        }
                    });
                }

                println!("EventBus Sink stopped");
            });

            Ok(())
        })
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_flow_like_dexie_blob_offload::init(Some(
            blob_dir,
        )))
        .invoke_handler(tauri::generate_handler![
            update,
            functions::file::get_path_meta,
            functions::ai::invoke::stream_chat_completion,
            functions::ai::invoke::chat_completion,
            functions::ai::invoke::find_best_model,
            functions::system::get_system_info,
            functions::system::list_apps_for_file,
            functions::system::open_file_with_app,
            functions::storage_management::get_local_storage_overview,
            functions::storage_management::set_log_retention_policy,
            functions::storage_management::run_log_cleanup,
            functions::storage_management::delete_local_storage_items,
            #[cfg(desktop)]
            tray::tray_update_state,
            #[cfg(not(desktop))]
            tray_update_state,
            functions::download::init::init_downloads,
            functions::download::init::get_downloads,
            functions::settings::profiles::get_profiles,
            functions::settings::profiles::get_profiles_raw,
            functions::settings::profiles::get_default_profiles,
            functions::settings::profiles::get_current_profile,
            functions::settings::profiles::get_current_profile_id,
            functions::settings::profiles::set_current_profile,
            functions::settings::profiles::upsert_profile,
            functions::settings::profiles::remap_profile_id,
            functions::settings::profiles::delete_profile,
            functions::settings::profiles::add_bit,
            functions::settings::profiles::remove_bit,
            functions::settings::profiles::get_bits_in_current_profile,
            functions::settings::profiles::change_profile_image,
            functions::settings::profiles::read_profile_icon,
            functions::settings::profiles::get_profile_icon_path,
            functions::settings::profiles::profile_update_app,
            functions::settings::profiles::profile_update_shortcuts,
            functions::app::app_configured,
            functions::app::upsert_board,
            functions::app::delete_app_board,
            functions::app::get_app,
            functions::app::push_app_meta,
            functions::app::push_app_media,
            functions::app::remove_app_media,
            functions::app::transform_media,
            functions::app::get_app_meta,
            functions::app::get_app_board,
            functions::app::get_app_boards,
            functions::app::set_app_config,
            functions::app::get_apps,
            functions::app::get_app_size,
            functions::app::create_app,
            functions::app::update_app,
            functions::app::delete_app,
            functions::app::app_add_package,
            functions::app::app_remove_package,
            functions::app::app_list_packages,
            functions::app::sharing::export_app_to_file,
            functions::app::sharing::import_app_from_file,
            functions::app::fork::apply_fork_bundle,
            functions::app::fork::summarize_local_app_bundle,
            functions::app::fork::upload_local_app_content_bundle,
            functions::app::tables::db_table_names,
            functions::app::tables::db_table_names_user,
            functions::app::tables::db_schema,
            functions::app::tables::db_create_table,
            functions::app::tables::db_list,
            functions::app::tables::db_count,
            functions::app::tables::build_index,
            functions::app::tables::db_add,
            functions::app::tables::db_delete,
            functions::app::tables::db_indices,
            functions::app::tables::db_query,
            functions::app::tables::db_optimize,
            functions::app::tables::db_update,
            functions::app::tables::db_drop_columns,
            functions::app::tables::db_add_column,
            functions::app::tables::db_alter_column,
            functions::app::tables::db_drop_index,
            functions::app::graph::graph_list_overlays,
            functions::app::graph::graph_list_imports,
            functions::app::graph::graph_create_overlay,
            functions::app::graph::graph_get_overlay,
            functions::app::graph::graph_prepare_ontology_action,
            functions::app::graph::graph_update_overlay,
            functions::app::graph::graph_delete_overlay,
            functions::app::graph::graph_get_schema,
            functions::app::graph::graph_validate_overlay,
            functions::app::graph::graph_cypher,
            functions::app::graph::graph_sql,
            functions::app::graph::graph_subgraph,
            functions::app::graph::graph_search_nodes,
            functions::app::graph::graph_neighbors,
            functions::app::graph::graph_overlay_children,
            functions::app::graph::graph_sample,
            functions::app::graph::graph_upsert_nodes,
            functions::app::graph::graph_upsert_edges,
            functions::app::graph::graph_paths,
            functions::app::graph::graph_analytics,
            functions::app::saved_queries::query_saved_list,
            functions::app::saved_queries::query_saved_get,
            functions::app::saved_queries::query_saved_create,
            functions::app::saved_queries::query_saved_update,
            functions::app::saved_queries::query_saved_delete,
            functions::app::saved_queries::query_execute_sql,
            functions::tmp::post_process_local_file,
            functions::bit::get_bit,
            functions::bit::is_bit_installed,
            functions::bit::get_bit_size,
            functions::bit::get_pack_from_bit,
            functions::bit::search_bits,
            functions::bit::download_bit,
            functions::bit::delete_bit,
            functions::bit::get_installed_bit,
            functions::flow::storage::storage_list,
            functions::flow::storage::storage_user_list,
            functions::flow::storage::storage_add,
            functions::flow::storage::storage_user_add,
            functions::flow::storage::storage_remove,
            functions::flow::storage::storage_user_remove,
            functions::flow::storage::storage_rename,
            functions::flow::storage::storage_get,
            functions::flow::storage::storage_user_get,
            functions::flow::storage::storage_to_fullpath,
            functions::flow::storage::storage_user_to_fullpath,
            functions::flow::catalog::get_catalog,
            functions::flow::board::create_board_version,
            functions::flow::board::get_board_versions,
            functions::flow::board::close_board,
            functions::flow::board::get_board,
            functions::flow::board::get_open_boards,
            functions::flow::board::undo_board,
            functions::flow::board::redo_board,
            functions::flow::board::execute_command,
            functions::flow::board::execute_commands,
            functions::flow::board::apply_flowscript,
            functions::flow::board::lint_flowscript,
            functions::flow::board::get_flowscript,
            functions::flow::board::get_execution_elements,
            functions::flow::board::save_board,
            functions::flow::run::execute_board,
            functions::flow::run::execute_event,
            functions::flow::run::list_runs,
            functions::flow::run::query_run,
            functions::flow::run::cancel_execution,
            functions::flow::event::validate_event,
            functions::flow::event::get_event,
            functions::flow::event::get_events,
            functions::flow::event::get_event_versions,
            functions::flow::event::upsert_event,
            functions::flow::event::delete_event,
            functions::flow::template::get_template,
            functions::flow::template::get_templates,
            functions::flow::template::get_template_versions,
            functions::flow::template::upsert_template,
            functions::flow::template::push_template_data,
            functions::flow::template::delete_template,
            functions::flow::template::get_template_meta,
            functions::flow::template::push_template_meta,
            functions::ai::copilot::copilot_chat,
            functions::ai::copilot::cancel_copilot_chat,
            functions::ai::copilot::flowpilot_flow_ir_commit_disposition,
            functions::ai::copilot::flowpilot_apply_flow_ir_commit,
            functions::ai::copilot::global_chat,
            functions::ai::copilot::global_chat_resume,
            functions::ai::copilot::global_chat_memory_status,
            functions::ai::copilot::global_chat_clear_memory,
            functions::ai::copilot::global_chat_list_memories,
            functions::ai::copilot::global_chat_delete_memory,
            functions::ai::copilot::copilot_sdk_start,
            functions::ai::copilot::copilot_sdk_stop,
            functions::ai::copilot::copilot_sdk_is_running,
            functions::ai::copilot::copilot_sdk_list_models,
            functions::ai::copilot::copilot_sdk_get_auth_status,
            functions::ai::copilot::copilot_sdk_create_agent_session,
            functions::ai::copilot::flowpilot_agent_backend_start,
            functions::ai::copilot::flowpilot_agent_backend_stop,
            functions::ai::copilot::flowpilot_agent_backend_is_running,
            functions::ai::copilot::flowpilot_agent_backend_list_models,
            functions::ai::copilot::flowpilot_agent_backend_get_auth_status,
            functions::ai::copilot::flowpilot_agent_backend_status,
            functions::ai::copilot::flowpilot_agent_backend_list,
            functions::ai::frontend_tool_bridge::flowpilot_frontend_tool_result,
            functions::a2ui::widget::get_widgets,
            functions::a2ui::widget::get_widget,
            functions::a2ui::widget::create_widget,
            functions::a2ui::widget::update_widget,
            functions::a2ui::widget::delete_widget,
            functions::a2ui::widget::create_widget_version,
            functions::a2ui::widget::get_widget_versions,
            functions::a2ui::widget::get_open_widgets,
            functions::a2ui::widget::close_widget,
            functions::a2ui::widget::get_widget_meta,
            functions::a2ui::widget::push_widget_meta,
            functions::a2ui::page::get_pages,
            functions::a2ui::page::get_page,
            functions::a2ui::page::get_page_by_route,
            functions::a2ui::page::create_page,
            functions::a2ui::page::update_page,
            functions::a2ui::page::delete_page,
            functions::a2ui::page::get_open_pages,
            functions::a2ui::page::close_page,
            functions::a2ui::page::get_page_meta,
            functions::a2ui::page::push_page_meta,
            functions::a2ui::route::get_app_routes,
            functions::a2ui::route::get_app_route_by_path,
            functions::a2ui::route::get_default_app_route,
            functions::a2ui::route::set_app_route,
            functions::a2ui::route::set_app_routes,
            functions::a2ui::route::delete_app_route_by_path,
            functions::a2ui::route::delete_app_route_by_event,
            functions::event_sink_commands::add_event_sink,
            functions::event_sink_commands::remove_event_sink,
            functions::event_sink_commands::get_event_sink,
            functions::event_sink_commands::list_event_sinks,
            functions::event_sink_commands::is_event_sink_active,
            functions::developer::developer_list_projects,
            functions::developer::developer_add_project,
            functions::developer::developer_remove_project,
            functions::developer::developer_list_local_files,
            functions::developer::developer_get_manifest,
            functions::developer::developer_save_manifest,
            functions::developer::developer_open_in_editor,
            functions::developer::developer_get_settings,
            functions::developer::developer_save_settings,
            functions::developer::developer_scaffold_project,
            functions::developer::developer_inspect_node,
            functions::developer::developer_inspect_package,
            functions::developer::developer_find_publish_wasm,
            functions::developer::developer_read_manifest,
            functions::developer::developer_run_node,
            functions::developer::developer_load_into_catalog,
            functions::developer::developer_check_staleness,
            functions::registry::registry_search_packages,
            functions::registry::registry_get_package,
            functions::registry::registry_install_package,
            functions::registry::registry_uninstall_package,
            functions::registry::registry_get_installed_packages,
            functions::registry::registry_is_package_installed,
            functions::registry::registry_get_installed_version,
            functions::registry::registry_update_package,
            functions::registry::registry_check_for_updates,
            functions::registry::registry_load_local,
            functions::registry::registry_init,
            functions::registry::registry_set_auth_token,
            functions::permissions::check_rpa_permissions,
            functions::permissions::request_rpa_permission,
            functions::recording::start_recording,
            functions::recording::pause_recording,
            functions::recording::resume_recording,
            functions::recording::stop_recording,
            functions::recording::get_recording_status,
            functions::recording::get_recorded_actions,
            functions::recording::insert_recording_to_board,
            functions::statistics::get_board_statistics,
            functions::statistics::get_cached_statistics,
            functions::interaction::respond_to_interaction,
            functions::notifications::get_pending_notification_tap,
            functions::device_id::get_stable_device_id,
            functions::feedback::upsert_offline_feedback,
            functions::feedback::get_offline_feedback,
            functions::feedback::get_offline_feedback_stats,
            functions::feedback::delete_offline_feedback,
        ]);

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(handle_instance));
    }

    #[cfg(debug_assertions)]
    {
        // `tauri_plugin_devtools` dynamically mutates tracing-subscriber filters. We have observed
        // a native SIGABRT in that initialization path on macOS. Keep it available for explicit
        // diagnostics, but do not make every development launch depend on the unstable layer.
        if std::env::var_os("FLOW_LIKE_ENABLE_DEVTOOLS_PLUGIN").is_some() {
            builder = builder.plugin(tauri_plugin_devtools::init());
        }
    }

    let context: tauri::Context<_> = std::thread::spawn(|| tauri::generate_context!())
        .join()
        .expect("context thread");

    builder
        .run(context)
        .expect("error while running tauri application");
}

fn handle_instance(app: &AppHandle, args: Vec<String>, _cwd: String) {
    #[cfg(desktop)]
    {
        let _ = app
            .get_webview_window("main")
            .expect("no main window")
            .set_focus();
    }

    println!(
        "a new app instance was opened with {args:?} and the deep link event was already triggered"
    );
}

#[cfg(desktop)]
#[tauri::command(async)]
async fn update(app_handle: AppHandle) -> tauri_plugin_updater::Result<()> {
    use tauri::window::{ProgressBarState, ProgressBarStatus};
    if let Some(update) = app_handle.updater()?.check().await? {
        if let Some(win) = app_handle.get_webview_window("main") {
            let _ = win.set_progress_bar(ProgressBarState {
                status: Some(ProgressBarStatus::Indeterminate),
                progress: None,
            });

            let mut downloaded: u64 = 0;
            let progress_win = win.clone();
            let done_win = win.clone();

            update
                .download_and_install(
                    move |chunk_len, content_len| {
                        downloaded += chunk_len as u64;

                        if let Some(total) = content_len {
                            let pct = ((downloaded as f64 / total as f64) * 100.0).clamp(0.0, 100.0)
                                as u64;

                            let _ = progress_win.set_progress_bar(ProgressBarState {
                                status: Some(ProgressBarStatus::Normal),
                                progress: Some(pct),
                            });
                        } else {
                            let _ = progress_win.set_progress_bar(ProgressBarState {
                                status: Some(ProgressBarStatus::Indeterminate),
                                progress: None,
                            });
                        }
                    },
                    move || {
                        let _ = done_win.set_progress_bar(ProgressBarState {
                            status: Some(ProgressBarStatus::None),
                            progress: None,
                        });
                    },
                )
                .await?;
        }

        app_handle.restart();
    }

    Ok(())
}

#[cfg(not(desktop))]
#[tauri::command(async)]
async fn update(_app_handle: AppHandle) -> Result<(), String> {
    // No-op on non-desktop targets (e.g., iOS)
    Ok(())
}

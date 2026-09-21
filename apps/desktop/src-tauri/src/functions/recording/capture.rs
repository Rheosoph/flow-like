use std::sync::Arc;

use flow_like::flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::tokio::sync::{RwLock, mpsc};
use serde::{Deserialize, Serialize};

use super::fingerprint::extract_fingerprint_at;
use super::screenshot::{capture_region_image, store_region};
use super::state::RecordingSettings;
use super::state::{
    ActionMetadata, ActionType, KeyModifier, MouseButton, RecordedAction, RecordingStateInner,
    ScrollDirection,
};
use crate::functions::TauriFunctionError;
use chrono::{DateTime, Utc};

/// Push a recorded action to the frontend. The capture loop runs on a tokio worker and fires per
/// input event, so it must never emit directly: `Emitter::emit` would hold Tauri's `webviews_lock`
/// while waiting on the main run loop, and here it would do so while holding the recording state
/// write guard.
fn emit_recorded_action(app_handle: &tauri::AppHandle, action: &RecordedAction) {
    crate::utils::emit_to_ui(app_handle, "recording:action", action.clone());
}

struct ClipboardRequest {
    deadline: std::time::Instant,
    reply: std::sync::mpsc::SyncSender<Option<String>>,
}

struct ClipboardReader {
    requests: std::sync::mpsc::SyncSender<ClipboardRequest>,
    busy: Arc<std::sync::atomic::AtomicBool>,
}

impl ClipboardReader {
    fn new(mut read: impl FnMut() -> Option<String> + Send + 'static) -> std::io::Result<Self> {
        let (requests, receiver) = std::sync::mpsc::sync_channel::<ClipboardRequest>(1);
        let busy = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_busy = busy.clone();
        std::thread::Builder::new()
            .name("recording-clipboard".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    let value = if std::time::Instant::now() < request.deadline {
                        read()
                    } else {
                        None
                    };
                    if std::time::Instant::now() < request.deadline {
                        let _ = request.reply.try_send(value);
                    }
                    worker_busy.store(false, std::sync::atomic::Ordering::Release);
                }
            })?;
        Ok(Self { requests, busy })
    }

    fn snapshot(&self, timeout: std::time::Duration) -> Option<String> {
        if self
            .busy
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_err()
        {
            return None;
        }
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        let deadline = std::time::Instant::now() + timeout;
        if self
            .requests
            .try_send(ClipboardRequest { deadline, reply })
            .is_err()
        {
            self.busy.store(false, std::sync::atomic::Ordering::Release);
            return None;
        }
        let value = receiver.recv_timeout(timeout).ok().flatten();
        (std::time::Instant::now() < deadline)
            .then_some(value)
            .flatten()
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn clipboard_reader() -> Option<&'static ClipboardReader> {
    static READER: std::sync::OnceLock<Option<ClipboardReader>> = std::sync::OnceLock::new();
    READER
        .get_or_init(|| {
            ClipboardReader::new(|| {
                arboard::Clipboard::new()
                    .ok()
                    .and_then(|mut clipboard| clipboard.get_text().ok())
            })
            .ok()
        })
        .as_ref()
}

/// Track the currently focused window
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FocusedWindow {
    pub id: String,
    pub title: String,
    pub process: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CapturedEvent {
    Unsupported {
        message: String,
    },
    Browser {
        action: ActionType,
    },
    MouseDown {
        x: i32,
        y: i32,
        button: MouseButton,
        modifiers: Vec<KeyModifier>,
    },
    MouseUp {
        x: i32,
        y: i32,
        button: MouseButton,
    },
    MouseMove {
        x: i32,
        y: i32,
    },
    KeyDown {
        key: String,
        modifiers: Vec<KeyModifier>,
    },
    KeyUp {
        key: String,
    },
    Scroll {
        x: i32,
        y: i32,
        dx: i32,
        dy: i32,
    },
    Character {
        ch: char,
    },
    Text {
        text: String,
    },
    WindowFocusChanged {
        title: String,
        process: String,
    },
}

#[derive(Clone)]
struct ClickTarget {
    screenshot: Option<image::DynamicImage>,
    fingerprint: Option<super::state::RecordedFingerprint>,
}

struct TargetPreview {
    coordinates: (i32, i32),
    focused: Option<FocusedWindow>,
    captured_at: std::time::Instant,
    target: ClickTarget,
}

type PreviewCache = Arc<std::sync::Mutex<Option<TargetPreview>>>;
type FocusCache = Arc<std::sync::Mutex<Option<(std::time::Instant, Option<FocusedWindow>)>>>;

fn cached_focus(cache: &FocusCache) -> Option<FocusedWindow> {
    cache
        .try_lock()
        .ok()?
        .as_ref()
        .filter(|(captured_at, _)| captured_at.elapsed() < std::time::Duration::from_millis(500))
        .and_then(|(_, focused)| focused.clone())
}

pub(super) struct InputEvent {
    event: CapturedEvent,
    timestamp: DateTime<Utc>,
    focused: Option<FocusedWindow>,
    target: Option<ClickTarget>,
    clipboard: Option<String>,
}

impl InputEvent {
    pub(super) fn browser(action: ActionType, timestamp: DateTime<Utc>) -> Self {
        Self {
            event: CapturedEvent::Browser { action },
            timestamp,
            focused: None,
            target: None,
            clipboard: None,
        }
    }
}

pub(super) enum CaptureMessage {
    Input(InputEvent),
    Flush(flow_like_types::tokio::sync::oneshot::Sender<()>),
}

#[derive(Clone)]
struct CaptureTarget {
    id: String,
    tx: mpsc::UnboundedSender<CaptureMessage>,
    active: Arc<std::sync::atomic::AtomicBool>,
    settings: RecordingSettings,
    preview: PreviewCache,
    focused: FocusCache,
    app_handle: tauri::AppHandle,
}

#[derive(Default)]
struct ListenerState {
    target: Option<CaptureTarget>,
    running: bool,
    error: Option<String>,
}

static LISTENER: std::sync::OnceLock<std::sync::Mutex<ListenerState>> = std::sync::OnceLock::new();

pub struct EventCapture {
    id: String,
    tx: Option<mpsc::UnboundedSender<CaptureMessage>>,
    active: Arc<std::sync::atomic::AtomicBool>,
    processor: Option<flow_like_types::tokio::task::JoinHandle<()>>,
    browser: Option<super::browser::BrowserCapture>,
    preview_stop: Arc<std::sync::atomic::AtomicBool>,
    preview_task: Option<flow_like_types::tokio::task::JoinHandle<()>>,
    focus_task: Option<flow_like_types::tokio::task::JoinHandle<()>>,
}

impl EventCapture {
    pub async fn new(
        state: Arc<RwLock<RecordingStateInner>>,
        app_handle: tauri::AppHandle,
        store: Option<Arc<FlowLikeStore>>,
        settings: RecordingSettings,
    ) -> Result<Self, TauriFunctionError> {
        let (tx, rx) = mpsc::unbounded_channel();
        let active = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let preview = PreviewCache::default();
        let focused = FocusCache::default();
        let preview_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let id = flow_like_types::create_id();
        let listener = LISTENER.get_or_init(Default::default);
        let browser_recording = settings
            .browser_debugger_address
            .as_ref()
            .is_some_and(|address| !address.trim().is_empty());
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        if !browser_recording {
            let _ = clipboard_reader();
        }
        let start_listener = if browser_recording {
            false
        } else {
            let mut listener = listener
                .lock()
                .map_err(|_| TauriFunctionError::new("Input listener lock failed"))?;
            listener.target = Some(CaptureTarget {
                id: id.clone(),
                tx: tx.clone(),
                active: active.clone(),
                settings: settings.clone(),
                preview: preview.clone(),
                focused: focused.clone(),
                app_handle: app_handle.clone(),
            });
            if listener.running {
                false
            } else {
                listener.running = true;
                listener.error = None;
                true
            }
        };
        if start_listener {
            std::thread::spawn(Self::start_event_loop_shared);
            // Native hook failures return immediately. Permission preflight happens before this call.
            flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let error = listener
                .lock()
                .ok()
                .and_then(|listener| listener.error.clone());
            if let Some(error) = error {
                if let Ok(mut listener) = listener.lock() {
                    listener.target = None;
                }
                return Err(TauriFunctionError::new(&error));
            }
        }
        let browser = if let Some(address) = settings
            .browser_debugger_address
            .as_deref()
            .filter(|address| !address.trim().is_empty())
        {
            match super::browser::BrowserCapture::start(
                address,
                &settings.browser_webdriver_url,
                &settings.browser_type,
                tx.clone(),
                active.clone(),
                app_handle.clone(),
            )
            .await
            {
                Ok(browser) => Some(browser),
                Err(error) => {
                    if let Ok(mut listener) = listener.lock() {
                        listener.target = None;
                    }
                    return Err(error);
                }
            }
        } else {
            None
        };
        let focus_task = if !browser_recording {
            let cache = focused.clone();
            let stopped = preview_stop.clone();
            let active = active.clone();
            let focus_app = app_handle.clone();
            Some(flow_like_types::tokio::spawn(async move {
                let mut reported_unavailable = false;
                while !stopped.load(std::sync::atomic::Ordering::SeqCst) {
                    if active.load(std::sync::atomic::Ordering::SeqCst) {
                        let current =
                            flow_like_types::tokio::task::spawn_blocking(Self::get_focused_window)
                                .await
                                .ok()
                                .flatten();
                        if !stopped.load(std::sync::atomic::Ordering::SeqCst)
                            && active.load(std::sync::atomic::Ordering::SeqCst)
                        {
                            if current.is_none() && !reported_unavailable {
                                crate::utils::emit_to_ui(
                                    &focus_app,
                                    "recording:error",
                                    "Focused window identity is unavailable. Add an explicit Focus Window step before replaying actions in that window.",
                                );
                            }
                            reported_unavailable = current.is_none();
                        }
                        if !stopped.load(std::sync::atomic::Ordering::SeqCst)
                            && let Ok(mut cache) = cache.lock()
                        {
                            *cache = Some((std::time::Instant::now(), current));
                        }
                    }
                    flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }))
        } else {
            None
        };
        let preview_task = if !browser_recording
            && (settings.capture_screenshots || settings.capture_fingerprints)
        {
            let active = active.clone();
            let stopped = preview_stop.clone();
            Some(flow_like_types::tokio::spawn(async move {
                while !stopped.load(std::sync::atomic::Ordering::SeqCst) {
                    if active.load(std::sync::atomic::Ordering::SeqCst) {
                        let settings = settings.clone();
                        let focused = cached_focus(&focused);
                        let sample = flow_like_types::tokio::task::spawn_blocking(move || {
                            let coordinates = Self::get_mouse_location()?;
                            if focused.as_ref().is_some_and(|window| {
                                is_ignored_process(&window.process, &settings)
                            }) {
                                return None;
                            }
                            let captured_at = std::time::Instant::now();
                            let target = ClickTarget {
                                screenshot: settings
                                    .capture_screenshots
                                    .then(|| {
                                        capture_region_image(
                                            coordinates.0,
                                            coordinates.1,
                                            settings.capture_region_size,
                                        )
                                        .ok()
                                    })
                                    .flatten(),
                                fingerprint: settings
                                    .capture_fingerprints
                                    .then(|| extract_fingerprint_at(coordinates.0, coordinates.1))
                                    .flatten(),
                            };
                            Some(TargetPreview {
                                coordinates,
                                focused,
                                captured_at,
                                target,
                            })
                        })
                        .await
                        .ok()
                        .flatten();
                        if !stopped.load(std::sync::atomic::Ordering::SeqCst)
                            && active.load(std::sync::atomic::Ordering::SeqCst)
                            && let Ok(mut cached) = preview.lock()
                        {
                            *cached = sample;
                        }
                    }
                    flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(100))
                        .await;
                }
            }))
        } else {
            None
        };
        let processor =
            flow_like_types::tokio::spawn(Self::process_events(rx, state, app_handle, store));
        Ok(Self {
            id,
            tx: Some(tx),
            active,
            processor: Some(processor),
            browser,
            preview_stop,
            preview_task,
            focus_task,
        })
    }

    pub fn set_active(&self, active: bool) {
        self.active
            .store(active, std::sync::atomic::Ordering::SeqCst);
    }

    pub async fn flush(&self) {
        let (tx, rx) = flow_like_types::tokio::sync::oneshot::channel();
        if self
            .tx
            .as_ref()
            .is_some_and(|sender| sender.send(CaptureMessage::Flush(tx)).is_ok())
        {
            let _ = rx.await;
        }
    }

    pub async fn finish(mut self) {
        self.set_active(false);
        self.preview_stop
            .store(true, std::sync::atomic::Ordering::SeqCst);
        for mut task in [self.preview_task.take(), self.focus_task.take()]
            .into_iter()
            .flatten()
        {
            if flow_like_types::tokio::time::timeout(std::time::Duration::from_secs(1), &mut task)
                .await
                .is_err()
            {
                task.abort();
            }
        }
        if let Some(browser) = self.browser.take() {
            browser.finish().await;
        }
        self.disconnect();
        if let Some(processor) = self.processor.take() {
            let _ = processor.await;
        }
    }

    fn disconnect(&mut self) {
        self.set_active(false);
        self.preview_stop
            .store(true, std::sync::atomic::Ordering::SeqCst);
        for task in [self.preview_task.take(), self.focus_task.take()]
            .into_iter()
            .flatten()
        {
            task.abort();
        }
        if let Some(listener) = LISTENER.get()
            && let Ok(mut listener) = listener.lock()
            && listener
                .target
                .as_ref()
                .is_some_and(|target| target.id == self.id)
        {
            listener.target = None;
        }
        self.tx.take();
    }

    fn dispatch(event: CapturedEvent) {
        if matches!(
            event,
            CapturedEvent::KeyUp { .. } | CapturedEvent::MouseMove { .. }
        ) {
            return;
        }
        let target = LISTENER
            .get()
            .and_then(|listener| listener.lock().ok()?.target.clone());
        let Some(target) = target else {
            return;
        };
        if is_stop_shortcut(&event) {
            target
                .active
                .store(false, std::sync::atomic::Ordering::SeqCst);
            let app_handle = target.app_handle.clone();
            drop(target);
            tauri::async_runtime::spawn(async move {
                let _ = super::stop_recording(app_handle).await;
            });
            return;
        }
        if !target.active.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        if let CapturedEvent::Unsupported { message } = event {
            crate::utils::emit_to_ui(&target.app_handle, "recording:error", message);
            return;
        }
        if target
            .settings
            .browser_debugger_address
            .as_ref()
            .is_some_and(|address| !address.trim().is_empty())
        {
            return;
        }
        let timestamp = Utc::now();
        let focused = cached_focus(&target.focused);
        if focused
            .as_ref()
            .is_some_and(|window| is_ignored_process(&window.process, &target.settings))
        {
            return;
        }
        // Only use a sample completed before MouseDown. The input hook never waits for screen or AX APIs.
        let click_target = if let CapturedEvent::MouseDown { x, y, .. } = &event {
            target.preview.lock().ok().and_then(|preview| {
                preview
                    .as_ref()
                    .filter(|preview| {
                        preview.coordinates == (*x, *y)
                            && preview.focused == focused
                            && preview.captured_at.elapsed() < std::time::Duration::from_millis(500)
                    })
                    .map(|preview| preview.target.clone())
            })
        } else {
            None
        };
        let clipboard = if is_paste_shortcut(&event) {
            let snapshot = Self::get_clipboard_text();
            if snapshot.is_none() {
                crate::utils::emit_to_ui(
                    &target.app_handle,
                    "recording:error",
                    "Clipboard text was unavailable within the recording deadline. The paste shortcut was recorded; replay will use the clipboard available at that step.",
                );
            }
            snapshot
        } else {
            None
        };
        if target.active.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = target.tx.send(CaptureMessage::Input(InputEvent {
                event,
                timestamp,
                focused,
                target: click_target,
                clipboard,
            }));
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn key_to_string(key: &rdev::Key) -> String {
        use rdev::Key;
        match key {
            // Letters
            Key::KeyA => "a".to_string(),
            Key::KeyB => "b".to_string(),
            Key::KeyC => "c".to_string(),
            Key::KeyD => "d".to_string(),
            Key::KeyE => "e".to_string(),
            Key::KeyF => "f".to_string(),
            Key::KeyG => "g".to_string(),
            Key::KeyH => "h".to_string(),
            Key::KeyI => "i".to_string(),
            Key::KeyJ => "j".to_string(),
            Key::KeyK => "k".to_string(),
            Key::KeyL => "l".to_string(),
            Key::KeyM => "m".to_string(),
            Key::KeyN => "n".to_string(),
            Key::KeyO => "o".to_string(),
            Key::KeyP => "p".to_string(),
            Key::KeyQ => "q".to_string(),
            Key::KeyR => "r".to_string(),
            Key::KeyS => "s".to_string(),
            Key::KeyT => "t".to_string(),
            Key::KeyU => "u".to_string(),
            Key::KeyV => "v".to_string(),
            Key::KeyW => "w".to_string(),
            Key::KeyX => "x".to_string(),
            Key::KeyY => "y".to_string(),
            Key::KeyZ => "z".to_string(),

            // Numbers
            Key::Num0 => "0".to_string(),
            Key::Num1 => "1".to_string(),
            Key::Num2 => "2".to_string(),
            Key::Num3 => "3".to_string(),
            Key::Num4 => "4".to_string(),
            Key::Num5 => "5".to_string(),
            Key::Num6 => "6".to_string(),
            Key::Num7 => "7".to_string(),
            Key::Num8 => "8".to_string(),
            Key::Num9 => "9".to_string(),

            // Function keys
            Key::F1 => "F1".to_string(),
            Key::F2 => "F2".to_string(),
            Key::F3 => "F3".to_string(),
            Key::F4 => "F4".to_string(),
            Key::F5 => "F5".to_string(),
            Key::F6 => "F6".to_string(),
            Key::F7 => "F7".to_string(),
            Key::F8 => "F8".to_string(),
            Key::F9 => "F9".to_string(),
            Key::F10 => "F10".to_string(),
            Key::F11 => "F11".to_string(),
            Key::F12 => "F12".to_string(),

            // Special keys
            Key::Alt => "Alt".to_string(),
            Key::AltGr => "AltGr".to_string(),
            Key::Backspace => "Backspace".to_string(),
            Key::CapsLock => "CapsLock".to_string(),
            Key::ControlLeft => "Ctrl".to_string(),
            Key::ControlRight => "Ctrl".to_string(),
            Key::Delete => "Delete".to_string(),
            Key::DownArrow => "Down".to_string(),
            Key::End => "End".to_string(),
            Key::Escape => "Escape".to_string(),
            Key::Home => "Home".to_string(),
            Key::LeftArrow => "Left".to_string(),
            Key::MetaLeft => "Meta".to_string(),
            Key::MetaRight => "Meta".to_string(),
            Key::PageDown => "PageDown".to_string(),
            Key::PageUp => "PageUp".to_string(),
            Key::Return => "Enter".to_string(),
            Key::RightArrow => "Right".to_string(),
            Key::ShiftLeft => "Shift".to_string(),
            Key::ShiftRight => "Shift".to_string(),
            Key::Space => "Space".to_string(),
            Key::Tab => "Tab".to_string(),
            Key::UpArrow => "Up".to_string(),

            // Punctuation
            Key::Comma => ",".to_string(),
            Key::Dot => ".".to_string(),
            Key::SemiColon => ";".to_string(),
            Key::Quote => "'".to_string(),
            Key::BackQuote => "`".to_string(),
            Key::Slash => "/".to_string(),
            Key::BackSlash => "\\".to_string(),
            Key::LeftBracket => "[".to_string(),
            Key::RightBracket => "]".to_string(),
            Key::Minus => "-".to_string(),
            Key::Equal => "=".to_string(),

            // Keypad
            Key::KpReturn => "Enter".to_string(),
            Key::KpMinus => "-".to_string(),
            Key::KpPlus => "+".to_string(),
            Key::KpMultiply => "*".to_string(),
            Key::KpDivide => "/".to_string(),
            Key::Kp0 => "0".to_string(),
            Key::Kp1 => "1".to_string(),
            Key::Kp2 => "2".to_string(),
            Key::Kp3 => "3".to_string(),
            Key::Kp4 => "4".to_string(),
            Key::Kp5 => "5".to_string(),
            Key::Kp6 => "6".to_string(),
            Key::Kp7 => "7".to_string(),
            Key::Kp8 => "8".to_string(),
            Key::Kp9 => "9".to_string(),
            Key::KpDelete => "Delete".to_string(),

            // Default for unknown keys
            _ => format!("{:?}", key),
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn key_to_char_fallback(key: &rdev::Key, _is_shift_held: bool) -> Option<char> {
        use rdev::Key;
        match key {
            Key::Space => Some(' '),
            Key::Kp0 => Some('0'),
            Key::Kp1 => Some('1'),
            Key::Kp2 => Some('2'),
            Key::Kp3 => Some('3'),
            Key::Kp4 => Some('4'),
            Key::Kp5 => Some('5'),
            Key::Kp6 => Some('6'),
            Key::Kp7 => Some('7'),
            Key::Kp8 => Some('8'),
            Key::Kp9 => Some('9'),
            Key::KpMinus => Some('-'),
            Key::KpPlus => Some('+'),
            Key::KpMultiply => Some('*'),
            Key::KpDivide => Some('/'),
            _ => None,
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn start_event_loop_shared() {
        use rdev::{Event, EventType, Key, listen};
        use std::sync::atomic::{AtomicI32, Ordering};

        tracing::debug!("start_event_loop: initializing rdev listener");

        #[cfg(target_os = "macos")]
        {
            rdev::set_is_main_thread(false);
            tracing::debug!("Set is_main_thread=false for macOS thread safety");
        }

        let (initial_x, initial_y) = Self::get_mouse_location().unwrap_or_default();
        let mouse_x = Arc::new(AtomicI32::new(initial_x));
        let mouse_y = Arc::new(AtomicI32::new(initial_y));
        let event_count = Arc::new(AtomicI32::new(0));
        let event_count_clone = event_count.clone();
        let mut held_keys = Vec::new();

        let callback = move |event: Event| {
            let count = event_count_clone.fetch_add(1, Ordering::Relaxed);
            if count % 1000 == 0 {
                tracing::trace!("rdev: processed {} raw events", count);
            }

            let captured = match event.event_type {
                EventType::MouseMove { x, y } => {
                    mouse_x.store(x as i32, Ordering::Relaxed);
                    mouse_y.store(y as i32, Ordering::Relaxed);
                    None
                }
                EventType::ButtonPress(button) => {
                    let mouse_button = match button {
                        rdev::Button::Left => MouseButton::Left,
                        rdev::Button::Right => MouseButton::Right,
                        rdev::Button::Middle => MouseButton::Middle,
                        _ => {
                            Self::dispatch(CapturedEvent::Unsupported { message: "Extra mouse buttons are not supported by the recorder. Add that action explicitly.".into() });
                            return;
                        }
                    };

                    let x = mouse_x.load(Ordering::Relaxed);
                    let y = mouse_y.load(Ordering::Relaxed);

                    let mods = held_modifiers(&held_keys);

                    Some(CapturedEvent::MouseDown {
                        x,
                        y,
                        button: mouse_button,
                        modifiers: mods,
                    })
                }
                EventType::ButtonRelease(button) => {
                    let mouse_button = match button {
                        rdev::Button::Left => MouseButton::Left,
                        rdev::Button::Right => MouseButton::Right,
                        rdev::Button::Middle => MouseButton::Middle,
                        _ => return,
                    };

                    let x = mouse_x.load(Ordering::Relaxed);
                    let y = mouse_y.load(Ordering::Relaxed);
                    Some(CapturedEvent::MouseUp {
                        x,
                        y,
                        button: mouse_button,
                    })
                }
                EventType::Wheel { delta_x, delta_y } => {
                    let x = mouse_x.load(Ordering::Relaxed);
                    let y = mouse_y.load(Ordering::Relaxed);
                    Some(CapturedEvent::Scroll {
                        x,
                        y,
                        dx: delta_x as i32,
                        dy: delta_y as i32,
                    })
                }
                EventType::KeyPress(key) => {
                    if !held_keys.contains(&key) {
                        held_keys.push(key);
                    }
                    let key_str = Self::key_to_string(&key);
                    let modifiers = held_modifiers(&held_keys);
                    let is_shift_held = modifiers.contains(&KeyModifier::Shift);
                    let has_ctrl = modifiers.contains(&KeyModifier::Control);
                    let has_meta = modifiers.contains(&KeyModifier::Meta);
                    let has_alt = modifiers.contains(&KeyModifier::Alt);

                    // Check if this is a modifier-only key (don't generate events for these)
                    let is_modifier_key = matches!(
                        key,
                        Key::ShiftLeft
                            | Key::ShiftRight
                            | Key::ControlLeft
                            | Key::ControlRight
                            | Key::MetaLeft
                            | Key::MetaRight
                            | Key::Alt
                            | Key::AltGr
                    );

                    if is_modifier_key {
                        // Don't send events for modifier keys themselves
                        return;
                    }

                    // Check if this is a special key or has modifiers (excluding just Shift for typing)
                    let has_cmd_or_ctrl = has_ctrl || has_meta;
                    let is_special_key = matches!(
                        key,
                        Key::Return
                            | Key::Tab
                            | Key::Escape
                            | Key::Backspace
                            | Key::Delete
                            | Key::UpArrow
                            | Key::DownArrow
                            | Key::LeftArrow
                            | Key::RightArrow
                            | Key::Home
                            | Key::End
                            | Key::PageUp
                            | Key::PageDown
                            | Key::F1
                            | Key::F2
                            | Key::F3
                            | Key::F4
                            | Key::F5
                            | Key::F6
                            | Key::F7
                            | Key::F8
                            | Key::F9
                            | Key::F10
                            | Key::F11
                            | Key::F12
                    );

                    // If Ctrl/Cmd/Alt is held, send as KeyDown (for shortcuts like Ctrl+C)
                    // If it's a special key, send as KeyDown
                    // Otherwise, try to get character for text input
                    if has_cmd_or_ctrl || has_alt || is_special_key {
                        // Send as KeyDown event (will be handled as special key or shortcut)
                        Some(CapturedEvent::KeyDown {
                            key: key_str,
                            modifiers,
                        })
                    } else {
                        // Try to get character for text input.
                        // event.name from rdev uses UCKeyTranslate dispatched to
                        // the main thread; it respects the active keyboard layout.
                        if let Some(text) = event.name.filter(|name| {
                            !name.is_empty() && name.chars().all(|ch| !ch.is_control())
                        }) {
                            Some(CapturedEvent::Text { text })
                        } else if let Some(ch) = Self::key_to_char_fallback(&key, is_shift_held) {
                            // Send ONLY Character event for text input (no KeyDown)
                            Some(CapturedEvent::Character { ch })
                        } else {
                            Some(CapturedEvent::Unsupported { message: "The operating system did not provide text for a key. Review the recorded text or use browser recording for composed input.".into() })
                        }
                    }
                }
                EventType::KeyRelease(key) => {
                    held_keys.retain(|held| *held != key);

                    let key_str = Self::key_to_string(&key);
                    Some(CapturedEvent::KeyUp { key: key_str })
                }
            };

            if let Some(captured) = captured {
                Self::dispatch(captured);
            }
        };

        let result = listen(callback);
        let error = format!("The native input listener stopped: {:?}", result.err());
        let target = LISTENER.get().and_then(|listener| {
            let mut listener = listener.lock().ok()?;
            listener.running = false;
            listener.error = Some(error.clone());
            listener.target.clone()
        });
        if let Some(target) = target {
            target
                .active
                .store(false, std::sync::atomic::Ordering::SeqCst);
            crate::utils::emit_to_ui(&target.app_handle, "recording:error", error);
            let app_handle = target.app_handle.clone();
            drop(target);
            tauri::async_runtime::spawn(async move {
                let _ = super::stop_recording(app_handle).await;
            });
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    fn start_event_loop_shared() {
        if let Some(listener) = LISTENER.get()
            && let Ok(mut listener) = listener.lock()
        {
            listener.running = false;
            listener.error = Some("Recording is unavailable on this platform".to_string());
        }
    }

    /// Read native identity outside the input hook; callers cache the completed result.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn get_focused_window() -> Option<FocusedWindow> {
        let focused = flow_like_catalog::computer::native::windows()
            .ok()?
            .into_iter()
            .find(|window| window.is_focused)?;
        Some(FocusedWindow {
            id: focused.id,
            title: focused.title,
            process: focused.app_name.unwrap_or_default(),
        })
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    fn get_focused_window() -> Option<FocusedWindow> {
        None
    }

    /// Get the current mouse position using CoreGraphics (same coordinate system as rdev)
    #[cfg(target_os = "macos")]
    fn get_mouse_location() -> Option<(i32, i32)> {
        use core_graphics::event::CGEvent;
        use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

        let result = std::thread::spawn(|| {
            CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
                .ok()
                .and_then(|source| CGEvent::new(source).ok())
                .map(|event| {
                    let point = event.location();
                    (point.x as i32, point.y as i32)
                })
        })
        .join()
        .ok()
        .flatten();

        tracing::debug!(" get_mouse_location() (CGEvent) returned: {:?}", result);
        result
    }

    /// Get the current mouse position using enigo
    #[cfg(target_os = "windows")]
    fn get_mouse_location() -> Option<(i32, i32)> {
        use enigo::{Enigo, Mouse, Settings};

        let result = std::thread::spawn(|| {
            Enigo::new(&Settings::default())
                .ok()
                .and_then(|enigo| enigo.location().ok())
        })
        .join()
        .ok()
        .flatten();

        tracing::debug!(" get_mouse_location() returned: {:?}", result);
        result
    }

    #[cfg(target_os = "linux")]
    fn get_mouse_location() -> Option<(i32, i32)> {
        use enigo::{Enigo, Mouse, Settings};

        let result = std::thread::spawn(|| {
            Enigo::new(&Settings::default())
                .ok()
                .and_then(|enigo| enigo.location().ok())
        })
        .join()
        .ok()
        .flatten();

        tracing::debug!(" get_mouse_location() returned: {:?}", result);
        result
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    fn get_mouse_location() -> Option<(i32, i32)> {
        None
    }

    /// Get text content from system clipboard
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn get_clipboard_text() -> Option<String> {
        clipboard_reader()?.snapshot(std::time::Duration::from_millis(25))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    fn get_clipboard_text() -> Option<String> {
        None
    }

    async fn process_events(
        mut rx: mpsc::UnboundedReceiver<CaptureMessage>,
        state: Arc<RwLock<RecordingStateInner>>,
        app_handle: tauri::AppHandle,
        store: Option<Arc<FlowLikeStore>>,
    ) {
        let state_for_uploads = state.clone();
        let mut pending_mouse: Option<InputEvent> = None;
        let mut last_focus: Option<FocusedWindow> = None;
        let mut last_click: Option<RecordedAction> = None;
        let mut uploads = flow_like_types::tokio::task::JoinSet::new();
        let mut timer =
            flow_like_types::tokio::time::interval(std::time::Duration::from_millis(100));
        loop {
            while uploads.try_join_next().is_some() {}
            let message = flow_like_types::tokio::select! {
                message = rx.recv() => match message { Some(message) => message, None => break },
                _ = timer.tick() => {
                    let mut state = state.write().await;
                    if state.should_flush_keystrokes() && let Some(action) = state.flush_keystroke_buffer() {
                        emit_recorded_action(&app_handle, &action);
                    }
                    continue;
                }
            };
            let input = match message {
                CaptureMessage::Flush(acknowledged) => {
                    pending_mouse = None;
                    last_click = None;
                    let mut state = state.write().await;
                    if let Some(action) = state.flush_keystroke_buffer() {
                        emit_recorded_action(&app_handle, &action);
                    }
                    let _ = acknowledged.send(());
                    continue;
                }
                CaptureMessage::Input(input) => input,
            };
            if !matches!(input.event, CapturedEvent::MouseUp { .. })
                && let Some(focused) = &input.focused
                && !last_focus.as_ref().is_some_and(|previous| {
                    previous.id == focused.id && previous.process == focused.process
                })
            {
                let mut state = state.write().await;
                if let Some(action) = state.flush_keystroke_buffer() {
                    emit_recorded_action(&app_handle, &action);
                }
                let mut action = RecordedAction::new(
                    flow_like_types::create_id(),
                    ActionType::WindowFocus {
                        window_title: focused.title.clone(),
                        process: focused.process.clone(),
                    },
                );
                action.timestamp = input.timestamp;
                action.metadata.window_id = Some(focused.id.clone());
                state.add_action(action.clone());
                emit_recorded_action(&app_handle, &action);
                last_focus = Some(focused.clone());
            }
            let mut pending_screenshot = None;
            let mut recorded_focus = input.focused.clone();
            let mut action = match &input.event {
                CapturedEvent::Browser { action } => {
                    let mut recorded =
                        RecordedAction::new(flow_like_types::create_id(), action.clone());
                    recorded.timestamp = input.timestamp;
                    recorded
                }
                CapturedEvent::MouseDown { .. } => {
                    pending_mouse = Some(input);
                    continue;
                }
                CapturedEvent::MouseUp { x, y, button } => {
                    let Some(mut down) = pending_mouse.take() else {
                        continue;
                    };
                    recorded_focus = down.focused.clone();
                    let CapturedEvent::MouseDown {
                        x: start_x,
                        y: start_y,
                        button: down_button,
                        modifiers,
                    } = &down.event
                    else {
                        continue;
                    };
                    if down_button != button {
                        continue;
                    }
                    let mut action = RecordedAction::new(
                        flow_like_types::create_id(),
                        if (x - start_x).abs() > 10 || (y - start_y).abs() > 10 {
                            ActionType::Drag {
                                start: (*start_x, *start_y),
                                end: (*x, *y),
                                button: button.clone(),
                                modifiers: modifiers.clone(),
                            }
                        } else {
                            ActionType::Click {
                                button: button.clone(),
                                modifiers: modifiers.clone(),
                            }
                        },
                    )
                    .with_coordinates(*start_x, *start_y);
                    action.timestamp = down.timestamp;
                    if let Some(target) = down.target.take() {
                        action.fingerprint = target.fingerprint;
                        pending_screenshot = target.screenshot;
                    }
                    if matches!(action.action_type, ActionType::Click { .. }) {
                        if last_click
                            .as_ref()
                            .is_some_and(|previous| is_double_click(previous, &action))
                        {
                            let previous = last_click.take().unwrap();
                            let mut state = state.write().await;
                            if let Some(session) = &mut state.session
                                && session
                                    .actions
                                    .last()
                                    .is_some_and(|last| last.id == previous.id)
                            {
                                let previous = session.actions.pop().unwrap();
                                action.id = previous.id.clone();
                                pending_screenshot = None;
                                action.action_type = ActionType::DoubleClick {
                                    button: button.clone(),
                                    modifiers: modifiers.clone(),
                                };
                                action.timestamp = previous.timestamp;
                                action.fingerprint = previous.fingerprint;
                                action.screenshot_ref = previous.screenshot_ref;
                            }
                        } else {
                            last_click = Some(action.clone());
                        }
                    } else {
                        last_click = None;
                    }
                    action
                }
                CapturedEvent::Scroll { x, y, dx, dy } => {
                    let mut state = state.write().await;
                    if let Some(action) = state.flush_keystroke_buffer() {
                        emit_recorded_action(&app_handle, &action);
                    }
                    for (direction, amount) in scroll_components(*dx, *dy) {
                        let mut action = RecordedAction::new(
                            flow_like_types::create_id(),
                            ActionType::Scroll { direction, amount },
                        )
                        .with_coordinates(*x, *y);
                        action.timestamp = input.timestamp;
                        state.add_action(action.clone());
                        emit_recorded_action(&app_handle, &action);
                    }
                    last_click = None;
                    continue;
                }
                CapturedEvent::Character { ch } => {
                    if !ch.is_control() {
                        let mut state = state.write().await;
                        state.buffer_keystroke_at(*ch, input.timestamp);
                    }
                    last_click = None;
                    continue;
                }
                CapturedEvent::Text { text } => {
                    let mut state = state.write().await;
                    for ch in text.chars().filter(|ch| !ch.is_control()) {
                        state.buffer_keystroke_at(ch, input.timestamp);
                    }
                    last_click = None;
                    continue;
                }
                CapturedEvent::KeyDown { .. } => {
                    last_click = None;
                    let Some(action_type) =
                        recorded_key_action(&input.event, input.clipboard.clone())
                    else {
                        continue;
                    };
                    let mut action = RecordedAction::new(flow_like_types::create_id(), action_type);
                    action.timestamp = input.timestamp;
                    action
                }
                _ => continue,
            };
            if let Some(focused) = recorded_focus {
                action.metadata = ActionMetadata {
                    window_id: Some(focused.id),
                    window_title: Some(focused.title),
                    process_name: Some(focused.process),
                    monitor_index: None,
                };
            }
            let mut state = state.write().await;
            if let Some(typed) = state.flush_keystroke_buffer() {
                emit_recorded_action(&app_handle, &typed);
            }
            state.add_action(action.clone());
            emit_recorded_action(&app_handle, &action);
            let (app_id, board_id) = state
                .session
                .as_ref()
                .map(|session| (session.app_id.clone(), session.target_board_id.clone()))
                .unwrap_or_default();
            drop(state);
            if let (Some(image), Some(store)) = (pending_screenshot, store.as_ref()) {
                if uploads.len() >= 16 {
                    crate::utils::emit_to_ui(
                        &app_handle,
                        "recording:error",
                        "Some screenshot templates could not be saved. Record those clicks again or disable pattern matching before inserting.",
                    );
                } else {
                    let store = store.clone();
                    let state = state_for_uploads.clone();
                    let app_handle = app_handle.clone();
                    uploads.spawn(async move {
                        match store_region(image, &store, app_id.as_deref(), board_id.as_deref())
                            .await
                        {
                            Ok(reference) => {
                                let mut state = state.write().await;
                                if let Some(session) = &mut state.session
                                    && let Some(recorded) = session
                                        .actions
                                        .iter_mut()
                                        .find(|recorded| recorded.id == action.id)
                                {
                                    recorded.screenshot_ref = Some(reference);
                                    emit_recorded_action(&app_handle, recorded);
                                }
                            }
                            Err(error) => crate::utils::emit_to_ui(
                                &app_handle,
                                "recording:error",
                                format!("Could not save a screenshot template: {error:?}"),
                            ),
                        }
                    });
                }
            }
        }
        while uploads.join_next().await.is_some() {}
        let mut state = state.write().await;
        if let Some(action) = state.flush_keystroke_buffer() {
            emit_recorded_action(&app_handle, &action);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn held_modifiers(keys: &[rdev::Key]) -> Vec<KeyModifier> {
    use rdev::Key;
    [
        (KeyModifier::Shift, [Key::ShiftLeft, Key::ShiftRight]),
        (KeyModifier::Control, [Key::ControlLeft, Key::ControlRight]),
        (KeyModifier::Alt, [Key::Alt, Key::AltGr]),
        (KeyModifier::Meta, [Key::MetaLeft, Key::MetaRight]),
    ]
    .into_iter()
    .filter_map(|(modifier, alternatives)| {
        alternatives
            .iter()
            .any(|key| keys.contains(key))
            .then_some(modifier)
    })
    .collect()
}

fn is_plain_shortcut(event: &CapturedEvent, expected: &str) -> bool {
    matches!(event, CapturedEvent::KeyDown { key, modifiers } if key.eq_ignore_ascii_case(expected)
        && modifiers.len() == 1 && modifiers.iter().any(|modifier| matches!(modifier, KeyModifier::Meta | KeyModifier::Control)))
}
fn is_copy_shortcut(event: &CapturedEvent) -> bool {
    is_plain_shortcut(event, "c")
}
fn is_paste_shortcut(event: &CapturedEvent) -> bool {
    is_plain_shortcut(event, "v")
}
fn recorded_key_action(event: &CapturedEvent, clipboard: Option<String>) -> Option<ActionType> {
    let CapturedEvent::KeyDown { key, modifiers } = event else {
        return None;
    };
    if is_copy_shortcut(event) {
        return Some(ActionType::Copy {
            clipboard_content: None,
        });
    }
    if is_paste_shortcut(event)
        && let Some(clipboard_content) = clipboard
    {
        return Some(ActionType::Paste {
            clipboard_content: Some(clipboard_content),
        });
    }
    Some(ActionType::KeyPress {
        key: if key == "Return" {
            "Enter".into()
        } else {
            key.clone()
        },
        modifiers: modifiers.clone(),
    })
}

fn is_stop_shortcut(event: &CapturedEvent) -> bool {
    matches!(event, CapturedEvent::KeyDown { key, modifiers } if key.eq_ignore_ascii_case("s")
        && modifiers.len() == 2 && modifiers.contains(&KeyModifier::Shift)
        && modifiers.contains(&if cfg!(target_os = "macos") { KeyModifier::Meta } else { KeyModifier::Control }))
}
fn is_ignored_process(process: &str, settings: &RecordingSettings) -> bool {
    let name = process.to_lowercase();
    name == "flow-like"
        || name == "flow like"
        || name == "flow-like.exe"
        || settings
            .ignore_system_apps
            .iter()
            .any(|ignored| ignored.eq_ignore_ascii_case(process))
}
fn is_double_click(previous: &RecordedAction, current: &RecordedAction) -> bool {
    let elapsed = current
        .timestamp
        .signed_duration_since(previous.timestamp)
        .num_milliseconds();
    let same_button = matches!((&previous.action_type, &current.action_type),
        (ActionType::Click { button: a, modifiers: am }, ActionType::Click { button: b, modifiers: bm }) if a == b && am == bm);
    same_button
        && (0..=400).contains(&elapsed)
        && matches!((previous.coordinates, current.coordinates), (Some((ax, ay)), Some((bx, by))) if (ax-bx).abs() <= 10 && (ay-by).abs() <= 10)
}
fn scroll_components(dx: i32, dy: i32) -> Vec<(ScrollDirection, i32)> {
    let mut result = Vec::new();
    // rdev preserves native wheel signs. Enigo uses positive vertical values for down.
    if dy != 0 {
        result.push((
            if dy > 0 {
                ScrollDirection::Up
            } else {
                ScrollDirection::Down
            },
            dy.saturating_abs(),
        ));
    }
    let dx = if cfg!(target_os = "macos") {
        dx.saturating_neg()
    } else {
        dx
    };
    if dx != 0 {
        result.push((
            if dx > 0 {
                ScrollDirection::Right
            } else {
                ScrollDirection::Left
            },
            dx.saturating_abs(),
        ));
    }
    result
}
impl Drop for EventCapture {
    fn drop(&mut self) {
        self.disconnect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hung_clipboard_is_single_flight_and_late_replies_are_never_reused() {
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_calls = calls.clone();
        let reader = Arc::new(
            ClipboardReader::new(move || {
                if worker_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Some("late snapshot".into())
                } else {
                    Some("fresh snapshot".into())
                }
            })
            .unwrap(),
        );
        let caller = reader.clone();
        let first =
            std::thread::spawn(move || caller.snapshot(std::time::Duration::from_millis(100)));
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(first.join().unwrap().is_none());
        for _ in 0..100 {
            assert!(
                reader
                    .snapshot(std::time::Duration::from_millis(25))
                    .is_none()
            );
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        release_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while reader.busy.load(std::sync::atomic::Ordering::Acquire)
            && std::time::Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        assert!(!reader.busy.load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(
            reader
                .snapshot(std::time::Duration::from_secs(1))
                .as_deref(),
            Some("fresh snapshot")
        );
    }

    #[test]
    fn unavailable_clipboard_preserves_the_paste_chord_without_a_clipboard_write() {
        let event = CapturedEvent::KeyDown {
            key: "v".into(),
            modifiers: vec![KeyModifier::Meta],
        };
        assert!(
            matches!(recorded_key_action(&event, None), Some(ActionType::KeyPress { key, modifiers }) if key == "v" && modifiers == [KeyModifier::Meta])
        );
        assert!(
            matches!(recorded_key_action(&event, Some("captured text".into())), Some(ActionType::Paste { clipboard_content: Some(text) }) if text == "captured text")
        );
    }

    #[test]
    fn focus_cache_never_waits_for_a_busy_native_lookup_or_reuses_stale_identity() {
        let cache = FocusCache::default();
        let focused = FocusedWindow {
            id: "42".into(),
            title: "Editor".into(),
            process: "editor".into(),
        };
        *cache.lock().unwrap() = Some((std::time::Instant::now(), Some(focused.clone())));
        assert_eq!(cached_focus(&cache), Some(focused.clone()));
        let mut guard = cache.lock().unwrap();
        assert!(cached_focus(&cache).is_none());
        *guard = Some((
            std::time::Instant::now() - std::time::Duration::from_secs(1),
            Some(focused),
        ));
        drop(guard);
        assert!(cached_focus(&cache).is_none());
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn releasing_one_side_does_not_release_the_other_modifier() {
        let mut held = vec![
            rdev::Key::ShiftLeft,
            rdev::Key::ShiftRight,
            rdev::Key::ControlRight,
        ];
        held.retain(|key| *key != rdev::Key::ShiftLeft);
        assert_eq!(
            held_modifiers(&held),
            [KeyModifier::Shift, KeyModifier::Control]
        );
        held.retain(|key| *key != rdev::Key::ShiftRight);
        assert_eq!(held_modifiers(&held), [KeyModifier::Control]);
    }

    #[test]
    fn shortcut_classification_preserves_modified_paste() {
        let plain = CapturedEvent::KeyDown {
            key: "v".into(),
            modifiers: vec![KeyModifier::Control],
        };
        assert!(is_paste_shortcut(&plain));
        let special = CapturedEvent::KeyDown {
            key: "v".into(),
            modifiers: vec![KeyModifier::Control, KeyModifier::Shift],
        };
        assert!(!is_paste_shortcut(&special));
        assert!(!is_stop_shortcut(&special));
        assert!(is_stop_shortcut(&CapturedEvent::KeyDown {
            key: "s".into(),
            modifiers: vec![
                KeyModifier::Shift,
                if cfg!(target_os = "macos") {
                    KeyModifier::Meta
                } else {
                    KeyModifier::Control
                }
            ]
        }));
    }

    #[test]
    fn double_click_uses_event_time_and_modifiers() {
        let first = RecordedAction::new(
            "first",
            ActionType::Click {
                button: MouseButton::Left,
                modifiers: vec![KeyModifier::Shift],
            },
        )
        .with_coordinates(25, 50);
        let mut second = first.clone();
        second.timestamp += chrono::Duration::milliseconds(200);
        assert!(is_double_click(&first, &second));
        second.action_type = ActionType::Click {
            button: MouseButton::Left,
            modifiers: vec![],
        };
        assert!(!is_double_click(&first, &second));
        second.action_type = first.action_type.clone();
        second.timestamp += chrono::Duration::milliseconds(250);
        assert!(!is_double_click(&first, &second));
    }

    #[test]
    fn diagonal_scroll_preserves_both_axes_and_native_vertical_sign() {
        let components = scroll_components(3, -7);
        assert_eq!(components.len(), 2);
        assert_eq!(components[0], (ScrollDirection::Down, 7));
        assert_eq!(
            components[1],
            (
                if cfg!(target_os = "macos") {
                    ScrollDirection::Left
                } else {
                    ScrollDirection::Right
                },
                3
            )
        );
    }

    #[test]
    fn ignored_processes_are_case_insensitive_and_include_recorder() {
        let settings = RecordingSettings::default();
        assert!(is_ignored_process("systemuiserver", &settings));
        assert!(is_ignored_process("Flow-Like", &settings));
        assert!(!is_ignored_process("TextEdit", &settings));
    }
}

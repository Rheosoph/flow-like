use std::sync::Arc;

use flow_like::flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::tokio::sync::{RwLock, mpsc};
use serde::{Deserialize, Serialize};
use tauri::Emitter;

use super::fingerprint::extract_fingerprint_at;
use super::screenshot::capture_region;
use super::state::{
    ActionMetadata, ActionType, KeyModifier, MouseButton, RecordedAction, RecordingStateInner,
    RecordingStatus, ScrollDirection,
};

/// Track the currently focused window
#[derive(Clone, Debug, Default)]
pub struct FocusedWindow {
    pub title: String,
    pub process: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CapturedEvent {
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
    WindowFocusChanged {
        title: String,
        process: String,
    },
}

pub struct EventCapture {
    _tx: mpsc::Sender<CapturedEvent>,
    active: Arc<std::sync::atomic::AtomicBool>,
}

/// Shared sender for the singleton rdev listener thread.
/// The rdev thread is spawned once and reused across recording sessions,
/// because `rdev::listen()` blocks forever and cannot be cancelled.
static RDEV_SENDER: std::sync::OnceLock<std::sync::Mutex<Option<mpsc::Sender<CapturedEvent>>>> =
    std::sync::OnceLock::new();

impl EventCapture {
    pub fn new(
        state: Arc<RwLock<RecordingStateInner>>,
        app_handle: tauri::AppHandle,
        store: Option<Arc<FlowLikeStore>>,
    ) -> Self {
        tracing::debug!("Creating new EventCapture instance");

        let (tx, rx) = mpsc::channel::<CapturedEvent>(10000);
        let active = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let active_clone = active.clone();

        // Install/replace the sender in the singleton rdev thread.
        // The first call spawns the thread; subsequent calls just swap the sender.
        let sender_slot = RDEV_SENDER.get_or_init(|| {
            let slot = std::sync::Mutex::new(Some(tx.clone()));
            let slot_ref = &RDEV_SENDER;

            tracing::debug!("Spawning singleton rdev listener thread");
            std::thread::spawn(move || {
                // Wait until the slot is initialized by OnceLock
                loop {
                    if let Some(s) = slot_ref.get() {
                        Self::start_event_loop_shared(s, active_clone);
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                tracing::warn!("rdev listener thread exited unexpectedly");
            });

            slot
        });

        // Replace the sender for subsequent sessions
        if let Ok(mut guard) = sender_slot.lock() {
            *guard = Some(tx.clone());
        }

        let state_for_processor = state;
        let active_for_processor = active.clone();
        tracing::debug!("Spawning event processor task...");
        flow_like_types::tokio::spawn(async move {
            Self::process_events(
                rx,
                state_for_processor,
                active_for_processor,
                app_handle,
                store,
            )
            .await;
        });

        Self { _tx: tx, active }
    }

    pub fn set_active(&self, active: bool) {
        tracing::debug!("set_active({})", active);
        self.active
            .store(active, std::sync::atomic::Ordering::SeqCst);
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

    /// Fallback character mapping using physical key positions (US QWERTY).
    /// Only used when the OS-provided `event.name` is unavailable.
    /// Note: on non-US layouts some keys (e.g. Y/Z on QWERTZ) may be swapped;
    /// shift+symbol mappings are intentionally omitted because they are
    /// layout-dependent.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn key_to_char_fallback(key: &rdev::Key, is_shift_held: bool) -> Option<char> {
        use rdev::Key;
        match key {
            // Letters — shift only toggles case (universal across layouts)
            Key::KeyA => Some(if is_shift_held { 'A' } else { 'a' }),
            Key::KeyB => Some(if is_shift_held { 'B' } else { 'b' }),
            Key::KeyC => Some(if is_shift_held { 'C' } else { 'c' }),
            Key::KeyD => Some(if is_shift_held { 'D' } else { 'd' }),
            Key::KeyE => Some(if is_shift_held { 'E' } else { 'e' }),
            Key::KeyF => Some(if is_shift_held { 'F' } else { 'f' }),
            Key::KeyG => Some(if is_shift_held { 'G' } else { 'g' }),
            Key::KeyH => Some(if is_shift_held { 'H' } else { 'h' }),
            Key::KeyI => Some(if is_shift_held { 'I' } else { 'i' }),
            Key::KeyJ => Some(if is_shift_held { 'J' } else { 'j' }),
            Key::KeyK => Some(if is_shift_held { 'K' } else { 'k' }),
            Key::KeyL => Some(if is_shift_held { 'L' } else { 'l' }),
            Key::KeyM => Some(if is_shift_held { 'M' } else { 'm' }),
            Key::KeyN => Some(if is_shift_held { 'N' } else { 'n' }),
            Key::KeyO => Some(if is_shift_held { 'O' } else { 'o' }),
            Key::KeyP => Some(if is_shift_held { 'P' } else { 'p' }),
            Key::KeyQ => Some(if is_shift_held { 'Q' } else { 'q' }),
            Key::KeyR => Some(if is_shift_held { 'R' } else { 'r' }),
            Key::KeyS => Some(if is_shift_held { 'S' } else { 's' }),
            Key::KeyT => Some(if is_shift_held { 'T' } else { 't' }),
            Key::KeyU => Some(if is_shift_held { 'U' } else { 'u' }),
            Key::KeyV => Some(if is_shift_held { 'V' } else { 'v' }),
            Key::KeyW => Some(if is_shift_held { 'W' } else { 'w' }),
            Key::KeyX => Some(if is_shift_held { 'X' } else { 'x' }),
            Key::KeyY => Some(if is_shift_held { 'Y' } else { 'y' }),
            Key::KeyZ => Some(if is_shift_held { 'Z' } else { 'z' }),

            // Number row — no shift-symbol mapping (layout-dependent)
            Key::Num0 => Some('0'),
            Key::Num1 => Some('1'),
            Key::Num2 => Some('2'),
            Key::Num3 => Some('3'),
            Key::Num4 => Some('4'),
            Key::Num5 => Some('5'),
            Key::Num6 => Some('6'),
            Key::Num7 => Some('7'),
            Key::Num8 => Some('8'),
            Key::Num9 => Some('9'),

            Key::Space => Some(' '),
            Key::Comma => Some(','),
            Key::Dot => Some('.'),
            Key::SemiColon => Some(';'),
            Key::Quote => Some('\''),
            Key::BackQuote => Some('`'),
            Key::Slash => Some('/'),
            Key::BackSlash => Some('\\'),
            Key::LeftBracket => Some('['),
            Key::RightBracket => Some(']'),
            Key::Minus => Some('-'),
            Key::Equal => Some('='),

            // Keypad keys are layout-independent
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
    fn start_event_loop_shared(
        _sender_slot: &std::sync::Mutex<Option<mpsc::Sender<CapturedEvent>>>,
        _active: Arc<std::sync::atomic::AtomicBool>,
    ) {
        use rdev::{Event, EventType, Key, listen};
        use std::sync::atomic::{AtomicI32, Ordering};

        tracing::debug!("start_event_loop: initializing rdev listener");

        #[cfg(target_os = "macos")]
        {
            rdev::set_is_main_thread(false);
            tracing::debug!("Set is_main_thread=false for macOS thread safety");
        }

        let mouse_x = Arc::new(AtomicI32::new(0));
        let mouse_y = Arc::new(AtomicI32::new(0));
        let event_count = Arc::new(AtomicI32::new(0));
        let event_count_clone = event_count.clone();
        let shift_pressed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shift_clone = shift_pressed.clone();
        let ctrl_pressed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ctrl_clone = ctrl_pressed.clone();
        let meta_pressed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let meta_clone = meta_pressed.clone();
        let alt_pressed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let alt_clone = alt_pressed.clone();

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
                        _ => MouseButton::Left,
                    };

                    let x = mouse_x.load(Ordering::Relaxed);
                    let y = mouse_y.load(Ordering::Relaxed);

                    let mut mods = Vec::new();
                    if shift_clone.load(Ordering::SeqCst) {
                        mods.push(KeyModifier::Shift);
                    }
                    if ctrl_clone.load(Ordering::SeqCst) {
                        mods.push(KeyModifier::Control);
                    }
                    if alt_clone.load(Ordering::SeqCst) {
                        mods.push(KeyModifier::Alt);
                    }
                    if meta_clone.load(Ordering::SeqCst) {
                        mods.push(KeyModifier::Meta);
                    }

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
                        _ => MouseButton::Left,
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
                    // Track modifier states
                    match key {
                        Key::ShiftLeft | Key::ShiftRight => {
                            shift_clone.store(true, Ordering::SeqCst)
                        }
                        Key::ControlLeft | Key::ControlRight => {
                            ctrl_clone.store(true, Ordering::SeqCst)
                        }
                        Key::MetaLeft | Key::MetaRight => meta_clone.store(true, Ordering::SeqCst),
                        Key::Alt | Key::AltGr => alt_clone.store(true, Ordering::SeqCst),
                        _ => {}
                    }

                    let key_str = Self::key_to_string(&key);
                    let is_shift_held = shift_clone.load(Ordering::SeqCst);
                    let has_ctrl = ctrl_clone.load(Ordering::SeqCst);
                    let has_meta = meta_clone.load(Ordering::SeqCst);
                    let has_alt = alt_clone.load(Ordering::SeqCst);

                    // Build modifiers array from current modifier state
                    let mut modifiers = Vec::new();
                    if is_shift_held {
                        modifiers.push(KeyModifier::Shift);
                    }
                    if has_ctrl {
                        modifiers.push(KeyModifier::Control);
                    }
                    if has_alt {
                        modifiers.push(KeyModifier::Alt);
                    }
                    if has_meta {
                        modifiers.push(KeyModifier::Meta);
                    }

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
                        let maybe_char = event
                            .name
                            .as_ref()
                            .and_then(|name| {
                                let mut chars = name.chars();
                                let first = chars.next();
                                let second = chars.next();
                                match (first, second) {
                                    (Some(ch), None) if !ch.is_control() => Some(ch),
                                    _ => None,
                                }
                            })
                            .or_else(|| {
                                tracing::debug!(
                                    "event.name was {:?} for key {:?}, using fallback",
                                    event.name,
                                    key
                                );
                                Self::key_to_char_fallback(&key, is_shift_held)
                            });

                        if let Some(ch) = maybe_char {
                            // Send ONLY Character event for text input (no KeyDown)
                            Some(CapturedEvent::Character { ch })
                        } else {
                            // Unknown key without character, send as KeyDown
                            Some(CapturedEvent::KeyDown {
                                key: key_str,
                                modifiers,
                            })
                        }
                    }
                }
                EventType::KeyRelease(key) => {
                    // Track modifier states
                    match key {
                        Key::ShiftLeft | Key::ShiftRight => {
                            shift_clone.store(false, Ordering::SeqCst)
                        }
                        Key::ControlLeft | Key::ControlRight => {
                            ctrl_clone.store(false, Ordering::SeqCst)
                        }
                        Key::MetaLeft | Key::MetaRight => meta_clone.store(false, Ordering::SeqCst),
                        Key::Alt | Key::AltGr => alt_clone.store(false, Ordering::SeqCst),
                        _ => {}
                    }

                    let key_str = Self::key_to_string(&key);
                    Some(CapturedEvent::KeyUp { key: key_str })
                }
            };

            if let Some(captured) = captured
                && let Some(slot) = RDEV_SENDER.get()
                && let Ok(guard) = slot.lock()
                && let Some(tx) = guard.as_ref()
            {
                let _ = tx.blocking_send(captured);
            }
        };

        tracing::debug!("Starting rdev::listen...");
        if let Err(error) = listen(callback) {
            tracing::warn!("rdev::listen returned error: {:?}", error);
        }
        tracing::warn!("rdev::listen exited unexpectedly");
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    fn start_event_loop_shared(
        _sender_slot: &std::sync::Mutex<Option<mpsc::Sender<CapturedEvent>>>,
        _active: Arc<std::sync::atomic::AtomicBool>,
    ) {
        tracing::debug!("Event capture not available on this platform");
    }

    /// Get the currently focused window using xcap
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    fn get_focused_window() -> Option<FocusedWindow> {
        use xcap::Window;

        let windows = Window::all().ok()?;
        let focused = windows.iter().find(|w| w.is_focused().unwrap_or(false))?;

        let title = focused.title().unwrap_or_default();
        let process = focused.app_name().unwrap_or_default();

        Some(FocusedWindow { title, process })
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
        use arboard::Clipboard;

        std::thread::spawn(|| {
            Clipboard::new()
                .ok()
                .and_then(|mut cb: Clipboard| cb.get_text().ok())
        })
        .join()
        .ok()
        .flatten()
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    fn get_clipboard_text() -> Option<String> {
        None
    }

    async fn process_events(
        mut rx: mpsc::Receiver<CapturedEvent>,
        state: Arc<RwLock<RecordingStateInner>>,
        active: Arc<std::sync::atomic::AtomicBool>,
        app_handle: tauri::AppHandle,
        store: Option<Arc<FlowLikeStore>>,
    ) {
        let mut last_mouse_down: Option<(
            i32,
            i32,
            MouseButton,
            Vec<KeyModifier>,
            std::time::Instant,
        )> = None;
        let mut drag_start: Option<(i32, i32)> = None;
        let mut last_focused_window: Option<FocusedWindow> = None;

        // Double-click detection - track completed clicks (not mouse downs)
        let mut last_completed_click: Option<(i32, i32, MouseButton, std::time::Instant)> = None;
        const DOUBLE_CLICK_THRESHOLD_MS: u128 = 400; // Standard OS double-click threshold

        // Pending copy detection - copy clipboard content on KeyUp after delay
        let mut pending_copy_key: Option<String> = None;
        const DOUBLE_CLICK_DISTANCE: i32 = 10; // Pixels

        tracing::debug!(" process_events: store available: {}", store.is_some());
        tracing::debug!(" process_events: waiting for events...");

        // Check session info
        {
            let state_guard = state.read().await;
            if let Some(session) = &state_guard.session {
                tracing::debug!(" Session ID: {}", session.id);
                tracing::debug!(" Target board ID: {:?}", session.target_board_id);
            } else {
                tracing::warn!(" No session in state!");
            }
        }

        let mut processed_count = 0u32;
        let mut action_count = 0u32;
        let mut last_event_time = std::time::Instant::now();
        // Reduce dedup interval - only skip very rapid duplicate non-click events
        let min_event_interval = std::time::Duration::from_millis(5);

        tracing::debug!(" About to enter event loop...");
        while let Some(event) = rx.recv().await {
            processed_count += 1;

            // Deduplicate rapid events EXCEPT mouse clicks and key events (to preserve timing)
            let now = std::time::Instant::now();
            let is_important_event = matches!(
                event,
                CapturedEvent::MouseDown { .. }
                    | CapturedEvent::MouseUp { .. }
                    | CapturedEvent::KeyDown { .. }
                    | CapturedEvent::Character { .. }
            );
            if !is_important_event && now.duration_since(last_event_time) < min_event_interval {
                continue;
            }
            last_event_time = now;

            if processed_count % 10 == 1 {
                tracing::debug!(" Received event #{}: {:?}", processed_count, event);
            }

            if !active.load(std::sync::atomic::Ordering::SeqCst) {
                tracing::debug!(" Skipping event #{} - not active", processed_count);
                continue;
            }

            {
                let state_guard = state.read().await;
                if state_guard.status != RecordingStatus::Recording {
                    tracing::debug!(
                        " Skipping event #{} - status is {:?}",
                        processed_count,
                        state_guard.status
                    );
                    continue;
                }
            }

            // Check for window focus changes on any mouse event (user is interacting with something)
            if matches!(
                event,
                CapturedEvent::MouseDown { .. } | CapturedEvent::MouseUp { .. }
            ) && let Some(current_window) = Self::get_focused_window()
            {
                let focus_changed = match &last_focused_window {
                    Some(last) => {
                        last.title != current_window.title || last.process != current_window.process
                    }
                    None => true, // First focus detection
                };

                if focus_changed
                    && (!current_window.title.is_empty() || !current_window.process.is_empty())
                {
                    tracing::debug!(
                        " Window focus changed to: {} ({})",
                        current_window.title,
                        current_window.process
                    );

                    // Flush any pending keystrokes before focus change
                    {
                        let mut state_guard = state.write().await;
                        if let Some(typed_action) = state_guard.flush_keystroke_buffer() {
                            let _ = app_handle.emit("recording:action", &typed_action);
                        }
                    }

                    // Create and emit WindowFocus action
                    let action = RecordedAction::new(
                        flow_like_types::create_id(),
                        ActionType::WindowFocus {
                            window_title: current_window.title.clone(),
                            process: current_window.process.clone(),
                        },
                    );

                    {
                        let mut state_guard = state.write().await;
                        state_guard.add_action(action.clone());
                    }
                    let _ = app_handle.emit("recording:action", &action);

                    last_focused_window = Some(current_window);
                }
            }

            match &event {
                CapturedEvent::MouseDown {
                    x,
                    y,
                    button,
                    modifiers,
                } => {
                    last_mouse_down = Some((
                        *x,
                        *y,
                        button.clone(),
                        modifiers.clone(),
                        std::time::Instant::now(),
                    ));
                    drag_start = Some((*x, *y));
                }
                CapturedEvent::MouseUp { x, y, button } => {
                    // Get fresh coordinates from enigo for accuracy (aligns with screenshot capture)
                    let (fresh_x, fresh_y) = Self::get_mouse_location().unwrap_or((*x, *y));
                    tracing::debug!(
                        " MouseUp: rdev coords=({}, {}), fresh coords=({}, {})",
                        x,
                        y,
                        fresh_x,
                        fresh_y
                    );
                    let (x, y) = (fresh_x, fresh_y);
                    let button = button.clone();

                    {
                        let mut state_guard = state.write().await;
                        if let Some(typed_action) = state_guard.flush_keystroke_buffer() {
                            let _ = app_handle.emit("recording:action", &typed_action);
                        }
                    }

                    // Get drag start position, or use current position if MouseDown was missed
                    let (start_x, start_y) = drag_start.take().unwrap_or((x, y));
                    let dx = (x - start_x).abs();
                    let dy = (y - start_y).abs();

                    // Only record as drag if significant movement, otherwise it's a click
                    if dx > 10 || dy > 10 {
                        let action = RecordedAction::new(
                            flow_like_types::create_id(),
                            ActionType::Drag {
                                start: (start_x, start_y),
                                end: (x, y),
                            },
                        )
                        .with_coordinates(start_x, start_y);

                        let mut state_guard = state.write().await;
                        state_guard.add_action(action.clone());
                        action_count += 1;
                        tracing::debug!(
                            " Drag action #{} added from ({}, {}) to ({}, {})",
                            action_count,
                            start_x,
                            start_y,
                            x,
                            y
                        );
                        let _ = app_handle.emit("recording:action", &action);
                    } else {
                        // This is a click (not a drag)
                        let click_time = std::time::Instant::now();

                        // Check for double-click against the last completed click
                        let is_double_click = if let Some((lx, ly, lb, lt)) = &last_completed_click
                        {
                            let distance = (x - lx).abs().max((y - ly).abs());
                            let time_diff = click_time.duration_since(*lt).as_millis();
                            // Double-click: same button, close position, within time threshold
                            *lb == button
                                && distance <= DOUBLE_CLICK_DISTANCE
                                && time_diff <= DOUBLE_CLICK_THRESHOLD_MS
                        } else {
                            false
                        };

                        let (capture_screenshots, region_size, app_id, board_id) = {
                            let state_guard = state.read().await;
                            state_guard
                                .session
                                .as_ref()
                                .map(|s| {
                                    (
                                        s.settings.capture_screenshots,
                                        s.settings.capture_region_size,
                                        s.app_id.clone(),
                                        s.target_board_id.clone(),
                                    )
                                })
                                .unwrap_or((false, 150, None, None))
                        };

                        let screenshot_ref = if capture_screenshots {
                            if let Some(ref store) = store {
                                capture_region(
                                    x,
                                    y,
                                    region_size,
                                    store,
                                    app_id.as_deref(),
                                    board_id.as_deref(),
                                )
                                .await
                                .ok()
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        // Extract UI element fingerprint at click location
                        let fingerprint = extract_fingerprint_at(x, y);

                        // Build metadata from current focused window
                        let click_metadata = if let Some(ref fw) = last_focused_window {
                            ActionMetadata {
                                window_title: Some(fw.title.clone()),
                                process_name: Some(fw.process.clone()),
                                monitor_index: None,
                            }
                        } else {
                            ActionMetadata::default()
                        };

                        if is_double_click {
                            // Remove the previous single click and replace with double-click
                            {
                                let mut state_guard = state.write().await;
                                if let Some(session) = &mut state_guard.session
                                    && let Some(last_action) = session.actions.last()
                                    && matches!(last_action.action_type, ActionType::Click { .. })
                                {
                                    session.actions.pop();
                                }
                            }

                            let mut action = RecordedAction::new(
                                flow_like_types::create_id(),
                                ActionType::DoubleClick {
                                    button: button.clone(),
                                },
                            )
                            .with_coordinates(x, y)
                            .with_metadata(click_metadata.clone());

                            if let Some(ref screenshot_id) = screenshot_ref {
                                action = action.with_screenshot_ref(screenshot_id);
                            }

                            if let Some(fp) = fingerprint {
                                action = action.with_fingerprint(fp);
                            }

                            let mut state_guard = state.write().await;
                            state_guard.add_action(action.clone());
                            action_count += 1;
                            let _ = app_handle.emit("recording:action", &action);

                            // Clear to prevent triple-click
                            last_completed_click = None;
                        } else {
                            let click_modifiers = last_mouse_down
                                .as_ref()
                                .map(|(_, _, _, mods, _)| mods.clone())
                                .unwrap_or_default();
                            let mut action = RecordedAction::new(
                                flow_like_types::create_id(),
                                ActionType::Click {
                                    button: button.clone(),
                                    modifiers: click_modifiers,
                                },
                            )
                            .with_coordinates(x, y)
                            .with_metadata(click_metadata);

                            if let Some(ref screenshot_id) = screenshot_ref {
                                action = action.with_screenshot_ref(screenshot_id);
                            }

                            if let Some(fp) = fingerprint {
                                action = action.with_fingerprint(fp);
                            }

                            let mut state_guard = state.write().await;
                            state_guard.add_action(action.clone());
                            action_count += 1;
                            let _ = app_handle.emit("recording:action", &action);

                            // Record for double-click detection
                            last_completed_click = Some((x, y, button.clone(), click_time));
                        }
                    }

                    last_mouse_down = None;
                }
                CapturedEvent::Scroll { x, y, dx, dy } => {
                    // Skip scroll events with no actual movement
                    if *dx == 0 && *dy == 0 {
                        continue;
                    }

                    // Get fresh coordinates for scroll position
                    let (x, y) = Self::get_mouse_location().unwrap_or((*x, *y));

                    let mut state_guard = state.write().await;
                    state_guard.flush_keystroke_buffer();

                    // Determine scroll direction and amount.
                    // rdev convention: positive dy = scroll down, negative dy = scroll up
                    // (matches macOS "natural" scrolling inverted at driver level).
                    // Positive dx = scroll right, negative dx = scroll left.
                    let (direction, amount) = if dy.abs() >= dx.abs() && *dy != 0 {
                        if *dy > 0 {
                            (ScrollDirection::Down, *dy)
                        } else {
                            (ScrollDirection::Up, -dy)
                        }
                    } else if *dx != 0 {
                        if *dx > 0 {
                            (ScrollDirection::Right, *dx)
                        } else {
                            (ScrollDirection::Left, -dx)
                        }
                    } else {
                        continue; // Both are 0, skip
                    };

                    let action = RecordedAction::new(
                        flow_like_types::create_id(),
                        ActionType::Scroll { direction, amount },
                    )
                    .with_coordinates(x, y);

                    state_guard.add_action(action.clone());
                    let _ = app_handle.emit("recording:action", &action);
                }
                CapturedEvent::KeyDown { key, modifiers } => {
                    tracing::debug!(" KeyDown: key='{}', modifiers={:?}", key, modifiers);

                    let is_modifier = matches!(
                        key.as_str(),
                        "Shift"
                            | "Ctrl"
                            | "Alt"
                            | "Meta"
                            | "ShiftLeft"
                            | "ShiftRight"
                            | "ControlLeft"
                            | "ControlRight"
                            | "AltLeft"
                            | "AltRight"
                            | "MetaLeft"
                            | "MetaRight"
                    );

                    let is_special = matches!(
                        key.as_str(),
                        "Return"
                            | "Enter"
                            | "Tab"
                            | "Escape"
                            | "Backspace"
                            | "Delete"
                            | "Up"
                            | "Down"
                            | "Left"
                            | "Right"
                            | "Home"
                            | "End"
                            | "PageUp"
                            | "PageDown"
                            | "F1"
                            | "F2"
                            | "F3"
                            | "F4"
                            | "F5"
                            | "F6"
                            | "F7"
                            | "F8"
                            | "F9"
                            | "F10"
                            | "F11"
                            | "F12"
                    );

                    // Check for Copy (Ctrl+C / Cmd+C) or Paste (Ctrl+V / Cmd+V)
                    let has_cmd_or_ctrl = modifiers.contains(&KeyModifier::Control)
                        || modifiers.contains(&KeyModifier::Meta);
                    let is_copy = has_cmd_or_ctrl && key.to_lowercase() == "c";
                    let is_paste = has_cmd_or_ctrl && key.to_lowercase() == "v";

                    tracing::debug!(
                        " KeyDown analysis: has_cmd_or_ctrl={}, is_copy={}, is_paste={}",
                        has_cmd_or_ctrl,
                        is_copy,
                        is_paste
                    );

                    // For Copy, defer clipboard reading until KeyUp (system processes copy after KeyDown)
                    if is_copy {
                        tracing::debug!(" Setting pending_copy_key to '{}'", key);
                        pending_copy_key = Some(key.clone());
                        continue;
                    }

                    // Record special keys (Enter, Tab, etc.) OR any key with modifiers (Ctrl+C, etc.)
                    // Skip pure modifier keys
                    if !is_modifier && (is_special || !modifiers.is_empty()) {
                        let mut state_guard = state.write().await;
                        // Flush any buffered keystrokes before adding the special key
                        if let Some(typed_action) = state_guard.flush_keystroke_buffer() {
                            let _ = app_handle.emit("recording:action", &typed_action);
                        }

                        let action = if is_paste {
                            // For paste, clipboard already has content - read immediately
                            let clipboard_content = Self::get_clipboard_text();
                            tracing::debug!(
                                " Paste detected, clipboard: {:?}",
                                clipboard_content.as_ref().map(|s| if s.len() > 50 {
                                    format!("{}...", &s[..50])
                                } else {
                                    s.clone()
                                })
                            );
                            RecordedAction::new(
                                flow_like_types::create_id(),
                                ActionType::Paste { clipboard_content },
                            )
                        } else {
                            // Normalize key name for the workflow
                            let normalized_key = match key.as_str() {
                                "Return" => "Enter".to_string(),
                                other => other.to_string(),
                            };

                            RecordedAction::new(
                                flow_like_types::create_id(),
                                ActionType::KeyPress {
                                    key: normalized_key.clone(),
                                    modifiers: modifiers.clone(),
                                },
                            )
                        };

                        state_guard.add_action(action.clone());
                        action_count += 1;
                        tracing::debug!(
                            " KeyPress action #{} added: {:?}",
                            action_count,
                            action.action_type
                        );
                        let _ = app_handle.emit("recording:action", &action);
                    }
                }
                CapturedEvent::KeyUp { key } => {
                    tracing::debug!(
                        " KeyUp: key='{}', pending_copy_key={:?}",
                        key,
                        pending_copy_key
                    );

                    // Handle deferred Copy detection - clipboard is now populated
                    let pending_matches = pending_copy_key.as_ref().map(|k| k.to_lowercase())
                        == Some(key.to_lowercase());
                    tracing::debug!(" KeyUp: pending_matches={}", pending_matches);

                    if pending_matches {
                        pending_copy_key = None;

                        // Retry clipboard read with increasing delay to handle OS clipboard latency
                        let mut clipboard_content = None;
                        for delay in [50, 100, 200] {
                            flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(
                                delay,
                            ))
                            .await;
                            clipboard_content = Self::get_clipboard_text();
                            if clipboard_content.is_some() {
                                break;
                            }
                        }
                        tracing::debug!(
                            " Copy detected (on KeyUp), clipboard: {:?}",
                            clipboard_content.as_ref().map(|s| if s.len() > 50 {
                                format!("{}...", &s[..50])
                            } else {
                                s.clone()
                            })
                        );

                        let mut state_guard = state.write().await;
                        if let Some(typed_action) = state_guard.flush_keystroke_buffer() {
                            let _ = app_handle.emit("recording:action", &typed_action);
                        }

                        let action = RecordedAction::new(
                            flow_like_types::create_id(),
                            ActionType::Copy { clipboard_content },
                        );

                        state_guard.add_action(action.clone());
                        action_count += 1;
                        tracing::debug!(" Copy action #{} added", action_count);
                        let _ = app_handle.emit("recording:action", &action);
                    }
                }
                CapturedEvent::Character { ch } => {
                    if ch.is_control() {
                        continue;
                    }
                    let mut state_guard = state.write().await;
                    state_guard.buffer_keystroke(*ch);
                    // Log every 10th character for debugging without spam
                    if state_guard.keystroke_buffer_len() % 10 == 1 {
                        tracing::debug!(
                            " Buffered char '{}', buffer len: {}",
                            ch,
                            state_guard.keystroke_buffer_len()
                        );
                    }
                }
                _ => {}
            }

            {
                let mut state_guard = state.write().await;
                if state_guard.should_flush_keystrokes()
                    && let Some(typed_action) = state_guard.flush_keystroke_buffer()
                {
                    let _ = app_handle.emit("recording:action", &typed_action);
                }
            }
        }
        tracing::debug!(" ========== PROCESSOR LOOP EXITED ==========");
        tracing::debug!(" Total events processed: {}", processed_count);
        tracing::debug!(" Total actions created: {}", action_count);

        let state_guard = state.read().await;
        if let Some(session) = &state_guard.session {
            tracing::debug!(
                " Session has {} actions at processor exit",
                session.actions.len()
            );
        }
    }
}

impl Drop for EventCapture {
    fn drop(&mut self) {
        self.active
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

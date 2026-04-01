use chrono::{DateTime, Utc};
use flow_like_types::tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

use crate::functions::TauriFunctionError;

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
pub enum RecordingStatus {
    #[default]
    Idle,
    Recording,
    Paused,
    Processing,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
pub enum ScrollDirection {
    #[default]
    Down,
    Up,
    Left,
    Right,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
pub enum KeyModifier {
    #[default]
    Shift,
    Control,
    Alt,
    Meta,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum ActionType {
    Click {
        button: MouseButton,
        modifiers: Vec<KeyModifier>,
    },
    DoubleClick {
        button: MouseButton,
    },
    Drag {
        start: (i32, i32),
        end: (i32, i32),
    },
    Scroll {
        direction: ScrollDirection,
        amount: i32,
    },
    KeyType {
        text: String,
    },
    KeyPress {
        key: String,
        modifiers: Vec<KeyModifier>,
    },
    AppLaunch {
        app_name: String,
        app_path: String,
    },
    WindowFocus {
        window_title: String,
        process: String,
    },
    /// Copy action - captures what was copied to clipboard
    Copy {
        /// The text content that was copied
        clipboard_content: Option<String>,
    },
    /// Paste action - captures what was pasted from clipboard
    Paste {
        /// The text content that was pasted
        clipboard_content: Option<String>,
    },
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct RecordedFingerprint {
    pub id: String,
    pub role: Option<String>,
    pub name: Option<String>,
    pub text: Option<String>,
    pub bounding_box: Option<(f64, f64, f64, f64)>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct ActionMetadata {
    pub window_title: Option<String>,
    pub process_name: Option<String>,
    pub monitor_index: Option<usize>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct RecordedAction {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub action_type: ActionType,
    pub coordinates: Option<(i32, i32)>,
    pub screenshot_ref: Option<String>,
    pub fingerprint: Option<RecordedFingerprint>,
    pub metadata: ActionMetadata,
}

impl RecordedAction {
    pub fn new(id: impl Into<String>, action_type: ActionType) -> Self {
        Self {
            id: id.into(),
            timestamp: Utc::now(),
            action_type,
            coordinates: None,
            screenshot_ref: None,
            fingerprint: None,
            metadata: ActionMetadata::default(),
        }
    }

    pub fn with_coordinates(mut self, x: i32, y: i32) -> Self {
        self.coordinates = Some((x, y));
        self
    }

    pub fn with_screenshot_ref(mut self, screenshot_ref: impl Into<String>) -> Self {
        self.screenshot_ref = Some(screenshot_ref.into());
        self
    }

    pub fn with_fingerprint(mut self, fingerprint: RecordedFingerprint) -> Self {
        self.fingerprint = Some(fingerprint);
        self
    }

    pub fn with_metadata(mut self, metadata: ActionMetadata) -> Self {
        self.metadata = metadata;
        self
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct RecordingSettings {
    pub capture_screenshots: bool,
    pub capture_fingerprints: bool,
    pub aggregate_keystrokes: bool,
    pub ignore_system_apps: Vec<String>,
    pub capture_region_size: u32,
    /// When true, generated workflows will use vision_click_template with pattern matching
    /// and fallback to coordinates if the template isn't found
    pub use_pattern_matching: bool,
    /// Confidence threshold for template matching (0.0-1.0)
    pub template_confidence: f64,
    /// When true, use natural curved mouse movements to avoid bot detection
    pub bot_detection_evasion: bool,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            capture_screenshots: true,
            capture_fingerprints: true,
            aggregate_keystrokes: true,
            ignore_system_apps: vec!["SystemUIServer".to_string(), "loginwindow".to_string()],
            capture_region_size: 150,
            // Disabled by default - requires RPA session instead of Computer session
            // When enabled, clicks with screenshots will use vision_click_template
            use_pattern_matching: false,
            template_confidence: 0.8,
            // When enabled, clicks use natural curved mouse movements
            bot_detection_evasion: false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct RecordingSession {
    pub id: String,
    pub status: RecordingStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub actions: Vec<RecordedAction>,
    pub app_id: Option<String>,
    pub target_board_id: Option<String>,
    pub settings: RecordingSettings,
}

impl RecordingSession {
    pub fn new(id: impl Into<String>, settings: RecordingSettings) -> Self {
        Self {
            id: id.into(),
            status: RecordingStatus::Idle,
            started_at: None,
            actions: Vec::new(),
            app_id: None,
            target_board_id: None,
            settings,
        }
    }
}

pub struct RecordingStateInner {
    pub status: RecordingStatus,
    pub session: Option<RecordingSession>,
    keystroke_buffer: String,
    last_keystroke_time: Option<DateTime<Utc>>,
}

impl RecordingStateInner {
    pub fn keystroke_buffer_len(&self) -> usize {
        self.keystroke_buffer.len()
    }
}

impl Default for RecordingStateInner {
    fn default() -> Self {
        Self {
            status: RecordingStatus::Idle,
            session: None,
            keystroke_buffer: String::new(),
            last_keystroke_time: None,
        }
    }
}

impl RecordingStateInner {
    pub async fn start_session(
        &mut self,
        app_id: Option<String>,
        board_id: Option<String>,
        settings: RecordingSettings,
    ) -> Result<String, TauriFunctionError> {
        if self.status != RecordingStatus::Idle {
            return Err(TauriFunctionError::new("Recording already in progress"));
        }

        let session_id = flow_like_types::create_id();
        let mut session = RecordingSession::new(&session_id, settings);
        session.app_id = app_id;
        session.target_board_id = board_id;
        session.started_at = Some(Utc::now());
        session.status = RecordingStatus::Recording;

        self.session = Some(session);
        self.status = RecordingStatus::Recording;
        self.keystroke_buffer.clear();
        self.last_keystroke_time = None;

        Ok(session_id)
    }

    pub async fn pause(&mut self) -> Result<(), TauriFunctionError> {
        if self.status != RecordingStatus::Recording {
            return Err(TauriFunctionError::new("Not currently recording"));
        }

        self.flush_keystroke_buffer();
        self.status = RecordingStatus::Paused;
        if let Some(session) = &mut self.session {
            session.status = RecordingStatus::Paused;
        }

        Ok(())
    }

    pub async fn resume(&mut self) -> Result<(), TauriFunctionError> {
        if self.status != RecordingStatus::Paused {
            return Err(TauriFunctionError::new("Not currently paused"));
        }

        self.status = RecordingStatus::Recording;
        if let Some(session) = &mut self.session {
            session.status = RecordingStatus::Recording;
        }

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<Vec<RecordedAction>, TauriFunctionError> {
        if self.status == RecordingStatus::Idle {
            return Err(TauriFunctionError::new("No recording in progress"));
        }

        self.flush_keystroke_buffer();
        self.status = RecordingStatus::Idle;

        let actions = self.session.take().map(|s| s.actions).unwrap_or_default();

        Ok(actions)
    }

    pub fn add_action(&mut self, action: RecordedAction) {
        if let Some(session) = &mut self.session {
            // Consolidate consecutive scroll events only if within 200ms of each other
            if let ActionType::Scroll {
                direction: new_dir,
                amount: new_amount,
            } = &action.action_type
                && let Some(last_action) = session.actions.last_mut()
                && let ActionType::Scroll {
                    direction: last_dir,
                    amount: last_amount,
                } = &mut last_action.action_type
            {
                // Only consolidate if same direction AND within time threshold
                let time_diff = action
                    .timestamp
                    .signed_duration_since(last_action.timestamp)
                    .num_milliseconds();
                if last_dir == new_dir && time_diff < 200 {
                    *last_amount += new_amount;
                    // Update coordinates and timestamp to the latest
                    if action.coordinates.is_some() {
                        last_action.coordinates = action.coordinates;
                    }
                    last_action.timestamp = action.timestamp;
                    return; // Don't add a new action, we merged into existing
                }
            }

            session.actions.push(action);
        }
    }

    pub fn buffer_keystroke(&mut self, ch: char) {
        if let Some(session) = &self.session
            && !session.settings.aggregate_keystrokes
        {
            let action = RecordedAction::new(
                flow_like_types::create_id(),
                ActionType::KeyType {
                    text: ch.to_string(),
                },
            );
            if let Some(session) = &mut self.session {
                session.actions.push(action);
            }
            return;
        }

        self.keystroke_buffer.push(ch);
        self.last_keystroke_time = Some(Utc::now());
    }

    pub fn flush_keystroke_buffer(&mut self) -> Option<RecordedAction> {
        if self.keystroke_buffer.is_empty() {
            return None;
        }

        let text = std::mem::take(&mut self.keystroke_buffer);
        tracing::debug!(
            "Flushing keystroke buffer: '{}' ({} chars)",
            text,
            text.len()
        );
        let action =
            RecordedAction::new(flow_like_types::create_id(), ActionType::KeyType { text });

        if let Some(session) = &mut self.session {
            session.actions.push(action.clone());
        }

        Some(action)
    }

    pub fn should_flush_keystrokes(&self) -> bool {
        if self.keystroke_buffer.is_empty() {
            return false;
        }

        if let Some(last_time) = self.last_keystroke_time {
            let elapsed = Utc::now() - last_time;
            // Flush after 300ms of inactivity (was 500ms) for more responsive text capture
            elapsed.num_milliseconds() > 300
        } else {
            false
        }
    }
}

pub struct RecordingState {
    pub inner: Arc<RwLock<RecordingStateInner>>,
    pub capture: Arc<RwLock<Option<super::capture::EventCapture>>>,
}

impl RecordingState {
    pub async fn construct(handler: &AppHandle) -> Result<Self, TauriFunctionError> {
        let state = handler
            .try_state::<TauriRecordingState>()
            .ok_or_else(|| TauriFunctionError::new("Recording state not found"))?;
        Ok(Self {
            inner: state.inner.clone(),
            capture: state.capture.clone(),
        })
    }
}

pub struct TauriRecordingState {
    pub inner: Arc<RwLock<RecordingStateInner>>,
    pub capture: Arc<RwLock<Option<super::capture::EventCapture>>>,
}

impl TauriRecordingState {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(RecordingStateInner::default())),
            capture: Arc::new(RwLock::new(None)),
        }
    }
}

impl Default for TauriRecordingState {
    fn default() -> Self {
        Self::new()
    }
}

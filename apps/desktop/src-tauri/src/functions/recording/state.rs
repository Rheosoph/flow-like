use chrono::{DateTime, Utc};
use flow_like_types::tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
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
    BrowserAttach {
        debugger_address: String,
        webdriver_url: String,
        #[serde(default = "default_browser_type")]
        browser_type: String,
    },
    Browser {
        action: BrowserAction,
    },
    Click {
        button: MouseButton,
        modifiers: Vec<KeyModifier>,
    },
    DoubleClick {
        button: MouseButton,
        #[serde(default)]
        modifiers: Vec<KeyModifier>,
    },
    Drag {
        start: (i32, i32),
        end: (i32, i32),
        #[serde(default)]
        button: MouseButton,
        #[serde(default)]
        modifiers: Vec<KeyModifier>,
    },
    MouseMove {
        x: i32,
        y: i32,
    },
    Wait {
        milliseconds: u64,
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

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserActionKind {
    Navigate,
    Click,
    DoubleClick,
    Type,
    Select,
    Key,
    WaitForUrl,
    WaitForElement,
    SelectTab,
    EnterFrame,
    LeaveFrame,
    Scroll,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct BrowserAction {
    pub kind: BrowserActionKind,
    #[serde(default)]
    pub tab_id: String,
    #[serde(default)]
    pub selector: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub frames: Vec<String>,
    #[serde(default)]
    pub button: MouseButton,
    #[serde(default)]
    pub modifiers: Vec<KeyModifier>,
    #[serde(default)]
    pub scroll_x: i32,
    #[serde(default)]
    pub scroll_y: i32,
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
    #[serde(default)]
    pub window_id: Option<String>,
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
    #[serde(default)]
    pub browser_debugger_address: Option<String>,
    #[serde(default = "default_webdriver_url")]
    pub browser_webdriver_url: String,
    #[serde(default = "default_browser_type")]
    pub browser_type: String,
    pub capture_screenshots: bool,
    pub capture_fingerprints: bool,
    pub aggregate_keystrokes: bool,
    pub ignore_system_apps: Vec<String>,
    pub capture_region_size: u32,
    /// Enable template matching on generated computer click nodes.
    pub use_pattern_matching: bool,
    /// Confidence threshold for template matching (0.0-1.0)
    pub template_confidence: f64,
    /// When true, use natural curved mouse movements to avoid bot detection
    pub bot_detection_evasion: bool,
}

fn default_webdriver_url() -> String {
    "http://127.0.0.1:9515".into()
}

fn default_browser_type() -> String {
    "Chrome".into()
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            browser_debugger_address: None,
            browser_webdriver_url: default_webdriver_url(),
            browser_type: default_browser_type(),
            capture_screenshots: true,
            capture_fingerprints: false,
            aggregate_keystrokes: true,
            ignore_system_apps: vec!["SystemUIServer".to_string(), "loginwindow".to_string()],
            capture_region_size: 150,
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RecordingOwner {
    profile_identity: String,
    app_id: Option<String>,
    board_id: Option<String>,
}

impl RecordingOwner {
    fn new(identity: &str, app_id: Option<&str>, board_id: Option<&str>) -> Self {
        Self {
            profile_identity: identity.to_string(),
            app_id: app_id.map(str::to_string),
            board_id: board_id.map(str::to_string),
        }
    }
}

pub struct RecordingStateInner {
    pub status: RecordingStatus,
    pub session: Option<RecordingSession>,
    completed_actions: HashMap<RecordingOwner, Vec<RecordedAction>>,
    owner: Option<RecordingOwner>,
    keystroke_buffer: String,
    last_keystroke_time: Option<DateTime<Utc>>,
    first_keystroke_time: Option<DateTime<Utc>>,
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
            completed_actions: HashMap::new(),
            owner: None,
            keystroke_buffer: String::new(),
            last_keystroke_time: None,
            first_keystroke_time: None,
        }
    }
}

impl RecordingStateInner {
    pub fn owns_context(
        &self,
        identity: &str,
        app_id: Option<&str>,
        board_id: Option<&str>,
    ) -> bool {
        self.owner.as_ref() == Some(&RecordingOwner::new(identity, app_id, board_id))
    }

    pub fn actions_for_context(
        &self,
        identity: &str,
        app_id: Option<&str>,
        board_id: Option<&str>,
    ) -> Vec<RecordedAction> {
        let owner = RecordingOwner::new(identity, app_id, board_id);
        if self.owner.as_ref() == Some(&owner)
            && let Some(session) = &self.session
        {
            return session.actions.clone();
        }
        self.completed_actions
            .get(&owner)
            .cloned()
            .unwrap_or_default()
    }

    pub fn last_completed_actions(&self, identity: &str) -> Vec<RecordedAction> {
        self.owner
            .as_ref()
            .filter(|owner| owner.profile_identity == identity)
            .and_then(|owner| self.completed_actions.get(owner))
            .cloned()
            .unwrap_or_default()
    }

    pub fn clear_completed(
        &mut self,
        identity: &str,
        app_id: Option<&str>,
        board_id: Option<&str>,
    ) {
        let owner = RecordingOwner::new(identity, app_id, board_id);
        self.completed_actions.remove(&owner);
        if self.status == RecordingStatus::Idle && self.owner.as_ref() == Some(&owner) {
            self.owner = None;
        }
    }

    pub fn validate_completed_actions(
        &self,
        identity: &str,
        app_id: Option<&str>,
        board_id: Option<&str>,
        actions: &[RecordedAction],
    ) -> Result<(), TauriFunctionError> {
        if self.status != RecordingStatus::Idle {
            return Err(TauriFunctionError::new(
                "Stop recording before inserting actions",
            ));
        }
        let owner = RecordingOwner::new(identity, app_id, board_id);
        let recorded = self.completed_actions.get(&owner).ok_or_else(|| TauriFunctionError::new(
            "No recording belongs to this profile, application and board. Reopen its original context to recover it.",
        ))?;
        if serde_json::to_value(actions)? != serde_json::to_value(recorded)? {
            return Err(TauriFunctionError::new(
                "The recorded actions changed. Refresh the recording before inserting.",
            ));
        }
        Ok(())
    }

    pub fn remap_profile_identity(
        &mut self,
        old: &str,
        new: &str,
    ) -> Result<(), TauriFunctionError> {
        if old == new {
            return Ok(());
        }
        let owners: Vec<_> = self
            .completed_actions
            .keys()
            .filter(|owner| owner.profile_identity == old)
            .cloned()
            .collect();
        for owner in &owners {
            let mut destination = owner.clone();
            destination.profile_identity = new.to_string();
            if self.completed_actions.contains_key(&destination) {
                return Err(TauriFunctionError::new(
                    "Both profile IDs have a pending recording for the same board. Insert or clear those recordings before syncing the profile ID.",
                ));
            }
        }
        for owner in owners {
            if let Some(actions) = self.completed_actions.remove(&owner) {
                let mut destination = owner;
                destination.profile_identity = new.to_string();
                self.completed_actions.insert(destination, actions);
            }
        }
        if let Some(owner) = &mut self.owner
            && owner.profile_identity == old
        {
            owner.profile_identity = new.to_string();
        }
        Ok(())
    }

    pub async fn start_session(
        &mut self,
        profile_identity: String,
        app_id: Option<String>,
        board_id: Option<String>,
        settings: RecordingSettings,
    ) -> Result<String, TauriFunctionError> {
        if self.status != RecordingStatus::Idle {
            return Err(TauriFunctionError::new("Recording already in progress"));
        }

        let owner = RecordingOwner::new(&profile_identity, app_id.as_deref(), board_id.as_deref());
        if self
            .completed_actions
            .get(&owner)
            .is_some_and(|actions| !actions.is_empty())
        {
            return Err(TauriFunctionError::new(
                "This board has a pending recording. Insert or clear it before starting another recording.",
            ));
        }
        let session_id = flow_like_types::create_id();
        let mut session = RecordingSession::new(&session_id, settings);
        session.app_id = app_id;
        session.target_board_id = board_id;
        session.started_at = Some(Utc::now());
        session.status = RecordingStatus::Recording;

        self.owner = Some(owner);
        self.session = Some(session);
        self.status = RecordingStatus::Recording;
        self.keystroke_buffer.clear();
        self.last_keystroke_time = None;
        self.first_keystroke_time = None;

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

        let actions = self
            .session
            .take()
            .map(|session| session.actions)
            .unwrap_or_default();
        if !actions.is_empty()
            && let Some(owner) = self.owner.clone()
        {
            self.completed_actions.insert(owner, actions.clone());
        }
        Ok(actions)
    }

    pub fn add_action(&mut self, action: RecordedAction) {
        if let Some(session) = &mut self.session {
            if let ActionType::Browser { action: incoming } = &action.action_type
                && incoming.kind == BrowserActionKind::DoubleClick
                && session.actions.last().is_some_and(|last| matches!(&last.action_type, ActionType::Browser { action: previous }
                    if previous.kind == BrowserActionKind::Click && previous.selector == incoming.selector
                    && previous.tab_id == incoming.tab_id && previous.frames == incoming.frames
                    && previous.modifiers == incoming.modifiers && previous.button == incoming.button)) {
                session.actions.pop();
            }
            if let ActionType::Browser { action: incoming } = &action.action_type
                && incoming.kind == BrowserActionKind::Type
                && let Some(last) = session.actions.last_mut()
                && let ActionType::Browser { action: previous } = &last.action_type
                && previous.kind == BrowserActionKind::Type
                && previous.selector == incoming.selector
                && previous.tab_id == incoming.tab_id
                && previous.url == incoming.url
                && previous.frames == incoming.frames
            {
                *last = action;
                return;
            }
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
                if last_dir == new_dir
                    && (0..200).contains(&time_diff)
                    && last_action.coordinates == action.coordinates
                {
                    *last_amount = last_amount.saturating_add(*new_amount);
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

    pub fn buffer_keystroke_at(&mut self, ch: char, timestamp: DateTime<Utc>) {
        if let Some(session) = &self.session
            && !session.settings.aggregate_keystrokes
        {
            let mut action = RecordedAction::new(
                flow_like_types::create_id(),
                ActionType::KeyType {
                    text: ch.to_string(),
                },
            );
            action.timestamp = timestamp;
            if let Some(session) = &mut self.session {
                session.actions.push(action);
            }
            return;
        }

        if self.keystroke_buffer.is_empty() {
            self.first_keystroke_time = Some(timestamp);
        }
        self.keystroke_buffer.push(ch);
        self.last_keystroke_time = Some(timestamp);
    }

    pub fn flush_keystroke_buffer(&mut self) -> Option<RecordedAction> {
        if self.keystroke_buffer.is_empty() {
            return None;
        }

        let text = std::mem::take(&mut self.keystroke_buffer);
        let mut action =
            RecordedAction::new(flow_like_types::create_id(), ActionType::KeyType { text });
        if let Some(timestamp) = self.first_keystroke_time.take() {
            action.timestamp = timestamp;
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    async fn started() -> RecordingStateInner {
        let mut state = RecordingStateInner::default();
        state
            .start_session(
                "profile-and-hub".into(),
                Some("app".into()),
                Some("board".into()),
                RecordingSettings::default(),
            )
            .await
            .unwrap();
        state
    }

    #[tokio::test]
    async fn recovery_is_scoped_to_profile_hub_application_and_board() {
        let mut state = started().await;
        state.add_action(
            RecordedAction::new(
                "click",
                ActionType::Click {
                    button: MouseButton::Left,
                    modifiers: vec![],
                },
            )
            .with_coordinates(10, 20)
            .with_screenshot_ref("original-board-template"),
        );
        let actions = state.stop().await.unwrap();
        for (identity, app, board) in [
            ("profile-and-hub", "other-app", "board"),
            ("profile-and-hub", "app", "other-board"),
            ("another-profile-same-hub", "app", "board"),
            ("same-profile-another-hub", "app", "board"),
        ] {
            assert!(
                state
                    .actions_for_context(identity, Some(app), Some(board))
                    .is_empty()
            );
            assert!(
                state
                    .validate_completed_actions(identity, Some(app), Some(board), &actions)
                    .is_err()
            );
            state.clear_completed(identity, Some(app), Some(board));
        }
        assert_eq!(
            state.actions_for_context("profile-and-hub", Some("app"), Some("board"))[0]
                .screenshot_ref
                .as_deref(),
            Some("original-board-template")
        );
        assert!(
            state
                .validate_completed_actions("profile-and-hub", Some("app"), Some("board"), &actions)
                .is_ok()
        );
        state.clear_completed("profile-and-hub", Some("app"), Some("board"));
        assert!(
            state
                .validate_completed_actions("profile-and-hub", Some("app"), Some("board"), &actions)
                .is_err()
        );
    }

    #[tokio::test]
    async fn recording_another_context_preserves_prior_recovery_and_requires_explicit_clear_on_restart()
     {
        let mut state = started().await;
        state.buffer_keystroke_at('A', Utc::now());
        let first = state.stop().await.unwrap();
        assert!(
            state
                .start_session(
                    "profile-and-hub".into(),
                    Some("app".into()),
                    Some("board".into()),
                    RecordingSettings::default()
                )
                .await
                .is_err()
        );
        for (identity, board) in [("profile-and-hub", "board-b"), ("another-profile", "board")] {
            state
                .start_session(
                    identity.into(),
                    Some("app".into()),
                    Some(board.into()),
                    RecordingSettings::default(),
                )
                .await
                .unwrap();
            state.buffer_keystroke_at('B', Utc::now());
            let second = state.stop().await.unwrap();
            assert!(
                state
                    .validate_completed_actions(identity, Some("app"), Some(board), &second)
                    .is_ok()
            );
            assert!(
                state
                    .validate_completed_actions(
                        "profile-and-hub",
                        Some("app"),
                        Some("board"),
                        &first
                    )
                    .is_ok()
            );
            state.clear_completed(identity, Some("app"), Some(board));
        }
        assert_eq!(
            state
                .actions_for_context("profile-and-hub", Some("app"), Some("board"))
                .len(),
            1
        );
        state.clear_completed("profile-and-hub", Some("app"), Some("board"));
        state
            .start_session(
                "profile-and-hub".into(),
                Some("app".into()),
                Some("board".into()),
                RecordingSettings::default(),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn trusted_profile_id_remap_migrates_recovery_without_changing_artifacts() {
        let mut state = started().await;
        state.add_action(
            RecordedAction::new(
                "click",
                ActionType::Click {
                    button: MouseButton::Left,
                    modifiers: vec![],
                },
            )
            .with_coordinates(10, 20)
            .with_screenshot_ref("same-hub-template"),
        );
        let actions = state.stop().await.unwrap();
        state
            .remap_profile_identity("profile-and-hub", "server-profile-and-same-hub")
            .unwrap();
        assert!(
            state
                .actions_for_context("profile-and-hub", Some("app"), Some("board"))
                .is_empty()
        );
        let recovered =
            state.actions_for_context("server-profile-and-same-hub", Some("app"), Some("board"));
        assert_eq!(
            recovered[0].screenshot_ref.as_deref(),
            Some("same-hub-template")
        );
        assert!(
            state
                .validate_completed_actions(
                    "server-profile-and-same-hub",
                    Some("app"),
                    Some("board"),
                    &actions
                )
                .is_ok()
        );
        state
            .start_session(
                "profile-and-hub".into(),
                Some("app".into()),
                Some("board".into()),
                RecordingSettings::default(),
            )
            .await
            .unwrap();
        state.buffer_keystroke_at('B', Utc::now());
        let other = state.stop().await.unwrap();
        assert!(
            state
                .remap_profile_identity("profile-and-hub", "server-profile-and-same-hub")
                .is_err()
        );
        assert!(
            state
                .validate_completed_actions("profile-and-hub", Some("app"), Some("board"), &other)
                .is_ok()
        );
        assert!(
            state
                .validate_completed_actions(
                    "server-profile-and-same-hub",
                    Some("app"),
                    Some("board"),
                    &actions
                )
                .is_ok()
        );
    }

    #[tokio::test]
    async fn stop_retains_completed_actions_and_original_typing_time() {
        let mut state = started().await;
        let typed_at = Utc::now() - chrono::Duration::seconds(2);
        state.buffer_keystroke_at('é', typed_at);
        state.buffer_keystroke_at('文', typed_at + chrono::Duration::milliseconds(30));
        let actions = state.stop().await.unwrap();
        assert_eq!(state.status, RecordingStatus::Idle);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].timestamp, typed_at);
        assert!(matches!(&actions[0].action_type, ActionType::KeyType { text } if text == "é文"));
        assert_eq!(
            state
                .actions_for_context("profile-and-hub", Some("app"), Some("board"))
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn scroll_consolidation_preserves_separate_hover_targets() {
        let mut state = started().await;
        let mut action = RecordedAction::new(
            "one",
            ActionType::Scroll {
                direction: ScrollDirection::Down,
                amount: 2,
            },
        )
        .with_coordinates(10, 20);
        state.add_action(action.clone());
        action.timestamp += chrono::Duration::milliseconds(20);
        state.add_action(action.clone());
        action.coordinates = Some((400, 20));
        state.add_action(action);
        let actions = &state.session.as_ref().unwrap().actions;
        assert_eq!(actions.len(), 2);
        assert!(matches!(
            actions[0].action_type,
            ActionType::Scroll { amount: 4, .. }
        ));
        assert_eq!(actions[1].coordinates, Some((400, 20)));
    }

    #[test]
    fn legacy_recordings_gain_default_buttons_and_modifiers() {
        let drag: ActionType =
            serde_json::from_value(serde_json::json!({"Drag":{"start":[0,0],"end":[10,20]}}))
                .unwrap();
        assert!(
            matches!(drag, ActionType::Drag { button: MouseButton::Left, modifiers, .. } if modifiers.is_empty())
        );
        let double: ActionType =
            serde_json::from_value(serde_json::json!({"DoubleClick":{"button":"Right"}})).unwrap();
        assert!(
            matches!(double, ActionType::DoubleClick { button: MouseButton::Right, modifiers } if modifiers.is_empty())
        );
    }

    #[tokio::test]
    async fn browser_typing_coalesces_only_inside_the_same_tab_and_frame() {
        let mut state = started().await;
        for (id, tab, value) in [
            ("one", "tab-a", "Z"),
            ("two", "tab-a", "Zoë"),
            ("three", "tab-b", "文"),
        ] {
            let action = serde_json::from_value(serde_json::json!({"kind": "type", "tab_id": tab, "selector": "#name", "url": "https://example.test/form", "value": value})).unwrap();
            state.add_action(RecordedAction::new(id, ActionType::Browser { action }));
        }
        let actions = state.stop().await.unwrap();
        assert_eq!(actions.len(), 2);
        assert!(
            matches!(&actions[0].action_type, ActionType::Browser { action } if action.value == "Zoë" && action.tab_id == "tab-a")
        );
        assert!(
            matches!(&actions[1].action_type, ActionType::Browser { action } if action.value == "文" && action.tab_id == "tab-b")
        );
    }

    #[tokio::test]
    async fn browser_double_click_replaces_its_first_click_only_on_the_same_target() {
        let mut state = started().await;
        for (id, kind) in [("click", "click"), ("double", "double_click")] {
            let action = serde_json::from_value(serde_json::json!({"kind": kind, "tab_id": "tab", "selector": "#target", "url": "https://example.test", "button": "Middle", "modifiers": ["Control"]})).unwrap();
            state.add_action(RecordedAction::new(id, ActionType::Browser { action }));
        }
        let actions = state.stop().await.unwrap();
        assert_eq!(actions.len(), 1);
        assert!(
            matches!(&actions[0].action_type, ActionType::Browser { action } if action.kind == BrowserActionKind::DoubleClick && action.button == MouseButton::Middle && action.modifiers == [KeyModifier::Control])
        );
    }
}

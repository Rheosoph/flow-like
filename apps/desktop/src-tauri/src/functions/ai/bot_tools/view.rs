//! What the bot panel's Computer tab shows: the latest picture of the bot's browser or screen,
//! and the Stop button. Pictures are pulled by the panel; there is no video. The machines
//! themselves live in `flow-like-bot-machine`.

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
use std::sync::OnceLock;
use std::time::Duration;

use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Serialize;

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
use flow_like_bot_machine::{Frame, FrameKind, MachineHost};

const ACTIVE_FOR: Duration = Duration::from_secs(180);

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
static HOST: OnceLock<MachineHost> = OnceLock::new();

/// The in-process machine host of this app, created on first use.
#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
pub(crate) fn machine_host(app: &tauri::AppHandle) -> MachineHost {
    use tauri::Manager;
    HOST.get_or_init(|| {
        let data_dir = app
            .path()
            .app_local_data_dir()
            .unwrap_or_else(|_| std::env::temp_dir().join("flow-like"));
        MachineHost::new(data_dir, flow_like_bot_machine::Mode::InProcess)
    })
    .clone()
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ViewState {
    Asleep,
    Active,
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    Browser,
    Computer,
}

#[derive(Serialize, Debug)]
pub struct ComputerView {
    state: ViewState,
    kind: Option<ViewKind>,
    caption: Option<String>,
    /// A `data:` URL of the latest picture.
    image: Option<String>,
    age_ms: Option<u64>,
    taken_over: bool,
}

impl ComputerView {
    fn asleep(taken_over: bool) -> Self {
        Self {
            state: ViewState::Asleep,
            kind: None,
            caption: None,
            image: None,
            age_ms: None,
            taken_over,
        }
    }

    fn active(
        kind: ViewKind,
        caption: String,
        jpeg: &[u8],
        age: Duration,
        taken_over: bool,
    ) -> Self {
        if age > ACTIVE_FOR {
            return Self::asleep(taken_over);
        }
        Self {
            state: ViewState::Active,
            kind: Some(kind),
            caption: Some(caption),
            image: Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg))),
            age_ms: Some(age.as_millis() as u64),
            taken_over,
        }
    }
}

#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
fn view_of(frame: Option<Frame>, taken_over: bool) -> ComputerView {
    let Some(frame) = frame else {
        return ComputerView::asleep(taken_over);
    };
    let kind = match frame.kind {
        FrameKind::Browser => ViewKind::Browser,
        FrameKind::Computer => ViewKind::Computer,
    };
    ComputerView::active(kind, frame.caption.clone(), &frame.jpeg, frame.at.elapsed(), taken_over)
}

/// The latest picture of the bot's browser or screen, or "asleep" when it has done nothing for
/// a while or nothing is running.
#[tauri::command(async)]
pub async fn bot_computer_view(bot_id: String) -> Result<ComputerView, String> {
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    if let Some(host) = HOST.get() {
        let frame = host.refresh_frame(&bot_id).await;
        return Ok(view_of(frame, host.is_taken_over(&bot_id)));
    }
    let _ = bot_id;
    Ok(ComputerView::asleep(false))
}

/// Ends the bot's browser sessions and any mouse or keyboard action in flight.
#[tauri::command(async)]
pub async fn bot_computer_stop(bot_id: String) -> Result<(), String> {
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    if let Some(host) = HOST.get() {
        host.stop(&bot_id).await;
    }
    let _ = bot_id;
    Ok(())
}

/// Pauses the bot's browser and computer calls while the owner uses the computer; the bot gets
/// a `taken_over` error until `bot_computer_hand_back`.
#[tauri::command(async)]
pub async fn bot_computer_take_over(bot_id: String) -> Result<(), String> {
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    if let Some(host) = HOST.get() {
        return host.take_over(&bot_id).map_err(|e| format!("{e:#}"));
    }
    let _ = bot_id;
    Ok(())
}

/// Lets the bot go on; its next call is told the owner used the computer.
#[tauri::command(async)]
pub async fn bot_computer_hand_back(bot_id: String) -> Result<(), String> {
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    if let Some(host) = HOST.get() {
        return host.hand_back(&bot_id).map_err(|e| format!("{e:#}"));
    }
    let _ = bot_id;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bot_without_a_frame_is_asleep() {
        let view = ComputerView::asleep(false);
        assert_eq!(view.state, ViewState::Asleep);
        assert!(view.image.is_none());
    }

    #[test]
    fn a_fresh_frame_is_active_and_an_old_one_asleep() {
        let active = ComputerView::active(
            ViewKind::Browser,
            "https://shop.example/".into(),
            &[0xFF, 0xD8],
            Duration::from_secs(1),
            false,
        );
        assert_eq!(active.state, ViewState::Active);
        assert_eq!(active.kind, Some(ViewKind::Browser));
        assert!(active.image.as_deref().unwrap().starts_with("data:image/jpeg;base64,"));
        let old = ComputerView::active(
            ViewKind::Computer,
            String::new(),
            &[],
            ACTIVE_FOR + Duration::from_secs(1),
            true,
        );
        assert_eq!(old.state, ViewState::Asleep);
        assert!(old.taken_over);
    }
}

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

#[cfg(desktop)]
use flow_like_catalog::automation_capabilities::capability_status;
#[cfg(desktop)]
pub use flow_like_catalog::automation_capabilities::{
    AutomationCapability, CapabilityState, CapabilityStatus,
};
#[cfg(mobile)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationCapability {
    Browser,
    Clipboard,
    ApplicationLaunch,
    InputControl,
    InputMonitoring,
    ScreenCapture,
    Accessibility,
    WindowManagement,
}

use crate::functions::TauriFunctionError;

#[cfg(mobile)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Unsupported,
}
#[cfg(mobile)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityStatus {
    pub capability: AutomationCapability,
    pub state: CapabilityState,
    pub detail: String,
    pub can_request: bool,
}
#[cfg(mobile)]
impl CapabilityStatus {
    pub fn available(&self) -> bool {
        false
    }
}
#[cfg(mobile)]
fn capability_status(capability: AutomationCapability) -> CapabilityStatus {
    CapabilityStatus {
        capability,
        state: CapabilityState::Unsupported,
        detail: "Desktop automation is not supported on this platform".into(),
        can_request: false,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionStatus {
    // Kept for existing clients; capabilities is the authoritative per-resource result.
    pub accessibility: bool,
    pub screen_recording: bool,
    pub input_monitoring: bool,
    pub executable_path: Option<String>,
    pub platform: String,
    pub capabilities: Vec<CapabilityStatus>,
    pub required: Vec<AutomationCapability>,
    pub all_granted: bool,
}

fn current_executable_path() -> Option<String> {
    std::env::current_exe()
        .ok()
        .map(|path| path.display().to_string())
}

pub fn permission_status(required: Vec<AutomationCapability>) -> PermissionStatus {
    use AutomationCapability::*;
    let capabilities: Vec<_> = [
        Browser,
        Clipboard,
        ApplicationLaunch,
        InputControl,
        InputMonitoring,
        ScreenCapture,
        Accessibility,
        WindowManagement,
    ]
    .into_iter()
    .map(capability_status)
    .collect();
    let available = |capability| {
        capabilities
            .iter()
            .any(|status| status.capability == capability && status.available())
    };
    PermissionStatus {
        accessibility: available(Accessibility),
        screen_recording: available(ScreenCapture),
        input_monitoring: available(InputMonitoring),
        executable_path: current_executable_path(),
        platform: std::env::consts::OS.into(),
        all_granted: required.iter().all(|capability| available(*capability)),
        capabilities,
        required,
    }
}

pub fn ensure_capabilities(required: Vec<AutomationCapability>) -> Result<(), TauriFunctionError> {
    let status = permission_status(required);
    if status.all_granted {
        return Ok(());
    }
    let missing: Vec<_> = status
        .capabilities
        .iter()
        .filter(|entry| status.required.contains(&entry.capability) && !entry.available())
        .map(|entry| format!("{:?}: {}", entry.capability, entry.detail))
        .collect();
    Err(TauriFunctionError::new(&format!(
        "Automation capabilities unavailable: {}",
        missing.join("; ")
    )))
}

pub async fn ensure_recording_permissions(
    _handler: &AppHandle,
    capture_screenshots: bool,
    capture_fingerprints: bool,
) -> Result<(), TauriFunctionError> {
    let mut required = vec![
        AutomationCapability::InputMonitoring,
        AutomationCapability::WindowManagement,
    ];
    if capture_screenshots {
        required.push(AutomationCapability::ScreenCapture);
    }
    if capture_fingerprints {
        required.push(AutomationCapability::Accessibility);
    }
    ensure_capabilities(required)
}

#[tauri::command(async)]
pub async fn check_rpa_permissions(
    _handler: AppHandle,
    required: Option<Vec<AutomationCapability>>,
) -> Result<PermissionStatus, TauriFunctionError> {
    Ok(permission_status(required.unwrap_or_else(|| {
        vec![
            AutomationCapability::InputControl,
            AutomationCapability::ScreenCapture,
        ]
    })))
}

#[tauri::command(async)]
pub async fn request_rpa_permission(
    handler: AppHandle,
    permission_type: String,
) -> Result<bool, TauriFunctionError> {
    let _ = handler;
    let capability = match permission_type.as_str() {
        "screen_recording" => AutomationCapability::ScreenCapture,
        _ => serde_json::from_value(serde_json::Value::String(permission_type))?,
    };
    #[cfg(desktop)]
    {
        Ok(flow_like_catalog::automation_capabilities::request_capability(capability).await?)
    }
    #[cfg(mobile)]
    {
        let _ = capability;
        Err(TauriFunctionError::new(
            "Desktop automation is not supported on this platform",
        ))
    }
}

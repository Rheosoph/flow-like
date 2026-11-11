use std::fmt;
#[cfg(target_os = "macos")]
use std::sync::OnceLock;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicU8, Ordering};

use crate::capabilities::AutomationCapability;
#[cfg(target_os = "macos")]
use crate::capabilities::{CapabilityState, capability_status, request_capability};

/// A system permission the computer needs. `permission` is `screen_recording` or
/// `accessibility`; the host turns this into its typed tool error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingPermission {
    pub permission: &'static str,
    pub restart: bool,
    pub message: String,
}

impl fmt::Display for MissingPermission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for MissingPermission {}

fn pane(capability: AutomationCapability) -> (&'static str, &'static str) {
    match capability {
        AutomationCapability::ScreenCapture => ("screen_recording", "Screen & System Audio Recording"),
        _ => ("accessibility", "Accessibility"),
    }
}

fn denied(capability: AutomationCapability) -> MissingPermission {
    let (permission, label) = pane(capability);
    MissingPermission {
        permission,
        restart: false,
        message: format!(
            "Flow Like has no {label} permission, so the computer {}. Ask the owner to enable Flow Like under System Settings > Privacy & Security > {label}, then quit and reopen Flow Like.",
            if permission == "accessibility" {
                "cannot be controlled or its windows listed"
            } else {
                "can only be captured as wallpaper without other apps' windows"
            }
        ),
    }
}

pub fn restart_required(capability: AutomationCapability) -> MissingPermission {
    let (permission, label) = pane(capability);
    MissingPermission {
        permission,
        restart: true,
        message: format!(
            "The {label} permission was granted after Flow Like started and macOS applies it only to a new process. Ask the owner to quit and reopen Flow Like, then try again."
        ),
    }
}

#[cfg(target_os = "macos")]
static AT_START: OnceLock<(bool, bool)> = OnceLock::new();
#[cfg(target_os = "macos")]
static REQUESTED: AtomicU8 = AtomicU8::new(0);

#[cfg(target_os = "macos")]
fn granted(capability: AutomationCapability) -> bool {
    capability_status(capability).state == CapabilityState::Granted
}

/// Remembers which permissions this process started with; macOS applies a later grant only
/// to a new process.
pub fn note_start() {
    #[cfg(target_os = "macos")]
    AT_START.get_or_init(|| {
        (
            granted(AutomationCapability::ScreenCapture),
            granted(AutomationCapability::Accessibility),
        )
    });
}

#[cfg(target_os = "macos")]
fn started_with(capability: AutomationCapability) -> bool {
    note_start();
    let (screen, accessibility) = AT_START.get().copied().unwrap_or((true, true));
    match capability {
        AutomationCapability::ScreenCapture => screen,
        _ => accessibility,
    }
}

/// Fails with [`MissingPermission`] when macOS would hand back wallpaper or refuse input.
/// The first denial of each permission also opens the system prompt.
pub async fn require(capability: AutomationCapability) -> flow_like_types::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let bit = if capability == AutomationCapability::ScreenCapture { 1 } else { 2 };
        if !granted(capability) {
            if REQUESTED.fetch_or(bit, Ordering::AcqRel) & bit == 0 {
                let _ = request_capability(capability).await;
            }
            return Err(denied(capability).into());
        }
        if capability == AutomationCapability::ScreenCapture && !started_with(capability) {
            return Err(restart_required(capability).into());
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = capability;
    Ok(())
}

/// An input failure while Accessibility is granted but was not at startup is the restart case.
pub fn explain_input_failure(error: flow_like_types::Error) -> flow_like_types::Error {
    #[cfg(target_os = "macos")]
    {
        if error.downcast_ref::<MissingPermission>().is_some() {
            return error;
        }
        let capability = AutomationCapability::Accessibility;
        if !granted(capability) {
            return denied(capability).into();
        }
        if !started_with(capability) {
            return restart_required(capability).into();
        }
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_the_permission_and_the_restart() {
        let denied = denied(AutomationCapability::ScreenCapture);
        assert_eq!(denied.permission, "screen_recording");
        assert!(denied.message.contains("System Settings"));
        let restart = restart_required(AutomationCapability::Accessibility);
        assert!(restart.restart && restart.message.contains("quit and reopen"));
        assert_eq!(restart.permission, "accessibility");
    }
}

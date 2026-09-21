use serde::{Deserialize, Serialize};

/// Native resources a workflow uses. Browser control does not imply desktop access.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
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

pub fn required_capabilities(name: &str) -> Vec<AutomationCapability> {
    use AutomationCapability::*;
    if name == "browser_start_driver" {
        return vec![Browser, ApplicationLaunch];
    }
    if name.starts_with("browser_") || name == "fingerprint_match" {
        return vec![Browser];
    }
    if name == "computer_launch_app" {
        return vec![ApplicationLaunch];
    }
    if name == "computer_wait"
        || name == "vision_get_screen_size"
        || name.contains("display") && name.starts_with("computer_")
    {
        return vec![];
    }
    if name == "computer_capture_window" {
        return vec![ScreenCapture];
    }
    if name.starts_with("computer_clipboard_") {
        return vec![Clipboard];
    }
    if name.contains("accessibility") && name.starts_with("computer_") {
        return vec![Accessibility];
    }
    if name.starts_with("computer_mouse_") || name == "computer_natural_mouse_move" {
        // Mouse nodes can resolve screen templates and accessibility fingerprints.
        return vec![InputControl, ScreenCapture];
    }
    if name.starts_with("computer_key_") || name == "computer_scroll" {
        return vec![InputControl];
    }
    if name.starts_with("vision_") {
        return if name.contains("click") {
            vec![ScreenCapture, InputControl]
        } else {
            vec![ScreenCapture]
        };
    }
    if name.starts_with("computer_") {
        return if name.contains("screenshot") || name.contains("display") {
            vec![ScreenCapture]
        } else {
            vec![WindowManagement]
        };
    }
    match name {
        "rpa_click_at_position" | "rpa_type_text" | "rpa_drag_drop" | "rpa_scroll" => {
            vec![InputControl]
        }
        "rpa_take_snapshot"
        | "rpa_diagnose_failure"
        | "rpa_assert_template_exists"
        | "rpa_assert_color"
        | "rpa_wait_for_template"
        | "rpa_wait_for_color"
        | "rpa_locate_template"
        | "rpa_locate_color" => vec![ScreenCapture],
        // Plans choose their action kinds at runtime.
        "rpa_execute_actions" | "llm_execute_actions" | "automation_execute_plan" => vec![
            Browser,
            InputControl,
            ScreenCapture,
            Accessibility,
            WindowManagement,
        ],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_and_control_flow_do_not_request_desktop_permissions() {
        assert_eq!(
            required_capabilities("browser_open"),
            vec![AutomationCapability::Browser]
        );
        assert!(required_capabilities("automation_start_session").is_empty());
        assert!(required_capabilities("rpa_retry_loop").is_empty());
        assert!(required_capabilities("vision_get_screen_size").is_empty());
    }

    #[test]
    fn computer_actions_require_their_native_resources() {
        assert_eq!(
            required_capabilities("computer_key_type"),
            vec![AutomationCapability::InputControl]
        );
        assert_eq!(
            required_capabilities("computer_get_accessibility_tree"),
            vec![AutomationCapability::Accessibility]
        );
        assert!(
            required_capabilities("vision_click_template")
                .contains(&AutomationCapability::InputControl)
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    Granted,
    NotRequired,
    Denied,
    Unavailable,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CapabilityStatus {
    pub capability: AutomationCapability,
    pub state: CapabilityState,
    pub detail: String,
    pub can_request: bool,
}

impl CapabilityStatus {
    pub fn available(&self) -> bool {
        matches!(
            self.state,
            CapabilityState::Granted | CapabilityState::NotRequired
        )
    }
}

#[cfg(all(feature = "execute", target_os = "macos"))]
mod macos {
    use std::ffi::c_void;
    use std::ptr;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
        fn CGPreflightListenEventAccess() -> bool;
        fn CGRequestListenEventAccess() -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            allocator: *const c_void,
            c_str: *const i8,
            encoding: u32,
        ) -> *const c_void;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
        static kCFBooleanTrue: *const c_void;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
    }

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x08000100;

    pub fn check_accessibility() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    pub fn check_screen_recording() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub fn request_accessibility() -> bool {
        unsafe {
            let key_str = b"AXTrustedCheckOptionPrompt\0";
            let key = CFStringCreateWithCString(
                ptr::null(),
                key_str.as_ptr() as *const i8,
                K_CF_STRING_ENCODING_UTF8,
            );

            if key.is_null() {
                return AXIsProcessTrustedWithOptions(ptr::null());
            }

            let keys = [key];
            let values = [kCFBooleanTrue];

            let options = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &kCFTypeDictionaryKeyCallBacks as *const c_void,
                &kCFTypeDictionaryValueCallBacks as *const c_void,
            );

            let trusted = AXIsProcessTrustedWithOptions(options);

            if !options.is_null() {
                CFRelease(options);
            }
            CFRelease(key);

            trusted
        }
    }

    pub fn request_screen_recording() -> bool {
        unsafe {
            CGRequestScreenCaptureAccess();
            CGPreflightScreenCaptureAccess()
        }
    }

    pub fn check_input_monitoring() -> bool {
        unsafe { CGPreflightListenEventAccess() }
    }

    pub fn request_input_monitoring() -> bool {
        unsafe { CGRequestListenEventAccess() }
    }
}

#[cfg(feature = "execute")]
pub fn capability_status(capability: AutomationCapability) -> CapabilityStatus {
    use AutomationCapability::*;
    use CapabilityState::*;
    if matches!(capability, Browser | Clipboard | ApplicationLaunch) {
        return CapabilityStatus { capability, state: NotRequired, detail: "No system permission prompt is required. The operation still checks backend availability.".into(), can_request: false };
    }
    #[cfg(target_os = "macos")]
    {
        let granted = match capability {
            ScreenCapture => macos::check_screen_recording(),
            InputMonitoring => macos::check_input_monitoring(),
            _ => macos::check_accessibility(),
        };
        CapabilityStatus {
            capability,
            state: if granted { Granted } else { Denied },
            detail: if granted {
                "Access is granted to the current executable.".into()
            } else {
                "Grant access to this executable in System Settings. If access stays denied after enabling it, quit and reopen the app and check its signing identity.".into()
            },
            can_request: !granted,
        }
    }
    #[cfg(target_os = "windows")]
    {
        let available = xcap::Monitor::all().is_ok_and(|monitors| !monitors.is_empty());
        CapabilityStatus {
            capability,
            state: if available { NotRequired } else { Unavailable },
            detail: if available {
                "Available in the interactive desktop. Elevated applications and the secure desktop may reject input; actions verify their result.".into()
            } else {
                "No interactive desktop is available.".into()
            },
            can_request: false,
        }
    }
    #[cfg(target_os = "linux")]
    {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|value| value == "wayland");
        let has_x11 = std::env::var_os("DISPLAY").is_some();
        if wayland && capability == InputControl {
            let granted = crate::computer::native::input::input_capability_granted();
            return CapabilityStatus {
                capability,
                state: if granted { Granted } else { Denied },
                detail: if granted {
                    "The compositor input connection is authorized for this app session."
                } else {
                    "Grant input control and select displays in the compositor portal. This requires RemoteDesktop and ScreenCast support with display geometry."
                }.into(),
                can_request: !granted,
            };
        }
        let (state, detail) = if wayland && matches!(capability, InputMonitoring | WindowManagement)
        {
            (
                Unsupported,
                "This backend cannot observe global input or manage native Wayland windows. Browser automation remains available.",
            )
        } else if capability == Accessibility {
            let session_bus = std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()
                || std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|directory| {
                    std::path::PathBuf::from(directory).join("bus").exists()
                });
            if session_bus {
                (
                    NotRequired,
                    "AT-SPI access is checked for each target application. No separate desktop permission prompt is required.",
                )
            } else {
                (
                    Unavailable,
                    "The AT-SPI desktop session bus is unavailable.",
                )
            }
        } else if (wayland || has_x11)
            && xcap::Monitor::all().is_ok_and(|monitors| !monitors.is_empty())
        {
            if wayland && capability == ScreenCapture {
                (
                    NotRequired,
                    "The compositor authorizes individual captures through its screenshot backend. It may prompt for each capture; each operation verifies the result.",
                )
            } else {
                (
                    NotRequired,
                    "The X11 display is available. Each operation checks access to its target.",
                )
            }
        } else {
            (
                Unavailable,
                "No supported interactive display is available. Start the app in the desktop session.",
            )
        };
        CapabilityStatus {
            capability,
            state,
            detail: detail.into(),
            can_request: false,
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        CapabilityStatus {
            capability,
            state: Unsupported,
            detail: "Desktop automation is not supported on this platform.".into(),
            can_request: false,
        }
    }
}

#[cfg(feature = "execute")]
pub async fn request_capability(capability: AutomationCapability) -> flow_like_types::Result<bool> {
    #[cfg(target_os = "macos")]
    {
        use AutomationCapability::*;
        let (granted, pane) = match capability {
            Browser | Clipboard | ApplicationLaunch => return Ok(true),
            ScreenCapture => (macos::request_screen_recording(), "Privacy_ScreenCapture"),
            InputMonitoring => (macos::request_input_monitoring(), "Privacy_ListenEvent"),
            _ => (macos::request_accessibility(), "Privacy_Accessibility"),
        };
        if !granted {
            let status = std::process::Command::new("open")
                .arg(format!(
                    "x-apple.systempreferences:com.apple.preference.security?{pane}"
                ))
                .status()?;
            if !status.success() {
                flow_like_types::bail!("Could not open System Settings");
            }
        }
        Ok(granted)
    }
    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(target_os = "linux")]
        if capability == AutomationCapability::InputControl {
            crate::computer::native::input::request_input_capability().await?;
        }
        let status = capability_status(capability);
        if status.available() {
            return Ok(true);
        }
        Err(flow_like_types::anyhow!("{}", status.detail))
    }
}

fn capability_node(name: &str, title: &str, request: bool) -> flow_like::flow::node::Node {
    use flow_like::flow::{node::Node, pin::PinOptions, variable::VariableType};
    let mut node = Node::new(
        name,
        title,
        if request {
            "Requests operating-system access and returns the verified capability state"
        } else {
            "Checks whether a native automation capability is granted, unavailable, or unsupported"
        },
        "Automation",
    );
    node.add_icon("/flow/icons/automation.svg");
    node.set_only_offline(true);
    node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);
    node.add_input_pin(
        "capability",
        "Capability",
        "Native capability to inspect",
        VariableType::String,
    )
    .set_default_value(Some(flow_like_types::json::json!("input_control")))
    .set_options(
        PinOptions::new()
            .set_valid_values(
                vec![
                    "browser",
                    "clipboard",
                    "application_launch",
                    "input_control",
                    "input_monitoring",
                    "screen_capture",
                    "accessibility",
                    "window_management",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            )
            .build(),
    );
    node.add_output_pin(
        "status",
        "Status",
        "Capability state and recovery information",
        VariableType::Struct,
    )
    .set_schema::<CapabilityStatus>();
    node.add_output_pin(
        "available",
        "Available",
        "Whether access is currently available",
        VariableType::Boolean,
    );
    node.add_output_pin("exec_out", "▶", "Checked", VariableType::Execution);
    node
}

#[crate::register_node]
#[derive(Default)]
pub struct CheckCapabilityNode;
#[flow_like_types::async_trait]
impl flow_like::flow::node::NodeLogic for CheckCapabilityNode {
    fn get_node(&self) -> flow_like::flow::node::Node {
        let mut node = capability_node(
            "automation_check_capability",
            "Check Automation Capability",
            false,
        );
        node.set_flowscript_name("automation", "checkCapability");
        node
    }
    async fn run(
        &self,
        context: &mut flow_like::flow::execution::context::ExecutionContext,
    ) -> flow_like_types::Result<()> {
        run_capability(context, false).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RequestCapabilityNode;
#[flow_like_types::async_trait]
impl flow_like::flow::node::NodeLogic for RequestCapabilityNode {
    fn get_node(&self) -> flow_like::flow::node::Node {
        let mut node = capability_node(
            "automation_request_capability",
            "Request Automation Capability",
            true,
        );
        node.set_flowscript_name("automation", "requestCapability");
        node
    }
    async fn run(
        &self,
        context: &mut flow_like::flow::execution::context::ExecutionContext,
    ) -> flow_like_types::Result<()> {
        run_capability(context, true).await
    }
}

async fn run_capability(
    context: &mut flow_like::flow::execution::context::ExecutionContext,
    request: bool,
) -> flow_like_types::Result<()> {
    #[cfg(feature = "execute")]
    {
        context.deactivate_exec_pin("exec_out").await?;
        let name: String = context.evaluate_pin("capability").await?;
        let capability: AutomationCapability =
            flow_like_types::json::from_value(flow_like_types::json::json!(name))?;
        if request {
            request_capability(capability).await?;
        }
        let status = capability_status(capability);
        context
            .set_pin_value(
                "available",
                flow_like_types::json::json!(status.available()),
            )
            .await?;
        context
            .set_pin_value("status", flow_like_types::json::json!(status))
            .await?;
        context.activate_exec_pin("exec_out").await
    }
    #[cfg(not(feature = "execute"))]
    {
        let _ = (context, request);
        Err(flow_like_types::anyhow!(
            "Capability checks require desktop execution"
        ))
    }
}

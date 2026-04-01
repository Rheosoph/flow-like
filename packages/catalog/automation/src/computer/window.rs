use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WindowInfo {
    pub id: String,
    pub title: String,
    pub app_name: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_focused: bool,
    pub is_minimized: bool,
}

#[crate::register_node]
#[derive(Default)]
pub struct ListWindowsNode {}

impl ListWindowsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListWindowsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_list_windows",
            "List Windows",
            "Lists all visible windows on the desktop",
            "Automation/Computer/Window",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "windows",
            "Windows",
            "List of window information",
            VariableType::Generic,
        );

        node.add_output_pin("count", "Count", "Number of windows", VariableType::Integer);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use xcap::Window;

        context.deactivate_exec_pin("exec_out").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;

        let windows =
            Window::all().map_err(|e| flow_like_types::anyhow!("Failed to list windows: {}", e))?;

        let window_infos: Vec<WindowInfo> = windows
            .iter()
            .filter_map(|w| {
                let title = w.title().ok()?;
                if title.is_empty() {
                    return None;
                }
                Some(WindowInfo {
                    id: w.id().map(|id| id.to_string()).unwrap_or_default(),
                    title,
                    app_name: w.app_name().ok(),
                    x: w.x().unwrap_or(0),
                    y: w.y().unwrap_or(0),
                    width: w.width().unwrap_or(0),
                    height: w.height().unwrap_or(0),
                    is_focused: w.is_focused().unwrap_or(false),
                    is_minimized: w.is_minimized().unwrap_or(false),
                })
            })
            .collect();

        let count = window_infos.len() as i64;

        context
            .set_pin_value("windows", json!(window_infos))
            .await?;
        context.set_pin_value("count", json!(count)).await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Window management requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetActiveWindowNode {}

impl GetActiveWindowNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetActiveWindowNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_get_active_window",
            "Get Active Window",
            "Gets information about the currently focused window",
            "Automation/Computer/Window",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "exec_none",
            "No Window",
            "No active window found",
            VariableType::Execution,
        );

        node.add_output_pin(
            "window",
            "Window",
            "Active window information",
            VariableType::Struct,
        )
        .set_schema::<WindowInfo>();

        node.add_output_pin("title", "Title", "Window title", VariableType::String);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use xcap::Window;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_none").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;

        let windows =
            Window::all().map_err(|e| flow_like_types::anyhow!("Failed to list windows: {}", e))?;

        let active = windows.iter().find(|w| w.is_focused().unwrap_or(false));

        match active {
            Some(w) => {
                let info = WindowInfo {
                    id: w.id().map(|id| id.to_string()).unwrap_or_default(),
                    title: w.title().unwrap_or_default(),
                    app_name: w.app_name().ok(),
                    x: w.x().unwrap_or(0),
                    y: w.y().unwrap_or(0),
                    width: w.width().unwrap_or(0),
                    height: w.height().unwrap_or(0),
                    is_focused: true,
                    is_minimized: w.is_minimized().unwrap_or(false),
                };

                context.set_pin_value("window", json!(info.clone())).await?;
                context.set_pin_value("title", json!(info.title)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            None => {
                context.activate_exec_pin("exec_none").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Window management requires the 'execute' feature"
        ))
    }
}
#[crate::register_node]
#[derive(Default)]
pub struct FindWindowByTitleNode {}

impl FindWindowByTitleNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for FindWindowByTitleNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_find_window_by_title",
            "Find Window By Title",
            "Finds a window by its title (partial match supported)",
            "Automation/Computer/Window",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "title",
            "Title",
            "Window title to search for (partial match)",
            VariableType::String,
        );

        node.add_input_pin(
            "exact_match",
            "Exact Match",
            "Require exact title match",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "▶", "Found", VariableType::Execution);

        node.add_output_pin(
            "exec_not_found",
            "Not Found",
            "Window not found",
            VariableType::Execution,
        );

        node.add_output_pin(
            "window",
            "Window",
            "Found window information",
            VariableType::Struct,
        )
        .set_schema::<WindowInfo>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use xcap::Window;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_not_found").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        let search_title: String = context.evaluate_pin("title").await?;
        let exact_match: bool = context.evaluate_pin("exact_match").await.unwrap_or(false);

        let windows =
            Window::all().map_err(|e| flow_like_types::anyhow!("Failed to list windows: {}", e))?;

        let search_lower = search_title.to_lowercase();
        let found = windows.iter().find(|w| {
            let title = w.title().unwrap_or_default();
            if exact_match {
                title == search_title
            } else {
                title.to_lowercase().contains(&search_lower)
            }
        });

        match found {
            Some(w) => {
                let info = WindowInfo {
                    id: w.id().map(|id| id.to_string()).unwrap_or_default(),
                    title: w.title().unwrap_or_default(),
                    app_name: w.app_name().ok(),
                    x: w.x().unwrap_or(0),
                    y: w.y().unwrap_or(0),
                    width: w.width().unwrap_or(0),
                    height: w.height().unwrap_or(0),
                    is_focused: w.is_focused().unwrap_or(false),
                    is_minimized: w.is_minimized().unwrap_or(false),
                };

                context.set_pin_value("window", json!(info)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            None => {
                context.activate_exec_pin("exec_not_found").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Window management requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct LaunchAppNode {}

impl LaunchAppNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LaunchAppNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_launch_app",
            "Launch Application",
            "Launches an application by path or name",
            "Automation/Computer/Window",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(2)
                .set_security(2)
                .set_performance(6)
                .set_governance(4)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "path",
            "Path",
            "Application path or command",
            VariableType::String,
        );

        node.add_input_pin(
            "args",
            "Arguments",
            "Command line arguments (space-separated)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "wait_ms",
            "Wait (ms)",
            "Time to wait after launching (ms)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1000)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "exec_error",
            "Error",
            "Launch failed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "pid",
            "PID",
            "Process ID if available",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::process::Command;
        use tokio::time::{Duration, sleep};

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        let path: String = context.evaluate_pin("path").await?;
        let args: String = context.evaluate_pin("args").await.unwrap_or_default();
        let wait_ms: i64 = context.evaluate_pin("wait_ms").await.unwrap_or(1000);

        let mut cmd = if cfg!(target_os = "macos") {
            let mut c = Command::new("open");
            c.arg("-a").arg(&path);
            if !args.is_empty() {
                c.arg("--args");
                for arg in args.split_whitespace() {
                    c.arg(arg);
                }
            }
            c
        } else if cfg!(target_os = "windows") {
            let mut c = Command::new("cmd");
            c.args(["/C", "start", "", &path]);
            if !args.is_empty() {
                for arg in args.split_whitespace() {
                    c.arg(arg);
                }
            }
            c
        } else {
            let mut c = Command::new(&path);
            if !args.is_empty() {
                for arg in args.split_whitespace() {
                    c.arg(arg);
                }
            }
            c
        };

        match cmd.spawn() {
            Ok(child) => {
                let pid = child.id() as i64;
                context.set_pin_value("pid", json!(pid)).await?;

                if wait_ms > 0 {
                    sleep(Duration::from_millis(wait_ms.max(0) as u64)).await;
                }

                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context.set_pin_value("pid", json!(-1)).await?;
                context.log_message(
                    &format!("Failed to launch app: {}", e),
                    flow_like::flow::execution::LogLevel::Error,
                );
                context.activate_exec_pin("exec_error").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Window management requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CaptureWindowNode {}

impl CaptureWindowNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CaptureWindowNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_capture_window",
            "Capture Window",
            "Captures a screenshot of a specific window",
            "Automation/Computer/Window",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(6)
                .set_governance(4)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "window_id",
            "Window ID",
            "ID of the window to capture",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "exec_error",
            "Error",
            "Capture failed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "screenshot",
            "Screenshot",
            "Base64-encoded PNG image",
            VariableType::String,
        );

        node.add_output_pin(
            "image",
            "Image",
            "Screenshot as NodeImage",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use image::ImageEncoder;
        use image::codecs::png::PngEncoder;
        use xcap::Window;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        let window_id: String = context.evaluate_pin("window_id").await?;

        let windows =
            Window::all().map_err(|e| flow_like_types::anyhow!("Failed to list windows: {}", e))?;

        let target = windows
            .iter()
            .find(|w| w.id().map(|id| id.to_string()).unwrap_or_default() == window_id);

        match target {
            Some(window) => {
                let capture = window
                    .capture_image()
                    .map_err(|e| flow_like_types::anyhow!("Failed to capture window: {}", e))?;

                let mut png_data = Vec::new();
                let encoder = PngEncoder::new(&mut png_data);
                encoder
                    .write_image(
                        capture.as_raw(),
                        capture.width(),
                        capture.height(),
                        image::ExtendedColorType::Rgba8,
                    )
                    .map_err(|e| flow_like_types::anyhow!("Failed to encode PNG: {}", e))?;

                use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
                let base64_str = STANDARD.encode(&png_data);

                // Create NodeImage from the captured image
                let dyn_image = flow_like_types::image::DynamicImage::ImageRgba8(capture);
                let node_image = NodeImage::new(context, dyn_image).await;

                context
                    .set_pin_value("screenshot", json!(base64_str))
                    .await?;
                context.set_pin_value("image", json!(node_image)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            None => {
                context.set_pin_value("screenshot", json!("")).await?;
                context.set_pin_value("image", json!(null)).await?;
                context.activate_exec_pin("exec_error").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Window management requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FocusWindowNode {}

impl FocusWindowNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for FocusWindowNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_focus_window",
            "Focus Window",
            "Brings a window to the front and gives it focus",
            "Automation/Computer/Window",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "window_title",
            "Window Title",
            "Title or app name to search for (partial match on both title and app name)",
            VariableType::String,
        );

        node.add_input_pin(
            "exact_match",
            "Exact Match",
            "Require exact title match",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "launch_if_not_found",
            "Launch If Not Found",
            "Try to launch the application if no window is found",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "exec_not_found",
            "Not Found",
            "Window not found",
            VariableType::Execution,
        );

        node.add_output_pin(
            "window",
            "Window",
            "Focused window information",
            VariableType::Struct,
        )
        .set_schema::<WindowInfo>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use xcap::Window;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_not_found").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        let search_title: String = context.evaluate_pin("window_title").await?;
        let exact_match: bool = context.evaluate_pin("exact_match").await.unwrap_or(false);
        let launch_if_not_found: bool = context
            .evaluate_pin("launch_if_not_found")
            .await
            .unwrap_or(true);

        tracing::info!(
            "[FocusWindow] Searching for window: '{}' (exact_match: {})",
            search_title,
            exact_match
        );

        let windows =
            Window::all().map_err(|e| flow_like_types::anyhow!("Failed to list windows: {}", e))?;

        // Debug: list all windows
        tracing::debug!("[FocusWindow] Found {} windows:", windows.len());
        for w in &windows {
            let title = w.title().unwrap_or_default();
            let app = w.app_name().unwrap_or_default();
            tracing::debug!("[FocusWindow]   - App: '{}', Title: '{}'", app, title);
        }

        let search_lower = search_title.to_lowercase();

        // Search by both app name AND window title
        let found = windows.iter().find(|w| {
            let title = w.title().unwrap_or_default();
            let app_name = w.app_name().unwrap_or_default();

            if exact_match {
                title == search_title || app_name == search_title
            } else {
                let title_lower = title.to_lowercase();
                let app_lower = app_name.to_lowercase();

                // Check if search matches title or app name
                title_lower.contains(&search_lower) || app_lower.contains(&search_lower)
            }
        });

        // If not found, try fuzzy matching (words in any order)
        let found = found.or_else(|| {
            if exact_match {
                return None;
            }

            let search_words: Vec<&str> = search_lower.split_whitespace().collect();
            windows.iter().find(|w| {
                let title = w.title().unwrap_or_default().to_lowercase();
                let app_name = w.app_name().unwrap_or_default().to_lowercase();
                let combined = format!("{} {}", app_name, title);

                // All search words must be present somewhere
                search_words.iter().all(|word| combined.contains(word))
            })
        });

        match found {
            Some(w) => {
                let app_name = w.app_name().unwrap_or_default();
                let window_title = w.title().unwrap_or_default();

                tracing::info!(
                    "[FocusWindow] Found window: App='{}', Title='{}'",
                    app_name,
                    window_title
                );

                #[cfg(target_os = "macos")]
                {
                    let script = format!(
                        r#"tell application "{}" to activate
tell application "System Events"
    tell process "{}"
        set frontmost to true
    end tell
end tell"#,
                        app_name, app_name
                    );
                    let output = std::process::Command::new("osascript")
                        .arg("-e")
                        .arg(&script)
                        .output()
                        .map_err(|e| flow_like_types::anyhow!("Failed to focus window: {}", e))?;

                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        tracing::warn!("[FocusWindow] osascript stderr: {}", stderr);
                    }
                }

                #[cfg(target_os = "windows")]
                {
                    let script = format!(
                        r#"$wshell = New-Object -ComObject wscript.shell; $wshell.AppActivate("{}")"#,
                        window_title
                    );
                    std::process::Command::new("powershell")
                        .args(["-Command", &script])
                        .output()
                        .map_err(|e| flow_like_types::anyhow!("Failed to focus window: {}", e))?;
                }

                #[cfg(target_os = "linux")]
                {
                    let _ = std::process::Command::new("wmctrl")
                        .args(["-a", &window_title])
                        .output()
                        .or_else(|_| {
                            std::process::Command::new("xdotool")
                                .args(["search", "--name", &window_title, "windowactivate"])
                                .output()
                        })
                        .map_err(|e| flow_like_types::anyhow!("Failed to focus window: {}", e))?;
                }

                let info = WindowInfo {
                    id: w.id().map(|id| id.to_string()).unwrap_or_default(),
                    title: window_title,
                    app_name: Some(app_name),
                    x: w.x().unwrap_or(0),
                    y: w.y().unwrap_or(0),
                    width: w.width().unwrap_or(0),
                    height: w.height().unwrap_or(0),
                    is_focused: true,
                    is_minimized: w.is_minimized().unwrap_or(false),
                };
                context.set_pin_value("window", json!(info)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            None => {
                tracing::warn!("[FocusWindow] No window found for '{}'", search_title);

                // Try to launch the app if enabled
                if launch_if_not_found {
                    tracing::info!("[FocusWindow] Attempting to launch app: '{}'", search_title);

                    let launch_result = Self::try_launch_app(&search_title);

                    if launch_result.is_ok() {
                        // Wait for app to start
                        tracing::info!("[FocusWindow] Waiting for app to launch...");
                        flow_like_types::tokio::time::sleep(std::time::Duration::from_secs(2))
                            .await;

                        // Try to find the window again
                        let windows = Window::all().unwrap_or_default();
                        let search_lower = search_title.to_lowercase();

                        let found_after_launch = windows.iter().find(|w| {
                            let title = w.title().unwrap_or_default().to_lowercase();
                            let app_name = w.app_name().unwrap_or_default().to_lowercase();
                            title.contains(&search_lower) || app_name.contains(&search_lower)
                        });

                        if let Some(w) = found_after_launch {
                            let app_name = w.app_name().unwrap_or_default();
                            let window_title = w.title().unwrap_or_default();

                            tracing::info!(
                                "[FocusWindow] App launched, found window: '{}'",
                                window_title
                            );

                            // Focus it
                            #[cfg(target_os = "macos")]
                            {
                                let script =
                                    format!(r#"tell application "{}" to activate"#, app_name);
                                let _ = std::process::Command::new("osascript")
                                    .arg("-e")
                                    .arg(&script)
                                    .output();
                            }

                            let info = WindowInfo {
                                id: w.id().map(|id| id.to_string()).unwrap_or_default(),
                                title: window_title,
                                app_name: Some(app_name),
                                x: w.x().unwrap_or(0),
                                y: w.y().unwrap_or(0),
                                width: w.width().unwrap_or(0),
                                height: w.height().unwrap_or(0),
                                is_focused: true,
                                is_minimized: w.is_minimized().unwrap_or(false),
                            };
                            context.set_pin_value("window", json!(info)).await?;
                            context.activate_exec_pin("exec_out").await?;
                            return Ok(());
                        }
                    }
                }

                context.set_pin_value("window", json!(null)).await?;
                context.activate_exec_pin("exec_not_found").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Window management requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
impl FocusWindowNode {
    fn try_launch_app(app_name: &str) -> Result<(), std::io::Error> {
        let app_lower = app_name.to_lowercase();

        #[cfg(target_os = "macos")]
        {
            // Common app name mappings for macOS
            let app_bundle = if app_lower.contains("edge") {
                "Microsoft Edge"
            } else if app_lower.contains("chrome") {
                "Google Chrome"
            } else if app_lower.contains("firefox") {
                "Firefox"
            } else if app_lower.contains("safari") {
                "Safari"
            } else if app_lower.contains("code") || app_lower.contains("vscode") {
                "Visual Studio Code"
            } else if app_lower.contains("terminal") {
                "Terminal"
            } else if app_lower.contains("finder") {
                "Finder"
            } else {
                app_name
            };

            tracing::info!("[FocusWindow] Launching macOS app: '{}'", app_bundle);

            std::process::Command::new("open")
                .args(["-a", app_bundle])
                .spawn()?;
        }

        #[cfg(target_os = "windows")]
        {
            // Common app mappings for Windows
            let exe = if app_lower.contains("edge") {
                "msedge"
            } else if app_lower.contains("chrome") {
                "chrome"
            } else if app_lower.contains("firefox") {
                "firefox"
            } else if app_lower.contains("notepad") {
                "notepad"
            } else if app_lower.contains("explorer") {
                "explorer"
            } else {
                app_name
            };

            tracing::info!("[FocusWindow] Launching Windows app: '{}'", exe);

            std::process::Command::new("cmd")
                .args(["/C", "start", "", exe])
                .spawn()?;
        }

        #[cfg(target_os = "linux")]
        {
            // Try common Linux app launchers
            let result = std::process::Command::new("xdg-open")
                .arg(format!("{}.desktop", app_lower))
                .spawn()
                .or_else(|_| std::process::Command::new(&app_lower).spawn());

            if let Err(e) = result {
                tracing::warn!("[FocusWindow] Failed to launch on Linux: {}", e);
                return Err(e);
            }
        }

        Ok(())
    }
}

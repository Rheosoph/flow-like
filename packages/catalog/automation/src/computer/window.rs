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
        node.set_version(1);
        node.set_flowscript_name("computer", "listWindows");
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
            VariableType::Struct,
        )
        .set_schema::<WindowInfo>()
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin("count", "Count", "Number of windows", VariableType::Integer);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        _session.ensure_active(context).await?;

        let window_infos = super::native::windows_async().await?;

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
        node.set_version(1);
        node.set_flowscript_name("computer", "getActiveWindow");
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
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_none").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        _session.ensure_active(context).await?;

        let active = super::native::select_window_async("", "", "", true).await?;

        match active {
            Some(w) => {
                let info = super::native::window_info(&w);

                context.set_pin_value("window", json!(info.clone())).await?;
                context.set_pin_value("title", json!(info.title)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            None => {
                context.set_pin_value("window", json!(null)).await?;
                context.set_pin_value("title", json!("")).await?;
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
        node.set_version(1);
        node.set_flowscript_name("computer", "findWindowByTitle");
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
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_not_found").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        _session.ensure_active(context).await?;
        let search_title: String = context.evaluate_pin("title").await?;
        let exact_match: bool = context.evaluate_pin("exact_match").await.unwrap_or(false);

        let found = super::native::select_window_async("", &search_title, "", exact_match).await?;

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
        node.set_version(1);
        node.set_flowscript_name("computer", "launchApp");
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
            "arguments",
            "Argument List",
            "Arguments passed directly without shell evaluation",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_default_value(Some(json!([])));

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
        use std::time::Duration;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        _session.ensure_active(context).await?;
        let path: String = context.evaluate_pin("path").await?;
        let args: String = context.evaluate_pin("args").await.unwrap_or_default();
        let wait_ms: i64 = context.evaluate_pin("wait_ms").await.unwrap_or(1000);

        let arguments: Vec<String> = context.evaluate_pin("arguments").await.unwrap_or_default();
        let arguments = if arguments.is_empty() {
            parse_arguments(&args)?
        } else {
            arguments
        };
        match launch_application_async(path, arguments, context.get_cancellation_token()).await {
            Ok(pid) => {
                context.set_pin_value("pid", json!(pid)).await?;

                if wait_ms > 0 {
                    crate::rpa::branch::delay(
                        context,
                        Duration::from_millis(wait_ms.clamp(0, 60_000) as u64),
                    )
                    .await?;
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
        node.set_version(1);
        node.set_flowscript_name("computer", "captureWindow");
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
        _session.ensure_active(context).await?;
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
        node.set_version(1);
        node.set_flowscript_name("computer", "focusWindow");
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
            "window_id",
            "Window ID",
            "Native window ID; fails if that window no longer exists",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "process_name",
            "Application",
            "Exact application name to disambiguate the window",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

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
        .set_default_value(Some(json!(false)));

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
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_not_found").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let title: String = context.evaluate_pin("window_title").await?;
        let id: String = context.evaluate_pin("window_id").await.unwrap_or_default();
        let process: String = context
            .evaluate_pin("process_name")
            .await
            .unwrap_or_default();
        let exact: bool = context.evaluate_pin("exact_match").await.unwrap_or(false);
        let launch: bool = context
            .evaluate_pin("launch_if_not_found")
            .await
            .unwrap_or(false);
        let mut found = super::native::select_window_async(&id, &title, &process, exact).await?;
        if found.is_none() && launch {
            let application = if process.is_empty() { &title } else { &process };
            launch_application_async(
                application.to_owned(),
                vec![],
                context.get_cancellation_token(),
            )
            .await?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while found.is_none() && std::time::Instant::now() < deadline {
                crate::rpa::branch::delay(context, std::time::Duration::from_millis(100)).await?;
                session.ensure_active(context).await?;
                found = super::native::select_window_async("", &title, &process, exact).await?;
            }
        }
        if let Some(window) = found {
            context.check_cancelled()?;
            let info = super::native::focus_window(
                &window.id()?.to_string(),
                context.get_cancellation_token(),
            )
            .await?;
            context.set_pin_value("window", json!(info)).await?;
            context.activate_exec_pin("exec_out").await?;
        } else {
            context.set_pin_value("window", json!(null)).await?;
            context.activate_exec_pin("exec_not_found").await?;
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
async fn launch_application_async(
    path: String,
    arguments: Vec<String>,
    cancellation: Option<flow_like_types::tokio_util::sync::CancellationToken>,
) -> flow_like_types::Result<i64> {
    tokio::task::spawn_blocking(move || {
        if cancellation.as_ref().is_some_and(|t| t.is_cancelled()) {
            return Err(flow_like_types::anyhow!("Automation cancelled"));
        }
        launch_application(&path, &arguments)
    })
    .await?
}

/// Parses legacy arguments without executing a shell. New flows should use Argument List.
fn parse_arguments(text: &str) -> flow_like_types::Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut started = false;
    for c in text.chars() {
        if Some(c) == quote {
            quote = None;
        } else if quote.is_none() && (c == '\'' || c == '"') {
            quote = Some(c);
            started = true;
        } else if quote.is_none() && c.is_whitespace() {
            if started {
                args.push(std::mem::take(&mut current));
                started = false;
            }
        } else {
            current.push(c);
            started = true;
        }
    }
    if quote.is_some() {
        return Err(flow_like_types::anyhow!(
            "Unclosed quote in arguments; use Argument List for literal quotes"
        ));
    }
    if started {
        args.push(current);
    }
    Ok(args)
}

#[cfg(feature = "execute")]
fn launch_application(path: &str, arguments: &[String]) -> flow_like_types::Result<i64> {
    if path.trim().is_empty() {
        return Err(flow_like_types::anyhow!("Application path is required"));
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = std::process::Command::new("open");
        command.arg("-a").arg(path);
        if !arguments.is_empty() {
            command.arg("--args").args(arguments);
        }
        let result = command.output()?;
        if !result.status.success() {
            return Err(flow_like_types::anyhow!(
                "Application launch failed: {}",
                String::from_utf8_lossy(&result.stderr)
            ));
        }
        Ok(-1)
    }
    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(target_os = "linux")]
        if path.ends_with(".desktop") {
            let result = std::process::Command::new("gtk-launch")
                .arg(path)
                .args(arguments)
                .output()?;
            if !result.status.success() {
                return Err(flow_like_types::anyhow!(
                    "Desktop application launch failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                ));
            }
            return Ok(-1);
        }
        let child = std::process::Command::new(path).args(arguments).spawn()?;
        let pid = child.id() as i64;
        std::thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
        });
        Ok(pid)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_arguments;
    #[test]
    fn arguments_preserve_quoted_spaces_and_metacharacters() {
        assert_eq!(
            parse_arguments("--file \"a b.txt\" '$HOME' \"\"").unwrap(),
            vec!["--file", "a b.txt", "$HOME", ""]
        );
        assert!(parse_arguments("'unfinished").is_err());
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerWindowOperationNode;
impl ComputerWindowOperationNode {
    pub fn new() -> Self {
        Self
    }
}
#[async_trait]
impl NodeLogic for ComputerWindowOperationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_window_operation",
            "Manage Window",
            "Restores, minimizes, maximizes, moves, resizes, or closes a window by native ID",
            "Automation/Computer/Window",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "manageWindow");
        node.set_only_offline(true);
        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);
        node.add_input_pin("session", "Session", "Active session", VariableType::Struct)
            .set_schema::<AutomationSession>();
        node.add_input_pin(
            "window_id",
            "Window ID",
            "ID returned by List Windows",
            VariableType::String,
        );
        node.add_input_pin(
            "operation",
            "Operation",
            "Native window operation",
            VariableType::String,
        )
        .set_default_value(Some(json!("restore")))
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_valid_values(
                    ["restore", "minimize", "maximize", "move_resize", "close"]
                        .iter()
                        .map(|s| s.to_string())
                        .collect(),
                )
                .build(),
        );
        for (name, label, default) in [
            ("x", "X", 0),
            ("y", "Y", 0),
            ("width", "Width", 800),
            ("height", "Height", 600),
        ] {
            node.add_input_pin(
                name,
                label,
                "Desktop coordinates for move_resize",
                VariableType::Integer,
            )
            .set_default_value(Some(json!(default)));
        }
        node.add_output_pin(
            "exec_out",
            "▶",
            "Operation accepted",
            VariableType::Execution,
        );
        node.add_output_pin("session_out", "Session", "Session", VariableType::Struct)
            .set_schema::<AutomationSession>();
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let id: String = context.evaluate_pin("window_id").await?;
        let operation: String = context.evaluate_pin("operation").await?;
        let x: i32 = context.evaluate_pin("x").await?;
        let y: i32 = context.evaluate_pin("y").await?;
        let width: i32 = context.evaluate_pin("width").await?;
        let height: i32 = context.evaluate_pin("height").await?;
        if id.is_empty() {
            return Err(flow_like_types::anyhow!("Window ID is required"));
        }
        if operation == "move_resize" && (width <= 0 || height <= 0) {
            return Err(flow_like_types::anyhow!("Window size must be positive"));
        }
        let cancellation = context.get_cancellation_token();
        tokio::task::spawn_blocking(move || {
            if cancellation
                .as_ref()
                .is_some_and(|token| token.is_cancelled())
            {
                return Err(flow_like_types::anyhow!("Automation cancelled"));
            }
            super::native::operate_window(&id, &operation, Some((x, y, width, height)))
        })
        .await??;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Native window operations require execute"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerWaitForWindowNode;
impl ComputerWaitForWindowNode {
    pub fn new() -> Self {
        Self
    }
}
#[async_trait]
impl NodeLogic for ComputerWaitForWindowNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_wait_for_window",
            "Wait for Window",
            "Waits for a uniquely matching window or a timeout",
            "Automation/Computer/Window",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "waitForWindow");
        node.set_only_offline(true);
        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);
        node.add_input_pin("session", "Session", "Active session", VariableType::Struct)
            .set_schema::<AutomationSession>();
        node.add_input_pin(
            "window_title",
            "Title",
            "Window title substring",
            VariableType::String,
        );
        node.add_input_pin(
            "process_name",
            "Application",
            "Exact application name",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "focused",
            "Require Focus",
            "Wait until the matched window is focused",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "timeout_ms",
            "Timeout",
            "Maximum wait in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10000)));
        node.add_output_pin("exec_out", "▶", "Window found", VariableType::Execution);
        node.add_output_pin(
            "exec_timeout",
            "Timeout",
            "Window not found in time",
            VariableType::Execution,
        );
        node.add_output_pin("window", "Window", "Matched window", VariableType::Struct)
            .set_schema::<WindowInfo>();
        node
    }
    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_timeout").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let title: String = context.evaluate_pin("window_title").await?;
        let process: String = context.evaluate_pin("process_name").await?;
        let focused: bool = context.evaluate_pin("focused").await?;
        let timeout: i64 = context.evaluate_pin("timeout_ms").await?;
        if timeout < 0 {
            return Err(flow_like_types::anyhow!("Timeout cannot be negative"));
        }
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(timeout.min(3_600_000) as u64);
        loop {
            context.check_cancelled()?;
            session.ensure_active(context).await?;
            if let Some(window) =
                super::native::select_window_async("", &title, &process, false).await?
            {
                if !focused || window.is_focused().unwrap_or(false) {
                    context
                        .set_pin_value("window", json!(super::native::window_info(&window)))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                    return Ok(());
                }
            }
            if std::time::Instant::now() >= deadline {
                context.set_pin_value("window", json!(null)).await?;
                context.activate_exec_pin("exec_timeout").await?;
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
    #[cfg(not(feature = "execute"))]
    async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Native window operations require execute"
        ))
    }
}

use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BrowserUploadFileNode {}

impl BrowserUploadFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserUploadFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_upload_file",
            "Upload File",
            "Uploads a file to an input element using its selector",
            "Automation/Browser/Files",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "selector",
            "Selector",
            "CSS selector for the file input element",
            VariableType::String,
        )
        .set_default_value(Some(json!("input[type='file']")));

        node.add_input_pin(
            "file_path",
            "File Path",
            "Absolute path to the file to upload",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Success", VariableType::Execution);
        node.add_output_pin(
            "exec_error",
            "Error",
            "Element not found",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::By;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let file_path: String = context.evaluate_pin("file_path").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let element = match driver.find(By::Css(&selector)).await {
            Ok(el) => el,
            Err(_) => {
                context.set_pin_value("session_out", json!(session)).await?;
                context.activate_exec_pin("exec_error").await?;
                return Ok(());
            }
        };

        element
            .send_keys(&file_path)
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to upload file: {}", e))?;

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserUploadMultipleFilesNode {}

impl BrowserUploadMultipleFilesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserUploadMultipleFilesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_upload_multiple_files",
            "Upload Multiple Files",
            "Uploads multiple files to a file input that accepts multiple",
            "Automation/Browser/Files",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(5)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "selector",
            "Selector",
            "CSS selector for the file input element",
            VariableType::String,
        )
        .set_default_value(Some(json!("input[type='file']")));

        node.add_input_pin(
            "file_paths",
            "File Paths",
            "Array of absolute paths to the files to upload",
            VariableType::Generic,
        );

        node.add_output_pin("exec_out", "▶", "Success", VariableType::Execution);
        node.add_output_pin(
            "exec_error",
            "Error",
            "Element not found",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "uploaded_count",
            "Uploaded Count",
            "Number of files uploaded",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::By;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let file_paths: Vec<String> = context.evaluate_pin("file_paths").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let element = match driver.find(By::Css(&selector)).await {
            Ok(el) => el,
            Err(_) => {
                context.set_pin_value("session_out", json!(session)).await?;
                context.set_pin_value("uploaded_count", json!(0)).await?;
                context.activate_exec_pin("exec_error").await?;
                return Ok(());
            }
        };

        let paths_joined = file_paths.join("\n");
        element
            .send_keys(&paths_joined)
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to upload files: {}", e))?;

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("uploaded_count", json!(file_paths.len() as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserSetDownloadDirNode {}

impl BrowserSetDownloadDirNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserSetDownloadDirNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_set_download_dir",
            "Set Download Directory",
            "Sets the default download directory for the browser (must be called before downloads)",
            "Automation/Browser/Files",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(9)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "download_path",
            "Download Path",
            "Absolute path to the download directory",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::extensions::cdp::ChromeDevTools;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let download_path: FlowPath = context.evaluate_pin("download_path").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let runtime = download_path.to_runtime(context).await?;
        let path_str = runtime.path.to_string();

        let dev_tools = ChromeDevTools::new(driver.handle.clone());
        dev_tools
            .execute_cdp_with_params(
                "Page.setDownloadBehavior",
                flow_like_types::json::json!({
                    "behavior": "allow",
                    "downloadPath": path_str
                }),
            )
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to set download directory: {}", e))?;

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserWaitForDownloadNode {}

impl BrowserWaitForDownloadNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserWaitForDownloadNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_wait_for_download",
            "Wait For Download",
            "Waits for a file to appear in the download directory",
            "Automation/Browser/Files",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(4)
                .set_governance(6)
                .set_reliability(6)
                .set_cost(8)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "download_dir",
            "Download Directory",
            "Directory to watch for downloads",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "file_pattern",
            "File Pattern",
            "File name pattern to match (e.g., '*.pdf', leave empty for any)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "Maximum time to wait for download",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30000)));

        node.add_output_pin("exec_out", "▶", "Success", VariableType::Execution);
        node.add_output_pin(
            "exec_timeout",
            "Timeout",
            "Download timed out",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "downloaded_file",
            "Downloaded File",
            "Path to the downloaded file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::path::PathBuf;
        use std::time::{Duration, Instant};

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_timeout").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let download_dir: FlowPath = context.evaluate_pin("download_dir").await?;
        let file_pattern: String = context.evaluate_pin("file_pattern").await?;
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;

        let runtime = download_dir.to_runtime(context).await?;
        let dir_path = PathBuf::from(runtime.path.to_string());

        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms.max(0) as u64);

        let initial_files: std::collections::HashSet<_> = if dir_path.exists() {
            std::fs::read_dir(&dir_path)
                .map(|entries| entries.filter_map(|e| e.ok()).map(|e| e.path()).collect())
                .unwrap_or_default()
        } else {
            std::collections::HashSet::new()
        };

        loop {
            if start.elapsed() > timeout {
                context.set_pin_value("session_out", json!(session)).await?;
                context.activate_exec_pin("exec_timeout").await?;
                return Ok(());
            }

            if dir_path.exists()
                && let Ok(entries) = std::fs::read_dir(&dir_path)
            {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.is_file() && !initial_files.contains(&path) {
                        let file_name = path.file_name().unwrap_or_default().to_string_lossy();

                        if file_name.ends_with(".crdownload")
                            || file_name.ends_with(".part")
                            || file_name.ends_with(".tmp")
                        {
                            continue;
                        }

                        let matches = if file_pattern.is_empty() {
                            true
                        } else if let Some(suffix) = file_pattern.strip_prefix('*') {
                            file_name.ends_with(suffix)
                        } else if file_pattern.ends_with('*') {
                            let prefix = &file_pattern[..file_pattern.len() - 1];
                            file_name.starts_with(prefix)
                        } else {
                            file_name.contains(&file_pattern)
                        };

                        if matches {
                            let result_path = FlowPath::from_pathbuf(path, context).await?;
                            context.set_pin_value("session_out", json!(session)).await?;
                            context
                                .set_pin_value("downloaded_file", json!(result_path))
                                .await?;
                            context.activate_exec_pin("exec_out").await?;
                            return Ok(());
                        }
                    }
                }
            }

            flow_like_types::tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserTriggerDownloadNode {}

impl BrowserTriggerDownloadNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserTriggerDownloadNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_trigger_download",
            "Trigger Download",
            "Clicks an element to trigger a download",
            "Automation/Browser/Files",
        );
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(4)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "selector",
            "Selector",
            "CSS selector for the download link/button",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Success", VariableType::Execution);
        node.add_output_pin(
            "exec_error",
            "Error",
            "Element not found",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use thirtyfour::By;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let element = match driver.find(By::Css(&selector)).await {
            Ok(el) => el,
            Err(_) => {
                context.set_pin_value("session_out", json!(session)).await?;
                context.activate_exec_pin("exec_error").await?;
                return Ok(());
            }
        };

        element
            .click()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to click download element: {}", e))?;

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

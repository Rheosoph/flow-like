#[cfg(feature = "execute")]
use super::driver::PageContext;
use crate::types::handles::AutomationSession;
#[cfg(feature = "execute")]
use crate::types::handles::{DownloadArm, locked};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_browser::{
    ConnectionKind,
    downloads::{DownloadDestination, DownloadRecord, DownloadTracker},
    types::DownloadState,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use std::path::{Path, PathBuf};
#[cfg(feature = "execute")]
use std::time::{Duration, SystemTime};

#[cfg(feature = "execute")]
const LOCATE_BUDGET: Duration = Duration::from_secs(2);
/// Slack for file system timestamp granularity and event latency.
#[cfg(feature = "execute")]
const MTIME_TOLERANCE: Duration = Duration::from_secs(2);

#[cfg(any(feature = "execute", test))]
fn filename_matches(pattern: &str, name: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    let mut matches = vec![false; name.len() + 1];
    matches[0] = true;
    for character in pattern.chars() {
        if character == '*' {
            for index in 1..=name.len() {
                matches[index] |= matches[index - 1];
            }
        } else {
            for index in (1..=name.len()).rev() {
                matches[index] =
                    matches[index - 1] && (character == '?' || character == name[index - 1]);
            }
            matches[0] = false;
        }
    }
    matches[name.len()]
}

#[cfg(any(feature = "execute", test))]
fn last_component(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// Whether `name` is the suggested file name or the ` (n)` copy Chrome writes when it exists.
#[cfg(any(feature = "execute", test))]
fn is_download_variant(name: &str, suggested: &str) -> bool {
    name == suggested
        || name.match_indices(" (").any(|(start, _)| {
            let rest = &name[start + 2..];
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            digits > 0
                && rest[digits..].starts_with(')')
                && format!("{}{}", &name[..start], &rest[digits + 1..]) == suggested
        })
}

#[cfg(test)]
mod download_tests {
    use super::*;
    #[test]
    fn download_patterns_match_complete_names() {
        assert!(filename_matches("*.pdf", "invoice.pdf"));
        assert!(filename_matches("*invoice*.pdf", "2026-invoice-paid.pdf"));
        assert!(filename_matches("invoice-?.pdf", "invoice-ä.pdf"));
        assert!(!filename_matches("*.pdf", "invoice.pdf.crdownload"));
        assert!(!filename_matches("invoice.pdf", "old-invoice.pdf"));
    }

    #[test]
    fn numbered_copies_count_as_the_suggested_download() {
        assert!(is_download_variant("report.pdf", "report.pdf"));
        assert!(is_download_variant("report (1).pdf", "report.pdf"));
        assert!(is_download_variant("report (12).pdf", "report.pdf"));
        assert!(is_download_variant("archive (2).tar.gz", "archive.tar.gz"));
        assert!(is_download_variant("notes (3)", "notes"));
        assert!(!is_download_variant("report ().pdf", "report.pdf"));
        assert!(!is_download_variant("report (x).pdf", "report.pdf"));
        assert!(!is_download_variant(
            "report (1).pdf.crdownload",
            "report.pdf"
        ));
        assert!(!is_download_variant("old-report.pdf", "report.pdf"));
        assert_eq!(last_component("folder/sub\\invoice.pdf"), "invoice.pdf");
        assert_eq!(last_component("invoice.pdf"), "invoice.pdf");
    }
}

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
        node.set_flowscript_name("browser", "uploadFile");
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

        super::selector::add_locator_pin(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;
        let file_path: String = context.evaluate_pin("file_path").await?;

        let page = session.browser_page(context).await?;

        let element = match super::selector::find_element(&page, &locator).await {
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
        node.set_flowscript_name("browser", "uploadMultipleFiles");
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
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

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

        super::selector::add_locator_pin(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;
        let file_paths: Vec<String> = context.evaluate_pin("file_paths").await?;

        let page = session.browser_page(context).await?;

        let element = match super::selector::find_element(&page, &locator).await {
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
            "Sets the default download directory for the browser (must be called before downloads). When attached to a debugging Chrome, this changes that Chrome's download handling until it restarts. Fails when attached to your everyday Chrome, whose downloads stay in its own download folder; there, use Trigger Download and point Wait For Download at that folder.",
            "Automation/Browser/Files",
        );
        node.set_flowscript_name("browser", "setDownloadDir");
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
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let download_path: FlowPath = context.evaluate_pin("download_path").await?;

        let page = session.browser_page(context).await?;

        let directory = native_directory(context, &download_path).await?;
        std::fs::create_dir_all(&directory).map_err(|e| {
            flow_like_types::anyhow!(
                "Failed to create the download directory {}: {e}",
                directory.display()
            )
        })?;
        page.browser
            .set_download_directory(&directory)
            .await
            .map_err(|e| {
                flow_like_types::anyhow!(
                    "Failed to set download directory {}: {e}",
                    directory.display()
                )
            })?;
        let downloads = page.browser.downloads();
        downloads.set_destination(DownloadDestination::Directory {
            path: directory.clone(),
        });
        arm_download(
            &page,
            DownloadArm {
                directory: Some(directory),
                cursor: downloads.cursor(),
                include_in_progress: false,
            },
        );
        drop(page);
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
            "Waits for the next download started after Set Download Directory or Trigger Download to finish, using browser download events. Downloads from popup windows count, except when attached to your everyday Chrome.",
            "Automation/Browser/Files",
        );
        node.set_flowscript_name("browser", "waitForDownload");
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
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_timeout").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let download_dir: FlowPath = context.evaluate_pin("download_dir").await?;
        let file_pattern: String = context.evaluate_pin("file_pattern").await?;
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;
        if timeout_ms < 0 {
            return Err(flow_like_types::anyhow!(
                "Download timeout must be nonnegative"
            ));
        }
        let browser = session.cdp_browser(context).await?;
        let slot = session.browser_slot(context).await?;
        context
            .set_pin_value("downloaded_file", json!(null))
            .await?;

        let dir_path = native_directory(context, &download_dir).await?;
        let deadline = tokio::time::Instant::now()
            .checked_add(Duration::from_millis(timeout_ms as u64))
            .ok_or_else(|| {
                flow_like_types::anyhow!("Download timeout of {timeout_ms} ms is too large")
            })?;

        let approval = browser.kind() == ConnectionKind::AttachedApproval;
        let downloads = browser.downloads();
        let window = locked(&slot.downloads, |arm| {
            download_window(approval, arm.as_ref(), &dir_path, downloads.cursor())
        })?;
        let directory = canonical(&dir_path);
        let matches = |record: &DownloadRecord| download_matches(record, &directory, &file_pattern);
        let Some((record, fresh_after)) =
            next_download(context, downloads, window, deadline, matches).await?
        else {
            context.set_pin_value("session_out", json!(session)).await?;
            context.activate_exec_pin("exec_timeout").await?;
            return Ok(());
        };
        let path =
            finished_download_path(context, record, &dir_path, approval, fresh_after).await?;

        let result_path = FlowPath::from_pathbuf(path, context).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("downloaded_file", json!(result_path))
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
        node.set_flowscript_name("browser", "triggerDownload");
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

        super::selector::add_locator_pin(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;

        let page = session.browser_page(context).await?;

        let element = match super::selector::find_element(&page, &locator).await {
            Ok(el) => el,
            Err(_) => {
                context.set_pin_value("session_out", json!(session)).await?;
                context.activate_exec_pin("exec_error").await?;
                return Ok(());
            }
        };

        rearm_download(&page)?;
        element
            .element_click()
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

#[cfg(feature = "execute")]
async fn native_directory(
    context: &mut ExecutionContext,
    path: &FlowPath,
) -> flow_like_types::Result<PathBuf> {
    let runtime = path.to_runtime(context).await?;
    match runtime.store.as_ref() {
        flow_like_storage::files::store::FlowLikeStore::Local(store) => {
            Ok(store.path_to_filesystem(&runtime.path)?)
        }
        _ => Err(flow_like_types::anyhow!(
            "Browser downloads require a local directory on this machine"
        )),
    }
}

/// Resolves the longest existing ancestor, so a file that does not exist (a failed move, a
/// file Chrome has not written yet) still compares with a canonical directory.
#[cfg(feature = "execute")]
fn canonical(path: &Path) -> PathBuf {
    if let Ok(resolved) = std::fs::canonicalize(path) {
        return resolved;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
            canonical(parent).join(name)
        }
        _ => path.to_path_buf(),
    }
}

#[cfg(feature = "execute")]
fn arm_download(page: &PageContext, arm: DownloadArm) {
    locked(&page.slot.downloads, |current| *current = Some(arm));
}

#[cfg(feature = "execute")]
fn rearm_download(page: &PageContext) -> flow_like_types::Result<()> {
    let approval = page.browser.kind() == ConnectionKind::AttachedApproval;
    let downloads = page.browser.downloads();
    locked(&page.slot.downloads, |arm| {
        let directory = match arm.as_ref() {
            Some(armed) => armed.directory.clone(),
            None if approval => None,
            None => {
                return Err(flow_like_types::anyhow!(
                    "Set Download Directory before triggering a download"
                ));
            }
        };
        *arm = Some(DownloadArm {
            directory,
            cursor: downloads.cursor(),
            include_in_progress: false,
        });
        Ok(())
    })
}

/// Where a wait starts: the download cursor and whether downloads already running at it
/// count.
#[cfg(feature = "execute")]
#[derive(Debug, PartialEq)]
struct DownloadWindow {
    since: u64,
    include_in_progress: bool,
}

#[cfg(feature = "execute")]
fn download_window(
    approval: bool,
    arm: Option<&DownloadArm>,
    directory: &Path,
    cursor_now: u64,
) -> flow_like_types::Result<DownloadWindow> {
    match arm {
        Some(arm) => match arm.directory.as_deref() {
            Some(armed) if canonical(armed) != canonical(directory) => {
                Err(flow_like_types::anyhow!(
                    "Download directory differs from the armed directory: waiting in {}, armed {}",
                    directory.display(),
                    armed.display()
                ))
            }
            _ => Ok(DownloadWindow {
                since: arm.cursor,
                include_in_progress: arm.include_in_progress,
            }),
        },
        None if approval => Ok(DownloadWindow {
            since: cursor_now,
            include_in_progress: true,
        }),
        None => Err(flow_like_types::anyhow!(
            "Set Download Directory before triggering a download"
        )),
    }
}

#[cfg(feature = "execute")]
fn download_name(record: &DownloadRecord) -> String {
    record
        .final_path
        .as_deref()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| last_component(&record.suggested_filename).to_owned())
}

/// `directory` must already be canonical. A record without an absolute path (canceled,
/// approval-attached, or a move that failed before it had a folder) matches on its name only.
#[cfg(feature = "execute")]
fn download_matches(record: &DownloadRecord, directory: &Path, pattern: &str) -> bool {
    (pattern.is_empty() || filename_matches(pattern, &download_name(record)))
        && record
            .final_path
            .as_deref()
            .filter(|path| path.is_absolute())
            .is_none_or(|path| canonical(path).starts_with(directory))
}

/// The oldest modification time a located file may have: the download was written after the
/// browser announced it, however late it finished and whenever the wait began.
#[cfg(feature = "execute")]
fn fresh_bound(record: &DownloadRecord) -> Option<SystemTime> {
    record.begun_at.checked_sub(MTIME_TOLERANCE)
}

/// The first matching download and the oldest modification time its file may have.
#[cfg(feature = "execute")]
async fn next_download(
    context: &ExecutionContext,
    downloads: &DownloadTracker,
    window: DownloadWindow,
    deadline: tokio::time::Instant,
    matches: impl Fn(&DownloadRecord) -> bool,
) -> flow_like_types::Result<Option<(DownloadRecord, Option<SystemTime>)>> {
    let finished = until_cancelled(
        context,
        downloads.wait_finished(window.since, window.include_in_progress, deadline, &matches),
    )
    .await??;
    Ok(finished.map(|record| {
        let fresh_after = fresh_bound(&record);
        (record, fresh_after)
    }))
}

#[cfg(feature = "execute")]
fn newest_download(
    directory: &Path,
    suggested: &str,
    fresh_after: Option<SystemTime>,
) -> Option<PathBuf> {
    std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| is_download_variant(name, suggested))
        })
        .filter_map(|entry| {
            let metadata = entry.metadata().ok().filter(std::fs::Metadata::is_file)?;
            Some((metadata.modified().ok()?, entry.path()))
        })
        .filter(|(modified, _)| fresh_after.is_none_or(|bound| *modified >= bound))
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// Page download events of an approval-attached browser carry no file path, so the file is
/// looked up by its suggested name in the watched directory.
#[cfg(feature = "execute")]
async fn locate_download(
    context: &ExecutionContext,
    directory: &Path,
    suggested: &str,
    approval: bool,
    fresh_after: Option<SystemTime>,
) -> flow_like_types::Result<PathBuf> {
    let deadline = tokio::time::Instant::now() + LOCATE_BUDGET;
    loop {
        if let Some(path) = newest_download(directory, suggested, fresh_after) {
            return Ok(path);
        }
        if tokio::time::Instant::now() >= deadline {
            let hint = if approval {
                "; downloads from your everyday Chrome go to Chrome's own download folder"
            } else {
                ""
            };
            return Err(flow_like_types::anyhow!(
                "Download of {suggested} finished, but it did not appear in {} within {} s{hint}",
                directory.display(),
                LOCATE_BUDGET.as_secs()
            ));
        }
        crate::rpa::branch::delay(context, Duration::from_millis(100)).await?;
    }
}

#[cfg(feature = "execute")]
async fn finished_download_path(
    context: &ExecutionContext,
    record: DownloadRecord,
    directory: &Path,
    approval: bool,
    fresh_after: Option<SystemTime>,
) -> flow_like_types::Result<PathBuf> {
    if record.state == DownloadState::Canceled {
        return Err(flow_like_types::anyhow!(
            "Download of {} was canceled",
            download_name(&record)
        ));
    }
    match record.final_path {
        Some(path) if path.is_file() => Ok(path),
        _ => {
            let suggested = last_component(&record.suggested_filename);
            locate_download(context, directory, suggested, approval, fresh_after).await
        }
    }
}

#[cfg(feature = "execute")]
async fn until_cancelled<T>(
    context: &ExecutionContext,
    work: impl std::future::Future<Output = T>,
) -> flow_like_types::Result<T> {
    let Some(token) = context.get_cancellation_token() else {
        return Ok(work.await);
    };
    tokio::select! {
        biased;
        _ = token.cancelled() => Err(flow_like_types::anyhow!("Execution was cancelled")),
        output = work => Ok(output),
    }
}

#[cfg(all(test, feature = "execute"))]
mod download_event_tests {
    use super::*;

    fn record(name: &str, state: DownloadState, final_path: Option<&str>) -> DownloadRecord {
        DownloadRecord {
            seq: 1,
            guid: "guid-1".to_string(),
            url: "https://example.test/file".to_string(),
            suggested_filename: name.to_string(),
            frame_id: None,
            state,
            final_path: final_path.map(PathBuf::from),
            begun_at: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        }
    }

    fn arm(directory: Option<&Path>, cursor: u64) -> DownloadArm {
        DownloadArm {
            directory: directory.map(Path::to_path_buf),
            cursor,
            include_in_progress: false,
        }
    }

    /// An absolute path that does not exist, on every platform.
    fn missing(relative: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!("flow-like-missing-{}", std::process::id()))
            .join(relative)
    }

    #[test]
    fn records_match_on_pattern_and_directory() {
        let directory = canonical(&missing("downloads"));
        let inside_path = missing("downloads").join("invoice (1).pdf");
        let inside = record(
            "invoice.pdf",
            DownloadState::Completed,
            inside_path.to_str(),
        );
        assert!(download_matches(&inside, &directory, ""));
        assert!(download_matches(&inside, &directory, "invoice*.pdf"));
        assert!(!download_matches(&inside, &directory, "*.csv"));
        let elsewhere_path = missing("other").join("invoice.pdf");
        let elsewhere = record(
            "invoice.pdf",
            DownloadState::Completed,
            elsewhere_path.to_str(),
        );
        assert!(!download_matches(&elsewhere, &directory, "*.pdf"));
        let unplaced = record("reports/q3.csv", DownloadState::Completed, None);
        assert!(download_matches(&unplaced, &directory, "q3.csv"));
        let canceled = record("q3.csv", DownloadState::Canceled, None);
        assert_eq!(download_name(&canceled), "q3.csv");
        assert!(!download_matches(&canceled, &directory, "*.pdf"));
        let unmoved = record("q3.csv", DownloadState::Completed, Some("q3.csv"));
        assert!(download_matches(&unmoved, &directory, "*.csv"));
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("flow-like-downloads-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn missing_files_under_a_linked_directory_match_the_real_directory() {
        let scratch = Scratch::new("linked");
        let real = scratch.0.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = scratch.0.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let directory = canonical(&real);
        let missing = link.join("missing.pdf");
        assert_eq!(canonical(&missing), directory.join("missing.pdf"));
        assert_eq!(
            canonical(&link.join("new").join("a.pdf")),
            directory.join("new").join("a.pdf")
        );
        let failed = record("missing.pdf", DownloadState::Completed, missing.to_str());
        assert!(download_matches(&failed, &directory, "*.pdf"));
        let outside = record(
            "missing.pdf",
            DownloadState::Completed,
            scratch.0.join("missing.pdf").to_str(),
        );
        assert!(!download_matches(&outside, &directory, "*.pdf"));
    }

    fn write_at(path: &Path, modified: SystemTime) {
        std::fs::write(path, b"download").unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }

    fn begun_at(name: &str, begun_at: SystemTime) -> DownloadRecord {
        DownloadRecord {
            begun_at,
            ..record(name, DownloadState::Completed, None)
        }
    }

    #[test]
    fn files_older_than_the_wait_are_not_the_download() {
        let scratch = Scratch::new("stale");
        let old = scratch.0.join("report.pdf");
        write_at(&old, SystemTime::now() - Duration::from_secs(3600));
        let fresh_after = SystemTime::now().checked_sub(MTIME_TOLERANCE);
        assert_eq!(newest_download(&scratch.0, "report.pdf", fresh_after), None);
        assert_eq!(
            newest_download(&scratch.0, "report.pdf", None),
            Some(old.clone())
        );
        let copy = scratch.0.join("report (1).pdf");
        std::fs::write(&copy, b"new").unwrap();
        assert_eq!(
            newest_download(&scratch.0, "report.pdf", fresh_after),
            Some(copy)
        );
        assert_eq!(newest_download(&scratch.0, "invoice.pdf", None), None);
    }

    #[test]
    fn files_written_before_the_download_began_are_not_the_download() {
        let scratch = Scratch::new("early");
        let old = scratch.0.join("report.pdf");
        let now = SystemTime::now();
        let an_hour_ago = now - Duration::from_secs(3600);
        write_at(&old, an_hour_ago);
        let bound = fresh_bound(&begun_at("report.pdf", now));
        assert_eq!(bound, now.checked_sub(MTIME_TOLERANCE));
        assert_eq!(newest_download(&scratch.0, "report.pdf", bound), None);
        let earlier = begun_at("report.pdf", an_hour_ago - Duration::from_secs(60));
        let bound = fresh_bound(&earlier);
        assert_eq!(newest_download(&scratch.0, "report.pdf", bound), Some(old));
    }

    #[test]
    fn a_download_finishing_during_the_wait_is_bounded_by_its_begin() {
        let scratch = Scratch::new("finished-late");
        let begun = SystemTime::now() - Duration::from_secs(60);
        let file = scratch.0.join("report.zip");
        write_at(&file, begun + Duration::from_secs(1));
        let bound = fresh_bound(&begun_at("report.zip", begun));
        assert_eq!(newest_download(&scratch.0, "report.zip", bound), Some(file));
    }

    #[test]
    fn a_download_running_before_an_unarmed_wait_skips_older_files_of_its_name() {
        let scratch = Scratch::new("unarmed-running");
        let begun = SystemTime::now() - Duration::from_secs(60);
        let old = scratch.0.join("report.zip");
        write_at(&old, begun - Duration::from_secs(3600));
        let window = download_window(true, None, &scratch.0, 9).unwrap();
        assert!(window.include_in_progress);
        let running = begun_at("report.zip", begun);
        assert!(running.seq <= window.since);
        let bound = fresh_bound(&running);
        assert_eq!(newest_download(&scratch.0, "report.zip", bound), None);
        let copy = scratch.0.join("report (1).zip");
        write_at(&copy, begun + Duration::from_secs(2));
        assert_eq!(newest_download(&scratch.0, "report.zip", bound), Some(copy));
    }

    #[test]
    fn wait_window_follows_the_arm_and_connection_kind() {
        let directory = missing("downloads");
        let armed = arm(Some(&directory), 7);
        assert_eq!(
            download_window(false, Some(&armed), &directory, 9).unwrap(),
            DownloadWindow {
                since: 7,
                include_in_progress: false,
            }
        );
        let approval_arm = arm(None, 3);
        assert_eq!(
            download_window(true, Some(&approval_arm), &directory, 9).unwrap(),
            DownloadWindow {
                since: 3,
                include_in_progress: false,
            }
        );
        assert_eq!(
            download_window(true, None, &directory, 9).unwrap(),
            DownloadWindow {
                since: 9,
                include_in_progress: true,
            }
        );
        let unarmed = download_window(false, None, &directory, 9).unwrap_err();
        assert_eq!(
            unarmed.to_string(),
            "Set Download Directory before triggering a download"
        );
        let other = arm(Some(&missing("other")), 7);
        let differs = download_window(false, Some(&other), &directory, 9).unwrap_err();
        assert!(
            differs
                .to_string()
                .starts_with("Download directory differs from the armed directory"),
            "{differs}"
        );
    }
}

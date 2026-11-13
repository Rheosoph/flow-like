//! `file_attach`: files a bot puts on its reply so the chat shows them as files, not links. A
//! file of the bot's workspace attaches as is, any other file is copied there with `file_save`
//! first, and a URL (such as a signed link an app returned) attaches as a link. Hosts that store
//! the reply pass [`ReplyFiles`] in [`super::HostTools::with_reply_files`] and put the files on
//! the reply body with [`ReplyFiles::attach`] before the tool trail.

use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use flow_like_bot_tools as tools;
use flow_like_types::bot_message::{
    MAX_FILE_NAME_CHARS, WorkspaceHandle, is_file_link, is_mime, is_workspace_path,
};
use flow_like_types::bot_reply_files::{MAX_REPLY_FILES, ReplyFile, attach_reply_files};
use serde::Deserialize;
use serde_json::{Value, json};

use super::platform::{Clearance, PlatformToolBridge};
use super::stream::stream_frame;
use super::tool_spec::{PlatformToolSpec, ToolApprovalSpec};
use super::turn_label::TurnLabel;

pub const FILE_ATTACH_TOOL: &str = "file_attach";
/// The stream frame that shows an attached file on the reply while the turn runs.
pub const REPLY_ATTACHMENT_FRAME: &str = "reply_attachment";

const TOOL_TIMEOUT_SECS: u64 = 180;

/// The files of one turn's reply, in the order they were attached.
#[derive(Clone, Default)]
pub struct ReplyFiles(Arc<Mutex<Vec<ReplyFile>>>);

impl ReplyFiles {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn files(&self) -> Vec<ReplyFile> {
        self.lock().clone()
    }

    /// Puts the turn's files on its reply `body` as attachment parts.
    pub fn attach(&self, body: &mut Value) {
        attach_reply_files(body, &self.files());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<ReplyFile>> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Adds `file` unless the reply has it already; `Err` when the reply is full.
    fn add(&self, file: ReplyFile) -> Result<usize, String> {
        let mut files = self.lock();
        if files.iter().any(|known| known.same_file(&file)) {
            return Ok(files.len());
        }
        if files.len() >= MAX_REPLY_FILES {
            return Err(format!(
                "This reply already carries {MAX_REPLY_FILES} files, the most one reply can hold"
            ));
        }
        files.push(file);
        Ok(files.len())
    }
}

impl fmt::Debug for ReplyFiles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReplyFiles")
            .field("files", &self.lock().len())
            .finish()
    }
}

impl PartialEq for ReplyFiles {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ReplyFiles {}

pub fn file_attach_spec() -> PlatformToolSpec {
    PlatformToolSpec {
        name: FILE_ATTACH_TOOL,
        description: "Attach a file to your reply so the chat shows it as a file people can open and download, instead of a link in your text. Pass `file` for a file you have: a handle another tool returned or a message attachment's `file` value, a workspace path or `app:<app_id>/<path>`; files outside your workspace are copied into it first. Or pass `url` for a file at a web address, such as a signed download link an app or connector returned; it is attached as a link, so it opens only as long as the link works. To keep such a file, save it with file_save `url` first and attach the handle it returns. Up to 8 files per reply. Mention the file in your text, but don't paste its link as well.",
        schema: file_attach_schema,
        approval: ToolApprovalSpec::None,
        timeout_secs: TOOL_TIMEOUT_SECS,
    }
}

fn file_attach_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "file": {
                "description": "A file you have: a handle object another tool returned or a message attachment's `file` value, a workspace path or `app:<app_id>/<path>`. Not together with `url`.",
                "oneOf": [
                    { "type": "string" },
                    { "type": "object" }
                ]
            },
            "url": {
                "type": "string",
                "description": "An http(s) address of the file, such as a signed download link. Not together with `file`."
            },
            "name": {
                "type": "string",
                "maxLength": MAX_FILE_NAME_CHARS,
                "description": "The file name shown, with its extension, e.g. 'Angebot Musterfirma.pdf'. Defaults to the file's own name."
            },
            "mime": {
                "type": "string",
                "description": "The type of a linked file, e.g. 'application/pdf'. Guessed from the name when omitted."
            }
        }
    })
}

#[derive(Deserialize)]
struct AttachArgs {
    #[serde(default)]
    file: Option<Value>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    mime: Option<String>,
}

/// Answers `file_attach` and hands every other call on.
pub(super) struct ReplyFilesBridge {
    files: ReplyFiles,
    workspace: Option<String>,
    label: Arc<TurnLabel>,
    frames: Option<Arc<dyn Fn(String) + Send + Sync>>,
    inner: Arc<dyn PlatformToolBridge>,
}

impl ReplyFilesBridge {
    pub(super) fn new(
        files: ReplyFiles,
        workspace: Option<String>,
        label: Arc<TurnLabel>,
        frames: Option<Arc<dyn Fn(String) + Send + Sync>>,
        inner: Arc<dyn PlatformToolBridge>,
    ) -> Self {
        Self {
            files,
            workspace,
            label,
            frames,
            inner,
        }
    }

    async fn attach(&self, arguments: Value) -> String {
        if let Some(refusal) = self.label.refusal(FILE_ATTACH_TOOL, &arguments) {
            return refusal;
        }
        let args = match serde_json::from_value::<AttachArgs>(arguments) {
            Ok(args) => args,
            Err(error) => return refusal("invalid_args", format!("Invalid arguments: {error}")),
        };
        let name = match args.name.as_deref().map(str::trim) {
            Some(name) if !valid_name(name) => {
                return refusal(
                    "invalid_args",
                    format!("`name` must be 1 to {MAX_FILE_NAME_CHARS} characters on one line"),
                );
            }
            name => name.filter(|name| !name.is_empty()).map(str::to_string),
        };
        let file = match (args.file, args.url) {
            (Some(file), None) => self.stored(file, name).await,
            (None, Some(url)) => linked(url.trim(), name, args.mime),
            _ => Err(refusal(
                "invalid_args",
                "Pass exactly one of `file` (a file you have) and `url` (a web address)",
            )),
        };
        let file = match file {
            Ok(file) => file,
            Err(refused) => return refused,
        };
        let count = match self.files.add(file.clone()) {
            Ok(count) => count,
            Err(message) => return refusal("too_many_files", message),
        };
        if let Some(frames) = &self.frames {
            frames(stream_frame(REPLY_ATTACHMENT_FRAME, &frame_of(&file)));
        }
        json!({
            "status": "ok",
            "tool": FILE_ATTACH_TOOL,
            "attached": summary_of(&file),
            "files_on_reply": count,
            "message": "The file shows on your reply. Refer to it in your text without pasting its link.",
        })
        .to_string()
    }

    /// The bot's own workspace file as is; anything else copied there with `file_save`.
    async fn stored(&self, file: Value, name: Option<String>) -> Result<ReplyFile, String> {
        let handle = match own_handle(&file, self.workspace.as_deref()) {
            Some(handle) => handle,
            None => self.saved_copy(file).await?,
        };
        let name = name.unwrap_or_else(|| file_name(&handle.path));
        Ok(ReplyFile::stored(name, handle))
    }

    async fn saved_copy(&self, file: Value) -> Result<WorkspaceHandle, String> {
        if self.workspace.is_none() {
            return Err(refusal(
                "no_workspace",
                "You have no workspace to keep the file in, so only a `url` can be attached",
            ));
        }
        let output = self
            .inner
            .call(tools::files::FILE_SAVE, json!({ "file": file }))
            .await;
        let result: Value = serde_json::from_str(&output).unwrap_or(Value::Null);
        if result.get("status").and_then(Value::as_str) != Some("ok") {
            let reason = result
                .get("error")
                .and_then(|error| error.get("message"))
                .or_else(|| result.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("the file could not be copied into your workspace");
            return Err(refusal(
                "not_attached",
                format!("The file was not attached: {reason}"),
            ));
        }
        result
            .get("handle")
            .cloned()
            .and_then(|handle| serde_json::from_value::<WorkspaceHandle>(handle).ok())
            .ok_or_else(|| {
                refusal(
                    "not_attached",
                    "The file was copied but its handle came back unreadable; attach the handle file_save returned",
                )
            })
    }
}

#[async_trait]
impl PlatformToolBridge for ReplyFilesBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        if tool_name == FILE_ATTACH_TOOL {
            return self.attach(arguments).await;
        }
        self.inner.call(tool_name, arguments).await
    }

    async fn clearance(&self, tool_name: &str, arguments: &Value) -> Clearance {
        if tool_name == FILE_ATTACH_TOOL {
            return Clearance::Clear;
        }
        self.inner.clearance(tool_name, arguments).await
    }

    fn supports_tool(&self, tool_name: &str) -> bool {
        tool_name == FILE_ATTACH_TOOL || self.inner.supports_tool(tool_name)
    }

    async fn drain_steering(&self) -> Vec<String> {
        self.inner.drain_steering().await
    }

    fn watches_room(&self) -> bool {
        self.inner.watches_room()
    }

    async fn drain_room_updates(&self) -> Vec<String> {
        self.inner.drain_room_updates().await
    }

    async fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled().await
    }
}

fn refusal(code: &str, message: impl Into<String>) -> String {
    json!({
        "status": "error",
        "tool": FILE_ATTACH_TOOL,
        "code": code,
        "retryable": false,
        "message": message.into(),
    })
    .to_string()
}

fn valid_name(name: &str) -> bool {
    (1..=MAX_FILE_NAME_CHARS).contains(&name.chars().count())
        && !name.chars().any(char::is_control)
}

/// A complete handle of a file in the bot's own workspace.
fn own_handle(file: &Value, workspace: Option<&str>) -> Option<WorkspaceHandle> {
    let handle = serde_json::from_value::<WorkspaceHandle>(file.clone()).ok()?;
    (Some(handle.app_id.as_str()) == workspace && is_workspace_path(&handle.path))
        .then_some(handle)
}

fn linked(url: &str, name: Option<String>, mime: Option<String>) -> Result<ReplyFile, String> {
    if !is_file_link(url) {
        return Err(refusal(
            "invalid_args",
            "`url` must be an http(s) address with a host and no user name or password",
        ));
    }
    let mime = match mime.map(|mime| mime.trim().to_ascii_lowercase()) {
        Some(mime) if !is_mime(&mime) => {
            return Err(refusal(
                "invalid_args",
                format!("`mime` '{mime}' is not type/subtype"),
            ));
        }
        mime => mime,
    };
    let name = name.unwrap_or_else(|| link_name(url));
    let mime = mime.or_else(|| {
        flow_like_types::mime_guess::from_path(&name)
            .first()
            .map(|guess| guess.essence_str().to_string())
    });
    Ok(ReplyFile::linked(name, url.to_string(), mime))
}

/// The last path segment of a link, decoded; `download` when it has none.
fn link_name(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .path_segments()?
                .rev()
                .find(|segment| !segment.is_empty())
                .map(|segment| flow_like_storage::decode_path_segment(segment).into_owned())
        })
        .filter(|name| valid_name(name))
        .unwrap_or_else(|| "download".to_string())
}

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// A link without its query and fragment: signed links carry their credentials there.
fn shown_link(url: &str) -> String {
    url.split(['?', '#']).next().unwrap_or_default().to_string()
}

fn summary_of(file: &ReplyFile) -> Value {
    match &file.handle {
        Some(handle) => json!({
            "name": file.name,
            "mime": handle.mime,
            "bytes": handle.bytes,
            "file": handle,
        }),
        None => json!({
            "name": file.name,
            "mime": file.mime,
            "link": shown_link(file.url.as_deref().unwrap_or_default()),
        }),
    }
}

fn frame_of(file: &ReplyFile) -> Value {
    match &file.handle {
        Some(handle) => json!({
            "name": file.name,
            "mime": handle.mime,
            "bytes": handle.bytes,
            "url": tools::files::markdown_url(handle),
            "handle": handle,
        }),
        None => json!({
            "name": file.name,
            "mime": file.mime,
            "url": file.url,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::copilot::bot_run::BotRunContext;
    use flow_like_types::bot_message::MessageBody;
    use flow_like_types::data_classification::{DataClassification, DataSensitivity};

    struct Host {
        calls: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl PlatformToolBridge for Host {
        async fn call(&self, tool_name: &str, arguments: Value) -> String {
            self.calls
                .lock()
                .unwrap()
                .push((tool_name.to_string(), arguments.clone()));
            match tool_name {
                "file_save" => json!({
                    "status": "ok",
                    "tool": "file_save",
                    "handle": handle("ws1", "bot/files/scan.pdf"),
                    "markdown_url": "storage://apps/ws1/app/bot/files/scan.pdf",
                })
                .to_string(),
                _ => json!({ "status": "ok", "tool": tool_name }).to_string(),
            }
        }
    }

    fn handle(app_id: &str, path: &str) -> WorkspaceHandle {
        WorkspaceHandle {
            app_id: app_id.to_string(),
            path: path.to_string(),
            bytes: 2048,
            sha256: "b".repeat(64),
            mime: "application/pdf".to_string(),
        }
    }

    fn label(seen: Option<DataClassification>) -> Arc<TurnLabel> {
        let mut bot = BotRunContext::general();
        bot.classification.classification = Some(DataClassification::Internal);
        let label = TurnLabel::new(&bot, None);
        label.raise(DataSensitivity {
            classification: seen,
            pii: false,
        });
        label
    }

    fn harness(
        workspace: Option<&str>,
        label: Arc<TurnLabel>,
    ) -> (ReplyFilesBridge, ReplyFiles, Arc<Host>, Arc<Mutex<Vec<String>>>) {
        let files = ReplyFiles::new();
        let host = Arc::new(Host {
            calls: Mutex::new(Vec::new()),
        });
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink = frames.clone();
        let bridge = ReplyFilesBridge::new(
            files.clone(),
            workspace.map(str::to_string),
            label,
            Some(Arc::new(move |frame: String| sink.lock().unwrap().push(frame))),
            host.clone(),
        );
        (bridge, files, host, frames)
    }

    fn status(output: &str) -> (String, String) {
        let value: Value = serde_json::from_str(output).unwrap();
        (
            value["status"].as_str().unwrap_or_default().to_string(),
            value["code"].as_str().unwrap_or_default().to_string(),
        )
    }

    #[tokio::test]
    async fn a_signed_link_attaches_as_a_link_and_the_result_hides_its_signature() {
        let (bridge, files, host, frames) = harness(Some("ws1"), label(None));
        let url = "https://vault.s3.eu-central-1.amazonaws.com/offers/Probe%20Offering.pdf?X-Amz-Signature=secret";
        let output = bridge.call(FILE_ATTACH_TOOL, json!({ "url": url })).await;
        assert_eq!(status(&output).0, "ok");
        assert!(!output.contains("secret"));
        assert!(host.calls.lock().unwrap().is_empty());

        let attached = files.files();
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].name, "Probe Offering.pdf");
        assert_eq!(attached[0].mime.as_deref(), Some("application/pdf"));
        assert_eq!(attached[0].url.as_deref(), Some(url));

        let frames = frames.lock().unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].starts_with("<reply_attachment>"));

        let mut body = json!({ "text": "Hier ist das Offering" });
        files.attach(&mut body);
        assert!(MessageBody::parse(&body).is_ok());
    }

    #[tokio::test]
    async fn a_workspace_handle_attaches_as_is_and_other_files_are_copied_first() {
        let (bridge, files, host, frames) = harness(Some("ws1"), label(None));
        let own = handle("ws1", "bot/renders/offer.pdf");
        let output = bridge
            .call(FILE_ATTACH_TOOL, json!({ "file": own, "name": "Angebot.pdf" }))
            .await;
        assert_eq!(status(&output).0, "ok");
        assert!(host.calls.lock().unwrap().is_empty());

        let output = bridge
            .call(FILE_ATTACH_TOOL, json!({ "file": "#12.a1" }))
            .await;
        assert_eq!(status(&output).0, "ok");
        let calls = host.calls.lock().unwrap().clone();
        assert_eq!(calls, [("file_save".to_string(), json!({ "file": "#12.a1" }))]);

        let attached = files.files();
        assert_eq!(attached[0].name, "Angebot.pdf");
        assert_eq!(attached[1].name, "scan.pdf");
        assert_eq!(attached[1].handle.as_ref().unwrap().path, "bot/files/scan.pdf");
        assert!(frames.lock().unwrap()[0].contains("storage://apps/ws1/app/bot/renders/offer.pdf"));

        let foreign = handle("other-app", "report.pdf");
        bridge
            .call(FILE_ATTACH_TOOL, json!({ "file": foreign.clone() }))
            .await;
        assert_eq!(host.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn bad_calls_are_refused_and_nothing_is_attached() {
        let (bridge, files, _, _) = harness(Some("ws1"), label(None));
        for args in [
            json!({}),
            json!({ "url": "https://h/a.pdf", "file": "bot/a.pdf" }),
            json!({ "url": "file:///etc/passwd" }),
            json!({ "url": "https://user:pw@h/a.pdf" }),
            json!({ "url": "https://h/a.pdf", "mime": "pdf" }),
            json!({ "url": "https://h/a.pdf", "name": "two\nlines" }),
        ] {
            let output = bridge.call(FILE_ATTACH_TOOL, args.clone()).await;
            assert_eq!(status(&output).0, "error", "{args} was accepted");
        }
        assert!(files.files().is_empty());

        let (no_workspace, _, _, _) = harness(None, label(None));
        let output = no_workspace
            .call(FILE_ATTACH_TOOL, json!({ "file": "bot/a.pdf" }))
            .await;
        assert_eq!(status(&output).1, "no_workspace");
    }

    #[tokio::test]
    async fn a_reply_holds_eight_files_and_the_same_file_once() {
        let (bridge, files, _, _) = harness(Some("ws1"), label(None));
        for _ in 0..2 {
            bridge
                .call(FILE_ATTACH_TOOL, json!({ "url": "https://h/same.pdf" }))
                .await;
        }
        assert_eq!(files.files().len(), 1);
        for index in 1..MAX_REPLY_FILES {
            let output = bridge
                .call(FILE_ATTACH_TOOL, json!({ "url": format!("https://h/{index}.pdf") }))
                .await;
            assert_eq!(status(&output).0, "ok");
        }
        let output = bridge
            .call(FILE_ATTACH_TOOL, json!({ "url": "https://h/ninth.pdf" }))
            .await;
        assert_eq!(status(&output).1, "too_many_files");
    }

    #[tokio::test]
    async fn a_turn_above_the_bots_level_may_not_attach_links() {
        let (bridge, files, _, _) = harness(Some("ws1"), label(Some(DataClassification::Confidential)));
        let output = bridge
            .call(FILE_ATTACH_TOOL, json!({ "url": "https://h/a.pdf" }))
            .await;
        assert_eq!(status(&output).1, "turn_label_outbound_refused");
        let output = bridge
            .call(FILE_ATTACH_TOOL, json!({ "file": handle("ws1", "bot/a.pdf") }))
            .await;
        assert_eq!(status(&output).0, "ok");
        assert_eq!(files.files().len(), 1);
    }
}

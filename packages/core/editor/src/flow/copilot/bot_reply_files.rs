//! `file_attach`: files a bot puts on its reply so the chat shows them as files, not links. A
//! file of the bot's workspace attaches as is, any other file is copied there with `file_save`
//! first, and a URL (such as a signed link an app returned) attaches as a link. Hosts that store
//! the reply pass [`ReplyFiles`] in [`super::HostTools::with_reply_files`] and put the files on
//! the reply body with [`ReplyFiles::attach`] before the tool trail.

use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use flow_like_bot_tools as tools;
use flow_like_types::app_chat::ChatFold;
use flow_like_types::bot_message::{
    MAX_FILE_NAME_CHARS, MAX_SUMMARY_CHARS, WorkspaceHandle, is_file_link, is_mime,
    is_workspace_path,
};
use flow_like_types::bot_reply_files::{
    MAX_INLINE_WIDGET_BYTES, MAX_REPLY_FILES, MAX_REPLY_WIDGETS, ReplyFile, ReplyWidget,
    app_widget_doc, attach_reply_files, attach_reply_widgets,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::platform::{Clearance, PlatformToolBridge};
use super::stream::stream_frame;
use super::tool_spec::{PlatformToolSpec, ToolApprovalSpec};
use super::turn_label::TurnLabel;

pub const FILE_ATTACH_TOOL: &str = "file_attach";
/// The stream frame that shows an attached file on the reply while the turn runs.
pub const REPLY_ATTACHMENT_FRAME: &str = "reply_attachment";
/// The stream frame that shows an app widget on the reply while the turn runs.
pub const REPLY_WIDGET_FRAME: &str = "reply_widget";
/// The query flag and result field through which the hub hands a device turn the [`AppOutput`]
/// of an app tool it ran for the device.
pub const REPLY_OUTPUT_FIELD: &str = "reply_output";

const TOOL_TIMEOUT_SECS: u64 = 180;
/// The tools whose app runs push [`AppOutput`]; only their results carry it to the reply, so a
/// call that runs alongside never takes another call's output.
const APP_OUTPUT_TOOLS: [&str; 2] = ["call_app_event", "call_app_chat"];
/// A widget document beyond this is shown while the turn runs but never stored.
const MAX_WIDGET_DOC_BYTES: usize = 2 * 1024 * 1024;
const WIDGET_DIR: &str = "chat-widgets";
const ON_REPLY_NOTE: &str = "These are on your reply already, shown to the person you answer. Mention them in your text; don't attach them again or paste their links.";

/// The app event whose board pushed a widget, and so runs the widget's actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppOrigin {
    pub app_id: String,
    pub board_id: String,
    pub event_id: String,
}

/// What one app call pushed to the chat: its files and its finished widgets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppOutput {
    pub origin: AppOrigin,
    /// Names the source in the widgets' summaries, such as the event's name.
    pub source: String,
    #[serde(default)]
    pub files: Vec<Value>,
    #[serde(default)]
    pub widgets: Vec<Value>,
}

impl AppOutput {
    /// The files and widgets a run pushed, the widgets with the updates sent after their push.
    pub fn from_fold(origin: AppOrigin, source: String, fold: &ChatFold) -> Self {
        Self {
            origin,
            source,
            files: fold.attachments(),
            widgets: crate::a2ui::chat_widget::finished_widgets(fold),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.widgets.is_empty()
    }
}

#[derive(Default)]
struct Reply {
    files: Vec<ReplyFile>,
    widgets: Vec<ReplyWidget>,
    pending: Vec<AppOutput>,
}

/// The files and app widgets of one turn's reply, in the order they joined it. App tools push
/// what an app showed with [`ReplyFiles::push_app_output`]; [`ReplyFilesBridge`] puts it on the
/// reply when the call returns.
#[derive(Clone, Default)]
pub struct ReplyFiles(Arc<Mutex<Reply>>);

impl ReplyFiles {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn files(&self) -> Vec<ReplyFile> {
        self.lock().files.clone()
    }

    pub fn widgets(&self) -> Vec<ReplyWidget> {
        self.lock().widgets.clone()
    }

    /// Puts the turn's files and widgets on its reply `body` as attachment and ui parts.
    pub fn attach(&self, body: &mut Value) {
        let (files, widgets) = {
            let reply = self.lock();
            (reply.files.clone(), reply.widgets.clone())
        };
        attach_reply_files(body, &files);
        attach_reply_widgets(body, &widgets);
    }

    /// Queues what an app call showed, for the reply once the call returns.
    pub fn push_app_output(&self, output: AppOutput) {
        if !output.is_empty() {
            self.lock().pending.push(output);
        }
    }

    /// The app output queued since the last take.
    pub fn take_app_output(&self) -> Vec<AppOutput> {
        std::mem::take(&mut self.lock().pending)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Reply> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Adds `file` unless the reply has it already; `Err` when the reply is full.
    fn add(&self, file: ReplyFile) -> Result<usize, String> {
        let files = &mut self.lock().files;
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

    /// Adds `widget`, replacing an earlier push of the same instance; `Err` when the reply is full.
    fn add_widget(&self, widget: ReplyWidget) -> Result<(), String> {
        let widgets = &mut self.lock().widgets;
        let instance = &widget.doc["widget"]["instance_id"];
        if let Some(known) = widgets
            .iter_mut()
            .find(|known| !instance.is_null() && known.doc["widget"]["instance_id"] == *instance)
        {
            *known = widget;
            return Ok(());
        }
        if widgets.len() >= MAX_REPLY_WIDGETS {
            return Err(format!(
                "this reply already carries {MAX_REPLY_WIDGETS} widgets, the most one reply can hold"
            ));
        }
        widgets.push(widget);
        Ok(())
    }
}

impl fmt::Debug for ReplyFiles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reply = self.lock();
        f.debug_struct("ReplyFiles")
            .field("files", &reply.files.len())
            .field("widgets", &reply.widgets.len())
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

    /// Puts what app calls pushed on the reply and says what joined it.
    async fn show(&self, outputs: Vec<AppOutput>) -> Value {
        if let Some(refused) = self
            .label
            .refusal(FILE_ATTACH_TOOL, &json!({ "url": "app output" }))
        {
            let reason = serde_json::from_str::<Value>(&refused)
                .ok()
                .and_then(|refused| refused["message"].as_str().map(str::to_string))
                .unwrap_or_default();
            return json!({ "withheld": reason });
        }
        let mut attached = Vec::new();
        let mut widgets = 0usize;
        let mut live_only = 0usize;
        let mut left_out = Vec::new();
        for output in outputs {
            for file in &output.files {
                match self.attach_pushed_file(file) {
                    Ok(name) => attached.push(name),
                    Err(reason) => left_out.push(reason),
                }
            }
            let origin = json!(output.origin);
            for widget in &output.widgets {
                match self
                    .attach_pushed_widget(&origin, &output.source, widget)
                    .await
                {
                    Ok(true) => widgets += 1,
                    Ok(false) => live_only += 1,
                    Err(reason) => left_out.push(reason),
                }
            }
        }
        let mut note = json!({ "note": ON_REPLY_NOTE });
        if !attached.is_empty() {
            note["attached_files"] = json!(attached);
        }
        if widgets > 0 {
            note["widgets"] = json!(widgets);
        }
        if live_only > 0 {
            note["widgets_shown_only_now"] = json!(live_only);
            note["widgets_shown_only_now_reason"] = json!(
                "too large to keep on the stored reply; the person sees it only while this reply is open"
            );
        }
        if !left_out.is_empty() {
            note["left_out"] = json!(left_out);
        }
        note
    }

    fn attach_pushed_file(&self, file: &Value) -> Result<String, String> {
        let url = file
            .as_str()
            .or_else(|| file["url"].as_str())
            .unwrap_or_default()
            .trim();
        let name = file["name"]
            .as_str()
            .map(str::trim)
            .filter(|name| valid_name(name))
            .map(str::to_string);
        let mime = file["type"]
            .as_str()
            .map(|mime| mime.trim().to_ascii_lowercase())
            .filter(|mime| is_mime(mime));
        let unusable = || {
            format!(
                "a file whose link is not an http(s) address ({})",
                shown_link(url)
            )
        };
        let file = linked(url, name, mime).map_err(|_| unusable())?;
        self.files.add(file.clone())?;
        if let Some(frames) = &self.frames {
            frames(stream_frame(REPLY_ATTACHMENT_FRAME, &frame_of(&file)));
        }
        Ok(file.name)
    }

    /// `Ok(true)` when the widget is kept on the stored reply, `Ok(false)` when it is only shown.
    async fn attach_pushed_widget(
        &self,
        origin: &Value,
        source: &str,
        widget: &Value,
    ) -> Result<bool, String> {
        let doc = app_widget_doc(origin, widget);
        let size = doc.to_string().len();
        if size > MAX_WIDGET_DOC_BYTES {
            return Err(format!(
                "a widget of {size} bytes, more than a reply can show ({MAX_WIDGET_DOC_BYTES})"
            ));
        }
        let handle = if size > MAX_INLINE_WIDGET_BYTES {
            self.saved_widget(&doc).await
        } else {
            None
        };
        let kept = size <= MAX_INLINE_WIDGET_BYTES || handle.is_some();
        let summary: String = format!("Widget from {source}")
            .chars()
            .take(MAX_SUMMARY_CHARS)
            .collect();
        self.files.add_widget(ReplyWidget {
            summary,
            doc,
            handle,
        })?;
        if let Some(frames) = &self.frames {
            frames(stream_frame(
                REPLY_WIDGET_FRAME,
                &json!({ "origin": origin, "widget": widget }),
            ));
        }
        Ok(kept)
    }

    /// The widget document as a file of the bot's workspace; `None` without one.
    async fn saved_widget(&self, doc: &Value) -> Option<WorkspaceHandle> {
        self.workspace.as_ref()?;
        let path = format!("{WIDGET_DIR}/{}.json", flow_like_types::create_id());
        let output = self
            .inner
            .call(
                tools::files::FILE_WRITE,
                json!({ "path": path, "content": doc.to_string(), "mime": "application/json" }),
            )
            .await;
        let result: Value = serde_json::from_str(&output).ok()?;
        if result.get("status").and_then(Value::as_str) != Some("ok") {
            tracing::warn!(
                path,
                error = %result,
                "a reply widget could not be stored in the bot's workspace"
            );
            return None;
        }
        serde_json::from_value(result.get("handle")?.clone()).ok()
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
        let output = self.inner.call(tool_name, arguments).await;
        if !APP_OUTPUT_TOOLS.contains(&tool_name) {
            return output;
        }
        let shown = self.files.take_app_output();
        if shown.is_empty() {
            return output;
        }
        self.label.observe(&output);
        let note = self.show(shown).await;
        with_reply_note(output, note)
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

/// Adds `note` to an object result as `reply`; any other result stays as it is.
fn with_reply_note(output: String, note: Value) -> String {
    match serde_json::from_str::<Value>(&output) {
        Ok(Value::Object(mut result)) => {
            result.insert("reply".to_string(), note);
            Value::Object(result).to_string()
        }
        _ => output,
    }
}

fn valid_name(name: &str) -> bool {
    (1..=MAX_FILE_NAME_CHARS).contains(&name.chars().count()) && !name.chars().any(char::is_control)
}

/// A complete handle of a file in the bot's own workspace.
fn own_handle(file: &Value, workspace: Option<&str>) -> Option<WorkspaceHandle> {
    let handle = serde_json::from_value::<WorkspaceHandle>(file.clone()).ok()?;
    (Some(handle.app_id.as_str()) == workspace && is_workspace_path(&handle.path)).then_some(handle)
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
    ) -> (
        ReplyFilesBridge,
        ReplyFiles,
        Arc<Host>,
        Arc<Mutex<Vec<String>>>,
    ) {
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
            Some(Arc::new(move |frame: String| {
                sink.lock().unwrap().push(frame)
            })),
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
            .call(
                FILE_ATTACH_TOOL,
                json!({ "file": own, "name": "Angebot.pdf" }),
            )
            .await;
        assert_eq!(status(&output).0, "ok");
        assert!(host.calls.lock().unwrap().is_empty());

        let output = bridge
            .call(FILE_ATTACH_TOOL, json!({ "file": "#12.a1" }))
            .await;
        assert_eq!(status(&output).0, "ok");
        let calls = host.calls.lock().unwrap().clone();
        assert_eq!(
            calls,
            [("file_save".to_string(), json!({ "file": "#12.a1" }))]
        );

        let attached = files.files();
        assert_eq!(attached[0].name, "Angebot.pdf");
        assert_eq!(attached[1].name, "scan.pdf");
        assert_eq!(
            attached[1].handle.as_ref().unwrap().path,
            "bot/files/scan.pdf"
        );
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
                .call(
                    FILE_ATTACH_TOOL,
                    json!({ "url": format!("https://h/{index}.pdf") }),
                )
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
        let (bridge, files, _, _) =
            harness(Some("ws1"), label(Some(DataClassification::Confidential)));
        let output = bridge
            .call(FILE_ATTACH_TOOL, json!({ "url": "https://h/a.pdf" }))
            .await;
        assert_eq!(status(&output).1, "turn_label_outbound_refused");
        let output = bridge
            .call(
                FILE_ATTACH_TOOL,
                json!({ "file": handle("ws1", "bot/a.pdf") }),
            )
            .await;
        assert_eq!(status(&output).0, "ok");
        assert_eq!(files.files().len(), 1);
    }

    /// An app tool that, like the hosts' app tools, pushes what the app run showed.
    struct AppHost {
        files: ReplyFiles,
        output: AppOutput,
        writes: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl PlatformToolBridge for AppHost {
        async fn call(&self, tool_name: &str, arguments: Value) -> String {
            match tool_name {
                "call_app_event" => {
                    self.files.push_app_output(self.output.clone());
                    json!({ "status": "ok", "result": "done" }).to_string()
                }
                "file_write" => {
                    let path = arguments["path"].as_str().unwrap().to_string();
                    self.writes.lock().unwrap().push(path.clone());
                    json!({ "status": "ok", "handle": WorkspaceHandle {
                        mime: "application/json".into(),
                        ..handle("ws1", &path)
                    } })
                    .to_string()
                }
                _ => json!({ "status": "ok" }).to_string(),
            }
        }
    }

    fn app_output(widget_bytes: &[usize]) -> AppOutput {
        AppOutput {
            origin: AppOrigin {
                app_id: "orders".into(),
                board_id: "board".into(),
                event_id: "event".into(),
            },
            source: "Order report".into(),
            files: vec![
                json!({ "url": "https://f.test/r.pdf?sig=1", "name": "Report.pdf", "type": "application/pdf" }),
                json!("storage://apps/orders/app/raw.csv"),
            ],
            widgets: widget_bytes
                .iter()
                .enumerate()
                .map(|(index, bytes)| {
                    json!({ "instance_id": format!("w{index}"), "component": { "blob": "x".repeat(*bytes) } })
                })
                .collect(),
        }
    }

    fn app_harness(
        workspace: Option<&str>,
        label: Arc<TurnLabel>,
        output: AppOutput,
    ) -> (
        ReplyFilesBridge,
        ReplyFiles,
        Arc<AppHost>,
        Arc<Mutex<Vec<String>>>,
    ) {
        let files = ReplyFiles::new();
        let host = Arc::new(AppHost {
            files: files.clone(),
            output,
            writes: Mutex::new(Vec::new()),
        });
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink = frames.clone();
        let bridge = ReplyFilesBridge::new(
            files.clone(),
            workspace.map(str::to_string),
            label,
            Some(Arc::new(move |frame: String| {
                sink.lock().unwrap().push(frame)
            })),
            host.clone(),
        );
        (bridge, files, host, frames)
    }

    #[tokio::test]
    async fn what_an_app_call_pushed_joins_the_reply_and_the_result_says_so() {
        let (bridge, files, host, frames) = app_harness(
            Some("ws1"),
            label(None),
            app_output(&[100, MAX_INLINE_WIDGET_BYTES + 10]),
        );
        let output = bridge.call("call_app_event", json!({})).await;
        let result: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(result["result"], "done");
        assert_eq!(result["reply"]["attached_files"], json!(["Report.pdf"]));
        assert_eq!(result["reply"]["widgets"], 2);
        assert_eq!(result["reply"]["left_out"].as_array().unwrap().len(), 1);
        assert_eq!(host.writes.lock().unwrap().len(), 1);

        let frames = frames.lock().unwrap().clone();
        assert_eq!(frames.len(), 3);
        assert!(frames[0].starts_with("<reply_attachment>"));
        assert!(frames[1].starts_with("<reply_widget>"));
        assert!(frames[1].contains("\"board_id\":\"board\""));

        let mut body = json!({ "text": "Here is the report" });
        files.attach(&mut body);
        let parsed = MessageBody::parse(&body).expect("valid body");
        let kinds: Vec<_> = parsed
            .parts
            .iter()
            .map(|part| part.id().to_string())
            .collect();
        assert_eq!(kinds, ["a1", "ui1", "ui2"]);
        assert_eq!(body["parts"][1]["a2ui"]["origin"]["app_id"], "orders");
        assert!(
            body["parts"][2]["handle"]["path"]
                .as_str()
                .unwrap()
                .starts_with("chat-widgets/")
        );

        let again = bridge.call("other_tool", json!({})).await;
        assert!(
            serde_json::from_str::<Value>(&again)
                .unwrap()
                .get("reply")
                .is_none()
        );

        files.push_app_output(app_output(&[10]));
        let unrelated = bridge.call("list_apps", json!({})).await;
        let unrelated: Value = serde_json::from_str(&unrelated).unwrap();
        assert!(unrelated.get("reply").is_none());
        assert_eq!(files.take_app_output().len(), 1);
    }

    #[tokio::test]
    async fn a_large_widget_without_a_workspace_is_shown_but_not_kept() {
        let (bridge, files, host, _) = app_harness(
            None,
            label(None),
            app_output(&[MAX_INLINE_WIDGET_BYTES + 10]),
        );
        let output = bridge.call("call_app_event", json!({})).await;
        let result: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(result["reply"]["widgets_shown_only_now"], 1);
        assert!(host.writes.lock().unwrap().is_empty());
        let mut body = json!({ "text": "x" });
        files.attach(&mut body);
        assert_eq!(
            body["parts"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|part| part["kind"] == "ui")
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn a_turn_above_the_bots_level_keeps_app_output_off_the_reply() {
        let (bridge, files, _, frames) = app_harness(
            Some("ws1"),
            label(Some(DataClassification::Confidential)),
            app_output(&[100]),
        );
        let output = bridge.call("call_app_event", json!({})).await;
        let result: Value = serde_json::from_str(&output).unwrap();
        assert!(
            result["reply"]["withheld"]
                .as_str()
                .unwrap()
                .contains("confidential")
        );
        assert!(files.files().is_empty() && files.widgets().is_empty());
        assert!(frames.lock().unwrap().is_empty());
    }
}

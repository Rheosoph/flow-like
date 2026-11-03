//! The bot tools from `flow-like-bot-tools` as core sees them: which are registered, which
//! capability switches each on, and which a host links and runs ([`HostTools`]).
//!
//! Core depends on the crate with no heavy features: its schemas are the single source of every
//! tool's arguments, and a test fails when the crate gains a tool nobody registered here. The
//! heavy work (code, documents, drivers) stays behind the crate's features, which only hosts
//! enable.

use std::collections::BTreeSet;

use flow_like_bot_tools as tools;
use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};
use flow_like_types::bot::BotCapability;
use serde_json::{Value, json};

use super::memory_tools::MemoryHandle;
use super::platform::PLATFORM_TOOL_IMAGE_URLS_FIELD;

use super::tool_spec::{PlatformToolSpec, ToolApprovalSpec, ToolApprovalTiming, bot_args_approval};

const TOOL_TIMEOUT_SECS: u64 = 180;

fn email_ask_message(args: &Value) -> String {
    let recipients = ["to", "cc"]
        .iter()
        .filter_map(|key| args.get(key).and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let subject = args
        .get("subject")
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!("The bot wants to email {recipients} with the subject '{subject}'.")
}

fn ask_message(tool: &str, args: &Value) -> String {
    if tool == "shell_run" {
        return shell_ask_message(args);
    }
    if tool == "email_send" {
        return email_ask_message(args);
    }
    let target = ["index", "path", "name", "url"]
        .iter()
        .find_map(|key| args.get(key).and_then(Value::as_str))
        .map(|value| format!(" '{value}'"))
        .unwrap_or_default();
    format!("The bot wants to run {tool}{target}.")
}

fn shell_ask_message(args: &Value) -> String {
    let command = args.get("command").and_then(Value::as_str).unwrap_or("?");
    let line = args
        .get("args")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(Value::as_str)
                .fold(command.to_string(), |line, part| format!("{line} {part}"))
        })
        .unwrap_or_else(|| command.to_string());
    let how = match args.get("shell").and_then(Value::as_bool) {
        Some(true) => " through a shell",
        _ => "",
    };
    format!("The bot wants to run the command `{line}`{how} on this device.")
}

macro_rules! registered_tools {
    ($($name:literal),+ $(,)?) => {
        const REGISTERED: &[&str] = &[$($name),+];

        fn schema_fn(name: &str) -> Option<fn() -> Value> {
            match name {
                $($name => Some((|| {
                    tools::spec($name).map_or(json!({ "type": "object" }), |spec| spec.input_schema)
                }) as fn() -> Value),)+
                _ => None,
            }
        }

        fn ask_fn(name: &str) -> Option<fn(&Value) -> String> {
            match name {
                $($name => Some((|args: &Value| ask_message($name, args)) as fn(&Value) -> String),)+
                _ => None,
            }
        }
    };
}

registered_tools!(
    "file_list",
    "file_read",
    "file_save",
    "file_write",
    "knowledge_create",
    "knowledge_add",
    "knowledge_search",
    "knowledge_list",
    "knowledge_drop",
    "file_summarize",
    "file_extract",
    "pdf_render",
    "excel_inspect",
    "data_sources",
    "data_query",
    "code_run",
    "notify_owner",
    "schedule_task",
    "list_tasks",
    "cancel_task",
    "shell_run",
    "browser_open",
    "browser_snapshot",
    "browser_act",
    "browser_screenshot",
    "browser_close",
    "computer_screenshot",
    "computer_act",
    "computer_wait",
    "email_send",
    "email_reply",
    "image_generate",
);

/// Tools that need the machine's own browser, screen or keyboard; `HostTools::linked` leaves
/// them out and `with_local_machine` adds them.
const LOCAL_MACHINE_TOOLS: &[&str] = &[
    "browser_open",
    "browser_snapshot",
    "browser_act",
    "browser_screenshot",
    "browser_close",
    "computer_screenshot",
    "computer_act",
    "computer_wait",
];

pub const TOOL_IMAGES_FIELD: &str = PLATFORM_TOOL_IMAGE_URLS_FIELD;

/// The text a host returns to the model for a finished tool: the envelope, with the outcome's
/// images moved into the field the tool loop turns into image parts of the result message
/// (up to 12 images, 8 MiB each, 24 MiB in total).
pub fn tool_outcome_output(outcome: &tools::ToolOutcome) -> String {
    let mut envelope = outcome.envelope();
    if !outcome.images.is_empty()
        && let Some(object) = envelope.as_object_mut()
    {
        let images: Vec<Value> = outcome
            .images
            .iter()
            .map(|image| json!({ "data": STANDARD.encode(&image.data), "media_type": image.mime }))
            .collect();
        object.insert(TOOL_IMAGES_FIELD.to_string(), Value::Array(images));
    }
    envelope.to_string()
}

/// The capability that switches `spec` on; `None` leaves the tool unoffered.
pub(super) fn required_capability(spec: &tools::ToolSpec) -> Option<BotCapability> {
    if let Some(id) = spec.capability {
        return BotCapability::parse(id);
    }
    match spec.name {
        "code_run" => return Some(BotCapability::CodeExecution),
        "image_generate" => return Some(BotCapability::ImageGeneration),
        _ => {}
    }
    match spec.module {
        "files" => Some(BotCapability::Storage),
        "knowledge" => Some(BotCapability::Knowledge),
        "code" => Some(BotCapability::CodeExecution),
        "data" => Some(BotCapability::DataStudio),
        "connectors" | "odbc" => Some(BotCapability::DataConnections),
        "documents" | "excel" => Some(BotCapability::Documents),
        "image" => Some(BotCapability::ImageGeneration),
        _ => None,
    }
}

/// The capability behind a registered tool that the bot-tools crate links in this build.
pub(super) fn tool_capability(name: &str) -> Option<BotCapability> {
    if !REGISTERED.contains(&name) {
        return None;
    }
    tools::spec(name).and_then(|spec| required_capability(&spec))
}

/// The core spec of a registered tool, with approval from the crate's own spec.
pub fn bot_tool_spec(name: &str) -> Option<PlatformToolSpec> {
    let schema = schema_fn(name)?;
    let spec = tools::spec(name)?;
    Some(PlatformToolSpec {
        name: spec.name,
        description: spec.description,
        schema,
        approval: match (spec.approval, ask_fn(name)) {
            (tools::Approval::Ask, Some(message)) => ToolApprovalSpec::Mutating {
                title: "Approve bot action",
                message,
                timing: ToolApprovalTiming::BeforeExecution,
            },
            _ if !spec.approval_args.is_empty() => bot_args_approval(),
            _ => ToolApprovalSpec::None,
        },
        timeout_secs: TOOL_TIMEOUT_SECS,
    })
}

/// Every registered tool this build of the bot-tools crate links, in registration order.
pub fn linked_tool_names() -> Vec<&'static str> {
    REGISTERED
        .iter()
        .copied()
        .filter(|name| tools::spec(name).is_some())
        .collect()
}

/// The bot-tools tools a host links and runs. A bot is offered a tool only when it holds the
/// tool's capability and the host's set contains it. Empty by default: a host that runs none of
/// them keeps today's behavior.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostTools {
    names: BTreeSet<&'static str>,
    code_languages: Vec<String>,
    memory: Option<MemoryHandle>,
}

impl HostTools {
    pub fn none() -> Self {
        Self::default()
    }

    /// Every tool the bot-tools crate links in this build: for a host that runs them all in
    /// process.
    pub fn linked() -> Self {
        Self {
            names: linked_tool_names()
                .into_iter()
                .filter(|name| !LOCAL_MACHINE_TOOLS.contains(name))
                .collect(),
            code_languages: Vec::new(),
            memory: None,
        }
    }

    /// Adds the tools that drive this machine's browser, screen and keyboard. Only a host with a
    /// desktop (and the backends behind `ToolEnv::browser` and `ToolEnv::computer`) asks for them.
    pub fn with_local_machine(mut self) -> Self {
        self.names.extend(
            linked_tool_names()
                .into_iter()
                .filter(|name| LOCAL_MACHINE_TOOLS.contains(name)),
        );
        self
    }

    /// Exactly the named tools that are registered and linked; unknown names are dropped.
    pub fn only<S: AsRef<str>>(names: &[S]) -> Self {
        Self {
            names: linked_tool_names()
                .into_iter()
                .filter(|linked| names.iter().any(|name| name.as_ref() == *linked))
                .collect(),
            code_languages: Vec::new(),
            memory: None,
        }
    }

    /// Runs the memory tools of a headless turn with `memory`; offered to bots that remember.
    pub fn with_memory(mut self, memory: MemoryHandle) -> Self {
        self.memory = Some(memory);
        self
    }

    pub fn memory(&self) -> Option<&MemoryHandle> {
        self.memory.as_ref()
    }

    /// The languages `code_run` runs on this host (`ToolEnv::code_languages`). None: the host
    /// can't run code, so `code_run` is not offered.
    pub fn with_code_languages<S: AsRef<str>>(mut self, languages: &[S]) -> Self {
        self.code_languages = languages.iter().map(|id| id.as_ref().to_string()).collect();
        if self.code_languages.is_empty() {
            self.names.remove("code_run");
        }
        self
    }

    /// Whether this host can run `shell_run` (it needs a process to start: not iOS, Android or a server).
    pub fn with_shell(mut self, available: bool) -> Self {
        if !available {
            self.names.remove("shell_run");
        }
        self
    }

    pub fn code_languages(&self) -> &[String] {
        &self.code_languages
    }

    pub fn runs(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.names.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_of_the_crate_is_registered_and_has_a_capability() {
        for spec in tools::specs() {
            assert!(
                REGISTERED.contains(&spec.name),
                "{} is not registered in capability_tools.rs",
                spec.name
            );
            assert!(
                required_capability(&spec).is_some(),
                "{} has no capability",
                spec.name
            );
            let core = bot_tool_spec(spec.name).expect(spec.name);
            assert_eq!((core.schema)(), spec.input_schema, "{}", spec.name);
        }
    }

    #[test]
    fn files_need_storage_and_knowledge_needs_knowledge() {
        assert_eq!(tool_capability("file_read"), Some(BotCapability::Storage));
        assert_eq!(
            tool_capability("knowledge_search"),
            Some(BotCapability::Knowledge)
        );
        assert_eq!(tool_capability("database_tool"), None);
    }

    #[test]
    fn only_knowledge_drop_and_tools_with_approval_args_have_an_approval() {
        for name in linked_tool_names() {
            let asks = !matches!(
                bot_tool_spec(name).expect(name).approval,
                ToolApprovalSpec::None
            );
            let outside = !tools::spec(name).expect(name).approval_args.is_empty();
            assert_eq!(
                asks,
                matches!(
                    name,
                    "knowledge_drop"
                        | "shell_run"
                        | "browser_open"
                        | "computer_screenshot"
                        | "computer_act"
                        | "email_send"
                ) || outside,
                "{name}"
            );
        }
    }

    #[test]
    fn browser_and_computer_tools_follow_their_capabilities() {
        for name in [
            "browser_open",
            "browser_snapshot",
            "browser_act",
            "browser_screenshot",
            "browser_close",
        ] {
            assert_eq!(
                tool_capability(name),
                Some(BotCapability::BrowserUse),
                "{name}"
            );
        }
        for name in ["computer_screenshot", "computer_act", "computer_wait"] {
            assert_eq!(
                tool_capability(name),
                Some(BotCapability::ComputerUse),
                "{name}"
            );
        }
    }

    #[test]
    fn outcome_images_become_the_tool_loop_image_field() {
        let mut output = tools::ToolOutput::new(json!({ "pages": 1 }));
        output.images.push(tools::ToolImage {
            label: "page 1".into(),
            mime: "image/png".into(),
            data: flow_like_types::Bytes::from_static(b"png"),
        });
        let outcome = tools::ToolOutcome::from_output("pdf_render", output, 16 * 1024);
        let text = tool_outcome_output(&outcome);
        let mut value: Value = serde_json::from_str(&text).unwrap();
        let images = super::super::platform::take_platform_tool_image_urls(&mut value);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].media_type, "image/png");
        assert_eq!(
            images[0].data.as_deref(),
            Some(STANDARD.encode(b"png").as_str())
        );
        assert!(value.get(TOOL_IMAGES_FIELD).is_none());
        assert_eq!(value["pages"], 1);
    }

    #[test]
    fn host_tools_keep_only_linked_names() {
        let tools = HostTools::only(&["file_read", "nonsense"]);
        assert!(tools.runs("file_read"));
        assert!(!tools.runs("nonsense"));
        assert!(!tools.runs("file_write"));
        assert!(HostTools::none().is_empty());
        let none: [&str; 0] = [];
        assert!(
            !HostTools::only(&["code_run"])
                .with_code_languages(&none)
                .runs("code_run")
        );
        let python = HostTools::only(&["code_run"]).with_code_languages(&["python"]);
        assert_eq!(python.code_languages(), ["python"]);
        assert!(python.runs("code_run") == linked_tool_names().contains(&"code_run"));
        assert!(HostTools::linked().runs("knowledge_drop"));
    }
}

//! Claude Code completions with Flow-Like owning history and tool execution.
//!
//! Each call starts a fresh CLI session and asks for a structured assistant turn.
//! Streaming emits the validated final turn, so it does not provide token latency.
//! Native CLI tools and MCP are disabled. Sampling parameters are not CLI controls.

use std::{any::Any, collections::HashSet, path::PathBuf, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use rig::{
    OneOrMany,
    completion::{
        CompletionError, CompletionModel, CompletionRequest, CompletionResponse, GetTokenUsage,
        Message, Usage,
    },
    message::{AssistantContent, ToolChoice, ToolResultContent, UserContent},
    streaming::{RawStreamingChoice, RawStreamingToolCall, StreamingCompletionResponse},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{ModelConstructor, ModelLogic};
use crate::provider::ModelProvider;

const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 64 * 1024;
const MAX_PROMPT_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOOL_CALLS: usize = 32;
const SYSTEM_PROMPT: &str = "You produce one assistant turn for the Flow-Like host. The JSON request on stdin is the complete conversation, including system instructions, prior assistant tool calls, and tool results. Respect those message roles. Answer the latest user turn using that history and the supplied documents. Return the required envelope: text is assistant response text; tool_calls contains only requested host function calls, with a fresh unique id, an exact registered name, and arguments_json encoding a JSON object matching that tool's parameter schema. The host executes these calls and will supply their results in a later request. Never execute tools yourself or invent their results. Follow tool_choice: none forbids calls, required requires at least one, and specific restricts calls to the named functions and requires at least one. When output_schema is set and no tool call is needed, text must encode JSON conforming to it. Sampling parameters in the request are metadata, not instructions. Return only the next assistant turn.";

pub struct ClaudeCodeModel {
    client: ClaudeCodeClient,
    default_model: String,
}

impl ClaudeCodeModel {
    pub async fn from_provider(provider: &ModelProvider) -> Result<Self> {
        #[cfg(target_family = "wasm")]
        bail!("Claude Code requires a native executor with the Claude CLI installed");
        #[cfg(not(target_family = "wasm"))]
        {
            let params = provider.params.as_ref();
            let default_model = params
                .and_then(|params| params.get("model_id"))
                .and_then(Value::as_str)
                .filter(|model| !model.trim().is_empty())
                .map(str::to_owned)
                .or_else(|| provider.model_id.clone())
                .filter(|model| !model.trim().is_empty())
                .unwrap_or_else(|| "default".to_owned());
            let timeout_seconds = match params.and_then(|params| params.get("timeout_seconds")) {
                Some(value) => value
                    .as_u64()
                    .filter(|seconds| (1..=1800).contains(seconds))
                    .ok_or_else(|| {
                        anyhow!("Claude Code timeout_seconds must be an integer from 1 to 1800")
                    })?,
                None => 300,
            };
            Ok(Self {
                client: ClaudeCodeClient {
                    executable: resolve_executable(provider)?,
                    timeout: Duration::from_secs(timeout_seconds),
                },
                default_model,
            })
        }
    }
}

/// Checks the installed CLI's authentication status without generating a completion.
pub async fn check_available(provider: &ModelProvider) -> Result<()> {
    #[cfg(target_family = "wasm")]
    bail!("Claude Code requires a native executor with the Claude CLI installed");
    #[cfg(not(target_family = "wasm"))]
    {
        let executable = resolve_executable(provider)?;
        let output = run_process(
            &executable,
            &["auth".into(), "status".into()],
            None,
            Duration::from_secs(8),
        )
        .await?;
        let authenticated = serde_json::from_slice::<Value>(&output.stdout)
            .ok()
            .and_then(|status| status.get("loggedIn").and_then(Value::as_bool))
            == Some(true);
        if !output.success || !authenticated {
            bail!(
                "Claude Code is unavailable or signed out. Run `claude auth status` and `claude auth login` on the executor"
            );
        }
        Ok(())
    }
}

impl Cacheable for ClaudeCodeModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for ClaudeCodeModel {
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor {
            inner: Box::new(self.client.clone()),
        })
    }

    async fn default_model(&self) -> Option<String> {
        Some(self.default_model.clone())
    }
}

#[derive(Clone)]
struct ClaudeCodeClient {
    executable: PathBuf,
    timeout: Duration,
}

impl rig::client::CompletionClient for ClaudeCodeClient {
    type CompletionModel = ClaudeCodeCompletion;
}

#[derive(Clone)]
struct ClaudeCodeCompletion {
    client: ClaudeCodeClient,
    model: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct ClaudeCodeResponse {
    model: String,
    usage: Option<Usage>,
    finish_reason: String,
}

impl GetTokenUsage for ClaudeCodeResponse {
    fn token_usage(&self) -> Option<Usage> {
        self.usage
    }
}

impl CompletionModel for ClaudeCodeCompletion {
    type Response = ClaudeCodeResponse;
    type StreamingResponse = ClaudeCodeResponse;
    type Client = ClaudeCodeClient;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        Self {
            client: client.clone(),
            model: model.into(),
        }
    }

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> std::result::Result<CompletionResponse<Self::Response>, CompletionError> {
        self.complete(request)
            .await
            .map_err(|error| CompletionError::ProviderError(error.to_string()))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> std::result::Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError>
    {
        let response = self.completion(request).await?;
        let mut events = Vec::new();
        for content in response.choice {
            match content {
                AssistantContent::Text(text) => {
                    events.push(Ok(RawStreamingChoice::Message(text.text)));
                }
                AssistantContent::ToolCall(call) => {
                    events.push(Ok(RawStreamingChoice::ToolCall(RawStreamingToolCall::new(
                        call.id,
                        call.function.name,
                        call.function.arguments,
                    ))));
                }
                _ => {}
            }
        }
        events.push(Ok(RawStreamingChoice::FinalResponse(response.raw_response)));
        Ok(StreamingCompletionResponse::stream(Box::pin(
            futures::stream::iter(events),
        )))
    }
}

impl ClaudeCodeCompletion {
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<ClaudeCodeResponse>> {
        validate_request(&request)?;
        #[cfg(target_family = "wasm")]
        bail!("Claude Code requires a native executor with the Claude CLI installed");
        #[cfg(not(target_family = "wasm"))]
        {
            let model = request.model.as_deref().unwrap_or(&self.model);
            let prompt = serde_json::to_vec(&request)?;
            if prompt.len() > MAX_PROMPT_BYTES {
                bail!("Claude Code request exceeds the 8 MiB input limit");
            }
            let args = completion_args(model, &request)?;
            let output = run_process(
                &self.client.executable,
                &args,
                Some(prompt),
                self.client.timeout,
            )
            .await?;
            if !output.success {
                bail!(
                    "Claude Code completion failed (exit {}). Check CLI authentication, model access, and support for --json-schema",
                    output
                        .code
                        .map_or_else(|| "signal".into(), |code| code.to_string())
                );
            }
            parse_response(&output.stdout, &request, model)
        }
    }
}

fn completion_args(model: &str, request: &CompletionRequest) -> Result<Vec<String>> {
    // --bare skips OAuth/keychain login. Disable ambient settings and execution
    // capabilities individually so the CLI can retain its subscription login.
    let mut args = [
        "--print",
        "--output-format",
        "json",
        "--input-format",
        "text",
        "--tools",
        "",
        "--disallowedTools",
        "mcp__*",
        "--strict-mcp-config",
        "--mcp-config",
        "{\"mcpServers\":{}}",
        "--permission-mode",
        "dontAsk",
        "--setting-sources",
        "",
        "--settings",
        "{\"disableAllHooks\":true,\"autoMemoryEnabled\":false}",
        "--disable-slash-commands",
        "--no-session-persistence",
        "--no-chrome",
        "--max-turns",
        "3",
        "--system-prompt",
        SYSTEM_PROMPT,
        "--json-schema",
    ]
    .map(str::to_owned)
    .to_vec();
    args.push(serde_json::to_string(&envelope_schema(request))?);
    if !model.trim().is_empty() && model != "default" {
        args.extend(["--model".to_owned(), model.to_owned()]);
    }
    Ok(args)
}

fn envelope_schema(request: &CompletionRequest) -> Value {
    let names: Vec<&str> = request
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect();
    let name_schema = if names.is_empty() {
        json!({"type":"string"})
    } else {
        json!({"type":"string", "enum":names})
    };
    json!({
        "type":"object", "additionalProperties":false,
        "required":["text","tool_calls"],
        "properties":{
            "text":{"type":"string"},
            "tool_calls":{
                "type":"array", "maxItems":if names.is_empty() || matches!(request.tool_choice, Some(ToolChoice::None)) { 0 } else { MAX_TOOL_CALLS },
                "items":{
                    "type":"object", "additionalProperties":false,
                    "required":["id","name","arguments_json"],
                    "properties":{
                        "id":{"type":"string"}, "name":name_schema,
                        "arguments_json":{"type":"string"}
                    }
                }
            }
        }
    })
}

fn validate_request(request: &CompletionRequest) -> Result<()> {
    let mut names = HashSet::new();
    for tool in &request.tools {
        if tool.name.trim().is_empty() || !names.insert(tool.name.as_str()) {
            bail!("Claude Code requires distinct, nonempty tool names");
        }
        jsonschema::validator_for(&tool.parameters)
            .map_err(|error| anyhow!("Invalid parameter schema for tool {}: {error}", tool.name))?;
    }
    if let Some(ToolChoice::Specific { function_names }) = &request.tool_choice {
        if function_names.is_empty()
            || function_names
                .iter()
                .any(|name| !names.contains(name.as_str()))
        {
            bail!("Claude Code tool_choice names an unregistered tool");
        }
    }
    if matches!(request.tool_choice, Some(ToolChoice::Required)) && names.is_empty() {
        bail!("Claude Code tool_choice requires a tool but no tools are registered");
    }
    if let Some(schema) = &request.output_schema {
        jsonschema::validator_for(schema.as_value())
            .map_err(|error| anyhow!("Invalid Claude Code output schema: {error}"))?;
    }
    for message in request.chat_history.iter() {
        let media = match message {
            Message::User { content } => content.iter().any(|content| match content {
                UserContent::Text(_) => false,
                UserContent::ToolResult(result) => result
                    .content
                    .iter()
                    .any(|content| matches!(content, ToolResultContent::Image(_))),
                _ => true,
            }),
            Message::Assistant { content, .. } => content
                .iter()
                .any(|content| matches!(content, AssistantContent::Image(_))),
            Message::System { .. } => false,
        };
        if media {
            bail!(
                "Claude Code model completions support text and text tool results; image, audio, video and document attachments are unsupported"
            );
        }
    }
    if request
        .additional_params
        .as_ref()
        .and_then(|params| params.get("tools"))
        .is_some()
    {
        bail!("Claude Code model completions do not support provider-hosted tools");
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssistantTurn {
    text: String,
    tool_calls: Vec<RequestedTool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestedTool {
    id: String,
    name: String,
    arguments_json: String,
}

fn parse_response(
    stdout: &[u8],
    request: &CompletionRequest,
    model: &str,
) -> Result<CompletionResponse<ClaudeCodeResponse>> {
    let response: Value =
        serde_json::from_slice(stdout).context("Claude Code did not return a JSON result")?;
    if response.get("is_error").and_then(Value::as_bool) == Some(true)
        || response
            .get("subtype")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "success")
    {
        bail!("Claude Code returned a failed result; no tool calls were accepted");
    }
    let turn: AssistantTurn = serde_json::from_value(response.get("structured_output").cloned()
        .ok_or_else(|| anyhow!("Claude Code returned no structured_output; update the CLI to a version supporting --json-schema"))?)
        .context("Claude Code returned an invalid assistant envelope")?;
    if turn.tool_calls.len() > MAX_TOOL_CALLS {
        bail!("Claude Code returned too many tool calls");
    }
    let has_tools = !turn.tool_calls.is_empty();
    match &request.tool_choice {
        Some(ToolChoice::None) if has_tools => {
            bail!("Claude Code returned tool calls despite tool_choice=none")
        }
        Some(ToolChoice::Required | ToolChoice::Specific { .. }) if !has_tools => {
            bail!("Claude Code did not return a required tool call")
        }
        _ => {}
    }
    let mut ids: HashSet<String> = request
        .chat_history
        .iter()
        .flat_map(|message| match message {
            Message::Assistant { content, .. } => content
                .iter()
                .filter_map(|item| match item {
                    AssistantContent::ToolCall(call) => Some(call.id.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            Message::User { content } => content
                .iter()
                .filter_map(|item| match item {
                    UserContent::ToolResult(result) => Some(result.id.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    let mut choice = Vec::new();
    if !turn.text.is_empty() || !has_tools {
        if !has_tools && let Some(schema) = &request.output_schema {
            let value: Value = serde_json::from_str(&turn.text)
                .context("Claude Code text is not the requested JSON output")?;
            let validator = jsonschema::validator_for(schema.as_value())?;
            if !validator.is_valid(&value) {
                bail!("Claude Code text does not match the requested output schema");
            }
        }
        choice.push(AssistantContent::text(turn.text));
    }
    for call in turn.tool_calls {
        if call.id.is_empty()
            || call.id.len() > 128
            || !call
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            || !ids.insert(call.id.clone())
        {
            bail!("Claude Code returned an invalid or reused tool call id");
        }
        let tool = request
            .tools
            .iter()
            .find(|tool| tool.name == call.name)
            .ok_or_else(|| anyhow!("Claude Code requested an unregistered tool"))?;
        if let Some(ToolChoice::Specific { function_names }) = &request.tool_choice
            && !function_names.contains(&call.name)
        {
            bail!("Claude Code requested a tool excluded by tool_choice");
        }
        let arguments: Value = serde_json::from_str(&call.arguments_json)
            .context("Claude Code returned invalid tool argument JSON")?;
        let validator = jsonschema::validator_for(&tool.parameters)?;
        if !arguments.is_object() || !validator.is_valid(&arguments) {
            bail!("Claude Code returned arguments that do not match the registered tool schema");
        }
        choice.push(AssistantContent::tool_call(call.id, call.name, arguments));
    }
    let usage = response
        .get("usage")
        .filter(|usage| usage.is_object())
        .map(|usage| {
            let number = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
            let cached = number("cache_read_input_tokens");
            let created = number("cache_creation_input_tokens");
            let input = number("input_tokens")
                .saturating_add(cached)
                .saturating_add(created);
            let output = number("output_tokens");
            Usage {
                input_tokens: input,
                output_tokens: output,
                total_tokens: input.saturating_add(output),
                cached_input_tokens: cached,
                cache_creation_input_tokens: created,
                ..Usage::new()
            }
        });
    Ok(CompletionResponse {
        choice: OneOrMany::many(choice)
            .map_err(|_| anyhow!("Claude Code returned an empty assistant turn"))?,
        usage: usage.unwrap_or_default(),
        raw_response: ClaudeCodeResponse {
            model: model.to_owned(),
            usage,
            finish_reason: if has_tools { "tool_calls" } else { "stop" }.to_owned(),
        },
        message_id: None,
    })
}

#[cfg(not(target_family = "wasm"))]
fn resolve_executable(provider: &ModelProvider) -> Result<PathBuf> {
    let explicit = provider
        .params
        .as_ref()
        .and_then(|params| params.get("executable"));
    if let Some(value) = explicit {
        let path = value
            .as_str()
            .filter(|path| !path.trim().is_empty())
            .ok_or_else(|| anyhow!("Claude Code executable must be a nonempty absolute path"))?;
        let path = PathBuf::from(path);
        if !path.is_absolute() || !is_executable(&path) {
            bail!("Claude Code executable must identify an executable file by absolute path");
        }
        return Ok(path);
    }
    if let Some(path) = std::env::var_os("CLAUDE_CODE_CLI_PATH") {
        let path = PathBuf::from(path);
        if path.is_absolute() && is_executable(&path) {
            return Ok(path);
        }
        bail!("CLAUDE_CODE_CLI_PATH must identify an executable file by absolute path");
    }
    let mut dirs = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|path| path.is_absolute())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        dirs.push(PathBuf::from(home).join(".local/bin"));
    }
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    let name = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    dirs.into_iter().map(|dir| dir.join(name)).find(|path| is_executable(path))
        .ok_or_else(|| anyhow!("Claude Code CLI was not found. Install it on the executor or configure an absolute executable path"))
}

#[cfg(not(target_family = "wasm"))]
fn is_executable(path: &std::path::Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(not(target_family = "wasm"))]
struct ProcessOutput {
    success: bool,
    code: Option<i32>,
    stdout: Vec<u8>,
}

#[cfg(not(target_family = "wasm"))]
struct WorkingDirectory(PathBuf);

#[cfg(not(target_family = "wasm"))]
impl WorkingDirectory {
    fn new() -> Result<Self> {
        let path =
            std::env::temp_dir().join(format!("flow-like-claude-{:032x}", rand::random::<u128>()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&path)
            .context("Could not create Claude Code working directory")?;
        Ok(Self(path))
    }
}

#[cfg(not(target_family = "wasm"))]
impl Drop for WorkingDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(not(target_family = "wasm"))]
async fn read_bounded(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    limit: usize,
) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut output = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(count) > limit {
            bail!("Claude Code process output exceeded its size limit");
        }
        output.extend_from_slice(&buffer[..count]);
    }
}

#[cfg(not(target_family = "wasm"))]
async fn run_process(
    executable: &std::path::Path,
    args: &[String],
    prompt: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<ProcessOutput> {
    use std::process::Stdio;
    use tokio::io::AsyncWriteExt;
    let cwd = WorkingDirectory::new()?;
    let mut child = tokio::process::Command::new(executable)
        .args(args)
        .current_dir(&cwd.0)
        .env("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1")
        .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", "1")
        .env("CLAUDE_CODE_DISABLE_ATTACHMENTS", "1")
        .env("CLAUDE_CODE_AUTO_CONNECT_IDE", "false")
        .env("ENABLE_CLAUDEAI_MCP_SERVERS", "false")
        .env("DISABLE_AUTOUPDATER", "1")
        .env_remove("CLAUDE_CODE_SIMPLE")
        .stdin(if prompt.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("Could not start Claude Code CLI")?;
    let stdin = child.stdin.take();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Claude Code stdout is unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("Claude Code stderr is unavailable"))?;
    let exchange = async {
        let write = async {
            if let (Some(mut stdin), Some(prompt)) = (stdin, prompt) {
                stdin.write_all(&prompt).await?;
                stdin.shutdown().await?;
            }
            Ok::<_, anyhow::Error>(())
        };
        let wait = async { child.wait().await.map_err(anyhow::Error::from) };
        let (_, stdout, _, status) = tokio::try_join!(
            write,
            read_bounded(stdout, MAX_OUTPUT_BYTES),
            read_bounded(stderr, MAX_ERROR_BYTES),
            wait
        )?;
        Ok::<_, anyhow::Error>(ProcessOutput {
            success: status.success(),
            code: status.code(),
            stdout,
        })
    };
    match tokio::time::timeout(timeout, exchange).await {
        Ok(Ok(output)) => Ok(output),
        outcome => {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
            match outcome {
                Ok(Err(error)) => Err(error),
                Err(_) => Err(anyhow!(
                    "Claude Code timed out after {} seconds",
                    timeout.as_secs()
                )),
                Ok(Ok(_)) => unreachable!(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::{client::CompletionClient, completion::ToolDefinition};

    fn model() -> ClaudeCodeCompletion {
        ClaudeCodeClient {
            executable: PathBuf::from("/unused/claude"),
            timeout: Duration::from_secs(5),
        }
        .completion_model("sonnet")
    }

    fn request() -> CompletionRequest {
        model().completion_request("What is the weather in Berlin?")
            .tools(vec![ToolDefinition {
                name: "weather".into(),
                description: "Look up weather".into(),
                parameters: json!({"type":"object","properties":{"city":{"type":"string"}},"required":["city"],"additionalProperties":false}),
            }]).build()
    }

    fn tool_result(name: &str, id: &str, arguments: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "type":"result", "subtype":"success", "is_error":false,
            "structured_output":{"text":"", "tool_calls":[{"id":id,"name":name,"arguments_json":arguments}]},
            "usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":3,"cache_creation_input_tokens":2}
        })).unwrap()
    }

    #[test]
    fn validates_tool_arguments_and_reports_actual_cli_usage() {
        let response = parse_response(
            &tool_result("weather", "call_1", r#"{"city":"Berlin"}"#),
            &request(),
            "sonnet",
        )
        .unwrap();
        assert_eq!(response.usage.total_tokens, 20);
        assert_eq!(response.usage.cached_input_tokens, 3);
        let AssistantContent::ToolCall(call) = response.choice.first() else {
            panic!("tool call expected")
        };
        assert_eq!(call.id, "call_1");
        assert_eq!(call.function.arguments, json!({"city":"Berlin"}));
        for (name, id, arguments) in [
            ("Bash", "call_1", r#"{"city":"Berlin"}"#),
            ("weather", "", r#"{"city":"Berlin"}"#),
            ("weather", "call 1", r#"{"city":"Berlin"}"#),
            ("weather", "call_1", r#"{"city":4}"#),
            ("weather", "call_1", r#"{"city":"Berlin","extra":true}"#),
            ("weather", "call_1", "[]"),
            ("weather", "call_1", "invalid"),
        ] {
            assert!(
                parse_response(&tool_result(name, id, arguments), &request(), "sonnet").is_err()
            );
        }
    }

    #[test]
    fn enforces_tool_choice_and_prevents_id_reuse() {
        let output = tool_result("weather", "call_1", r#"{"city":"Berlin"}"#);
        let mut request = request();
        request.tool_choice = Some(ToolChoice::None);
        assert!(parse_response(&output, &request, "sonnet").is_err());
        request.tool_choice = Some(ToolChoice::Specific {
            function_names: vec!["other".into()],
        });
        assert!(validate_request(&request).is_err());
        assert!(parse_response(&output, &request, "sonnet").is_err());
        request.tool_choice = Some(ToolChoice::Required);
        let text = br#"{"structured_output":{"text":"done","tool_calls":[]}}"#;
        assert!(parse_response(text, &request, "sonnet").is_err());
        request.tool_choice = None;
        request.chat_history = OneOrMany::many([
            Message::Assistant {
                id: None,
                content: OneOrMany::one(AssistantContent::tool_call(
                    "call_1",
                    "weather",
                    json!({"city":"Berlin"}),
                )),
            },
            Message::tool_result("call_1", "Sunny"),
        ])
        .unwrap();
        assert!(parse_response(&output, &request, "sonnet").is_err());
        assert!(
            parse_response(
                &tool_result("weather", "call_2", r#"{"city":"Berlin"}"#),
                &request,
                "sonnet"
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_failed_or_unstructured_cli_results() {
        for response in [
            json!({"result":"plain text"}),
            json!({"is_error":true,"structured_output":{"text":"pretend success","tool_calls":[]}}),
            json!({"subtype":"error_max_turns","structured_output":{"text":"partial","tool_calls":[]}}),
            json!({"structured_output":{"text":"done","tool_calls":[],"unexpected":"value"}}),
        ] {
            assert!(
                parse_response(
                    &serde_json::to_vec(&response).unwrap(),
                    &request(),
                    "sonnet"
                )
                .is_err()
            );
        }
    }

    #[test]
    fn output_schema_is_validated_before_text_is_returned() {
        let mut request = request();
        request.output_schema = Some(
            json!({"type":"object","properties":{"count":{"type":"integer"}},"required":["count"]})
                .try_into()
                .unwrap(),
        );
        let response = |text: &str| {
            serde_json::to_vec(&json!({"structured_output":{"text":text,"tool_calls":[]}})).unwrap()
        };
        assert!(parse_response(&response(r#"{"count":4}"#), &request, "sonnet").is_ok());
        assert!(parse_response(&response(r#"{"count":"four"}"#), &request, "sonnet").is_err());
    }

    #[test]
    fn cli_does_not_receive_execution_tools_or_session_resume_flags() {
        let args = completion_args("sonnet", &request()).unwrap();
        let value = |flag: &str| {
            args.windows(2)
                .find(|pair| pair[0] == flag)
                .map(|pair| pair[1].as_str())
        };
        assert_eq!(value("--tools"), Some(""));
        assert_eq!(value("--disallowedTools"), Some("mcp__*"));
        assert_eq!(value("--mcp-config"), Some(r#"{"mcpServers":{}}"#));
        assert_eq!(value("--setting-sources"), Some(""));
        assert!(args.iter().any(|arg| arg == "--strict-mcp-config"));
        assert!(args.iter().any(|arg| arg == "--no-session-persistence"));
        for forbidden in [
            "--resume",
            "--continue",
            "--bare",
            "--dangerously-skip-permissions",
        ] {
            assert!(!args.iter().any(|arg| arg == forbidden));
        }
    }

    #[cfg(unix)]
    fn fake_cli(script: &str) -> (WorkingDirectory, ClaudeCodeClient) {
        use std::os::unix::fs::PermissionsExt;
        let dir = WorkingDirectory::new().unwrap();
        let path = dir.0.join("claude");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        (
            dir,
            ClaudeCodeClient {
                executable: path,
                timeout: Duration::from_secs(5),
            },
        )
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_receives_history_and_stream_returns_rig_tool_calls() {
        use futures::StreamExt;
        use rig::streaming::StreamedAssistantContent;
        let output =
            String::from_utf8(tool_result("weather", "call_2", r#"{"city":"Berlin"}"#)).unwrap();
        let (_dir, client) = fake_cli(&format!(
            "printf '%s\\n' \"$@\" > \"$0.args\"\ncat > \"$0.stdin\"\nprintf '%s' '{output}'"
        ));
        let model = client.completion_model("sonnet");
        let request = model
            .completion_request("and tomorrow?")
            .messages(vec![
                Message::Assistant {
                    id: None,
                    content: OneOrMany::one(AssistantContent::tool_call(
                        "call_1",
                        "weather",
                        json!({"city":"Berlin"}),
                    )),
                },
                Message::tool_result("call_1", "Sunny"),
            ])
            .tools(request().tools)
            .build();
        let expected = serde_json::to_value(&request).unwrap();
        let mut stream = model.stream(request).await.unwrap();
        let mut tool_calls = 0;
        while let Some(event) = stream.next().await {
            if matches!(event.unwrap(), StreamedAssistantContent::ToolCall { .. }) {
                tool_calls += 1;
            }
        }
        assert_eq!(tool_calls, 1);
        let input: Value = serde_json::from_slice(
            &std::fs::read(format!("{}.stdin", client.executable.display())).unwrap(),
        )
        .unwrap();
        assert_eq!(input, expected);
        assert_eq!(stream.response.unwrap().usage.unwrap().total_tokens, 20);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn process_timeout_and_bounded_output_fail_without_a_response() {
        let (_dir, client) = fake_cli("exec sleep 30");
        let result = run_process(&client.executable, &[], None, Duration::from_millis(50)).await;
        assert!(result.err().unwrap().to_string().contains("timed out"));
        let output = vec![b'x'; 10];
        assert!(read_bounded(output.as_slice(), 8).await.is_err());
        assert_eq!(read_bounded(output.as_slice(), 10).await.unwrap(), output);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn availability_probe_does_not_generate_or_expose_auth_details() {
        let (_dir, client) = fake_cli(
            "[ \"$1\" = auth ] && [ \"$2\" = status ] || exit 2\nprintf '{\"loggedIn\":true,\"email\":\"private@example.test\"}'",
        );
        let provider = ModelProvider {
            provider_name: "custom:claude-code".into(),
            model_id: Some("sonnet".into()),
            version: None,
            api_surface: None,
            params: Some(std::collections::HashMap::from([(
                "executable".into(),
                json!(client.executable),
            )])),
        };
        check_available(&provider).await.unwrap();
        let loaded = ClaudeCodeModel::from_provider(&provider).await.unwrap();
        assert_eq!(loaded.default_model().await.as_deref(), Some("sonnet"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn availability_requires_affirmative_authentication_json() {
        for output in [r#"{"loggedIn":false}"#, "", "{}", "not-json"] {
            let (_dir, client) = fake_cli(&format!("printf '%s' '{output}'"));
            let provider = ModelProvider {
                provider_name: "custom:claude-code".into(),
                model_id: None,
                version: None,
                api_surface: None,
                params: Some(std::collections::HashMap::from([(
                    "executable".into(),
                    json!(client.executable),
                )])),
            };
            assert!(check_available(&provider).await.is_err());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rig_executes_host_tool_and_replays_its_result_on_the_next_turn() {
        use rig::completion::Prompt;
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct WeatherTool(Arc<AtomicUsize>);
        #[derive(Deserialize)]
        struct WeatherArgs {
            city: String,
        }
        impl rig::tool::Tool for WeatherTool {
            const NAME: &'static str = "weather";
            type Error = std::convert::Infallible;
            type Args = WeatherArgs;
            type Output = String;
            async fn definition(&self, _prompt: String) -> ToolDefinition {
                request().tools.remove(0)
            }
            async fn call(&self, args: Self::Args) -> std::result::Result<String, Self::Error> {
                assert_eq!(args.city, "Berlin");
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok("HOST_WEATHER_RESULT_SUNNY".to_owned())
            }
        }
        let first =
            String::from_utf8(tool_result("weather", "weather_1", r#"{"city":"Berlin"}"#)).unwrap();
        let second = json!({"subtype":"success", "structured_output":{"text":"It is sunny in Berlin.","tool_calls":[]}}).to_string();
        let (_dir, client) = fake_cli(&format!(
            "input=$(cat)\nprintf '%s\\n' \"$input\" >> \"$0.turns\"\ncase \"$input\" in\n*HOST_WEATHER_RESULT_SUNNY*) printf '%s' '{second}' ;;\n*) printf '%s' '{first}' ;;\nesac"
        ));
        let count = Arc::new(AtomicUsize::new(0));
        let agent = client
            .agent("sonnet")
            .tool(WeatherTool(count.clone()))
            .build();
        let reply = agent
            .prompt("What is the weather in Berlin?")
            .max_turns(2)
            .await
            .unwrap();
        assert_eq!(reply, "It is sunny in Berlin.");
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let turns =
            std::fs::read_to_string(format!("{}.turns", client.executable.display())).unwrap();
        let turns: Vec<Value> = turns
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(turns.len(), 2);
        let replayed = &turns[1]["chat_history"];
        assert!(replayed.to_string().contains("weather_1"));
        assert!(replayed.to_string().contains("HOST_WEATHER_RESULT_SUNNY"));
    }
}

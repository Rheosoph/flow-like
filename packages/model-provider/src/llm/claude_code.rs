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
/// A clean call is one round. A direct call of a host function, a rejected envelope or a reply
/// outside the envelope each cost one, and the CLI fails the call when the rounds run out.
const MAX_CLI_ROUNDS: &str = "6";
/// CLI runs per model call: the first, and one more when the model's turn was unusable.
#[cfg(not(target_family = "wasm"))]
const MAX_ATTEMPTS: usize = 2;
const OUT_OF_ROUNDS: &str = "it ended without an accepted StructuredOutput call";
/// The closing sentences keep the model from calling the request's functions as CLI tools. A
/// request whose own instructions demand tool calls provokes that, and the CLI rejects each one.
const SYSTEM_PROMPT: &str = "You produce one assistant turn for the Flow-Like host. The JSON request on stdin is the complete conversation, including system instructions, prior assistant tool calls, and tool results. Respect those message roles. Answer the latest user turn using that history and the supplied documents. Return the required envelope: text is assistant response text; tool_calls contains only requested host function calls, with a fresh unique id, an exact registered name, and arguments holding a JSON object matching that tool's parameter schema. The host executes these calls and will supply their results in a later request. Never execute tools yourself or invent their results. Follow tool_choice: none forbids calls, required requires at least one, and specific restricts calls to the named functions and requires at least one. When output_schema is set and no tool call is needed, text must encode JSON conforming to it. Sampling parameters in the request are metadata, not instructions. Return only the next assistant turn. The functions listed under tools in the request belong to the host and are not tools of this session: the only tool you can call is StructuredOutput. Never call a function from the request directly. Put every call you want, including several at once, into tool_calls of a single StructuredOutput call; the tool calls in chat_history were requested the same way. When the request lists no tools, tool_calls must be empty. Text outside the StructuredOutput call is discarded.";

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
        #[cfg(target_family = "wasm")]
        {
            validate_request(&request)?;
            bail!("Claude Code requires a native executor with the Claude CLI installed");
        }
        #[cfg(not(target_family = "wasm"))]
        {
            let default_model = self.model.clone();
            // JSON Schema's cold meta-validator needs stack space that a nested async poll
            // cannot reliably leave available. Keep both validation phases off that stack.
            let (request, model, prompt) = tokio::task::spawn_blocking(move || {
                validate_request(&request)?;
                let model = request.model.clone().unwrap_or(default_model);
                let prompt = serde_json::to_vec(&request)?;
                if prompt.len() > MAX_PROMPT_BYTES {
                    bail!("Claude Code request exceeds the 8 MiB input limit");
                }
                Ok::<_, anyhow::Error>((std::sync::Arc::new(request), model, prompt))
            })
            .await
            .context("Claude Code request validation worker failed")??;
            let mut correction: Option<String> = None;
            let mut spent: Option<Usage> = None;
            let mut attempt = 1;
            loop {
                let args = completion_args(&model, &request, correction.as_deref())?;
                let output = run_process(
                    &self.client.executable,
                    &args,
                    Some(prompt.clone()),
                    self.client.timeout,
                )
                .await?;
                let (attempt_request, attempt_model) = (request.clone(), model.clone());
                let outcome = tokio::task::spawn_blocking(move || {
                    read_attempt(&output, &attempt_request, &attempt_model)
                })
                .await
                .context("Claude Code response validation worker failed")?;
                let error = match outcome {
                    Ok(mut response) => {
                        if let Some(spent) = spent {
                            response.usage += spent;
                            response.raw_response.usage = Some(response.usage);
                        }
                        return Ok(response);
                    }
                    Err(error) => error,
                };
                match error.downcast_ref::<Rejected>() {
                    Some(rejected) if attempt < MAX_ATTEMPTS => {
                        tracing::warn!(
                            model = %model,
                            attempt,
                            reason = %rejected.message,
                            "Claude Code turn rejected; asking once more"
                        );
                        if let Some(usage) = rejected.usage {
                            *spent.get_or_insert_default() += usage;
                        }
                        correction = Some(rejected.correction.clone());
                        attempt += 1;
                    }
                    _ => return Err(error),
                }
            }
        }
    }
}

/// A turn the model can get right on another attempt. `correction` goes into that attempt's
/// system prompt, so it is host-written: it never quotes the model's output, and it names a
/// tool or schema keyword only as a plain identifier.
#[derive(Debug)]
#[cfg_attr(target_family = "wasm", allow(dead_code))]
struct Rejected {
    message: String,
    correction: String,
    usage: Option<Usage>,
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Rejected {}

fn ran_out_of_rounds(result: &Value) -> bool {
    matches!(
        result.get("subtype").and_then(Value::as_str),
        Some("error_max_turns" | "error_max_structured_output_retries")
    )
}

#[cfg(not(target_family = "wasm"))]
fn read_attempt(
    output: &ProcessOutput,
    request: &CompletionRequest,
    model: &str,
) -> Result<CompletionResponse<ClaudeCodeResponse>> {
    if output.success {
        return parse_response(&output.stdout, request, model);
    }
    let code = output
        .code
        .map_or_else(|| "signal".into(), |code| code.to_string());
    let message = match cli_failure_detail(&output.stdout, &output.stderr) {
        Some(detail) => {
            format!("Claude Code completion failed (exit {code}, model {model}): {detail}")
        }
        None => format!(
            "Claude Code completion failed (exit {code}, model {model}) without output. Check CLI authentication, model access, and support for --json-schema"
        ),
    };
    match serde_json::from_slice::<Value>(&output.stdout) {
        Ok(result) if ran_out_of_rounds(&result) => Err(Rejected {
            message,
            correction: OUT_OF_ROUNDS.to_owned(),
            usage: reported_usage(&result),
        }
        .into()),
        _ => bail!(message),
    }
}

fn completion_args(
    model: &str,
    request: &CompletionRequest,
    correction: Option<&str>,
) -> Result<Vec<String>> {
    let system_prompt = match correction {
        Some(correction) => format!(
            "{SYSTEM_PROMPT} The host rejected your previous attempt at this turn because {correction}. Correct that in this attempt."
        ),
        None => SYSTEM_PROMPT.to_owned(),
    };
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
        MAX_CLI_ROUNDS,
    ]
    .map(str::to_owned)
    .to_vec();
    args.extend([
        "--system-prompt".to_owned(),
        system_prompt,
        "--json-schema".to_owned(),
        serde_json::to_string(&envelope_schema(request))?,
    ]);
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
                    "required":["id","name","arguments"],
                    "properties":{
                        "id":{"type":"string"}, "name":name_schema,
                        "arguments":{"type":"object"}
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

const MAX_FAILURE_DETAIL_CHARS: usize = 800;

/// The CLI's own reason for a failed run: the result's subtype, text and errors from stdout, then
/// the end of stderr.
fn cli_failure_detail(stdout: &[u8], stderr: &[u8]) -> Option<String> {
    let mut parts = Vec::new();
    if let Ok(result) = serde_json::from_slice::<Value>(stdout) {
        if let Some(kind) = result
            .get("subtype")
            .and_then(Value::as_str)
            .filter(|kind| *kind != "success")
        {
            parts.push(kind.to_owned());
        }
        if let Some(text) = result.get("result").and_then(Value::as_str) {
            parts.push(text.trim().to_owned());
        }
        if let Some(errors) = result.get("errors").and_then(Value::as_array) {
            parts.extend(errors.iter().map(|error| match error {
                Value::String(text) => text.trim().to_owned(),
                other => other.to_string(),
            }));
        }
    } else {
        let text = String::from_utf8_lossy(stdout);
        parts.push(text.trim().to_owned());
    }
    let stderr = String::from_utf8_lossy(stderr);
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        let skip = stderr
            .chars()
            .count()
            .saturating_sub(MAX_FAILURE_DETAIL_CHARS);
        parts.push(stderr.chars().skip(skip).collect());
    }
    parts.retain(|part| !part.is_empty());
    if parts.is_empty() {
        return None;
    }
    let detail = parts.join(" | ");
    if detail.chars().count() <= MAX_FAILURE_DETAIL_CHARS {
        return Some(detail);
    }
    let mut cut: String = detail.chars().take(MAX_FAILURE_DETAIL_CHARS).collect();
    cut.push('…');
    Some(cut)
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
    arguments: Value,
}

fn parse_response(
    stdout: &[u8],
    request: &CompletionRequest,
    model: &str,
) -> Result<CompletionResponse<ClaudeCodeResponse>> {
    let response: Value =
        serde_json::from_slice(stdout).context("Claude Code did not return a JSON result")?;
    let usage = reported_usage(&response);
    let reject = |message: &str, correction: &str| -> anyhow::Error {
        Rejected {
            message: message.to_owned(),
            correction: correction.to_owned(),
            usage,
        }
        .into()
    };
    if response.get("is_error").and_then(Value::as_bool) == Some(true)
        || response
            .get("subtype")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "success")
    {
        let detail = cli_failure_detail(stdout, &[]).unwrap_or_else(|| "no reason given".into());
        let message =
            format!("Claude Code returned a failed result ({detail}); no tool calls were accepted");
        if ran_out_of_rounds(&response) {
            return Err(reject(&message, OUT_OF_ROUNDS));
        }
        bail!(message);
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
            return Err(reject(
                "Claude Code returned tool calls despite tool_choice=none",
                "it requested tool calls although tool_choice forbids them",
            ));
        }
        Some(ToolChoice::Required | ToolChoice::Specific { .. }) if !has_tools => {
            return Err(reject(
                "Claude Code did not return a required tool call",
                "it requested no tool call although tool_choice requires one",
            ));
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
            const CORRECTION: &str = "its text was not JSON matching output_schema";
            let Ok(value) = serde_json::from_str::<Value>(&turn.text) else {
                return Err(reject(
                    "Claude Code text is not the requested JSON output",
                    CORRECTION,
                ));
            };
            let validator = jsonschema::validator_for(schema.as_value())?;
            if !validator.is_valid(&value) {
                return Err(reject(
                    "Claude Code text does not match the requested output schema",
                    CORRECTION,
                ));
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
            return Err(reject(
                "Claude Code returned an invalid or reused tool call id",
                "a tool call id was empty, longer than 128 characters, used characters other than letters, digits, _ and -, or repeated an id from chat_history",
            ));
        }
        let Some(tool) = request.tools.iter().find(|tool| tool.name == call.name) else {
            return Err(reject(
                "Claude Code requested an unregistered tool",
                "it requested a function that the request does not register",
            ));
        };
        if let Some(ToolChoice::Specific { function_names }) = &request.tool_choice
            && !function_names.contains(&call.name)
        {
            return Err(reject(
                "Claude Code requested a tool excluded by tool_choice",
                "it requested a function that tool_choice excludes",
            ));
        }
        let validator = jsonschema::validator_for(&tool.parameters)?;
        let mismatch = if call.arguments.is_object() {
            validator
                .iter_errors(&call.arguments)
                .next()
                .map(|error| error.schema_path.to_string())
        } else {
            Some("/type".to_owned())
        };
        if let Some(schema_path) = mismatch {
            return Err(reject(
                &format!(
                    "Claude Code returned arguments for {} that do not match the registered tool schema at {schema_path}",
                    tool.name
                ),
                &arguments_correction(&tool.name, &schema_path),
            ));
        }
        choice.push(AssistantContent::tool_call(
            call.id,
            call.name,
            call.arguments,
        ));
    }
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

fn reported_usage(result: &Value) -> Option<Usage> {
    let usage = result.get("usage").filter(|usage| usage.is_object())?;
    let number = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    let cached = number("cache_read_input_tokens");
    let created = number("cache_creation_input_tokens");
    let input = number("input_tokens")
        .saturating_add(cached)
        .saturating_add(created);
    let output = number("output_tokens");
    Some(Usage {
        input_tokens: input,
        output_tokens: output,
        total_tokens: input.saturating_add(output),
        cached_input_tokens: cached,
        cache_creation_input_tokens: created,
        ..Usage::new()
    })
}

/// Names the tool and the failed schema keyword only when both are plain identifiers: a tool's
/// name and its schema's property names come from whoever registered the tool.
fn arguments_correction(tool: &str, schema_path: &str) -> String {
    let plain = |text: &str, extra: &[char]| {
        (1..=64).contains(&text.len())
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
    };
    let tool = if plain(tool, &['_', '-', '.']) {
        tool
    } else {
        "a function"
    };
    match schema_path
        .rsplit('/')
        .next()
        .filter(|keyword| plain(keyword, &[]))
    {
        Some(keyword) => format!(
            "the arguments for {tool} did not match its parameter schema (failed keyword: {keyword})"
        ),
        None => format!("the arguments for {tool} did not match its parameter schema"),
    }
}

#[cfg(not(target_family = "wasm"))]
fn resolve_executable(provider: &ModelProvider) -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let mut dirs = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|path| path.is_absolute())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(home) = &home {
        dirs.push(home.join(".local/bin"));
    }
    dirs.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    resolve_executable_with_paths(
        provider,
        std::env::var_os("CLAUDE_CODE_CLI_PATH").as_deref(),
        &dirs,
        home.as_deref(),
    )
}

#[cfg(not(target_family = "wasm"))]
fn resolve_executable_with_paths(
    provider: &ModelProvider,
    executable_override: Option<&std::ffi::OsStr>,
    dirs: &[PathBuf],
    home: Option<&std::path::Path>,
) -> Result<PathBuf> {
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
    if let Some(path) = executable_override {
        let path = PathBuf::from(path);
        if path.is_absolute() && is_executable(&path) {
            return Ok(path);
        }
        bail!("CLAUDE_CODE_CLI_PATH must identify an executable file by absolute path");
    }
    dirs.iter()
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(claude_binary_name()))
        .find(|path| is_executable(path))
        .or_else(|| {
            home.filter(|home| home.is_absolute())
                .and_then(|home| claude_ide_extension_binaries(home).into_iter().next())
        })
        .ok_or_else(|| anyhow!("Claude Code CLI was not found. Install it on the executor or configure an absolute executable path"))
}

#[cfg(not(target_family = "wasm"))]
fn claude_binary_name() -> &'static str {
    if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    }
}

/// Native Claude Code executables from installed IDE extensions, newest version first.
/// Shared with desktop CLI discovery so custom Bits can use an extension-only installation.
#[cfg(not(target_family = "wasm"))]
pub fn claude_ide_extension_binaries(home: &std::path::Path) -> Vec<PathBuf> {
    let mut extension_dirs = Vec::new();
    for editor in [
        ".vscode",
        ".vscode-insiders",
        ".vscode-oss",
        ".cursor",
        ".windsurf",
    ] {
        let Ok(entries) = std::fs::read_dir(home.join(editor).join("extensions")) else {
            continue;
        };
        extension_dirs.extend(entries.flatten().map(|entry| entry.path()).filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("anthropic.claude-code-"))
        }));
    }
    // Compare numeric versions across editors: 2.1.204 is newer than 2.1.9.
    extension_dirs.sort_by_key(|dir| std::cmp::Reverse(claude_extension_version_key(dir)));
    extension_dirs
        .into_iter()
        .map(|dir| {
            dir.join("resources/native-binary")
                .join(claude_binary_name())
        })
        .filter(|path| is_executable(path))
        .collect()
}

#[cfg(not(target_family = "wasm"))]
fn claude_extension_version_key(path: &std::path::Path) -> Vec<u64> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("anthropic.claude-code-"))
        .map(|rest| {
            rest.split('-')
                .next()
                .unwrap_or_default()
                .split('.')
                .map(|part| part.parse::<u64>().unwrap_or(0))
                .collect()
        })
        .unwrap_or_default()
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
    stderr: Vec<u8>,
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
        let (_, stdout, stderr, status) = tokio::try_join!(
            write,
            read_bounded(stdout, MAX_OUTPUT_BYTES),
            read_bounded(stderr, MAX_ERROR_BYTES),
            wait
        )?;
        Ok::<_, anyhow::Error>(ProcessOutput {
            success: status.success(),
            code: status.code(),
            stdout,
            stderr,
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

    fn tool_result(name: &str, id: &str, arguments: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "type":"result", "subtype":"success", "is_error":false,
            "structured_output":{"text":"", "tool_calls":[{"id":id,"name":name,"arguments":arguments}]},
            "usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":3,"cache_creation_input_tokens":2}
        })).unwrap()
    }

    #[test]
    fn failed_runs_carry_the_cli_reason() {
        let max_turns = serde_json::to_vec(&json!({
            "type":"result", "subtype":"error_max_turns", "is_error":true,
            "errors":["Reached maximum number of turns (3)"]
        }))
        .unwrap();
        assert_eq!(
            cli_failure_detail(&max_turns, b"").as_deref(),
            Some("error_max_turns | Reached maximum number of turns (3)")
        );
        let too_long = serde_json::to_vec(&json!({
            "type":"result", "subtype":"success", "is_error":true, "result":"Prompt is too long"
        }))
        .unwrap();
        assert_eq!(
            cli_failure_detail(&too_long, b"").as_deref(),
            Some("Prompt is too long")
        );
        assert_eq!(
            cli_failure_detail(b"", b"error: unknown option '--json-schema'\n").as_deref(),
            Some("error: unknown option '--json-schema'")
        );
        assert_eq!(cli_failure_detail(b"", b"  \n"), None);
        let noisy = "x".repeat(5_000);
        let detail = cli_failure_detail(b"", noisy.as_bytes()).unwrap();
        assert_eq!(detail.chars().count(), MAX_FAILURE_DETAIL_CHARS);
        let Err(error) = parse_response(&too_long, &request(), "sonnet") else {
            panic!("a failed result must not be accepted");
        };
        assert!(error.to_string().contains("Prompt is too long"), "{error}");
    }

    #[test]
    fn validates_tool_arguments_and_reports_actual_cli_usage() {
        let response = parse_response(
            &tool_result("weather", "call_1", json!({"city":"Berlin"})),
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
            ("Bash", "call_1", json!({"city":"Berlin"})),
            ("weather", "", json!({"city":"Berlin"})),
            ("weather", "call 1", json!({"city":"Berlin"})),
            ("weather", "call_1", json!({"city":4})),
            ("weather", "call_1", json!({"city":"Berlin","extra":true})),
            ("weather", "call_1", json!([])),
            ("weather", "call_1", json!(r#"{"city":"Berlin"}"#)),
        ] {
            assert!(
                parse_response(&tool_result(name, id, arguments), &request(), "sonnet").is_err()
            );
        }
    }

    #[test]
    fn envelope_takes_tool_arguments_as_an_object() {
        let envelope = jsonschema::validator_for(&envelope_schema(&request())).unwrap();
        let accepts = |arguments: Value| {
            envelope.is_valid(&json!({
                "text":"", "tool_calls":[{"id":"call_1","name":"weather","arguments":arguments}]
            }))
        };
        assert!(accepts(json!({"city":"Berlin"})));
        // A JSON-encoded string nests quotes one level deeper, which the model gets wrong.
        assert!(!accepts(json!(r#"{"city":"Berlin"}"#)));
    }

    #[test]
    fn enforces_tool_choice_and_prevents_id_reuse() {
        let output = tool_result("weather", "call_1", json!({"city":"Berlin"}));
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
                &tool_result("weather", "call_2", json!({"city":"Berlin"})),
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
        let args = completion_args("sonnet", &request(), None).unwrap();
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

    #[cfg(not(target_family = "wasm"))]
    fn write_discovery_executable(path: &std::path::Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn executable_discovery_uses_newest_extension_after_standalone_paths() {
        let home = WorkingDirectory::new().unwrap();
        let extension_binary = |editor: &str, version: &str| {
            home.0
                .join(editor)
                .join("extensions")
                .join(format!("anthropic.claude-code-{version}"))
                .join("resources/native-binary")
                .join(claude_binary_name())
        };
        let older = extension_binary(".vscode", "2.1.9");
        let newest = extension_binary(".cursor", "2.1.204");
        write_discovery_executable(&older);
        write_discovery_executable(&newest);
        // An incomplete extension install and an unrelated extension are not CLI candidates.
        std::fs::create_dir_all(extension_binary(".vscode", "99.0.0")).unwrap();
        write_discovery_executable(
            &home
                .0
                .join(".vscode/extensions/other.extension/resources/native-binary")
                .join(claude_binary_name()),
        );
        assert_eq!(
            claude_ide_extension_binaries(&home.0),
            vec![newest.clone(), older]
        );
        let provider = ModelProvider {
            provider_name: "custom:claude-code".into(),
            model_id: Some("sonnet".into()),
            version: None,
            api_surface: None,
            params: None,
        };
        let standalone_dir = home.0.join(".local/bin");
        let dirs = [standalone_dir.clone()];
        assert_eq!(
            resolve_executable_with_paths(&provider, None, &dirs, Some(&home.0)).unwrap(),
            newest
        );
        let standalone = standalone_dir.join(claude_binary_name());
        write_discovery_executable(&standalone);
        assert_eq!(
            resolve_executable_with_paths(&provider, None, &dirs, Some(&home.0)).unwrap(),
            standalone
        );
        assert!(resolve_executable_with_paths(&provider, None, &[], None).is_err());
    }

    #[cfg(not(target_family = "wasm"))]
    #[test]
    fn explicit_executable_and_environment_override_remain_authoritative() {
        let home = WorkingDirectory::new().unwrap();
        let standalone = home.0.join(claude_binary_name());
        let selected = home.0.join("selected").join(claude_binary_name());
        write_discovery_executable(&standalone);
        write_discovery_executable(&selected);
        let mut provider = ModelProvider {
            provider_name: "custom:claude-code".into(),
            model_id: Some("sonnet".into()),
            version: None,
            api_surface: None,
            params: None,
        };
        let dirs = [home.0.clone()];
        assert_eq!(
            resolve_executable_with_paths(
                &provider,
                Some(selected.as_os_str()),
                &dirs,
                Some(&home.0)
            )
            .unwrap(),
            selected
        );
        let missing = home.0.join("missing");
        assert!(
            resolve_executable_with_paths(
                &provider,
                Some(missing.as_os_str()),
                &dirs,
                Some(&home.0)
            )
            .is_err()
        );
        provider.params = Some(std::collections::HashMap::from([(
            "executable".into(),
            json!(selected),
        )]));
        assert_eq!(
            resolve_executable_with_paths(
                &provider,
                Some(standalone.as_os_str()),
                &dirs,
                Some(&home.0)
            )
            .unwrap(),
            selected
        );
        provider.params = Some(std::collections::HashMap::from([(
            "executable".into(),
            json!(missing),
        )]));
        assert!(
            resolve_executable_with_paths(
                &provider,
                Some(standalone.as_os_str()),
                &dirs,
                Some(&home.0)
            )
            .is_err()
        );
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
    #[test]
    fn cold_schema_validation_fits_default_async_worker_stack() {
        const CHILD: &str = "FLOW_LIKE_CLAUDE_SCHEMA_STACK_TEST";
        if std::env::var_os(CHILD).as_deref() != Some(std::ffi::OsStr::new("1")) {
            // Other tests warm jsonschema's global meta-validator. A fresh process keeps this
            // first-completion regression independent of test order and confines a stack abort.
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "llm::claude_code::tests::cold_schema_validation_fits_default_async_worker_stack",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .env_remove("RUST_MIN_STACK")
                .output()
                .unwrap();
            assert!(
                output.status.success()
                    && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "Cold Claude schema validation failed on a 2 MiB async worker:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }

        #[inline(never)]
        fn poll_with_application_stack<F: std::future::Future>(
            future: std::pin::Pin<&mut F>,
            context: &mut std::task::Context<'_>,
        ) -> std::task::Poll<F::Output> {
            // Flux reaches the provider through nested agent, tool and stream polls.
            let caller_stack = [0_u8; 1024 * 1024];
            std::hint::black_box(&caller_stack);
            let result = future.poll(context);
            std::hint::black_box(&caller_stack);
            result
        }

        async fn nested_completion(
            model: &ClaudeCodeCompletion,
            request: CompletionRequest,
        ) -> std::result::Result<CompletionResponse<ClaudeCodeResponse>, CompletionError> {
            let mut future = Box::pin(model.completion(request));
            std::future::poll_fn(|context| poll_with_application_stack(future.as_mut(), context))
                .await
        }

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_stack_size(2 * 1024 * 1024)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            tokio::spawn(async {
                let arguments = json!({"app_id":"fixture_app", "detail":{"include":["boards"]}});
                let valid = String::from_utf8(tool_result(
                    "get_app_detail", "stack_call", arguments.clone(),
                )).unwrap();
                let invalid = String::from_utf8(tool_result(
                    "get_app_detail", "stack_call", json!({"app_id":1,"detail":{}}),
                )).unwrap();
                let (_dir, client) = fake_cli(&format!(r#"input=$(cat)
printf '%s\n' called >> "$0.calls"
case "$input" in
  *invalid_arguments*) printf '%s' '{invalid}' ;;
  *) printf '%s' '{valid}' ;;
esac"#));
                let model = client.completion_model("selected-stack-test-model");
                let request = |prompt: &str| model.completion_request(prompt).tools(vec![ToolDefinition {
                    name: "get_app_detail".into(),
                    description: "Read an app and its requested details".into(),
                    parameters: json!({
                        "$schema":"https://json-schema.org/draft/2020-12/schema",
                        "$defs":{"details":{
                            "type":"object", "additionalProperties":false,
                            "properties":{"include":{"type":"array", "items":{"enum":["boards","metadata"]}}}
                        }},
                        "type":"object", "additionalProperties":false,
                        "properties":{
                            "app_id":{"type":"string", "pattern":"^[a-z][a-z_]+$"},
                            "detail":{"$ref":"#/$defs/details"}
                        },
                        "required":["app_id","detail"]
                    }),
                }]).build();
                let response = nested_completion(&model, request("Read fixture_app")).await.unwrap();
                let AssistantContent::ToolCall(call) = response.choice.first() else {
                    panic!("validated host tool call expected")
                };
                assert_eq!(call.function.name, "get_app_detail");
                assert_eq!(call.function.arguments, arguments);
                assert_eq!(response.raw_response.model, "selected-stack-test-model");
                assert_eq!(response.usage.output_tokens, 5);

                let error = nested_completion(&model, request("invalid_arguments"))
                    .await.err().unwrap().to_string();
                assert!(error.contains("do not match the registered tool schema"), "{error}");
                let mut malformed = request("invalid_schema");
                malformed.tools[0].parameters["type"] = json!("not_a_json_schema_type");
                let error = nested_completion(&model, malformed).await.err().unwrap().to_string();
                assert!(error.contains("Invalid parameter schema"), "{error}");
                let calls = std::fs::read_to_string(client.executable.with_extension("calls")).unwrap();
                assert_eq!(
                    calls.lines().count(),
                    1 + MAX_ATTEMPTS,
                    "Rejected arguments get every attempt; invalid schemas must fail before invoking the CLI"
                );
            }).await.unwrap();
        });
    }

    /// A CLI that answers `outputs[n]` with exit code `codes[n]` on its n-th run, repeating the
    /// last pair, and records each run's system prompt.
    #[cfg(unix)]
    fn scripted_cli(outputs: &[String], codes: &[i32]) -> (WorkingDirectory, ClaudeCodeClient) {
        let mut cases = String::new();
        for (index, (output, code)) in outputs.iter().zip(codes).enumerate() {
            let pattern = if index + 1 == outputs.len() {
                "*".to_owned()
            } else {
                (index + 1).to_string()
            };
            cases.push_str(&format!(
                "  {pattern}) printf '%s' '{output}'; exit {code} ;;\n"
            ));
        }
        fake_cli(&format!(
            r#"run=$(($(cat "$0.runs" 2>/dev/null || echo 0) + 1))
echo "$run" > "$0.runs"
while [ "$#" -gt 0 ]; do
  if [ "$1" = --system-prompt ]; then
    shift
    printf '%s' "$1" > "$0.prompt.$run"
  fi
  shift
done
cat > /dev/null
case "$run" in
{cases}esac"#
        ))
    }

    #[cfg(unix)]
    fn runs(client: &ClaudeCodeClient) -> usize {
        std::fs::read_to_string(client.executable.with_extension("runs"))
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_unusable_turn_gets_one_corrected_attempt() {
        let text = |bytes: Vec<u8>| String::from_utf8(bytes).unwrap();
        let valid = text(tool_result("weather", "call_1", json!({"city":"Berlin"})));
        let wrong_arguments = text(tool_result("weather", "call_1", json!({"city":4})));
        let out_of_rounds = json!({
            "type":"result", "subtype":"error_max_turns", "is_error":true,
            "errors":["Reached maximum number of turns (6)"],
            "usage":{"input_tokens":7,"output_tokens":3}
        })
        .to_string();

        let (_dir, client) = scripted_cli(&[wrong_arguments.clone(), valid.clone()], &[0, 0]);
        let response = client
            .completion_model("sonnet")
            .completion(request())
            .await
            .unwrap();
        let AssistantContent::ToolCall(call) = response.choice.first() else {
            panic!("tool call expected")
        };
        assert_eq!(call.function.arguments, json!({"city":"Berlin"}));
        assert_eq!(runs(&client), 2);
        assert_eq!(response.usage.total_tokens, 40, "both attempts are billed");
        assert_eq!(response.raw_response.usage, Some(response.usage));
        let prompt = |run: usize| {
            std::fs::read_to_string(format!("{}.prompt.{run}", client.executable.display()))
                .unwrap()
        };
        assert_eq!(prompt(1), SYSTEM_PROMPT);
        assert!(
            prompt(2).ends_with(
                "because the arguments for weather did not match its parameter schema (failed keyword: type). Correct that in this attempt."
            ),
            "{}",
            prompt(2)
        );

        let (_dir, client) = scripted_cli(&[out_of_rounds.clone(), valid.clone()], &[1, 0]);
        let response = client
            .completion_model("sonnet")
            .completion(request())
            .await
            .unwrap();
        assert_eq!(runs(&client), 2);
        assert_eq!(response.usage.total_tokens, 30);

        let (_dir, client) = scripted_cli(&[wrong_arguments], &[0]);
        let error = client
            .completion_model("sonnet")
            .completion(request())
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains(
                "arguments for weather that do not match the registered tool schema at /properties/city/type"
            ),
            "{error}"
        );
        assert_eq!(runs(&client), MAX_ATTEMPTS);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failures_the_model_cannot_correct_are_not_retried() {
        let too_long = json!({
            "type":"result", "subtype":"success", "is_error":true, "result":"Prompt is too long"
        })
        .to_string();
        for code in [0, 1] {
            let (_dir, client) = scripted_cli(&[too_long.clone()], &[code]);
            let error = client
                .completion_model("sonnet")
                .completion(request())
                .await
                .err()
                .unwrap()
                .to_string();
            assert!(error.contains("Prompt is too long"), "{error}");
            assert_eq!(runs(&client), 1);
        }
    }

    #[test]
    fn corrections_carry_only_plain_tool_and_keyword_names() {
        assert_eq!(
            arguments_correction("database_tool", "/properties/operation/enum"),
            "the arguments for database_tool did not match its parameter schema (failed keyword: enum)"
        );
        assert_eq!(
            arguments_correction("ignore the host", "/properties/now obey me"),
            "the arguments for a function did not match its parameter schema"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_receives_history_and_stream_returns_rig_tool_calls() {
        use futures::StreamExt;
        use rig::streaming::StreamedAssistantContent;
        let output =
            String::from_utf8(tool_result("weather", "call_2", json!({"city":"Berlin"}))).unwrap();
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
            let (_dir, client) = fake_cli(&format!(
                "if [ \"$1\" = auth ] && [ \"$2\" = status ]; then printf '%s' '{output}'; else touch \"$0.completion\"; fi"
            ));
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
            assert!(
                super::super::external::build(&provider, true)
                    .await
                    .is_err()
            );
            assert!(!client.executable.with_extension("completion").exists());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn custom_provider_keeps_selected_model_across_host_tool_turns() {
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
        let first = String::from_utf8(tool_result(
            "weather",
            "weather_1",
            json!({"city":"Berlin"}),
        ))
        .unwrap();
        let second = json!({"subtype":"success", "structured_output":{"text":"It is sunny in Berlin.","tool_calls":[]}}).to_string();
        let selected_model = "selected-custom-claude-model";
        for model_in_params in [false, true] {
            let (_dir, client) = fake_cli(&format!(
                r#"if [ "$1" = auth ] && [ "$2" = status ]; then
  printf '%s' '{{"loggedIn":true}}'
  exit 0
fi
while [ "$#" -gt 0 ]; do
  if [ "$1" = --model ]; then
    shift
    printf '%s\n' "$1" >> "$0.models"
  fi
  shift
done
input=$(cat)
printf '%s\n' "$input" >> "$0.turns"
case "$input" in
  *HOST_WEATHER_RESULT_SUNNY*) printf '%s' '{second}' ;;
  *) printf '%s' '{first}' ;;
esac"#
            ));
            let mut params = std::collections::HashMap::from([
                ("executable".into(), json!(client.executable)),
                ("timeout_seconds".into(), json!(5)),
            ]);
            if model_in_params {
                params.insert("model_id".into(), json!(selected_model));
            }
            let provider = ModelProvider {
                provider_name: "custom:claude-code".into(),
                model_id: (!model_in_params).then(|| selected_model.to_owned()),
                version: None,
                api_surface: None,
                params: Some(params),
            };
            let loaded = super::super::external::build(&provider, true)
                .await
                .unwrap();
            let model_id = loaded.default_model().await.unwrap();
            assert_eq!(model_id, selected_model);
            let constructor = loaded.provider().await.unwrap();
            let count = Arc::new(AtomicUsize::new(0));
            let agent = constructor
                .client()
                .agent(&model_id)
                .tool(WeatherTool(count.clone()))
                .build();
            let reply = agent
                .prompt("What is the weather in Berlin?")
                .max_turns(2)
                .await
                .unwrap();
            assert_eq!(reply, "It is sunny in Berlin.");
            assert_eq!(count.load(Ordering::SeqCst), 1);
            let models =
                std::fs::read_to_string(client.executable.with_extension("models")).unwrap();
            assert_eq!(models.lines().collect::<Vec<_>>(), [selected_model; 2]);
            let turns = std::fs::read_to_string(client.executable.with_extension("turns")).unwrap();
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
}

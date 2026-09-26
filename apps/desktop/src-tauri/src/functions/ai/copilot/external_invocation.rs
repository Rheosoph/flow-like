//! External-provider command lines and transport configuration.

use super::attachments::write_chat_image_temp_files;
use super::backend_types::FlowPilotAgentBackendKind;
use super::cli_resolution::CliResolution;
use flow_like::{copilot::ChatImage, flow::copilot::tool_spec::MAX_DELEGATED_RUN_DISPATCH_SECS};
use std::path::PathBuf;

pub(super) struct ExternalAgentInvocation {
    pub(super) backend: FlowPilotAgentBackendKind,
    pub(super) executable: std::path::PathBuf,
    pub(super) path_dirs: Vec<PathBuf>,
    pub(super) args: Vec<String>,
    pub(super) prompt: String,
    pub(super) final_output_path: Option<std::path::PathBuf>,
    pub(super) envs: Vec<(String, String)>,
    pub(super) env_removals: Vec<String>,
    /// True for continuation/repair phases of a run whose earlier phase already
    /// streamed answer text. Seeds the stream state so the next phase's first
    /// token starts a new paragraph instead of splicing mid-sentence onto the
    /// previous phase's output.
    pub(super) continues_streamed_text: bool,
}

/// Normalize the optional UI override. An omitted/blank value, or the explicit
/// `default` sentinel, lets the selected backend use its own configured model default.
pub(super) fn explicit_reasoning_effort(reasoning_effort: Option<&str>) -> Option<&str> {
    reasoning_effort
        .map(str::trim)
        .filter(|effort| !effort.is_empty() && !effort.eq_ignore_ascii_case("default"))
}

impl ExternalAgentInvocation {
    /// `global_orchestrator` grants the CLI's own web search and fetch. Specialists hold private
    /// app data without a public-web mandate, so they never receive them.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        backend: FlowPilotAgentBackendKind,
        cli: CliResolution,
        model_id: &str,
        reasoning_effort: Option<&str>,
        mcp_url: &str,
        prompt: String,
        tool_names: Vec<String>,
        global_orchestrator: bool,
        images: &[ChatImage],
        resume_session: Option<&str>,
        append_system_prompt: Option<&str>,
    ) -> Result<Self, String> {
        match backend {
            // Codex has no session-resume or system-prompt-append surface; both options are
            // Claude-only and deliberately ignored here.
            FlowPilotAgentBackendKind::Codex => Self::codex(
                backend,
                cli,
                model_id,
                reasoning_effort,
                mcp_url,
                prompt,
                tool_names,
                global_orchestrator,
                images,
            ),
            FlowPilotAgentBackendKind::ClaudeCode => Self::claude(
                backend,
                cli,
                model_id,
                reasoning_effort,
                mcp_url,
                prompt,
                tool_names,
                global_orchestrator,
                images,
                resume_session,
                append_system_prompt,
            ),
            FlowPilotAgentBackendKind::GithubCopilot => Err(
                "GitHub Copilot uses the direct SDK backend, not the external runner.".to_string(),
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn codex(
        backend: FlowPilotAgentBackendKind,
        cli: CliResolution,
        model_id: &str,
        reasoning_effort: Option<&str>,
        mcp_url: &str,
        prompt: String,
        tool_names: Vec<String>,
        global_orchestrator: bool,
        images: &[ChatImage],
    ) -> Result<Self, String> {
        let web_search = if global_orchestrator && !tool_names.is_empty() {
            "live"
        } else {
            "disabled"
        };
        // Mirrors @openai/codex-sdk's stdio protocol: spawn
        // `codex exec --experimental-json`, pass config overrides as repeated
        // --config entries, and stream JSONL events from stdout.
        let mut args = vec![
            "exec".to_string(),
            "--experimental-json".to_string(),
            // Keep authentication in CODEX_HOME, but do not inherit user-configured MCP servers,
            // browser tools, or web-search settings. FlowPilot must expose exactly its scoped MCP
            // surface plus, for the global orchestrator only, native web search.
            "--ignore-user-config".to_string(),
            "--sandbox".to_string(),
            "read-only".to_string(),
            "--cd".to_string(),
            // FlowPilot supplies its own scoped context and tools. A neutral cwd
            // prevents project discovery and macOS Desktop/Documents permission
            // prompts when the desktop app happened to inherit a protected cwd.
            std::env::temp_dir().display().to_string(),
            "--skip-git-repo-check".to_string(),
            "--config".to_string(),
            "approval_policy=\"never\"".to_string(),
            "--config".to_string(),
            // Explicit either way, even with --ignore-user-config, so Codex defaults or future
            // profile layers cannot decide which surface reaches the public web.
            format!("web_search={web_search:?}"),
        ];
        if tool_names.is_empty() {
            // Ontology query planning is a pure text transformation. Remove Codex's native data
            // access surfaces as well as the empty FlowPilot MCP server. Keep only the isolated
            // V8 code-mode host available because some Codex models require it; optional code mode
            // stays disabled and the host has no Node, filesystem, network, or nested data tools.
            // The remaining CLI-owned interaction and patch tools cannot read data, and the
            // read-only sandbox prevents the patch tool from changing the neutral temporary
            // working directory.
            args.extend(["--ephemeral".to_string(), "--ignore-rules".to_string()]);
            for feature in [
                "apps",
                "artifact",
                "browser_use",
                "browser_use_external",
                "code_mode",
                "computer_use",
                "goals",
                "hooks",
                "image_generation",
                "in_app_browser",
                "in_app_local_automation",
                "memories",
                "multi_agent",
                "plugins",
                "plugin_sharing",
                "request_permissions_tool",
                "shell_snapshot",
                "shell_snapshot_v2",
                "shell_tool",
                "skill_mcp_dependency_install",
                "skill_search",
                "sleep_tool",
                "tool_suggest",
                "unified_exec",
                "view_image",
                "workspace_dependencies",
            ] {
                args.extend(["--disable".to_string(), feature.to_string()]);
            }
        } else {
            args.extend([
                "--config".to_string(),
                format!("mcp_servers.flowpilot.url={:?}", mcp_url),
                "--config".to_string(),
                "mcp_servers.flowpilot.startup_timeout_sec=10".to_string(),
                "--config".to_string(),
                // Outer bound for every FlowPilot MCP tool call. Must be >= the longest per-tool
                // `timeout_secs` in the shared platform tool specs, which is now the delegated
                // board run. It earns wall clock by proving progress and can run for hours.
                format!("mcp_servers.flowpilot.tool_timeout_sec={MAX_DELEGATED_RUN_DISPATCH_SECS}"),
                "--config".to_string(),
                "mcp_servers.flowpilot.default_tools_approval_mode=\"approve\"".to_string(),
                "--config".to_string(),
                "features.use_rmcp_client=true".to_string(),
            ]);
        }
        // Model ids reach this point straight from Codex's own auth-aware catalog
        // (discovered via `codex app-server`'s `model/list`), so an explicit
        // selection is safe to forward. "default" defers to Codex's configured
        // runtime model by omitting `--model` entirely.
        if !model_id.trim().is_empty() && model_id != "default" {
            args.extend(["--model".to_string(), model_id.to_string()]);
        }
        if let Some(effort) = explicit_reasoning_effort(reasoning_effort) {
            // `codex exec` exposes model effort through its regular TOML config
            // override surface rather than a dedicated command-line flag.
            args.extend([
                "--config".to_string(),
                format!("model_reasoning_effort={effort:?}"),
            ]);
        }
        // `codex exec` attaches images to the initial prompt via repeated
        // `--image` flags; the prompt itself stays on stdin. The `=` form is
        // required: bare `--image <file>` parses greedily (num_args=1..) and
        // would swallow any argument appended after it.
        if !images.is_empty() {
            for path in write_chat_image_temp_files(images)? {
                args.push(format!("--image={}", path.display()));
            }
        }

        Ok(Self {
            backend,
            executable: cli.executable,
            path_dirs: cli.path_dirs,
            args,
            prompt,
            final_output_path: None,
            envs: Vec::new(),
            env_removals: Vec::new(),
            continues_streamed_text: false,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn claude(
        backend: FlowPilotAgentBackendKind,
        cli: CliResolution,
        model_id: &str,
        reasoning_effort: Option<&str>,
        mcp_url: &str,
        prompt: String,
        tool_names: Vec<String>,
        global_orchestrator: bool,
        images: &[ChatImage],
        resume_session: Option<&str>,
        append_system_prompt: Option<&str>,
    ) -> Result<Self, String> {
        let mcp_config_path = std::env::temp_dir().join(format!(
            "flowpilot-claude-mcp-{}.json",
            uuid::Uuid::new_v4()
        ));
        // The global surface is large. Let Claude's native MCP ToolSearch keep those schemas
        // deferred. Small role-scoped specialists retain eager loading because their exact
        // lifecycle tools are all immediately relevant.
        let server_config = if global_orchestrator {
            serde_json::json!({
                "type": "http",
                "url": mcp_url,
            })
        } else {
            serde_json::json!({
                "type": "http",
                "url": mcp_url,
                "alwaysLoad": true,
            })
        };
        let mcp_config = serde_json::json!({
            "mcpServers": {
                "flowpilot": server_config
            }
        });
        std::fs::write(
            &mcp_config_path,
            serde_json::to_vec_pretty(&mcp_config)
                .map_err(|e| format!("Failed to serialize Claude MCP config: {e}"))?,
        )
        .map_err(|e| format!("Failed to write Claude MCP config: {e}"))?;

        let mut args = vec![
            "-p".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
            "--verbose".to_string(),
            // Stream assistant tokens as content_block_delta frames so FlowPilot
            // can render the reply live instead of only at the final result.
            "--include-partial-messages".to_string(),
            "--strict-mcp-config".to_string(),
            "--mcp-config".to_string(),
            mcp_config_path.display().to_string(),
        ];
        const DISALLOWED_BUILTIN_TOOLS: &str = "Task,Bash,Glob,Grep,Read,Edit,Write,NotebookEdit";
        const WEB_BUILTIN_TOOLS: [&str; 2] = ["WebSearch", "WebFetch"];
        let native_web = global_orchestrator && !tool_names.is_empty();
        if tool_names.is_empty() {
            // Claude documents an empty --tools value as the way to remove every built-in tool.
            // The ontology query planner has no MCP tools either, so this produces a genuinely
            // tool-free completion instead of leaving file, shell, or web tools in its context.
            args.extend(["--tools".to_string(), String::new()]);
        } else {
            let mut allowed_tools = tool_names
                .iter()
                .map(|name| format!("mcp__flowpilot__{name}"))
                .collect::<Vec<_>>();
            if native_web {
                allowed_tools.extend(WEB_BUILTIN_TOOLS.map(str::to_string));
            }
            // Do NOT pass `--tools` here: it controls which tools are visible in
            // context and only understands built-in tool names, so listing MCP
            // tools there hides the whole toolset and the agent degrades to
            // text-only answers. Allow the FlowPilot MCP tools, auto-deny
            // everything else via `dontAsk`, and strip the built-in file/shell
            // tools from context entirely so headless runs cannot stall on them.
            args.extend(["--allowedTools".to_string(), allowed_tools.join(",")]);
        }
        let disallowed_tools = if native_web {
            DISALLOWED_BUILTIN_TOOLS.to_string()
        } else {
            format!("{DISALLOWED_BUILTIN_TOOLS},{}", WEB_BUILTIN_TOOLS.join(","))
        };
        // Keep the built-ins out even when the reviewed MCP allowlist is empty. `dontAsk` makes
        // any unexpected capability fail closed instead of stalling a headless request.
        args.extend([
            "--disallowedTools".to_string(),
            disallowed_tools,
            "--permission-mode".to_string(),
            "dontAsk".to_string(),
        ]);
        if !model_id.trim().is_empty() && model_id != "default" {
            args.extend(["--model".to_string(), model_id.to_string()]);
        }
        if let Some(effort) = explicit_reasoning_effort(reasoning_effort) {
            args.extend(["--effort".to_string(), effort.to_string()]);
        }
        // The bounded role/lifecycle appendix belongs in the real system prompt, not the user
        // message; the board-embedding platform content stays on stdin because argv has OS
        // length limits.
        if let Some(appendix) = append_system_prompt
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            args.extend(["--append-system-prompt".to_string(), appendix.to_string()]);
        }
        // Continuation phases within one run resume the previous phase's session so the model
        // keeps its own transcript instead of a lossy host reconstruction. Each resumed print
        // run mints a NEW session id; the phase loop always resumes the latest captured one.
        if let Some(session) = resume_session.map(str::trim).filter(|s| !s.is_empty()) {
            args.extend(["--resume".to_string(), session.to_string()]);
        }

        // Text-only turns deliver the prompt via stdin as plain text (`-p` reads
        // stdin when no positional prompt is given): the prompt embeds the whole
        // board as FlowScript and can exceed OS argv length limits, so it must
        // never be passed positionally. Image turns switch to stream-json stdin
        // input so the user message can carry Anthropic image content blocks
        // (requires --output-format stream-json, already set above). Either way
        // the stdin writer thread sends the payload and closes the pipe, which
        // ends the turn.
        let stdin_prompt = if images.is_empty() {
            prompt
        } else {
            args.extend(["--input-format".to_string(), "stream-json".to_string()]);
            let mut content = vec![serde_json::json!({ "type": "text", "text": prompt })];
            for image in images {
                content.push(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": image.media_type,
                        "data": image.data,
                    }
                }));
            }
            let message = serde_json::json!({
                "type": "user",
                "message": { "role": "user", "content": content }
            });
            let mut line = serde_json::to_string(&message)
                .map_err(|e| format!("Failed to serialize Claude user message: {e}"))?;
            line.push('\n');
            line
        };

        let mut envs = vec![
            (
                "MCP_TOOL_TIMEOUT".to_string(),
                (MAX_DELEGATED_RUN_DISPATCH_SECS * 1000).to_string(),
            ),
            // Disable Claude's independent no-progress watchdog for long nested FlowPilot calls;
            // FlowPilot still owns explicit cancellation and per-tool lifecycle bounds.
            (
                "CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT".to_string(),
                "0".to_string(),
            ),
        ];
        if !global_orchestrator {
            // Preserve the existing eager path for small role-scoped specialist surfaces.
            envs.push(("ENABLE_TOOL_SEARCH".to_string(), "auto".to_string()));
        }

        Ok(Self {
            backend,
            executable: cli.executable,
            path_dirs: cli.path_dirs,
            args,
            // Delivered via stdin (`-p` reads it when no positional prompt is
            // given). Text turns send the plain prompt; image turns send a
            // stream-json user message. Either way it can embed the whole board
            // as FlowScript and exceed OS argv length limits, so it stays off argv.
            prompt: stdin_prompt,
            // Claude Code applies MCP_TOOL_TIMEOUT as the overall MCP-call bound. A delegated board
            // run earns wall clock by proving progress and can run for hours, so this tracks the
            // same shared dispatch ceiling as the Codex path above.
            envs,
            // The global surface relies on Claude's supported-model default ToolSearch behavior.
            // Do not let an ambient desktop/shell override force eager loading or disable it.
            env_removals: if global_orchestrator {
                vec!["ENABLE_TOOL_SEARCH".to_string()]
            } else {
                Default::default()
            },
            final_output_path: Some(mcp_config_path),
            continues_streamed_text: false,
        })
    }
}

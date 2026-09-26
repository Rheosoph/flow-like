use super::*;

#[test]
fn codex_invocation_uses_streamable_http_mcp_server() {
    let invocation = ExternalAgentInvocation::new(
        FlowPilotAgentBackendKind::Codex,
        CliResolution::new(
            std::path::PathBuf::from("/usr/bin/codex"),
            CliResolutionSource::Path,
        ),
        "default",
        None,
        "http://127.0.0.1:12345/mcp",
        "hello".to_string(),
        vec!["edit_flowscript".to_string()],
        false,
        &[],
        None,
        None,
    )
    .expect("codex invocation should build");

    assert_eq!(invocation.backend, FlowPilotAgentBackendKind::Codex);
    assert!(invocation.args.contains(&"exec".to_string()));
    assert!(invocation.args.contains(&"--experimental-json".to_string()));
    assert!(
        invocation
            .args
            .contains(&"--ignore-user-config".to_string())
    );
    assert!(
        invocation
            .args
            .contains(&"--skip-git-repo-check".to_string())
    );
    let cd_index = invocation
        .args
        .iter()
        .position(|arg| arg == "--cd")
        .expect("Codex invocation should set a neutral working directory");
    assert_eq!(
        invocation.args.get(cd_index + 1),
        Some(&std::env::temp_dir().display().to_string()),
        "Codex must not inspect an incidental protected desktop working directory"
    );
    assert!(invocation.args.contains(&"--config".to_string()));
    assert!(
        !invocation.args.contains(&"--model".to_string()),
        "the \"default\" model selection must defer to Codex's configured runtime model by omitting --model: {:?}",
        invocation.args
    );
    assert!(
        !invocation
            .args
            .iter()
            .any(|arg| arg.starts_with("model_reasoning_effort=")),
        "an omitted effort must preserve Codex's configured default: {:?}",
        invocation.args
    );
    assert!(
        invocation
            .args
            .windows(2)
            .any(|args| args == ["--sandbox", "read-only"]),
        "codex invocation should keep FlowPilot workspace edits in MCP tools, not shell writes: {:?}",
        invocation.args
    );
    assert!(
        invocation
            .args
            .iter()
            .any(|arg| arg.contains("mcp_servers.flowpilot.url=")
                && arg.contains("127.0.0.1:12345/mcp")),
        "codex args should contain MCP URL: {:?}",
        invocation.args
    );
    assert!(
        invocation
            .args
            .iter()
            .any(|arg| arg == "mcp_servers.flowpilot.default_tools_approval_mode=\"approve\""),
        "codex exec must explicitly approve the session-local FlowPilot MCP tools in headless mode: {:?}",
        invocation.args
    );
    assert!(
        invocation
            .args
            .iter()
            .any(|arg| arg == "approval_policy=\"never\""),
        "codex invocation should run non-interactively through FlowPilot approvals/tools"
    );
    assert!(invocation.prompt.contains("hello"));
}

#[test]
fn codex_native_web_search_is_live_only_for_the_global_orchestrator() {
    for (tool_names, global_orchestrator, expected) in [
        (vec!["edit_flowscript".to_string()], false, "disabled"),
        (
            vec!["list_apps".to_string(), "flowpilot_board".to_string()],
            true,
            "live",
        ),
        (Vec::new(), true, "disabled"),
    ] {
        let invocation = ExternalAgentInvocation::new(
            FlowPilotAgentBackendKind::Codex,
            CliResolution::new(
                std::path::PathBuf::from("/usr/bin/codex"),
                CliResolutionSource::Path,
            ),
            "default",
            None,
            "http://127.0.0.1:12345/mcp",
            "hello".to_string(),
            tool_names,
            global_orchestrator,
            &[],
            None,
            None,
        )
        .expect("codex invocation should build");

        let web_search_overrides = invocation
            .args
            .windows(2)
            .filter(|args| args[0] == "--config" && args[1].starts_with("web_search="))
            .map(|args| args[1].clone())
            .collect::<Vec<_>>();
        assert_eq!(
            web_search_overrides,
            vec![format!("web_search=\"{expected}\"")],
            "every FlowPilot Codex invocation must pin web search explicitly, live only for the global orchestrator: {:?}",
            invocation.args
        );
        assert_eq!(
            invocation
                .args
                .iter()
                .filter(|arg| arg.as_str() == "--ignore-user-config")
                .count(),
            1,
            "every FlowPilot Codex invocation must exclude user-configured MCP/browser tools while retaining CODEX_HOME auth: {:?}",
            invocation.args
        );
    }
}

#[test]
fn codex_data_isolated_invocation_disables_native_data_access() {
    let invocation = ExternalAgentInvocation::new(
        FlowPilotAgentBackendKind::Codex,
        CliResolution::new(
            std::path::PathBuf::from("/usr/bin/codex"),
            CliResolutionSource::Path,
        ),
        "default",
        None,
        "http://127.0.0.1:12345/mcp",
        "return one query envelope".to_string(),
        Vec::new(),
        false,
        &[],
        None,
        None,
    )
    .expect("data-isolated Codex invocation should build");

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
        assert!(
            invocation
                .args
                .windows(2)
                .any(|args| args == ["--disable", feature]),
            "data-isolated Codex invocation must disable {feature}: {:?}",
            invocation.args
        );
    }
    assert!(
        invocation.args.contains(&"--ephemeral".to_string())
            && invocation.args.contains(&"--ignore-rules".to_string()),
        "data-isolated Codex invocation must avoid persisted sessions and ambient rules: {:?}",
        invocation.args
    );
    assert!(
        !invocation
            .args
            .windows(2)
            .any(|args| args == ["--disable", "code_mode_host"]),
        "data-isolated Codex invocation must retain the isolated host required by code-mode-only models: {:?}",
        invocation.args
    );
    assert!(
        !invocation
            .args
            .iter()
            .any(|arg| arg.starts_with("mcp_servers.flowpilot.")),
        "data-isolated Codex invocation must not attach an empty MCP surface: {:?}",
        invocation.args
    );
}

#[test]
fn codex_invocation_forwards_selected_model() {
    let invocation = ExternalAgentInvocation::new(
        FlowPilotAgentBackendKind::Codex,
        CliResolution::new(
            std::path::PathBuf::from("/usr/bin/codex"),
            CliResolutionSource::Path,
        ),
        "gpt-5.5",
        Some("xhigh"),
        "http://127.0.0.1:12345/mcp",
        "hello".to_string(),
        vec!["edit_flowscript".to_string()],
        false,
        &[],
        None,
        None,
    )
    .expect("codex invocation should build");

    assert!(
        invocation
            .args
            .windows(2)
            .any(|args| args == ["--model", "gpt-5.5"]),
        "a discovered Codex model id must be forwarded via --model: {:?}",
        invocation.args
    );
    assert!(
        invocation
            .args
            .windows(2)
            .any(|args| args == ["--config", "model_reasoning_effort=\"xhigh\""]),
        "the selected Codex reasoning effort must be forwarded as a config override: {:?}",
        invocation.args
    );
}

#[test]
fn parse_codex_model_catalog_maps_and_filters() {
    let entries = vec![
        serde_json::json!({
            "id": "gpt-5.5",
            "displayName": "GPT-5.5",
            "hidden": false,
            "isDefault": true,
            "supportedReasoningEfforts": [
                {
                    "reasoningEffort": "low",
                    "description": "Fast responses with lighter reasoning"
                },
                {
                    "reasoningEffort": "xhigh",
                    "description": "Extra high reasoning depth"
                }
            ],
            "defaultReasoningEffort": "low"
        }),
        serde_json::json!({ "model": "gpt-5.4-mini", "hidden": false }),
        serde_json::json!({ "id": "internal", "displayName": "Internal", "hidden": true }),
        serde_json::json!({ "displayName": "No id here" }),
    ];

    let models = parse_codex_model_catalog(&entries);

    assert_eq!(models.len(), 2, "hidden and id-less entries are dropped");
    assert_eq!(models[0].id, "gpt-5.5");
    assert_eq!(models[0].name, "GPT-5.5");
    assert!(models[0].is_default);
    assert_eq!(models[0].default_reasoning_effort.as_deref(), Some("low"));
    assert_eq!(
        models[0].supported_reasoning_efforts,
        vec![
            ReasoningEffortOption {
                id: "low".to_string(),
                name: "Low".to_string(),
                description: Some("Fast responses with lighter reasoning".to_string()),
            },
            ReasoningEffortOption {
                id: "xhigh".to_string(),
                name: "Extra high".to_string(),
                description: Some("Extra high reasoning depth".to_string()),
            },
        ]
    );
    assert_eq!(models[1].id, "gpt-5.4-mini");
    assert_eq!(
        models[1].name, "gpt-5.4-mini",
        "displayName falls back to the model id"
    );

    let serialized = serde_json::to_value(&models[0]).expect("model DTO serializes");
    assert!(serialized.get("supportedReasoningEfforts").is_some());
    assert_eq!(serialized["defaultReasoningEffort"], "low");
    assert_eq!(serialized["isDefault"], true);
    assert!(serialized.get("supported_reasoning_efforts").is_none());

    let with_default = codex_models_with_configured_default(models);
    assert_eq!(with_default[0].id, "default");
    assert_eq!(
        with_default[0].default_reasoning_effort.as_deref(),
        Some("low")
    );
    assert_eq!(
        with_default[0].supported_reasoning_efforts, with_default[1].supported_reasoning_efforts,
        "the configured-default sentinel must inherit the runtime default's effort metadata"
    );
}

#[test]
fn parse_claude_model_catalog_maps_and_dedupes() {
    let entries = vec![
        serde_json::json!({
            "value": "default",
            "resolvedModel": "claude-opus-4-8[1m]",
            "displayName": "Default (recommended)",
            "supportsEffort": true,
            "supportedEffortLevels": ["low", "medium", "high", "max"]
        }),
        serde_json::json!({
            "value": "sonnet",
            "displayName": "Sonnet",
            "supportsEffort": true,
            "supportedEffortLevels": ["low", "high"]
        }),
        serde_json::json!({ "value": "sonnet", "displayName": "Sonnet duplicate" }),
        serde_json::json!({
            "value": "claude-fable-5[1m]",
            "supportsEffort": false,
            "supportedEffortLevels": ["low"]
        }),
        serde_json::json!({ "displayName": "No value here" }),
    ];

    let models = parse_claude_model_catalog(&entries);

    assert_eq!(
        models.len(),
        3,
        "duplicate value and value-less entries drop"
    );
    assert_eq!(models[0].id, "default");
    assert_eq!(models[0].name, "Default (recommended)");
    assert!(models[0].is_default);
    assert_eq!(
        models[0]
            .supported_reasoning_efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect::<Vec<_>>(),
        vec!["low", "medium", "high", "max"]
    );
    assert_eq!(models[1].id, "sonnet");
    assert_eq!(
        models[1]
            .supported_reasoning_efforts
            .iter()
            .map(|effort| effort.id.as_str())
            .collect::<Vec<_>>(),
        vec!["low", "high"]
    );
    assert_eq!(
        models[2].id, "claude-fable-5[1m]",
        "bracketed model ids are passed through verbatim for --model"
    );
    assert_eq!(
        models[2].name, "claude-fable-5[1m]",
        "displayName falls back to the value"
    );
    assert!(
        models[2].supported_reasoning_efforts.is_empty(),
        "supportsEffort=false must suppress stale level metadata"
    );
}

#[test]
fn claude_invocation_resumes_sessions_and_appends_the_role_prompt() {
    let invocation = ExternalAgentInvocation::new(
        FlowPilotAgentBackendKind::ClaudeCode,
        CliResolution::new(
            std::path::PathBuf::from("/usr/bin/claude"),
            CliResolutionSource::Path,
        ),
        "sonnet",
        None,
        "http://127.0.0.1:23456/mcp",
        "continuation payload".to_string(),
        vec!["write_flowscript".to_string()],
        false,
        &[],
        Some("session-1234"),
        Some("ROLE APPENDIX"),
    )
    .expect("claude invocation should build");

    let resume_index = invocation
        .args
        .iter()
        .position(|arg| arg == "--resume")
        .expect("claude must resume the captured session");
    assert_eq!(invocation.args[resume_index + 1], "session-1234");
    let append_index = invocation
        .args
        .iter()
        .position(|arg| arg == "--append-system-prompt")
        .expect("claude must append the role prompt");
    assert_eq!(invocation.args[append_index + 1], "ROLE APPENDIX");
    assert_eq!(
        invocation.prompt, "continuation payload",
        "a resumed continuation sends only its compact payload on stdin"
    );

    let codex = ExternalAgentInvocation::new(
        FlowPilotAgentBackendKind::Codex,
        CliResolution::new(
            std::path::PathBuf::from("/usr/bin/codex"),
            CliResolutionSource::Path,
        ),
        "default",
        None,
        "http://127.0.0.1:12345/mcp",
        "hello".to_string(),
        vec!["edit_flowscript".to_string()],
        false,
        &[],
        Some("session-1234"),
        Some("ROLE APPENDIX"),
    )
    .expect("codex invocation should build");
    assert!(
        !codex.args.iter().any(|arg| arg == "--resume"
            || arg == "--append-system-prompt"
            || arg.contains("session-1234")),
        "codex has no resume/append surface; both options must be ignored"
    );
}

#[test]
fn external_agent_prompt_split_keeps_the_full_wrap_byte_identical() {
    let full = build_external_agent_prompt("SYS", "USER", CopilotScope::Board, true, false);
    let appendix = external_agent_role_appendix(CopilotScope::Board, true, false);
    assert_eq!(
        full,
        format!("SYSTEM INSTRUCTIONS\nSYS\n\n{appendix}\n\nUSER REQUEST\nUSER")
    );
    assert_eq!(
        build_external_agent_prompt_body("SYS", "USER"),
        "SYSTEM INSTRUCTIONS\nSYS\n\nUSER REQUEST\nUSER"
    );
    assert!(appendix.contains("BOARD specialist"));
    assert!(appendix.contains("WORKFLOW MUTATION RUN"));
    assert!(appendix.contains("test_flowscript"));
    assert!(appendix.contains("expected_output derived from the request"));
    assert!(appendix.contains("A blocked test remains unverified"));
}

#[test]
fn claude_invocation_uses_shared_mcp_config() {
    let invocation = ExternalAgentInvocation::new(
        FlowPilotAgentBackendKind::ClaudeCode,
        CliResolution::new(
            std::path::PathBuf::from("/usr/bin/claude"),
            CliResolutionSource::Path,
        ),
        "sonnet",
        Some("max"),
        "http://127.0.0.1:23456/mcp",
        "hello".to_string(),
        vec![
            "get_declarations".to_string(),
            "write_flowscript".to_string(),
            "patch_flowscript".to_string(),
            "check_flowscript".to_string(),
            "test_flowscript".to_string(),
            "commit_flowscript".to_string(),
        ],
        false,
        &[],
        None,
        None,
    )
    .expect("claude invocation should build");

    assert_eq!(invocation.backend, FlowPilotAgentBackendKind::ClaudeCode);
    assert!(invocation.args.contains(&"--mcp-config".to_string()));
    assert!(invocation.args.contains(&"stream-json".to_string()));
    assert!(invocation.args.contains(&"--strict-mcp-config".to_string()));
    assert!(invocation.args.contains(&"--allowedTools".to_string()));
    assert!(
        !invocation.args.contains(&"--tools".to_string()),
        "--tools only understands built-in tool names; passing MCP names there hides the whole toolset"
    );
    assert!(invocation.args.contains(&"--disallowedTools".to_string()));
    assert!(invocation.args.contains(&"dontAsk".to_string()));
    assert_eq!(
        invocation.prompt, "hello",
        "prompt must be delivered via stdin"
    );
    assert!(
        !invocation.args.contains(&"hello".to_string()),
        "prompt must not be passed as argv (OS arg-length limits on large boards)"
    );
    assert!(
        invocation
            .args
            .iter()
            .any(|arg| arg.contains("mcp__flowpilot__get_declarations")
                && arg.contains("mcp__flowpilot__write_flowscript")
                && arg.contains("mcp__flowpilot__patch_flowscript")
                && arg.contains("mcp__flowpilot__check_flowscript")
                && arg.contains("mcp__flowpilot__test_flowscript")
                && arg.contains("mcp__flowpilot__commit_flowscript")),
        "claude invocation should allow only shared FlowPilot MCP tools: {:?}",
        invocation.args
    );
    assert!(invocation.args.contains(&"sonnet".to_string()));
    assert!(
        invocation
            .args
            .windows(2)
            .any(|args| args == ["--effort", "max"]),
        "Claude Code must receive the selected dynamic effort level: {:?}",
        invocation.args
    );
    assert!(
        invocation
            .args
            .contains(&"--include-partial-messages".to_string()),
        "claude invocation must stream partial messages for live tokens: {:?}",
        invocation.args
    );
    assert!(
        invocation.envs.iter().any(|(key, value)| {
            key == "MCP_TOOL_TIMEOUT"
                && value == &(MAX_DELEGATED_RUN_DISPATCH_SECS * 1000).to_string()
        }),
        "claude invocation must carry the shared delegated-run dispatch ceiling, or a board build that earned hours of wall clock dies at the MCP layer: {:?}",
        invocation.envs
    );
    assert!(
        invocation
            .envs
            .iter()
            .any(|(key, value)| { key == "CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT" && value == "0" }),
        "claude invocation must disable Claude's independent 300s MCP idle watchdog for nested FlowPilot runs: {:?}",
        invocation.envs
    );
    assert!(
        invocation
            .envs
            .iter()
            .any(|(key, value)| key == "ENABLE_TOOL_SEARCH" && value == "auto"),
        "Claude ToolSearch should preload the small FlowPilot surface instead of discovering each schema turn-by-turn: {:?}",
        invocation.envs
    );
    assert!(
        !invocation.args.iter().any(|arg| arg == "--max-turns"),
        "Claude must use its normal turn lifecycle rather than an arbitrary hard cap: {:?}",
        invocation.args
    );

    let config_path = invocation
        .final_output_path
        .as_ref()
        .expect("claude invocation stores temp MCP config");
    let config = std::fs::read_to_string(config_path).expect("temp MCP config is readable");
    assert!(config.contains("flowpilot"));
    assert!(config.contains("127.0.0.1:23456/mcp"));
    assert!(config.contains("\"alwaysLoad\": true"));
    let _ = std::fs::remove_file(config_path);

    let disallowed = invocation
        .args
        .windows(2)
        .find(|args| args[0] == "--disallowedTools")
        .map(|args| args[1].clone())
        .expect("claude invocation disallows built-ins");
    assert!(
        disallowed.contains("WebSearch") && disallowed.contains("WebFetch"),
        "specialists must stay off the public web: {disallowed}"
    );
}

#[test]
fn claude_global_surface_defers_mcp_tool_schemas_and_allows_native_web() {
    let invocation = ExternalAgentInvocation::new(
        FlowPilotAgentBackendKind::ClaudeCode,
        CliResolution::new(
            std::path::PathBuf::from("/usr/bin/claude"),
            CliResolutionSource::Path,
        ),
        "sonnet",
        None,
        "http://127.0.0.1:23456/mcp",
        "hello".to_string(),
        vec!["list_apps".to_string(), "flowpilot_board".to_string()],
        true,
        &[],
        None,
        None,
    )
    .expect("global Claude invocation should build");

    let flag_value = |flag: &str| {
        invocation
            .args
            .windows(2)
            .find(|args| args[0] == flag)
            .map(|args| args[1].clone())
            .unwrap_or_default()
    };
    let allowed = flag_value("--allowedTools");
    assert!(
        allowed.contains("mcp__flowpilot__list_apps")
            && allowed.contains("WebSearch")
            && allowed.contains("WebFetch"),
        "the global orchestrator must reach its native web tools under dontAsk: {allowed}"
    );
    let disallowed = flag_value("--disallowedTools");
    assert!(
        !disallowed.contains("WebSearch") && !disallowed.contains("WebFetch"),
        "a disallow rule would override the allowlist: {disallowed}"
    );
    assert!(disallowed.contains("Bash") && disallowed.contains("Read"));

    assert!(
        !invocation
            .envs
            .iter()
            .any(|(key, _)| key == "ENABLE_TOOL_SEARCH"),
        "native Claude MCP deferral should use its default supported-model behavior"
    );
    assert!(
        invocation
            .env_removals
            .iter()
            .any(|key| key == "ENABLE_TOOL_SEARCH"),
        "ambient tool-search overrides must not defeat global schema deferral"
    );
    let config_path = invocation
        .final_output_path
        .as_ref()
        .expect("Claude invocation stores temp MCP config");
    let config = std::fs::read_to_string(config_path).expect("temp MCP config is readable");
    assert!(config.contains("flowpilot"));
    assert!(!config.contains("alwaysLoad"));
    let _ = std::fs::remove_file(config_path);
}

#[test]
fn codex_invocation_attaches_images_via_image_flag() {
    let invocation = ExternalAgentInvocation::new(
        FlowPilotAgentBackendKind::Codex,
        CliResolution::new(
            std::path::PathBuf::from("/usr/bin/codex"),
            CliResolutionSource::Path,
        ),
        "default",
        None,
        "http://127.0.0.1:12345/mcp",
        "hello".to_string(),
        vec!["edit_flowscript".to_string()],
        false,
        &[test_chat_image()],
        None,
        None,
    )
    .expect("codex invocation should build");

    // `--image=<path>` single-arg form: the bare two-arg form parses
    // greedily and would swallow trailing arguments as image paths.
    assert!(
        invocation
            .args
            .iter()
            .any(|arg| arg.starts_with("--image=") && arg.ends_with(".png")),
        "codex invocation must attach images via --image=<path>: {:?}",
        invocation.args
    );
    assert!(
        invocation.prompt.contains("hello"),
        "codex prompt must stay on stdin"
    );
}

#[test]
fn claude_tool_free_invocation_disables_builtin_and_mcp_tools() {
    let invocation = ExternalAgentInvocation::new(
        FlowPilotAgentBackendKind::ClaudeCode,
        CliResolution::new(
            std::path::PathBuf::from("/usr/bin/claude"),
            CliResolutionSource::Path,
        ),
        "sonnet",
        None,
        "http://127.0.0.1:23456/mcp",
        "query proposal".to_string(),
        Vec::new(),
        false,
        &[],
        None,
        None,
    )
    .expect("tool-free Claude invocation should build");

    assert!(
        invocation
            .args
            .windows(2)
            .any(|args| args == ["--tools", ""]),
        "an empty reviewed tool set must also remove Claude's built-in tools: {:?}",
        invocation.args
    );
    assert!(
        !invocation.args.contains(&"--allowedTools".to_string()),
        "a tool-free invocation must not advertise an MCP allowlist: {:?}",
        invocation.args
    );
    assert!(invocation.args.contains(&"--disallowedTools".to_string()));
    assert!(invocation.args.contains(&"dontAsk".to_string()));
    let config_path = invocation
        .final_output_path
        .as_ref()
        .expect("tool-free Claude invocation stores temp MCP config");
    let _ = std::fs::remove_file(config_path);
}

#[test]
fn claude_invocation_sends_images_as_stream_json_stdin() {
    let invocation = ExternalAgentInvocation::new(
        FlowPilotAgentBackendKind::ClaudeCode,
        CliResolution::new(
            std::path::PathBuf::from("/usr/bin/claude"),
            CliResolutionSource::Path,
        ),
        "default",
        None,
        "http://127.0.0.1:23456/mcp",
        "hello".to_string(),
        vec![],
        false,
        &[test_chat_image()],
        None,
        None,
    )
    .expect("claude invocation should build");

    assert!(
        invocation
            .args
            .windows(2)
            .any(|args| args == ["--input-format", "stream-json"]),
        "image turns must switch Claude to stream-json stdin input: {:?}",
        invocation.args
    );
    assert!(
        !invocation.args.contains(&"hello".to_string()),
        "image turns must not also pass the prompt positionally: {:?}",
        invocation.args
    );

    let line: serde_json::Value = serde_json::from_str(invocation.prompt.trim())
        .expect("stdin prompt is a single JSON user message line");
    assert_eq!(line["type"], "user");
    let content = line["message"]["content"]
        .as_array()
        .expect("content blocks");
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[0]["text"], "hello");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(content[1]["source"]["type"], "base64");
    assert_eq!(content[1]["source"]["media_type"], "image/png");

    if let Some(config_path) = invocation.final_output_path.as_ref() {
        let _ = std::fs::remove_file(config_path);
    }
}

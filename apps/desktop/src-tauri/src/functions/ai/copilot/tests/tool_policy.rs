use super::*;

#[test]
fn model_selection_routes_agent_backend_prefixes() {
    let github = FlowPilotModelSelection::parse(Some("github-copilot:gpt-5-mini".to_string()));
    assert_eq!(
        github.backend,
        FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::GithubCopilot)
    );
    assert_eq!(github.model_id.as_deref(), Some("gpt-5-mini"));

    let legacy = FlowPilotModelSelection::parse(Some("copilot:claude".to_string()));
    assert_eq!(
        legacy.backend,
        FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::GithubCopilot)
    );
    assert_eq!(legacy.model_id.as_deref(), Some("claude"));

    let codex = FlowPilotModelSelection::parse(Some("codex:default".to_string()));
    assert_eq!(
        codex.backend,
        FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::Codex)
    );

    let claude = FlowPilotModelSelection::parse(Some("claude-code:default".to_string()));
    assert_eq!(
        claude.backend,
        FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::ClaudeCode)
    );
}

#[test]
fn persisted_tool_previews_redact_credentials_recursively() {
    let arguments = serde_json::json!({
        "operation": "connect",
        "password": "visible-password-must-not-leak",
        "nested": {
            "access_token": "visible-token-must-not-leak",
            "api_key": "visible-key-must-not-leak",
        },
    });
    let preview = preview_tool_arguments("future_tool", Some(&arguments));
    assert!(preview.contains("<redacted>"));
    assert!(!preview.contains("visible-password-must-not-leak"));
    assert!(!preview.contains("visible-token-must-not-leak"));
    assert!(!preview.contains("visible-key-must-not-leak"));

    let result_preview = preview_tool_result(&arguments.to_string());
    assert!(result_preview.contains("<redacted>"));
    assert!(!result_preview.contains("visible-password-must-not-leak"));
}

#[test]
fn flowscript_and_non_json_previews_keep_safe_debug_content() {
    let flowscript = r#"function pollSupportInbox() {
    const password = "must-not-leak"
    logInfo({ message: "polling support inbox" })
}"#;
    let arguments = serde_json::json!({ "flowscript": flowscript });
    let arguments_preview = preview_tool_arguments("edit_flowscript", Some(&arguments));
    assert!(arguments_preview.contains("pollSupportInbox"));
    assert!(arguments_preview.contains("logInfo"));
    assert!(arguments_preview.contains("<redacted>"));
    assert!(!arguments_preview.contains("must-not-leak"));

    let result_preview = preview_tool_result(
        "validation failed: pollSupportInbox has no Done edge; token=must-not-leak",
    );
    assert!(result_preview.contains("validation failed"));
    assert!(result_preview.contains("pollSupportInbox"));
    assert!(result_preview.contains("token=<redacted>"));
    assert!(!result_preview.contains("must-not-leak"));
}

#[test]
fn nested_stream_frames_carry_parent_request_id_without_touching_text() {
    let frame = flowpilot_stream_tag(
        "tool_start",
        &serde_json::json!({
            "tool_call_id": "child-tool-1",
            "tool": "edit_flowscript",
            "arguments_preview": "safe preview",
        }),
    );
    let correlated = correlate_stream_frame(&frame, Some("flowpilot-tool-parent-1"));
    assert!(correlated.contains("\"parent_request_id\":\"flowpilot-tool-parent-1\""));
    assert!(correlated.contains("\"tool_call_id\":\"child-tool-1\""));
    assert_eq!(
        correlate_stream_frame("assistant text", Some("flowpilot-tool-parent-1")),
        "assistant text"
    );

    let workspace = flowpilot_stream_tag(
        "flowscript_workspace",
        &serde_json::json!({
            "source": "eventsSimple() { logInfo({ message: \"live\" }) }",
            "status": "drafting",
            "tool_call_id": "child-tool-1",
        }),
    );
    let correlated_workspace = correlate_stream_frame(&workspace, Some("flowpilot-tool-parent-1"));
    assert!(correlated_workspace.contains("\"parent_request_id\":\"flowpilot-tool-parent-1\""));
    assert!(correlated_workspace.contains("\"status\":\"drafting\""));
    assert!(correlated_workspace.contains("logInfo"));
}

#[test]
fn correlated_payload_preserves_an_existing_parent_id() {
    let payload = serde_json::json!({
        "tool_call_id": "child-tool-1",
        "parent_request_id": "authoritative-parent",
    });
    let correlated = correlated_stream_payload(&payload, Some("fallback-parent"));
    assert_eq!(
        correlated
            .get("parent_request_id")
            .and_then(serde_json::Value::as_str),
        Some("authoritative-parent")
    );
}

#[test]
fn model_selection_keeps_bits_model_ids_unprefixed() {
    let selection = FlowPilotModelSelection::parse(Some("hub:model".to_string()));
    assert_eq!(selection.backend, FlowPilotChatBackend::Bits);
    assert_eq!(selection.model_id.as_deref(), Some("hub:model"));
}

#[test]
fn board_runtime_bridge_uses_scoped_specs_before_context_injection() {
    use flow_like::flow::copilot::tool_spec::{
        ARCHIVE_LOOKUP_TOOL, INTERNET_SEARCH_TOOL, OPEN_URL_TOOL, ToolApprovalSpec,
        missing_required_args,
    };

    for global_only_tool in [INTERNET_SEARCH_TOOL, OPEN_URL_TOOL, ARCHIVE_LOOKUP_TOOL] {
        let scope_error = global_orchestrator_tool_scope_error(
            FrontendPlatformToolSet::BoardRuntime,
            global_only_tool,
        )
        .expect("board runtime must reject global-only tools before execution");
        assert!(scope_error.contains("global_orchestrator_tool_only"));
        assert!(
            global_orchestrator_tool_scope_error(FrontendPlatformToolSet::Global, global_only_tool)
                .is_none()
        );
        assert!(
            frontend_platform_tool_spec(FrontendPlatformToolSet::BoardRuntime, global_only_tool)
                .is_none(),
            "board runtime must not expose global-only tool {global_only_tool}"
        );
        assert!(
            frontend_platform_tool_spec(FrontendPlatformToolSet::Global, global_only_tool)
                .is_some(),
            "global FlowPilot must expose {global_only_tool}"
        );
    }

    for name in [
        "execute_event",
        "execute_node",
        "query_execution_logs",
        "read_flowscript_source",
        "search_workspace",
        "read_symbol",
    ] {
        assert!(
            frontend_platform_tool_spec(FrontendPlatformToolSet::BoardRuntime, name).is_some(),
            "Bits board bridge must expose the shared runtime spec for {name}"
        );
    }

    let execute_node =
        frontend_platform_tool_spec(FrontendPlatformToolSet::BoardRuntime, "execute_node")
            .expect("board-scoped execute_node spec");
    let node_args = serde_json::json!({ "board_id": "board", "node_id": "node" });
    assert!(
        missing_required_args(&execute_node, &node_args).is_none(),
        "app_id is supplied by FrontendToolContext after scoped validation"
    );
    assert!(matches!(
        execute_node.approval,
        ToolApprovalSpec::Execute { .. }
    ));

    let global_execute_node =
        frontend_platform_tool_spec(FrontendPlatformToolSet::Global, "execute_node")
            .expect("global execute_node spec");
    assert!(
        missing_required_args(&global_execute_node, &node_args)
            .is_some_and(|error| error.contains("app_id")),
        "the global schema must not accidentally validate board-scoped calls"
    );

    let query_logs = frontend_platform_tool_spec(
        FrontendPlatformToolSet::BoardRuntime,
        "query_execution_logs",
    )
    .expect("board-scoped log-query spec");
    assert!(
        missing_required_args(
            &query_logs,
            &serde_json::json!({ "board_id": "board", "run_id": "run" }),
        )
        .is_none()
    );
    assert!(matches!(query_logs.approval, ToolApprovalSpec::None));
}

#[test]
fn specialist_capabilities_follow_exact_tool_policy() {
    let frontend = FlowPilotAgentCapabilitySet::shared_for(CopilotScope::Frontend, false, false);
    assert_eq!(
        frontend.tool_names,
        vec![
            "call_app_chat".to_string(),
            "emit_ui".to_string(),
            "execute_event".to_string(),
            "get_component_schema".to_string(),
            "interact_app_page".to_string(),
            "query_execution_logs".to_string(),
            "ui_inspect".to_string(),
        ]
    );

    let board = FlowPilotAgentCapabilitySet::shared_for(CopilotScope::Board, true, true);
    let board_names = board
        .tool_names
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    assert_eq!(
        board_names,
        specialist_tool_policy(CopilotScope::Board, true, true)
    );
    for board_tool in [
        "search_workspace",
        "read_symbol",
        "get_current_flowscript",
        "write_flowscript",
        "patch_flowscript",
        "check_flowscript",
        "test_flowscript",
        "commit_flowscript",
        "database_tool",
        "storage_tool",
        "ui_inspect",
        "execute_event",
        "execute_node",
        "query_execution_logs",
    ] {
        assert!(
            board_names.contains(board_tool),
            "board must expose {board_tool}"
        );
    }
    assert!(!board_names.contains("emit_ui"));
    assert!(!board_names.contains("graph_overlay_tool"));
    assert!(!board_names.contains("internet_search"));
    assert!(!board_names.contains("open_url"));
    assert!(!board_names.contains("archive_lookup"));
    assert!(!board_names.contains("ask_user"));

    let data = FlowPilotAgentCapabilitySet::shared_for(CopilotScope::DataStudio, false, false);
    let data_names = data
        .tool_names
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    assert_eq!(
        data_names,
        specialist_tool_policy(CopilotScope::DataStudio, false, false)
    );
    for data_tool in [
        "database_tool",
        "graph_overlay_tool",
        "graph_query_tool",
        "graph_element_tool",
        "ontology_action_tool",
        "list_apps",
        "describe_app_interface",
    ] {
        assert!(
            data_names.contains(data_tool),
            "Data Studio must expose {data_tool}"
        );
    }
    for foreign_tool in [
        "emit_ui",
        "write_flowscript",
        "execute_node",
        "storage_tool",
        "internet_search",
        "open_url",
        "archive_lookup",
        "ask_user",
    ] {
        assert!(!data_names.contains(foreign_tool));
    }

    let home = FlowPilotAgentCapabilitySet::shared_for(CopilotScope::Home, false, false);
    assert_eq!(
        home.tool_names,
        vec![
            "apply_home_layout".to_string(),
            "describe_app_interface".to_string(),
            "get_home_context".to_string(),
            "get_home_widget_catalog".to_string(),
            "list_apps".to_string(),
            "list_home_data_sources".to_string(),
            "validate_home_layout".to_string(),
        ]
    );

    for legacy_typed_tool in [
        "plan_flow_ir",
        "begin_flow_ir_draft",
        "update_flow_ir_draft",
        "upsert_flow_ir_module",
        "validate_flow_ir_draft",
        "commit_flow_ir_draft",
    ] {
        assert!(!board_names.contains(legacy_typed_tool));
        assert!(!data_names.contains(legacy_typed_tool));
    }

    assert_eq!(
        board.prompt_source, "flow_like::copilot::prompts",
        "all agent backends must use the shared prompt module"
    );
}

#[test]
fn bits_specialists_advertise_exactly_the_agent_backend_tool_policy() {
    use flow_like::flow::copilot::tool_spec::{
        data_studio_specialist_tool_specs, home_specialist_tool_specs, scout_specialist_tool_specs,
    };

    // The Bits loop advertises these specs directly, while the agent-CLI backends filter their
    // SDK tools through `specialist_tool_policy`. A model must not gain or lose authority by
    // switching backends, so the two lists are one contract.
    for (scope, specs) in [
        (
            CopilotScope::DataStudio,
            data_studio_specialist_tool_specs(),
        ),
        (CopilotScope::Scout, scout_specialist_tool_specs()),
        (CopilotScope::Home, home_specialist_tool_specs()),
    ] {
        let advertised = specs
            .iter()
            .map(|spec| spec.name)
            .collect::<HashSet<&'static str>>();
        assert_eq!(
            advertised,
            specialist_tool_policy(scope, false, false),
            "the Bits and agent-CLI tool sets for {scope:?} have drifted"
        );
        // Every advertised tool must also resolve a spec through the bridge, or its approval
        // policy and timeout would silently fall back to the unspecced defaults.
        for name in advertised {
            let tool_set = match scope {
                CopilotScope::DataStudio => FrontendPlatformToolSet::DataStudio,
                CopilotScope::Scout => FrontendPlatformToolSet::Scout,
                CopilotScope::Home => FrontendPlatformToolSet::Home,
                _ => unreachable!("only direct platform specialists are tested"),
            };
            assert!(
                frontend_platform_tool_spec(tool_set, name).is_some(),
                "{scope:?} bridge cannot resolve the spec for {name}"
            );
            assert!(
                global_orchestrator_tool_scope_error(tool_set, name).is_none(),
                "{scope:?} must be allowed to call its own tool {name}"
            );
        }
    }
}

#[test]
fn workspace_source_tools_stay_out_of_ui_home_and_public_research() {
    for scope in [
        CopilotScope::Board,
        CopilotScope::Both,
        CopilotScope::Scout,
        CopilotScope::DataStudio,
    ] {
        let allowed = specialist_tool_policy(scope, true, true);
        assert!(allowed.contains("search_workspace"));
        assert!(allowed.contains("read_symbol"));
    }
    for scope in [
        CopilotScope::Frontend,
        CopilotScope::Home,
        CopilotScope::Research,
    ] {
        let allowed = specialist_tool_policy(scope, true, true);
        assert!(!allowed.contains("search_workspace"));
        assert!(!allowed.contains("read_symbol"));
    }
    for name in ["search_workspace", "read_symbol"] {
        assert!(is_flowpilot_read_only_tool(name));
        assert!(frontend_platform_tool_spec(FrontendPlatformToolSet::Global, name).is_none());
    }
}

#[test]
fn bits_home_review_matches_agent_read_only_tool_authority() {
    use flow_like::flow::copilot::tool_spec::home_specialist_tool_specs_for_access;

    let agent_tools = specialist_tool_policy(CopilotScope::Home, false, false)
        .into_iter()
        .filter(|name| is_flowpilot_read_only_tool(name))
        .collect::<HashSet<_>>();
    let bits_tools = home_specialist_tool_specs_for_access(true)
        .into_iter()
        .map(|spec| spec.name)
        .collect::<HashSet<_>>();
    assert_eq!(bits_tools, agent_tools);
    for name in bits_tools {
        assert!(frontend_platform_tool_spec(FrontendPlatformToolSet::HomeReadOnly, name).is_some());
    }
    for name in [
        "apply_home_layout",
        "database_tool",
        "emit_ui",
        "unknown_tool",
    ] {
        assert!(frontend_platform_tool_spec(FrontendPlatformToolSet::HomeReadOnly, name).is_none());
    }
    assert!(
        frontend_platform_tool_spec(FrontendPlatformToolSet::Home, "apply_home_layout").is_some()
    );
}

#[test]
fn specialist_host_context_names_the_ids_the_host_already_knows() {
    let context = specialist_host_context(
        Some(&FrontendToolContext {
            app_id: Some("app-1".to_string()),
            overlay_id: Some("crm".to_string()),
            ..Default::default()
        }),
        Some("## HOST RUN CONTEXT\nrun-1"),
    );
    assert!(context.contains("- app_id: app-1"));
    assert!(context.contains("- overlay_id: crm"));
    assert!(context.contains("## HOST RUN CONTEXT"));

    assert!(specialist_host_context(None, None).is_empty());
    assert!(
        specialist_host_context(
            Some(&FrontendToolContext {
                app_id: Some("   ".to_string()),
                ..Default::default()
            }),
            None,
        )
        .is_empty()
    );
}

#[test]
fn global_agent_capability_set_advertises_only_sealed_public_research() {
    let capabilities =
        FlowPilotAgentCapabilitySet::for_surface(CopilotScope::Both, true, true, true);
    assert!(
        capabilities
            .tool_names
            .iter()
            .any(|name| name == RESEARCH_AGENT_TOOL)
    );
    for raw_web_tool in ["internet_search", "open_url", "archive_lookup"] {
        assert!(
            !capabilities
                .tool_names
                .iter()
                .any(|name| name == raw_web_tool)
        );
    }

    for scope in [
        CopilotScope::Board,
        CopilotScope::Frontend,
        CopilotScope::Both,
        CopilotScope::DataStudio,
        CopilotScope::Home,
    ] {
        let specialist = FlowPilotAgentCapabilitySet::for_surface(scope, true, true, false);
        for tool in ["internet_search", "open_url", "archive_lookup"] {
            assert!(
                !specialist.tool_names.iter().any(|name| name == tool),
                "specialist surface {scope:?} must not advertise global-only tool {tool}"
            );
        }
    }
}

#[test]
fn scope_neutral_backend_status_does_not_claim_global_web_tools() {
    let capabilities =
        FlowPilotAgentCapabilitySet::for_status(FlowPilotAgentTransportKind::DirectSdkTools);
    for tool in ["internet_search", "open_url", "archive_lookup"] {
        assert!(
            !capabilities.tool_names.iter().any(|name| name == tool),
            "scope-neutral backend status must not advertise global-only tool {tool}"
        );
    }
}

#[test]
fn board_explain_policy_is_an_exact_read_only_allowlist() {
    for tool in [
        "catalog_search",
        "get_declarations",
        "get_current_flowscript",
        "get_node_details",
        "get_unconfigured_nodes",
        "list_board_nodes",
        "database_tool",
        "storage_tool",
        "ui_inspect",
        "query_execution_logs",
        "get_home_context",
        "get_home_widget_catalog",
        "list_home_data_sources",
        "validate_home_layout",
    ] {
        assert!(
            is_flowpilot_read_only_tool(tool),
            "read-only inspection should retain {tool}"
        );
    }
    for tool in [
        "execute_event",
        "execute_node",
        "interact_app_page",
        "call_app_chat",
        "emit_commands",
        "write_flowscript",
        "patch_flowscript",
        "check_flowscript",
        "test_flowscript",
        "commit_flowscript",
        "emit_ui",
        "graph_overlay_tool",
        "apply_home_layout",
        "ask_user",
    ] {
        assert!(
            !is_flowpilot_read_only_tool(tool),
            "read-only FlowPilot surfaces must hide {tool}"
        );
    }

    // Reading a public page mutates nothing, so the web tools ARE read-only and
    // survive explain-mode filtering — that is what lets the Research scope run
    // read-only. What keeps them away from board scope is the tool POLICY, not
    // this predicate, and the policy is the stronger invariant to assert.
    for tool in ["internet_search", "open_url", "archive_lookup"] {
        assert!(
            is_flowpilot_read_only_tool(tool),
            "{tool} reads without mutating"
        );
    }
    for scope in [
        CopilotScope::Board,
        CopilotScope::Frontend,
        CopilotScope::Both,
        CopilotScope::DataStudio,
        CopilotScope::Scout,
        CopilotScope::Home,
    ] {
        let policy = specialist_tool_policy(scope, true, true);
        for tool in ["internet_search", "open_url", "archive_lookup"] {
            assert!(
                !policy.contains(tool),
                "{scope:?} must not reach the public web; only Research holds {tool}"
            );
        }
    }
    let research = specialist_tool_policy(CopilotScope::Research, false, false);
    for tool in ["internet_search", "open_url", "archive_lookup"] {
        assert!(research.contains(tool), "Research owns {tool}");
    }
    // And it owns nothing else: no app, data, storage or memory reach.
    assert_eq!(research.len(), 3, "Research must hold ONLY the web tools");
}

#[test]
fn external_frontend_prompt_has_no_workflow_lifecycle() {
    let prompt = build_external_agent_prompt(
        "frontend-system",
        "Build a dashboard and wire its save button",
        CopilotScope::Frontend,
        false,
        false,
    );
    assert!(prompt.contains("You are the UI specialist"));
    assert!(prompt.contains("parent must call the board specialist"));
    for lifecycle_tool in [
        "get_current_flowscript",
        "write_flowscript",
        "patch_flowscript",
        "check_flowscript",
        "test_flowscript",
        "commit_flowscript",
    ] {
        assert!(!prompt.contains(lifecycle_tool));
    }
}

#[test]
fn external_home_prompt_enforces_the_layout_boundary() {
    let prompt = build_external_agent_prompt(
        "home-system",
        "Improve my Home landing page",
        CopilotScope::Home,
        false,
        false,
    );
    assert!(prompt.contains("You are the HOME specialist"));
    assert!(prompt.contains("current profile's Home landing-page layout JSON"));
    assert!(prompt.contains("validate the complete candidate"));
    assert!(prompt.contains("apply_home_layout"));
    assert!(prompt.contains("pure explain or review request"));
    assert!(prompt.contains("answer without staging"));
    assert!(prompt.contains("Never author FlowScript"));
}

#[test]
fn external_global_prompt_keeps_the_platform_orchestrator_role() {
    let prompt = build_external_agent_prompt(
        "global-system",
        "Create an app with a widget, data table, board, and event",
        CopilotScope::Frontend,
        false,
        true,
    );
    assert!(prompt.contains("You are the PLATFORM orchestrator"));
    assert!(prompt.contains("coordinate every required specialist"));
    assert!(prompt.contains("only when the user explicitly requests Home work"));
    assert!(prompt.contains("Keep Home out of ordinary app builds"));
    assert!(!prompt.contains("You are the UI specialist"));
}

#[test]
fn mcp_server_instructions_are_derived_from_specialist_tools() {
    let ui = flowpilot_mcp_server_instructions(["emit_ui", "get_component_schema"], false);
    assert!(ui.contains("UI specialist"));
    assert!(ui.contains("Hand workflow wiring back to the board specialist"));
    assert!(!ui.contains("write_flowscript"));

    let board = flowpilot_mcp_server_instructions(
        [
            "get_current_flowscript",
            "write_flowscript",
            "commit_flowscript",
        ],
        true,
    );
    assert!(board.contains("BOARD specialist"));
    assert!(board.contains("commit_flowscript"));
    assert!(board.contains("Cross-domain context tools are read-only"));

    let global = flowpilot_mcp_server_instructions(
        [
            "list_apps",
            "flowpilot_board",
            "flowpilot_home",
            RESEARCH_AGENT_TOOL,
        ],
        false,
    );
    assert!(global.contains("platform orchestrator"));
    assert!(global.contains("DIRECT"));
    assert!(global.contains("SOLVE"));
    assert!(global.contains("at least three apps/interfaces"));
    assert!(global.contains("Active configured chat/page/headless Events"));
    assert!(global.contains("including REST/API and MCP"));
    assert!(global.contains("Call data_studio_agent directly"));
    assert!(global.contains("on existing apps as well as during a build"));
    assert!(global.contains("it needs no preflight"));
    assert!(global.contains("BUILD"));
    assert!(global.contains("flowpilot_home"));
    assert!(global.contains("only when the user explicitly requests"));
    assert!(global.contains("Keep it out of ordinary app builds"));
    assert!(global.contains("sealed no-argument research_agent"));
    assert!(global.len() < 2_000);

    let native_web_global = flowpilot_mcp_server_instructions(
        ["list_apps", "flowpilot_board", "flowpilot_home"],
        false,
    );
    assert!(native_web_global.contains("platform orchestrator"));
    assert!(native_web_global.contains("built-in web search"));
    assert!(native_web_global.contains("one that errors never blocks web research"));
    assert!(native_web_global.contains("Specialists have no web access"));
    assert!(!native_web_global.contains("research_agent"));
    assert!(native_web_global.len() < 2_000);

    let home = flowpilot_mcp_server_instructions(
        [
            "get_home_context",
            "validate_home_layout",
            "apply_home_layout",
        ],
        false,
    );
    assert!(home.contains("HOME specialist"));
    assert!(home.contains("current profile's Home landing-page layout JSON"));
    assert!(home.contains("validate the complete candidate"));
    assert!(home.contains("Never author A2UI pages"));

    let home_read_only =
        flowpilot_mcp_server_instructions(["get_home_context", "validate_home_layout"], false);
    assert!(home_read_only.contains("read-only FlowPilot HOME specialist"));
    assert!(home_read_only.contains("answer without staging a change"));
    assert!(!home_read_only.contains("apply_home_layout"));
}

#[test]
fn source_lifecycle_classification_keeps_commit_boundary_explicit() {
    for tool in [
        "write_flowscript",
        "patch_flowscript",
        "check_flowscript",
        "commit_flowscript",
    ] {
        assert!(is_workflow_loop_tool(tool));
        assert!(is_flowscript_draft_operation_tool(tool));
        assert!(is_order_sensitive_workflow_tool(tool));
    }
    assert!(!is_workflow_commit_tool("write_flowscript"));
    assert!(!is_workflow_commit_tool("patch_flowscript"));
    assert!(!is_workflow_commit_tool("check_flowscript"));
    assert!(is_workflow_commit_tool("commit_flowscript"));
    assert!(is_workflow_commit_tool("edit_flowscript"));
    assert!(is_workflow_loop_tool("test_flowscript"));
    assert!(is_order_sensitive_workflow_tool("test_flowscript"));
    assert!(!is_flowscript_draft_operation_tool("test_flowscript"));
    assert!(!is_workflow_commit_tool("test_flowscript"));
}

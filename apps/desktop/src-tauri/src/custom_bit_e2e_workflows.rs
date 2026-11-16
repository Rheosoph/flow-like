use std::{sync::Arc, time::Duration};

use flow_like::{
    app::AppVisibility,
    bit::{Bit, BitModelPreference},
    flow::{
        board::{Board, ExecutionStage},
        execution::{
            InternalRun, LogLevel, RunPayload, context::ExecutionContext,
            internal_node::InternalNode,
        },
        node::{Node, NodeLogic, NodeState},
    },
    flow_like_model_provider::{
        history::{History, HistoryMessage, Role},
        response::{LLMUsageStats, Response},
    },
    models::device::Interaction,
    profile::Profile,
    state::FlowLikeState,
};
use flow_like_catalog::generative::{
    agent::{Agent, from_model::AgentFromModelNode, invoke::InvokeAgentNode},
    llm::{find_llm::FindLLMNode, invoke::InvokeLLM},
};
use flow_like_types::{Result, Value, anyhow, json::json, tokio::time::timeout};

/// Called only by the isolated desktop E2E command after reloading its saved profile.
pub(crate) async fn execute_workflows(
    state: Arc<FlowLikeState>,
    profile: Arc<Profile>,
    bit_id: &str,
    nonce: &str,
) -> Result<Value, String> {
    if nonce.is_empty()
        || nonce.len() > 100
        || !nonce
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("Live workflow checks require a bounded alphanumeric nonce".into());
    }
    let bit = profile
        .custom_bits
        .iter()
        .find(|custom| custom.0.id == bit_id)
        .map(|custom| custom.0.clone())
        .ok_or_else(|| "The saved profile did not reload the selected custom Bit".to_string())?;
    let provider = bit
        .try_to_provider()
        .ok_or_else(|| "The saved custom Bit has invalid model parameters".to_string())?;
    if !matches!(
        provider.provider_name.as_str(),
        "custom:claude-code" | "custom:codex"
    ) {
        return Err(
            "Live workflow checks accept only the seeded Claude Code and Codex Bits".into(),
        );
    }

    let invoke_marker = format!("{nonce}-{bit_id}-invoke");
    let invoke = match timeout(
        Duration::from_secs(180),
        run_profile_invoke(state.clone(), profile.clone(), &bit, &invoke_marker),
    )
    .await
    {
        Ok(Ok(evidence)) => evidence,
        Ok(Err(error)) => failure(&bit, &invoke_marker, &error.to_string()),
        Err(_) => failure(
            &bit,
            &invoke_marker,
            "Find Model / Invoke Model exceeded 180 seconds",
        ),
    };

    // Both assertions describe the same generation through Find Model -> Invoke Model.
    let mut use_case = invoke.clone();
    use_case["path"] = json!("profile-find-model");
    let mut invoke_model = invoke;
    invoke_model["path"] = json!("invoke-model");

    let agent_marker = format!("{nonce}-{bit_id}-agent");
    let mut agent = match timeout(
        Duration::from_secs(180),
        run_agent(state, profile, &bit, &agent_marker),
    )
    .await
    {
        Ok(Ok(evidence)) => evidence,
        Ok(Err(error)) => failure(&bit, &agent_marker, &error.to_string()),
        Err(_) => failure(
            &bit,
            &agent_marker,
            "Agent from Model / Invoke Agent exceeded 180 seconds",
        ),
    };
    agent["path"] = json!("agent-from-model");
    Ok(json!({
        "useCase": use_case,
        "invokeModel": invoke_model,
        "agentFromModel": agent,
    }))
}

async fn run_profile_invoke(
    state: Arc<FlowLikeState>,
    profile: Arc<Profile>,
    expected: &Bit,
    marker: &str,
) -> Result<Value> {
    let capabilities = FlowLikeState::completion_model_capabilities(&state).await;
    let devices = state.device_model_probe(Interaction::Allowed {
        run_label: Some(marker.to_string()),
    });
    let saved_selection = profile
        .resolve_completion_model(
            Some(&expected.id),
            &BitModelPreference::default(),
            false,
            capabilities,
            devices.as_ref(),
            state.http_client.clone(),
        )
        .await?;
    if saved_selection.id != expected.id || saved_selection.parameters != expected.parameters {
        return Err(anyhow!(
            "Saved use-case selection resolved `{}` instead of `{}`",
            saved_selection.id,
            expected.id,
        ));
    }
    let find_logic: Arc<dyn NodeLogic> = Arc::new(FindLLMNode::default());
    let invoke_logic: Arc<dyn NodeLogic> = Arc::new(InvokeLLM::default());
    let mut find = node(find_logic.as_ref(), "find-model", &[])?;
    let mut invoke = node(
        invoke_logic.as_ref(),
        "invoke-model",
        &[("history", json!(history(marker)))],
    )?;
    connect(&mut find, "model", &mut invoke, "model")?;
    let run = run(
        &state,
        &profile,
        marker,
        "find-model",
        vec![(find, find_logic), (invoke, invoke_logic)],
    )
    .await?;
    let find = &run.nodes["find-model"];
    let invoke = &run.nodes["invoke-model"];
    let mut context = context(&state, &run, find).await;
    trigger(&mut context).await?;
    let selected: Bit = value(&find, "model").await?;
    if selected.id != expected.id || selected.parameters != expected.parameters {
        return Err(anyhow!(
            "Find Model selected `{}` instead of the saved custom Bit `{}`",
            selected.id,
            expected.id,
        ));
    }
    let runtime_model = verify_runtime_model(&context, &selected).await?;
    let mut invoke_context = context.create_sub_context(&invoke).await;
    trigger(&mut invoke_context).await?;
    let response: Response = value(&invoke, "result").await?;
    let stats: LLMUsageStats = value(&invoke, "stats").await?;
    let response_model_matches = response.model.as_deref() == Some(runtime_model.as_str());
    let mut evidence =
        completion_evidence(expected, &selected, marker, &runtime_model, response, stats);
    evidence["responseModelMatches"] = json!(response_model_matches);
    if !response_model_matches {
        evidence["passed"] = json!(false);
        evidence["error"] = json!("Invoke Model reported a different runtime model");
    }
    evidence["profileSelectionVerified"] = json!(true);
    evidence["savedUseCaseBitId"] = json!(saved_selection.id);
    evidence["nodes"] = json!([find.node_name(), invoke.node_name()]);
    evidence["runId"] = json!(run.meta.run_id);
    Ok(evidence)
}

async fn run_agent(
    state: Arc<FlowLikeState>,
    profile: Arc<Profile>,
    expected: &Bit,
    marker: &str,
) -> Result<Value> {
    let selected = profile
        .find_bit(&expected.id, state.http_client.clone())
        .await?;
    let builder_logic: Arc<dyn NodeLogic> = Arc::new(AgentFromModelNode::default());
    let invoke_logic: Arc<dyn NodeLogic> = Arc::new(InvokeAgentNode::default());
    let mut builder = node(
        builder_logic.as_ref(),
        "agent-from-model",
        &[
            ("model", json!(selected)),
            ("max_iter", json!(2)),
            ("infinite_context", json!(false)),
        ],
    )?;
    let mut invoke = node(
        invoke_logic.as_ref(),
        "invoke-agent",
        &[("history", json!(history(marker)))],
    )?;
    connect(&mut builder, "agent_out", &mut invoke, "agent")?;
    let run = run(
        &state,
        &profile,
        marker,
        "invoke-agent",
        vec![(builder, builder_logic), (invoke, invoke_logic)],
    )
    .await?;
    let builder = &run.nodes["agent-from-model"];
    let invoke = &run.nodes["invoke-agent"];
    let mut context = context(&state, &run, invoke).await;
    let runtime_model = verify_runtime_model(&context, &selected).await?;
    trigger(&mut context).await?;
    let agent: Agent = value(&builder, "agent_out").await?;
    let response: Response = value(&invoke, "response").await?;
    let stats: LLMUsageStats = value(&invoke, "stats").await?;
    let mut evidence = completion_evidence(
        expected,
        &agent.model,
        marker,
        &runtime_model,
        response,
        stats,
    );
    evidence["nodes"] = json!([builder.node_name(), invoke.node_name()]);
    evidence["runId"] = json!(run.meta.run_id);
    Ok(evidence)
}

fn history(marker: &str) -> History {
    let mut history = History::new(
        String::new(),
        vec![HistoryMessage::from_string(
            Role::User,
            &format!("Reply with exactly this verification marker: {marker}"),
        )],
    );
    history.max_completion_tokens = Some(1024);
    history.set_stream(false);
    history
}

fn completion_evidence(
    expected: &Bit,
    selected: &Bit,
    marker: &str,
    runtime_model: &str,
    response: Response,
    stats: LLMUsageStats,
) -> Value {
    let text = response
        .choices
        .iter()
        .filter_map(|choice| choice.message.content.as_deref())
        .collect::<Vec<_>>()
        .join("\n");
    let selected_matches = expected.id == selected.id && expected.parameters == selected.parameters;
    let runtime_matches = configured_model_id(expected).as_deref() == Some(runtime_model);
    let nonce_matches = text.contains(marker);
    let output_tokens = stats.usage.completion_tokens;
    let passed = selected_matches && runtime_matches && nonce_matches && output_tokens > 0;
    let mut evidence = identity(selected, marker);
    evidence["requestedBitId"] = json!(expected.id);
    evidence["passed"] = json!(passed);
    evidence["selectedBitMatches"] = json!(selected_matches);
    evidence["runtimeModelId"] = json!(runtime_model);
    evidence["runtimeModelMatches"] = json!(runtime_matches);
    evidence["nonceMatched"] = json!(nonce_matches);
    evidence["text"] = json!(text);
    // Some adapters retain the requested model or its display name in this field.
    evidence["responseModel"] = json!(response.model);
    evidence["responseModelSource"] = json!("adapter");
    evidence["usage"] = json!(stats.usage);
    evidence["outputTokens"] = json!(output_tokens);
    evidence["stats"] = json!(stats);
    if !passed {
        evidence["error"] = json!(
            "Completion must retain the selected Bit and runtime model, include the nonce, and report output tokens"
        );
    }
    evidence
}

fn configured_model_id(bit: &Bit) -> Option<String> {
    bit.try_to_provider().and_then(|provider| {
        provider
            .params
            .as_ref()
            .and_then(|params| params.get("model_id"))
            .and_then(Value::as_str)
            .filter(|model| !model.trim().is_empty())
            .map(str::to_owned)
            .or_else(|| provider.model_id.clone())
    })
}

async fn verify_runtime_model(context: &ExecutionContext, bit: &Bit) -> Result<String> {
    let configured = configured_model_id(bit)
        .filter(|model| !model.trim().is_empty() && !model.eq_ignore_ascii_case("default"))
        .ok_or_else(|| anyhow!("The persisted custom Bit must name a specific model"))?;
    let model = context
        .app_state
        .model_factory
        .build(
            bit,
            context.app_state.clone(),
            context.token.clone(),
            context.model_usage_context(),
        )
        .await?;
    let runtime_model = model
        .default_model()
        .await
        .ok_or_else(|| anyhow!("The model factory returned no runtime model identity"))?;
    if runtime_model != configured {
        return Err(anyhow!(
            "The model factory selected `{runtime_model}` instead of configured model `{configured}`"
        ));
    }
    Ok(runtime_model)
}

fn identity(bit: &Bit, marker: &str) -> Value {
    let provider = bit.try_to_provider();
    json!({
        "generationId": marker,
        "resolvedBitId": bit.id,
        "providerName": provider.as_ref().map(|provider| &provider.provider_name),
        "configuredModelId": configured_model_id(bit),
    })
}

fn failure(bit: &Bit, marker: &str, error: &str) -> Value {
    let mut evidence = identity(bit, marker);
    evidence["requestedBitId"] = json!(bit.id);
    evidence["resolvedBitId"] = Value::Null;
    evidence["passed"] = json!(false);
    evidence["error"] = json!(error);
    evidence
}

fn node(logic: &dyn NodeLogic, id: &str, inputs: &[(&str, Value)]) -> Result<Node> {
    let mut node = logic.get_node();
    node.id = id.to_string();
    for (name, value) in inputs {
        node.get_pin_mut_by_name(name)
            .ok_or_else(|| anyhow!("Catalog input `{name}` is missing"))?
            .set_default_value(Some(value.clone()));
    }
    Ok(node)
}

fn connect(source: &mut Node, output: &str, target: &mut Node, input: &str) -> Result<()> {
    let output = source
        .get_pin_mut_by_name(output)
        .ok_or_else(|| anyhow!("Catalog output `{output}` is missing"))?;
    let input = target
        .get_pin_mut_by_name(input)
        .ok_or_else(|| anyhow!("Catalog input `{input}` is missing"))?;
    output.connected_to.insert(input.id.clone());
    input.depends_on.insert(output.id.clone());
    Ok(())
}

async fn run(
    state: &Arc<FlowLikeState>,
    profile: &Profile,
    marker: &str,
    entry: &str,
    nodes: Vec<(Node, Arc<dyn NodeLogic>)>,
) -> Result<InternalRun> {
    let mut board = Board::new_detached(Some(marker.to_string()), Default::default());
    for (node, logic) in nodes {
        state.node_registry().write().await.push_node(logic);
        board.nodes.insert(node.id.clone(), node);
    }
    let mut run = InternalRun::new(
        "custom-bit-e2e-workflow",
        Arc::new(board),
        None,
        state,
        profile,
        &RunPayload {
            id: entry.to_string(),
            payload: None,
            runtime_variables: None,
            filter_secrets: None,
        },
        false,
        None,
        None,
        None,
        Default::default(),
    )
    .await?;
    run.set_usage_attribution_from_visibility(&AppVisibility::Offline)
        .await;
    run.set_offline_user_context();
    Ok(run)
}

async fn context(
    state: &Arc<FlowLikeState>,
    run: &InternalRun,
    node: &Arc<InternalNode>,
) -> ExecutionContext {
    ExecutionContext::with_meta(
        run.nodes.clone(),
        &Arc::downgrade(&run.run),
        &run.meta,
        state,
        node,
        &run.variables,
        &run.cache,
        LogLevel::Info,
        ExecutionStage::Dev,
        run.profile.clone(),
        run.callback.clone(),
        Arc::new(flow_like_types::sync::RwLock::new(Vec::new())),
        None,
        None,
        run.oauth_tokens.clone(),
        Some(run.channel.clone()),
    )
    .await
}

async fn trigger(context: &mut ExecutionContext) -> Result<()> {
    InternalNode::trigger(context, &mut None, false)
        .await
        .map_err(|error| anyhow!("Workflow node failed: {error:?}"))?;
    if context.get_state() != NodeState::Success {
        return Err(anyhow!("Workflow node did not complete successfully"));
    }
    Ok(())
}

async fn value<T: serde::de::DeserializeOwned>(node: &InternalNode, name: &str) -> Result<T> {
    node.get_pin_by_name(name)
        .await?
        .get_value()
        .await
        .ok_or_else(|| anyhow!("Workflow output `{name}` is missing or invalid"))
}

use flow_like::{
    bit::Bit,
    flow::{
        board::Board,
        execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
        node::{Node, NodeLogic, NodeScores, remove_unwired_pins},
        pin::{PinOptions, PinType},
        variable::VariableType,
    },
};
use flow_like_model_provider::{
    history::{History, Tool, ToolChoice},
    response::{LLMUsageStats, Response, ResponseFunction},
};
use flow_like_types::{Error, Value, anyhow, async_trait, json, regex::Regex};
use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

/// Extract tagged substrings, e.g. Hello, <tool>extract this</tool> and <tool>this</tool>, good bye.
pub fn extract_tagged_simple(text: &str, tag: &str) -> Result<Vec<String>, Error> {
    let pattern = format!(r"(?s)<{tag}>(.*?)</{tag}>", tag = regex::escape(tag));
    let re = Regex::new(&pattern)?;
    Ok(re
        .captures_iter(text)
        .filter_map(|caps| caps.get(1).map(|m| m.as_str().to_string()))
        .collect())
}

/// Extract tagged substrings, e.g. Hello, <tool>extract this</tool> and <tool>this</tool>, good bye.
/// This is a more robust version that ignores tags not being closed.
pub fn extract_tagged(text: &str, tag: &str) -> Result<Vec<String>, Error> {
    let open_tag = format!("<{tag}>");
    let close_tag = format!("</{tag}>");

    // 1) Find all open-tag positions
    let mut starts = Vec::new();
    let mut pos = 0;
    while let Some(idx) = text[pos..].find(&open_tag) {
        let real = pos + idx;
        starts.push(real);
        pos = real + open_tag.len();
    }

    // 2) Find all close-tag positions
    let mut ends = Vec::new();
    let mut pos = 0;
    while let Some(idx) = text[pos..].find(&close_tag) {
        let real = pos + idx;
        ends.push(real);
        pos = real + close_tag.len();
    }

    // 3) For each opener, match to the first unused closer that comes after it,
    //    but only if there’s no *other* opener in between them.
    let mut used_ends = vec![false; ends.len()];
    let mut out = Vec::new();

    for &start in &starts {
        let content_start = start + open_tag.len();
        // find the first unused closing tag after this opener
        if let Some((ei, &end_pos)) = ends
            .iter()
            .enumerate()
            .find(|&(i, &e)| !used_ends[i] && e > content_start)
        {
            // check for any *other* opener nested between this opener and that closer:
            let has_inner_opener = starts.iter().any(|&other| other > start && other < end_pos);

            if has_inner_opener {
                // this opener is “orphaned” by an inner start—skip it
                continue;
            }

            // otherwise, we have a proper pair: extract, mark this closer used
            let slice = &text[content_start..end_pos];
            out.push(slice.to_string());
            used_ends[ei] = true;
        }
    }
    Ok(out)
}

#[crate::register_node]
#[derive(Default)]
pub struct InvokeLLMWithToolsNode {}

impl InvokeLLMWithToolsNode {
    pub fn new() -> Self {
        InvokeLLMWithToolsNode {}
    }
}

#[async_trait]
impl NodeLogic for InvokeLLMWithToolsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "invoke_llm_with_tools",
            "Invoke with Tools",
            "Invokes an LLM that can call Flow tools/functions and routes each call to execution pins.",
            "AI/Generative",
        );
        node.set_flowscript_name("ai", "invokeWithTools");
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_version(4);

        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(5)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(6)
                .set_cost(4)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to start the tool-enabled invocation",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Bit describing the provider/model to execute",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "history",
            "History",
            "Conversation history the model should continue from",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "tools",
            "Tools",
            "JSON array of tool/function definitions (OpenAI format)",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("[]")));

        node.add_input_pin(
            "tool_choice",
            "Tool Choice",
            "Controls whether the model must, may, or must not call tools",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Auto".to_string(),
                    "Required".to_string(),
                    "None".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json::json!("Auto")));

        node.add_output_pin(
            "exec_done",
            "Done",
            "Signals when the invocation and tool routing finished",
            VariableType::Execution,
        );

        node.add_output_pin(
            "response",
            "Response",
            "LLM response if the model answered directly without tool calls",
            VariableType::Struct,
        )
        .set_schema::<Response>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "tool_call_args",
            "Tool Call Args",
            "Parsed JSON arguments for the latest tool call",
            VariableType::Struct,
        )
        .set_open_schema();

        node.add_output_pin(
            "stats",
            "Stats",
            "Token usage, cost, and model statistics",
            VariableType::Struct,
        )
        .set_schema::<LLMUsageStats>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_done").await?;

        let model_bit = context.evaluate_pin::<Bit>("model").await?;
        let tools_str: String = context.evaluate_pin("tools").await?;
        let tool_choice: String = context.evaluate_pin("tool_choice").await?;
        let tool_choice = match tool_choice.as_str() {
            "Required" => ToolChoice::Required,
            "None" => ToolChoice::None,
            _ => ToolChoice::Auto,
        };

        let tools: Vec<Tool> =
            json::from_str(&tools_str).map_err(|e| anyhow!("Failed to parse tools: {e:?}"))?;

        for tool in &tools {
            let _ = context.deactivate_exec_pin(&tool.function.name).await;
        }

        let _provider = model_bit
            .try_to_provider()
            .ok_or_else(|| anyhow!("Model Bit does not contain provider information"))?;

        // log model name
        if let Some(meta) = model_bit.meta.get("en") {
            context.log_message(&format!("Loading model {:?}", meta.name), LogLevel::Debug);
        }

        let mut history = context.evaluate_pin::<History>("history").await?;

        if !tools.is_empty() && !matches!(tool_choice, ToolChoice::None) {
            history.tools = Some(tools.clone());
            history.tool_choice = Some(tool_choice.clone());
        }

        // --- Invoke model
        let response = {
            let model_factory = context.app_state.model_factory.clone();
            let model = model_factory
                .lock()
                .await
                .build(
                    &model_bit,
                    context.app_state.clone(),
                    context.token.clone(),
                    context.model_usage_context(),
                )
                .await?;

            let start = Instant::now();

            let res = model.invoke(&history, None).await?;
            let duration_ms = start.elapsed().as_millis() as u64;
            (res, duration_ms)
        };

        let (response, duration_ms) = response;

        // --- Parse response
        let mut response_string = String::new();
        if let Some(msg) = response.last_message() {
            response_string = msg.content.clone().unwrap_or_default();
        }
        context.log_message(
            &format!("llm response: '{}'", response_string),
            LogLevel::Debug,
        );

        let mut tool_calls: Vec<ResponseFunction> = Vec::new();

        if let Some(msg) = response.last_message() {
            tool_calls = msg
                .tool_calls
                .iter()
                .map(|tc| ResponseFunction {
                    name: tc.function.name.clone(),
                    arguments: tc.function.arguments.clone(),
                })
                .collect();
        }

        if tool_calls.is_empty() && response_string.contains("<tooluse>") {
            let tool_calls_strs = extract_tagged(&response_string, "tooluse")?;
            tool_calls = tool_calls_strs
                .iter()
                .map(|s| {
                    flow_like_types::json::from_str::<ResponseFunction>(s)
                        .map_err(|e| anyhow!("Failed to parse tool call: {e}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
        }

        // --- Execute tools if any
        if !tool_calls.is_empty() {
            if let ToolChoice::None = tool_choice {
                return Err(anyhow!("LLM made tool calls but tool choice is None!"));
            }

            let tool_call_args_pin = context.get_pin_by_name("tool_call_args").await?;
            for tc in &tool_calls {
                let args: Value = json::from_str(&tc.arguments)?;
                context.log_message(&format!("exec tool {}", tc.name), LogLevel::Debug);

                // Deactivate all tool exec pins (best-effort)
                for t in &tools {
                    let _ = context.deactivate_exec_pin(&t.function.name).await;
                }

                // Set args & activate the specific tool pin
                tool_call_args_pin.set_value(args).await;
                context.activate_exec_pin(&tc.name).await?;

                // Run connected subgraph
                let tool_exec_pin = context.get_pin_by_name(&tc.name).await?;
                let tool_flow = tool_exec_pin.get_connected_nodes();

                for node in &tool_flow {
                    let mut sub_ctx = context.create_sub_context(node).await;
                    let run = InternalNode::trigger(&mut sub_ctx, &mut None, true).await;

                    sub_ctx.end_trace();
                    context.push_sub_context(&mut sub_ctx);
                    if let Err(err) = run {
                        context.log_message(
                            &format!("Error executing tool {}: {:?}", tc.name, err),
                            LogLevel::Error,
                        );
                    }
                }
            }

            // Deactivate all tool exec pins at the end (best-effort)
            for t in &tools {
                let _ = context.deactivate_exec_pin(&t.function.name).await;
            }
        } else {
            // No tool calls; enforce Required vs return final response
            match tool_choice {
                ToolChoice::Required => {
                    return Err(anyhow!(
                        "LLM made no tool calls but at least one is required!"
                    ));
                }
                _ => {
                    context
                        .set_pin_value("response", json::json!(response))
                        .await?;
                }
            }
        }

        context.activate_exec_pin("exec_done").await?;

        let mut stats = LLMUsageStats::from_response(&response);
        stats.set_duration_ms(duration_ms);
        context.set_pin_value("stats", json::json!(stats)).await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;

        // Every output pin the definition does not declare is a tool-execution pin this
        // `on_update` minted. Deriving that from `get_node()` rather than a hand-kept exclusion
        // list is what keeps a newly added static pin from being mistaken for a stale tool pin
        // and deleted on every board parse — which is exactly what happened to `stats`.
        let declared_pins: HashSet<String> = self
            .get_node()
            .pins
            .values()
            .map(|pin| pin.name.clone())
            .collect();

        let current_tool_exec_pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output && !declared_pins.contains(&p.name))
            .collect();

        let tools_str: String = node
            .get_pin_by_name("tools")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        let mut current_tool_exec_refs = current_tool_exec_pins
            .iter()
            .map(|p| (p.name.clone(), *p))
            .collect::<HashMap<_, _>>();

        let update_tools: Vec<Tool> = match json::from_str(&tools_str) {
            Ok(tools) => tools,
            Err(err) => {
                node.error = Some(format!("Failed to parse tools: {err:?}").to_string());
                return;
            }
        };

        let mut all_tool_exec_refs = HashSet::new();
        let mut missing_tool_exec_refs = HashSet::new();

        for update_tool in update_tools {
            all_tool_exec_refs.insert(update_tool.function.name.clone());
            if current_tool_exec_refs
                .remove(&update_tool.function.name)
                .is_none()
            {
                missing_tool_exec_refs.insert(update_tool.function.name.clone());
            }
        }

        let ids_to_remove = current_tool_exec_refs
            .values()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>();
        remove_unwired_pins(node, &ids_to_remove);

        for missing_tool_ref in missing_tool_exec_refs {
            node.add_output_pin(
                &missing_tool_ref,
                &missing_tool_ref,
                "Executes when the LLM requests this tool",
                VariableType::Execution,
            );
        }
    }
}

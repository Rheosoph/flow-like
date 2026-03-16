use super::config::MemoryConfig;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    Value, async_trait,
    json::{self, json},
};

const CHARS_PER_TOKEN: usize = 4;

#[crate::register_node]
#[derive(Default)]
pub struct BuildMemoryContextNode {}

impl BuildMemoryContextNode {
    pub fn new() -> Self {
        BuildMemoryContextNode {}
    }
}

#[async_trait]
impl NodeLogic for BuildMemoryContextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "memory_build_context",
            "Build Memory Context",
            "Assembles retrieved memory records into a token-budgeted context string for injection into agent system prompts",
            "AI/Memory",
        );
        node.add_icon("/flow/icons/bot-invoke.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(9)
                .set_performance(9)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "memory_config",
            "Memory Config",
            "MemoryConfig for token budget",
            VariableType::Struct,
        )
        .set_schema::<MemoryConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "memories",
            "Memories",
            "Array of memory records from Search Memory node",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "header",
            "Header",
            "Optional header text prepended to the context block",
            VariableType::String,
        )
        .set_default_value(Some(json!("[Memory Context]")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires when context assembly completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "context_text",
            "Context Text",
            "Assembled memory context string, ready for system prompt injection",
            VariableType::String,
        );

        node.add_output_pin(
            "token_estimate",
            "Token Estimate",
            "Approximate token count of the assembled context",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let config: MemoryConfig = context.evaluate_pin("memory_config").await?;
        let memories: Vec<Value> = context.evaluate_pin("memories").await.unwrap_or_default();
        let header: String = context
            .evaluate_pin("header")
            .await
            .unwrap_or_else(|_| "[Memory Context]".to_string());

        let budget = config.max_context_tokens as usize;
        let mut output = String::new();
        let mut used_tokens = 0usize;

        if !header.is_empty() {
            output.push_str(&header);
            output.push('\n');
            used_tokens += header.len() / CHARS_PER_TOKEN + 1;
        }

        // Summaries first (they're compressed, high information density)
        let (summaries, observations): (Vec<&Value>, Vec<&Value>) =
            memories.iter().partition(|m| {
                m.get("role")
                    .and_then(|r| r.as_str())
                    .is_some_and(|r| r == "summary")
            });

        for memory in summaries.iter().chain(observations.iter()) {
            let content = match memory.get("content").and_then(|c| c.as_str()) {
                Some(c) => c,
                None => continue,
            };
            let role = memory
                .get("role")
                .and_then(|r| r.as_str())
                .unwrap_or("observation");

            let line = format!("[{}] {}\n", role, content);
            let line_tokens = line.len() / CHARS_PER_TOKEN + 1;

            if used_tokens + line_tokens > budget {
                break;
            }

            output.push_str(&line);
            used_tokens += line_tokens;
        }

        context
            .set_pin_value("context_text", json::json!(output))
            .await?;
        context
            .set_pin_value("token_estimate", json!(used_tokens as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

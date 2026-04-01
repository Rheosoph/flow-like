use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_model_provider::text_splitter::{ChunkConfig, MarkdownSplitter, TextSplitter};
use flow_like_types::{anyhow, async_trait, json::json, tokio};

#[crate::register_node]
#[derive(Default)]
pub struct ChunkTextChar {}

impl ChunkTextChar {
    pub fn new() -> Self {
        ChunkTextChar {}
    }
}

#[async_trait]
impl NodeLogic for ChunkTextChar {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "chunk_text_char",
            "Character Chunk Text",
            "Splits raw text locally using simple character-based chunking",
            "AI/Preprocessing",
        );

        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(9)
                .set_performance(9)
                .set_governance(9)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.set_long_running(true);
        node.add_icon("/flow/icons/bot-invoke.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "text",
            "Text",
            "Source string that should be chunked",
            VariableType::String,
        );

        node.add_input_pin(
            "capacity",
            "Capacity",
            "Maximum characters per chunk",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(512)));

        node.add_input_pin(
            "overlap",
            "Overlap",
            "Character overlap between adjacent chunks",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(20)));

        node.add_input_pin(
            "markdown",
            "Markdown",
            "Use Markdown-aware splitting (true) or basic splitter",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Fires when chunking is done",
            VariableType::Execution,
        );

        node.add_output_pin(
            "chunks",
            "Chunks",
            "Character chunk array",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let text: String = context.evaluate_pin("text").await?;
        let capacity: i64 = context.evaluate_pin("capacity").await?;
        let overlap: i64 = context.evaluate_pin("overlap").await?;
        let markdown: bool = context.evaluate_pin("markdown").await?;

        let chunks =
            tokio::task::spawn_blocking(move || -> flow_like_types::Result<Vec<String>> {
                if markdown {
                    let config =
                        ChunkConfig::new(capacity as usize).with_overlap(overlap as usize)?;
                    let splitter = TextSplitter::new(config);
                    Ok(splitter
                        .chunks(&text)
                        .map(|c| c.to_string())
                        .collect::<Vec<String>>())
                } else {
                    let config =
                        ChunkConfig::new(capacity as usize).with_overlap(overlap as usize)?;
                    let splitter = MarkdownSplitter::new(config);
                    Ok(splitter
                        .chunks(&text)
                        .map(|c| c.to_string())
                        .collect::<Vec<String>>())
                }
            })
            .await
            .map_err(|e| anyhow!("Blocking task failed: {}", e))??;

        context.set_pin_value("chunks", json!(chunks)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

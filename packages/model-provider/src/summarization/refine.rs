use super::config::TextChunk;
use super::prompts;
use super::{invoke_llm, noop_instructions};
use crate::llm::ModelLogic;

/// Iterative Refinement summarization: process chunks sequentially, each step
/// extending a rolling summary. Produces highly coherent output at the cost
/// of strictly sequential execution.
pub async fn refine_summarize(
    chunks: &[TextChunk],
    instructions: &str,
    entity_context: &str,
    initial_summary: &str,
    model: &dyn ModelLogic,
    model_name: &str,
) -> anyhow::Result<(String, usize)> {
    let total = chunks.len();
    if total == 0 {
        return Ok((initial_summary.to_string(), 0));
    }

    let instr = noop_instructions(instructions);
    let sys = format!("{}\n{}", prompts::system_prompt(), entity_context);
    let mut running = initial_summary.to_string();
    let mut llm_calls = 0usize;

    for chunk in chunks {
        let prompt = if running.is_empty() {
            prompts::map_prompt(&chunk.content, chunk.index, total, &instr)
        } else {
            prompts::refine_prompt(&running, &chunk.content, chunk.index, total, &instr)
        };

        running = invoke_llm(model, model_name, &sys, &prompt).await?;
        llm_calls += 1;

        tracing::debug!("Refined through chunk {}/{}", chunk.index + 1, total);
    }

    Ok((running, llm_calls))
}

use super::config::TextChunk;
use super::prompts;
use super::{invoke_llm, noop_instructions};
use crate::llm::ModelLogic;

/// Sliding Window with Memory summarization.
///
/// Maintains a fixed-size memory buffer that is updated after each chunk.
/// Unlike naive Refine, the memory buffer is explicitly size-managed — old
/// information is compressed or evicted based on importance, preventing
/// the "context pressure" problem where the running summary bloats.
///
/// After all chunks are processed, a final synthesis pass converts the
/// compressed memory into a polished summary.
pub async fn sliding_window_summarize(
    chunks: &[TextChunk],
    instructions: &str,
    entity_context: &str,
    initial_memory: &str,
    model: &dyn ModelLogic,
    model_name: &str,
    memory_budget_ratio: f32,
    chunk_capacity: usize,
) -> flow_like_types::Result<(String, usize)> {
    let total = chunks.len();
    if total == 0 {
        return Ok((initial_memory.to_string(), 0));
    }

    let memory_budget = (chunk_capacity as f32 * memory_budget_ratio.clamp(0.1, 0.8)) as usize;
    let budget_hint = format!("approximately {} characters", memory_budget);
    let sys = format!("{}\n{}", prompts::system_prompt(), entity_context);
    let instr = noop_instructions(instructions);
    let mut memory = initial_memory.to_string();
    let mut llm_calls = 0usize;

    for chunk in chunks {
        let prompt = prompts::sliding_window_prompt(
            &memory,
            &chunk.content,
            chunk.index,
            total,
            &budget_hint,
            &instr,
        );

        memory = invoke_llm(model, model_name, &sys, &prompt).await?;
        llm_calls += 1;

        // Enforce memory budget: if model returns too much, we re-compress
        if memory.len() > memory_budget * 2 {
            let compress_prompt = format!(
                "Compress the following text to approximately {} characters while preserving \
                 all key facts, entities, and important details:\n\n<text>\n{}\n</text>\n\n\
                 Output ONLY the compressed text:",
                memory_budget, memory
            );
            memory = invoke_llm(model, model_name, &sys, &compress_prompt).await?;
            llm_calls += 1;
        }

        tracing::debug!(
            "Sliding window chunk {}/{}, memory size: {} chars",
            chunk.index + 1,
            total,
            memory.len()
        );
    }

    // Final synthesis: convert compressed memory to polished summary
    let finalize_prompt = prompts::sliding_window_finalize_prompt(&memory, &instr);
    let final_summary = invoke_llm(model, model_name, &sys, &finalize_prompt).await?;
    llm_calls += 1;

    Ok((final_summary, llm_calls))
}

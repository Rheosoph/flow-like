use super::config::TextChunk;
use super::map_reduce;
use super::refine;
use crate::llm::ModelLogic;

/// Hybrid Map-Reduce + Refine summarization.
///
/// Uses Map-Reduce to generate initial chunk summaries in parallel (fast),
/// then applies Refine sequentially over those summaries for narrative coherence.
/// Captures the speed advantage of Map-Reduce and the coherence of Refine.
pub async fn hybrid_summarize(
    chunks: &[TextChunk],
    instructions: &str,
    entity_context: &str,
    initial_summary: &str,
    model: &dyn ModelLogic,
    model_name: &str,
    concurrency: usize,
    chunk_capacity: usize,
) -> flow_like_types::Result<(String, usize)> {
    let total = chunks.len();
    if total == 0 {
        return Ok((initial_summary.to_string(), 0));
    }

    let mut llm_calls = 0usize;

    // Phase 1: Map — summarize each chunk independently (parallel)
    let sys = format!(
        "{}\n{}",
        super::prompts::system_prompt(),
        entity_context
    );
    let instr = super::noop_instructions(instructions);
    let effective_concurrency = if concurrency == 0 { total } else { concurrency.min(total) };

    let mut map_summaries = Vec::with_capacity(total);

    if effective_concurrency <= 1 {
        for chunk in chunks {
            let prompt = super::prompts::map_prompt(&chunk.content, chunk.index, total, &instr);
            let result = super::invoke_llm(model, model_name, &sys, &prompt).await?;
            map_summaries.push(result);
            llm_calls += 1;
        }
    } else {
        use futures::stream::{self, StreamExt};
        let tasks: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                let prompt = super::prompts::map_prompt(&chunk.content, chunk.index, total, &instr);
                let sys = sys.clone();
                let mn = model_name.to_string();
                async move { super::invoke_llm(model, &mn, &sys, &prompt).await }
            })
            .collect();

        let results: Vec<_> = stream::iter(tasks)
            .buffer_unordered(effective_concurrency)
            .collect::<Vec<_>>()
            .await;

        for r in results {
            map_summaries.push(r?);
            llm_calls += 1;
        }
    }

    tracing::debug!("Hybrid map phase: {} summaries", map_summaries.len());

    // Phase 2: Refine — sequentially refine over the map summaries for coherence
    let refine_chunks: Vec<TextChunk> = map_summaries
        .iter()
        .enumerate()
        .map(|(i, s)| TextChunk::new(s.clone(), i))
        .collect();

    let (result, refine_calls) =
        refine::refine_summarize(&refine_chunks, instructions, entity_context, initial_summary, model, model_name)
            .await?;
    llm_calls += refine_calls;

    // If still too long, do a reduce pass
    if result.len() > chunk_capacity {
        let (reduced, reduce_calls) = map_reduce::recursive_reduce(
            &[result],
            instructions,
            entity_context,
            model,
            model_name,
            chunk_capacity,
        )
        .await?;
        llm_calls += reduce_calls;
        return Ok((reduced, llm_calls));
    }

    Ok((result, llm_calls))
}

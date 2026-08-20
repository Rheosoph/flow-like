use futures::stream::{self, StreamExt};
use std::sync::Arc;

use super::config::TextChunk;
use super::prompts;
use super::{invoke_llm, noop_instructions};
use crate::llm::ModelLogic;

/// Map-Reduce summarization: summarize chunks in parallel, then reduce.
///
/// - Map phase: each chunk summarized independently (parallelizable)
/// - Reduce phase: partial summaries merged recursively until output fits
pub async fn map_reduce_summarize(
    chunks: &[TextChunk],
    instructions: &str,
    entity_context: &str,
    model: &dyn ModelLogic,
    model_name: &str,
    concurrency: usize,
    chunk_capacity: usize,
) -> flow_like_types::Result<(String, usize)> {
    let total = chunks.len();
    if total == 0 {
        return Ok((String::new(), 0));
    }

    let mut llm_calls = 0usize;

    // --- Map phase ---
    let sys = format!("{}\n{}", prompts::system_prompt(), entity_context);
    let system_prompt = Arc::new(sys);
    let instr = Arc::new(noop_instructions(instructions));
    let effective_concurrency = if concurrency == 0 {
        total
    } else {
        concurrency.min(total)
    };

    let summaries: Vec<String> = if effective_concurrency <= 1 {
        let mut results = Vec::with_capacity(total);
        for chunk in chunks {
            let prompt = prompts::map_prompt(&chunk.content, chunk.index, total, &instr);
            let result = invoke_llm(model, model_name, &system_prompt, &prompt).await?;
            results.push(result);
        }
        results
    } else {
        let model_ref: &dyn ModelLogic = model;

        // Build futures but execute with bounded concurrency
        let tasks: Vec<_> = chunks
            .iter()
            .map(|chunk| {
                let prompt = prompts::map_prompt(&chunk.content, chunk.index, total, &instr);
                let sys = Arc::clone(&system_prompt);
                let mn = model_name.to_string();
                async move { invoke_llm(model_ref, &mn, &sys, &prompt).await }
            })
            .collect();

        stream::iter(tasks)
            .buffer_unordered(effective_concurrency)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
    };

    llm_calls += summaries.len();
    tracing::debug!("Map phase complete: {} summaries", summaries.len());

    // --- Reduce phase ---
    let (result, reduce_calls) = recursive_reduce(
        &summaries,
        instructions,
        entity_context,
        model,
        model_name,
        chunk_capacity,
    )
    .await?;
    llm_calls += reduce_calls;

    Ok((result, llm_calls))
}

/// Boxed future of one reduce pass: the merged summary plus the number of LLM calls it cost.
/// Boxed because [`recursive_reduce`] recurses into itself.
pub type ReduceFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = flow_like_types::Result<(String, usize)>> + Send + 'a>,
>;

/// Recursively reduces summaries until they fit in a single context window.
pub fn recursive_reduce<'a>(
    summaries: &'a [String],
    instructions: &'a str,
    entity_context: &'a str,
    model: &'a dyn ModelLogic,
    model_name: &'a str,
    chunk_capacity: usize,
) -> ReduceFuture<'a> {
    Box::pin(async move {
        if summaries.len() <= 1 {
            return Ok((summaries.first().cloned().unwrap_or_default(), 0));
        }

        let combined_len: usize = summaries.iter().map(|s| s.len()).sum();
        let mut llm_calls = 0usize;

        if combined_len <= chunk_capacity {
            let sys = format!("{}\n{}", prompts::system_prompt(), entity_context);
            let prompt = prompts::reduce_prompt(summaries, instructions);
            let result = invoke_llm(model, model_name, &sys, &prompt).await?;
            return Ok((result, 1));
        }

        // Batch summaries into groups that fit the context
        let batches = super::chunking::batch_chunks(summaries, chunk_capacity);
        let mut batch_results = Vec::with_capacity(batches.len());
        let sys = format!("{}\n{}", prompts::system_prompt(), entity_context);

        for batch in &batches {
            if batch.len() == 1 {
                batch_results.push(batch[0].clone());
            } else {
                let parts: Vec<String> = batch.iter().map(|s| (*s).clone()).collect();
                let prompt = prompts::reduce_prompt(&parts, instructions);
                let result = invoke_llm(model, model_name, &sys, &prompt).await?;
                batch_results.push(result);
                llm_calls += 1;
            }
        }

        if batch_results.len() <= 1 {
            return Ok((
                batch_results.into_iter().next().unwrap_or_default(),
                llm_calls,
            ));
        }

        let (final_result, nested_calls) = recursive_reduce(
            &batch_results,
            instructions,
            entity_context,
            model,
            model_name,
            chunk_capacity,
        )
        .await?;
        llm_calls += nested_calls;

        Ok((final_result, llm_calls))
    })
}

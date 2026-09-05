pub mod chunking;
pub mod config;
pub mod density;
pub mod entity;
pub mod hierarchical;
pub mod hybrid;
pub mod map_reduce;
pub mod prompts;
pub mod refine;
pub mod sliding_window;

pub use config::*;

use crate::history::{History, HistoryMessage, Role};
use crate::llm::{LLMCallback, ModelLogic};
use crate::response_chunk::ResponseChunk;

fn noop_callback() -> LLMCallback {
    std::sync::Arc::new(move |_chunk: ResponseChunk| Box::pin(async move { Ok(()) }))
}

fn noop_instructions(instructions: &str) -> String {
    instructions.to_string()
}

async fn invoke_llm(
    model: &dyn ModelLogic,
    model_name: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> anyhow::Result<String> {
    let mut history = History::new(model_name.to_string(), vec![]);
    history.set_system_prompt(system_prompt.to_string());
    history.push_message(HistoryMessage::from_string(Role::User, user_prompt));

    let response = model.invoke(&history, Some(noop_callback())).await?;
    response
        .content()
        .ok_or_else(|| anyhow::anyhow!("No response content from model"))
}

/// Summarize raw text using the full pipeline: chunk → strategy → densification.
///
/// This is the primary entry point for text summarization.
pub async fn summarize(
    text: &str,
    config: &SummarizationConfig,
    model: &dyn ModelLogic,
    model_name: &str,
) -> anyhow::Result<SummarizationResult> {
    let chunks = chunking::chunk_text(
        text,
        config.chunking,
        config.chunk_size,
        config.chunk_overlap_percent,
    );

    summarize_chunks(&chunks, config, model, model_name).await
}

/// Summarize pre-chunked text. Use this when you have already split the document
/// (e.g. by pages or custom logic).
pub async fn summarize_chunks(
    chunks: &[TextChunk],
    config: &SummarizationConfig,
    model: &dyn ModelLogic,
    model_name: &str,
) -> anyhow::Result<SummarizationResult> {
    if chunks.is_empty() {
        return Ok(SummarizationResult {
            summary: String::new(),
            entities: vec![],
            section_summaries: vec![],
            stats: SummarizationStats::default(),
        });
    }

    let input_chars: usize = chunks.iter().map(|c| c.content.len()).sum();
    let mut total_llm_calls = 0usize;
    let mut section_summaries = Vec::new();

    // Entity tracking
    let mut tracker = entity::EntityTracker::new();
    let mut entity_context = String::new();

    if config.track_entities {
        // Extract entities from a sample of chunks (first, middle, last)
        let sample_indices = sample_chunk_indices(chunks.len(), 3);
        for &idx in &sample_indices {
            if let Some(chunk) = chunks.get(idx) {
                if let Err(e) = tracker
                    .extract_and_track(&chunk.content, model, model_name)
                    .await
                {
                    tracing::warn!("Entity extraction failed for chunk {}: {}", idx, e);
                }
                total_llm_calls += 1;
            }
        }
        entity_context = tracker.context_block();
        tracing::debug!("Tracked {} entities", tracker.len());
    }

    // Run the chosen strategy
    let mut iteration = 0u32;
    let mut current_summary = String::new();

    while iteration < config.max_iterations {
        let working_chunks = if iteration == 0 {
            chunks.to_vec()
        } else {
            // Re-chunk the current summary for another pass
            chunking::chunk_text(
                &current_summary,
                config.chunking,
                config.chunk_size,
                config.chunk_overlap_percent,
            )
        };

        if working_chunks.len() <= 1 && iteration > 0 {
            break;
        }

        let initial = if iteration == 0 {
            config.prior_summary.as_str()
        } else {
            ""
        };

        let (summary, calls, sections) = run_strategy(
            &working_chunks,
            config,
            &entity_context,
            initial,
            model,
            model_name,
        )
        .await?;

        total_llm_calls += calls;
        if !sections.is_empty() && iteration == 0 {
            section_summaries = sections;
        }

        current_summary = summary;
        iteration += 1;

        // Check if output fits within capacity
        if current_summary.len() <= config.chunk_size {
            break;
        }

        tracing::debug!(
            "Iteration {}: summary {} chars, target {}",
            iteration,
            current_summary.len(),
            config.chunk_size
        );
    }

    // Apply Chain of Density if configured
    let densification_applied = config.densification == DensificationStrategy::ChainOfDensity;
    if densification_applied {
        let (densified, calls) = density::apply_chain_of_density(
            &current_summary,
            &config.instructions,
            model,
            model_name,
            config.density_steps,
        )
        .await?;
        current_summary = densified;
        total_llm_calls += calls;
    }

    Ok(SummarizationResult {
        summary: current_summary.clone(),
        entities: tracker.entities(),
        section_summaries,
        stats: SummarizationStats {
            total_chunks: chunks.len(),
            llm_calls: total_llm_calls,
            strategy_used: config.strategy.as_str().to_string(),
            densification_applied,
            input_chars,
            output_chars: current_summary.len(),
        },
    })
}

async fn run_strategy(
    chunks: &[TextChunk],
    config: &SummarizationConfig,
    entity_context: &str,
    initial_summary: &str,
    model: &dyn ModelLogic,
    model_name: &str,
) -> anyhow::Result<(String, usize, Vec<SectionSummary>)> {
    match config.strategy {
        SummarizationStrategy::MapReduce => {
            let (summary, calls) = map_reduce::map_reduce_summarize(
                chunks,
                &config.instructions,
                entity_context,
                model,
                model_name,
                config.concurrency,
                config.chunk_size,
            )
            .await?;
            Ok((summary, calls, vec![]))
        }
        SummarizationStrategy::Refine => {
            let (summary, calls) = refine::refine_summarize(
                chunks,
                &config.instructions,
                entity_context,
                initial_summary,
                model,
                model_name,
            )
            .await?;
            Ok((summary, calls, vec![]))
        }
        SummarizationStrategy::Hierarchical => {
            let (summary, sections, calls) = hierarchical::hierarchical_summarize(
                chunks,
                &config.instructions,
                entity_context,
                model,
                model_name,
                config.chunk_size,
            )
            .await?;
            Ok((summary, calls, sections))
        }
        SummarizationStrategy::Hybrid => {
            let (summary, calls) = hybrid::hybrid_summarize(
                chunks,
                &config.instructions,
                entity_context,
                initial_summary,
                model,
                model_name,
                config.concurrency,
                config.chunk_size,
            )
            .await?;
            Ok((summary, calls, vec![]))
        }
        SummarizationStrategy::SlidingWindow => {
            let (summary, calls) = sliding_window::sliding_window_summarize(
                chunks,
                &config.instructions,
                entity_context,
                initial_summary,
                model,
                model_name,
                config.memory_budget_ratio,
                config.chunk_size,
            )
            .await?;
            Ok((summary, calls, vec![]))
        }
    }
}

fn sample_chunk_indices(total: usize, sample_size: usize) -> Vec<usize> {
    if total <= sample_size {
        return (0..total).collect();
    }
    let mut indices = Vec::with_capacity(sample_size);
    indices.push(0);
    if sample_size >= 2 {
        indices.push(total - 1);
    }
    if sample_size >= 3 && total > 2 {
        indices.push(total / 2);
    }
    indices.sort();
    indices.dedup();
    indices
}

use super::config::{SectionSummary, TextChunk};
use super::prompts;
use super::{invoke_llm, noop_instructions};
use crate::llm::ModelLogic;

/// Hierarchical summarization: exploits document structure to build a tree of summaries.
///
/// Detects markdown headings and groups chunks under sections, then merges
/// sibling summaries bottom-up. Falls back to balanced binary-tree grouping
/// when no heading structure is detected.
pub async fn hierarchical_summarize(
    chunks: &[TextChunk],
    instructions: &str,
    entity_context: &str,
    model: &dyn ModelLogic,
    model_name: &str,
    chunk_capacity: usize,
) -> flow_like_types::Result<(String, Vec<SectionSummary>, usize)> {
    let total = chunks.len();
    if total == 0 {
        return Ok((String::new(), vec![], 0));
    }

    let instr = noop_instructions(instructions);
    let sys = format!("{}\n{}", prompts::system_prompt(), entity_context);
    let mut llm_calls = 0usize;

    // Detect heading structure to group chunks into sections
    let sections = detect_sections(chunks);

    if sections.len() <= 1 {
        // No detectable structure: use balanced binary grouping
        return balanced_hierarchical(
            chunks,
            instructions,
            entity_context,
            model,
            model_name,
            chunk_capacity,
        )
        .await;
    }

    // Level 0: summarize each section's chunks
    let mut section_summaries = Vec::with_capacity(sections.len());
    for (heading, section_chunks) in &sections {
        let chunk_refs: Vec<&TextChunk> = section_chunks.iter().collect();
        let mut section_text = String::new();

        for c in &chunk_refs {
            if !section_text.is_empty() {
                section_text.push_str("\n\n");
            }
            section_text.push_str(&c.content);
        }

        let prompt = prompts::map_prompt(&section_text, 0, sections.len(), &instr);
        let summary = invoke_llm(model, model_name, &sys, &prompt).await?;
        llm_calls += 1;

        let indices: Vec<usize> = chunk_refs.iter().map(|c| c.index).collect();
        section_summaries.push(SectionSummary {
            title: heading.clone(),
            summary,
            chunk_indices: indices,
        });
    }

    tracing::debug!(
        "Hierarchical level 0: {} section summaries",
        section_summaries.len()
    );

    // Level 1+: merge section summaries into final
    let section_texts: Vec<String> = section_summaries
        .iter()
        .map(|s| s.summary.clone())
        .collect();
    let combined_len: usize = section_texts.iter().map(|s| s.len()).sum();

    let final_summary = if combined_len <= chunk_capacity {
        let prompt = prompts::reduce_prompt(&section_texts, &instr);
        llm_calls += 1;
        invoke_llm(model, model_name, &sys, &prompt).await?
    } else {
        let (result, calls) = super::map_reduce::recursive_reduce(
            &section_texts,
            instructions,
            entity_context,
            model,
            model_name,
            chunk_capacity,
        )
        .await?;
        llm_calls += calls;
        result
    };

    Ok((final_summary, section_summaries, llm_calls))
}

/// Balanced binary-tree grouping fallback when no headings detected.
async fn balanced_hierarchical(
    chunks: &[TextChunk],
    instructions: &str,
    entity_context: &str,
    model: &dyn ModelLogic,
    model_name: &str,
    chunk_capacity: usize,
) -> flow_like_types::Result<(String, Vec<SectionSummary>, usize)> {
    let instr = noop_instructions(instructions);
    let sys = format!("{}\n{}", prompts::system_prompt(), entity_context);
    let mut llm_calls = 0usize;

    // Group chunks into pairs/triples and summarize each group
    let group_size = 3;
    let mut level_summaries: Vec<String> = Vec::new();
    let mut section_summaries: Vec<SectionSummary> = Vec::new();

    for group in chunks.chunks(group_size) {
        let children: Vec<String> = group.iter().map(|c| c.content.clone()).collect();
        let indices: Vec<usize> = group.iter().map(|c| c.index).collect();

        let prompt = prompts::hierarchical_merge_prompt("", &children, &instr);
        let summary = invoke_llm(model, model_name, &sys, &prompt).await?;
        llm_calls += 1;

        section_summaries.push(SectionSummary {
            title: format!(
                "Group {}-{}",
                indices.first().unwrap_or(&0) + 1,
                indices.last().unwrap_or(&0) + 1
            ),
            summary: summary.clone(),
            chunk_indices: indices,
        });
        level_summaries.push(summary);
    }

    // Recursively merge until we have one summary
    while level_summaries.len() > 1 {
        let combined_len: usize = level_summaries.iter().map(|s| s.len()).sum();
        if combined_len <= chunk_capacity {
            let prompt = prompts::reduce_prompt(&level_summaries, &instr);
            let result = invoke_llm(model, model_name, &sys, &prompt).await?;
            llm_calls += 1;
            return Ok((result, section_summaries, llm_calls));
        }

        let mut next_level = Vec::new();
        for group in level_summaries.chunks(group_size) {
            if group.len() == 1 {
                next_level.push(group[0].clone());
            } else {
                let prompt = prompts::hierarchical_merge_prompt("", group, &instr);
                let result = invoke_llm(model, model_name, &sys, &prompt).await?;
                llm_calls += 1;
                next_level.push(result);
            }
        }
        level_summaries = next_level;
    }

    let final_summary = level_summaries.into_iter().next().unwrap_or_default();
    Ok((final_summary, section_summaries, llm_calls))
}

/// Detects heading-based sections in chunks. Returns (heading, chunks_in_section).
fn detect_sections(chunks: &[TextChunk]) -> Vec<(String, Vec<TextChunk>)> {
    let mut sections: Vec<(String, Vec<TextChunk>)> = Vec::new();
    let mut current_heading = String::from("Introduction");
    let mut current_chunks: Vec<TextChunk> = Vec::new();

    let heading_re = flow_like_types::regex::Regex::new(r"^#{1,4}\s+(.+)").unwrap();

    for chunk in chunks {
        // Check if this chunk starts with a heading
        let first_line = chunk.content.lines().next().unwrap_or("");
        if let Some(caps) = heading_re.captures(first_line.trim()) {
            if !current_chunks.is_empty() {
                sections.push((current_heading.clone(), current_chunks));
                current_chunks = Vec::new();
            }
            current_heading = caps.get(1).map_or("", |m| m.as_str()).to_string();
        }
        current_chunks.push(chunk.clone());
    }

    if !current_chunks.is_empty() {
        sections.push((current_heading, current_chunks));
    }

    // Only return structured sections if we found multiple headings
    if sections.len() <= 1 {
        return vec![];
    }

    sections
}

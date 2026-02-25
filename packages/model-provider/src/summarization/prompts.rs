/// Shared system prompt for all summarization strategies.
pub fn system_prompt() -> &'static str {
    "You are an expert document analyst and summarizer.\n\n\
     Rules:\n\
     - Preserve all key facts, entities, dates, numbers, and named concepts exactly as stated.\n\
     - Preserve the document's structural intent (keep markdown formatting when the source uses it).\n\
     - Do not add information, opinions, or interpretations not present in the source text.\n\
     - Do not repeat yourself. Each sentence must add unique value.\n\
     - Write in clear, direct prose unless instructed otherwise."
}

/// Map prompt: summarize an individual chunk, extracting key information.
pub fn map_prompt(chunk: &str, idx: usize, total: usize, instructions: &str) -> String {
    let instr = instruction_block(instructions);
    format!(
        "Summarize the following text section. Extract and preserve all key information, \
         entities, facts, numbers, and structural elements.\n\
         This is section {idx_1} of {total}.{instr}\n\n\
         <text>\n{chunk}\n</text>\n\n\
         Provide a comprehensive summary of this section:",
        idx_1 = idx + 1,
    )
}

/// Refine prompt: extend a running summary with new content.
pub fn refine_prompt(
    running: &str,
    chunk: &str,
    idx: usize,
    total: usize,
    instructions: &str,
) -> String {
    let instr = instruction_block(instructions);
    format!(
        "You have a running summary of a document. A new section follows.\n\
         Update and extend the summary to incorporate the new information.\n\
         This is section {idx_1} of {total}.{instr}\n\n\
         <current_summary>\n{running}\n</current_summary>\n\n\
         <new_section>\n{chunk}\n</new_section>\n\n\
         Produce an updated, unified summary integrating both. \
         Do not drop information from the current summary unless directly superseded by the new section:",
        idx_1 = idx + 1,
    )
}

/// Reduce prompt: synthesize multiple partial summaries into one.
pub fn reduce_prompt(parts: &[String], instructions: &str) -> String {
    let instr = instruction_block(instructions);
    let joined = parts
        .iter()
        .enumerate()
        .map(|(i, s)| format!("<part index=\"{}\">\n{}\n</part>", i + 1, s))
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "Synthesize the following document summaries into one coherent, comprehensive summary.{instr}\n\n\
         <summaries>\n{joined}\n</summaries>\n\n\
         The final summary must:\n\
         - Flow logically from beginning to end\n\
         - Eliminate redundancy without losing unique information\n\
         - Preserve all key facts, entities, numbers, and structural elements\n\
         - Maintain chronological or logical order from the source\n\
         - Not add or infer anything not present in the summaries above:",
    )
}

/// Hierarchical merge prompt: merge sibling section summaries under a parent heading.
pub fn hierarchical_merge_prompt(
    heading: &str,
    children: &[String],
    instructions: &str,
) -> String {
    let instr = instruction_block(instructions);
    let joined = children
        .iter()
        .enumerate()
        .map(|(i, s)| format!("<subsection index=\"{}\">\n{}\n</subsection>", i + 1, s))
        .collect::<Vec<_>>()
        .join("\n\n");
    let heading_ctx = if heading.is_empty() {
        String::new()
    } else {
        format!("\nParent section: {heading}\n")
    };
    format!(
        "Merge these subsection summaries into a single coherent summary for their parent section.{heading_ctx}{instr}\n\n\
         {joined}\n\n\
         Produce a unified summary that:\n\
         - Preserves all important details from each subsection\n\
         - Flows logically as a single narrative\n\
         - Eliminates redundancy across subsections:",
    )
}

/// Sliding window memory update prompt: compress running memory + new chunk into a fixed budget.
pub fn sliding_window_prompt(
    memory: &str,
    chunk: &str,
    idx: usize,
    total: usize,
    memory_budget_hint: &str,
    instructions: &str,
) -> String {
    let instr = instruction_block(instructions);
    format!(
        "You maintain a compressed memory of a document being read sequentially.\n\
         Update the memory to incorporate the new section below.\n\
         This is section {idx_1} of {total}.{instr}\n\n\
         IMPORTANT: The updated memory must be concise ({memory_budget_hint}). \
         Prioritize retaining:\n\
         1. Key entities, facts, dates, and numbers\n\
         2. Main arguments and conclusions\n\
         3. Information that may be referenced by later sections\n\
         Aggressively compress or drop boilerplate and repetitive details.\n\n\
         <current_memory>\n{memory}\n</current_memory>\n\n\
         <new_section>\n{chunk}\n</new_section>\n\n\
         Produce the updated, compressed memory:",
        idx_1 = idx + 1,
    )
}

/// Final synthesis prompt for sliding window: turn the memory into a proper summary.
pub fn sliding_window_finalize_prompt(memory: &str, instructions: &str) -> String {
    let instr = instruction_block(instructions);
    format!(
        "The following is a compressed memory of an entire document, built incrementally.{instr}\n\n\
         <memory>\n{memory}\n</memory>\n\n\
         Transform this into a well-written, coherent summary that:\n\
         - Reads as polished prose, not telegraphic notes\n\
         - Preserves all key facts and entities from the memory\n\
         - Flows logically from beginning to end\n\
         - Does not add any information not present in the memory:",
    )
}

/// Chain of Density initial prompt: produce a verbose first draft.
pub fn chain_of_density_initial_prompt(summary: &str, instructions: &str) -> String {
    let instr = instruction_block(instructions);
    format!(
        "You will iteratively improve the following summary to be more information-dense.{instr}\n\n\
         <current_summary>\n{summary}\n</current_summary>\n\n\
         Step 1: Identify 1-3 important entities, facts, or details from the original \
         document that are missing from this summary.\n\n\
         Step 2: Rewrite the summary to incorporate these missing elements. \
         Keep the summary approximately the same length — do not simply append new sentences. \
         Instead, compress existing text to make room for the new information.\n\n\
         Output ONLY the improved summary, nothing else:",
    )
}

/// Chain of Density iteration prompt for subsequent steps.
pub fn chain_of_density_step_prompt(
    summary: &str,
    step: u32,
    total_steps: u32,
    instructions: &str,
) -> String {
    let instr = instruction_block(instructions);
    format!(
        "Densification step {step} of {total_steps}.{instr}\n\n\
         <current_summary>\n{summary}\n</current_summary>\n\n\
         Identify 1-2 important entities or facts still missing and integrate them into the summary. \
         Keep the length approximately constant by compressing less important details. \
         Maintain coherence and readability — do not create awkward sentence fusions.\n\n\
         Output ONLY the improved summary:",
    )
}

/// Entity extraction prompt.
pub fn entity_extraction_prompt(text: &str) -> String {
    format!(
        "Extract all important named entities from the following text. \
         Include: people, organizations, locations, dates, specific numbers/statistics, \
         technical terms, product names, and key concepts.\n\n\
         <text>\n{text}\n</text>\n\n\
         Return a comma-separated list of entities. Output ONLY the list, nothing else:",
    )
}

/// Entity-aware context block to prepend to summarization prompts.
pub fn entity_context_block(entities: &[String]) -> String {
    if entities.is_empty() {
        return String::new();
    }
    let list = entities.join(", ");
    format!(
        "\n<important_entities>\nThe following entities have been identified as important in this document. \
         Ensure they are preserved in your summary when relevant: {list}\n</important_entities>\n"
    )
}

fn instruction_block(instructions: &str) -> String {
    if instructions.is_empty() {
        String::new()
    } else {
        format!("\n<instructions>\n{instructions}\n</instructions>")
    }
}

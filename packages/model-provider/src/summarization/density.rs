use super::prompts;
use super::{invoke_llm, noop_instructions};
use crate::llm::ModelLogic;

/// Chain of Density (CoD) post-processing.
///
/// Iteratively revises a summary to increase information density while keeping
/// length approximately constant. Each step identifies 1-3 missing entities and
/// integrates them by compressing less important details.
///
/// Based on research from Columbia/MIT/Salesforce showing optimal density at
/// ~0.15 entities/token, typically reached at step 3 of 5.
///
/// Returns the densified summary and number of LLM calls.
pub async fn apply_chain_of_density(
    summary: &str,
    instructions: &str,
    model: &dyn ModelLogic,
    model_name: &str,
    steps: u32,
) -> flow_like_types::Result<(String, usize)> {
    if summary.is_empty() || steps == 0 {
        return Ok((summary.to_string(), 0));
    }

    let clamped_steps = steps.min(5);
    let instr = noop_instructions(instructions);
    let sys = format!(
        "{}\n\nYou are performing Chain of Density summarization. \
         Each step should make the summary more information-dense without increasing length.",
        prompts::system_prompt()
    );

    let mut current = summary.to_string();
    let mut llm_calls = 0usize;

    // Step 1: initial densification
    let initial_prompt = prompts::chain_of_density_initial_prompt(&current, &instr);
    current = invoke_llm(model, model_name, &sys, &initial_prompt).await?;
    llm_calls += 1;

    // Steps 2..N: iterative refinement
    for step in 2..=clamped_steps {
        let prompt =
            prompts::chain_of_density_step_prompt(&current, step, clamped_steps, &instr);
        current = invoke_llm(model, model_name, &sys, &prompt).await?;
        llm_calls += 1;

        tracing::debug!("Chain of Density step {}/{}", step, clamped_steps);
    }

    Ok((current, llm_calls))
}

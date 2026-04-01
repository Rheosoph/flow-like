use std::collections::HashSet;

use super::invoke_llm;
use super::prompts;
use crate::llm::ModelLogic;

/// Tracks important entities across summarization chunks to prevent information loss.
#[derive(Debug, Clone, Default)]
pub struct EntityTracker {
    entities: HashSet<String>,
}

impl EntityTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Extracts entities from text using the LLM and adds them to the tracked set.
    pub async fn extract_and_track(
        &mut self,
        text: &str,
        model: &dyn ModelLogic,
        model_name: &str,
    ) -> flow_like_types::Result<()> {
        let max_extract_len = 6000;
        let sample = if text.len() > max_extract_len {
            &text[..max_extract_len]
        } else {
            text
        };

        let prompt = prompts::entity_extraction_prompt(sample);
        let response = invoke_llm(
            model,
            model_name,
            "You extract named entities from text. Output only a comma-separated list.",
            &prompt,
        )
        .await?;

        for entity in response.split(',') {
            let trimmed = entity.trim();
            if !trimmed.is_empty() && trimmed.len() > 1 {
                self.entities.insert(trimmed.to_string());
            }
        }

        Ok(())
    }

    /// Adds entities manually (e.g. from a prior extraction).
    pub fn add_entities(&mut self, entities: &[String]) {
        for e in entities {
            let trimmed = e.trim();
            if !trimmed.is_empty() {
                self.entities.insert(trimmed.to_string());
            }
        }
    }

    /// Returns all tracked entities as a sorted vector.
    pub fn entities(&self) -> Vec<String> {
        let mut v: Vec<String> = self.entities.iter().cloned().collect();
        v.sort();
        v
    }

    /// Produces a context block suitable for prepending to prompts.
    pub fn context_block(&self) -> String {
        prompts::entity_context_block(&self.entities())
    }

    /// Returns the number of tracked entities.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
}

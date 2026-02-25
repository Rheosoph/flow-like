use super::config::ChunkingMethod;
use crate::text_splitter::{ChunkConfig, MarkdownSplitter, TextSplitter};

use super::config::TextChunk;

/// Splits text into chunks according to the chosen method, size, and overlap.
pub fn chunk_text(
    text: &str,
    method: ChunkingMethod,
    chunk_size: usize,
    overlap_percent: u8,
) -> Vec<TextChunk> {
    if text.is_empty() {
        return vec![];
    }

    let capacity = chunk_size.max(100);
    let overlap = calculate_overlap(capacity, overlap_percent);

    match method {
        ChunkingMethod::Markdown => chunk_markdown(text, capacity, overlap),
        ChunkingMethod::FixedSize => chunk_fixed(text, capacity, overlap),
    }
}

fn calculate_overlap(capacity: usize, percent: u8) -> usize {
    let clamped = percent.min(50) as usize;
    (capacity * clamped) / 100
}

fn chunk_markdown(text: &str, capacity: usize, overlap: usize) -> Vec<TextChunk> {
    let config = match ChunkConfig::new(capacity).with_overlap(overlap) {
        Ok(c) => c,
        Err(_) => ChunkConfig::new(capacity),
    };
    let splitter = MarkdownSplitter::new(config);
    splitter
        .chunks(text)
        .enumerate()
        .map(|(i, s)| TextChunk::new(s.to_string(), i))
        .collect()
}

fn chunk_fixed(text: &str, capacity: usize, overlap: usize) -> Vec<TextChunk> {
    let config = match ChunkConfig::new(capacity).with_overlap(overlap) {
        Ok(c) => c,
        Err(_) => ChunkConfig::new(capacity),
    };
    let splitter = TextSplitter::new(config);
    splitter
        .chunks(text)
        .enumerate()
        .map(|(i, s)| TextChunk::new(s.to_string(), i))
        .collect()
}

/// Estimates token count from character length (rough heuristic: 1 token ≈ 4 chars for English).
pub fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

/// Groups chunks into batches that fit within a target combined size.
pub fn batch_chunks(chunks: &[String], batch_target_chars: usize) -> Vec<Vec<&String>> {
    let mut batches = Vec::new();
    let mut current_batch = Vec::new();
    let mut current_size = 0;

    for chunk in chunks {
        if current_size + chunk.len() > batch_target_chars && !current_batch.is_empty() {
            batches.push(current_batch);
            current_batch = Vec::new();
            current_size = 0;
        }
        current_batch.push(chunk);
        current_size += chunk.len();
    }

    if !current_batch.is_empty() {
        batches.push(current_batch);
    }

    batches
}

/// Detects markdown headings and splits text into sections preserving heading hierarchy.
/// Returns (heading_text, body_text) pairs.
pub fn split_by_headings(text: &str) -> Vec<(String, String)> {
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut current_heading = String::new();
    let mut current_body = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            if !current_heading.is_empty() || !current_body.is_empty() {
                sections.push((current_heading.clone(), current_body.trim().to_string()));
            }
            current_heading = trimmed.to_string();
            current_body = String::new();
        } else {
            if !current_body.is_empty() {
                current_body.push('\n');
            }
            current_body.push_str(line);
        }
    }

    if !current_heading.is_empty() || !current_body.is_empty() {
        sections.push((current_heading, current_body.trim().to_string()));
    }

    sections
}

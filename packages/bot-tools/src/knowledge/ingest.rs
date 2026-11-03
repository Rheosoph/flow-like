use flow_like_types::data_classification::DataClassification;
use serde_json::to_string;

use crate::convert::Converted;
use crate::embed::KnowledgeEmbedder;
use crate::error::{ToolError, ToolResult, code};
use crate::files::ResolvedFile;
use crate::knowledge::store::ChunkRow;

pub struct Chunk {
    pub text: String,
    pub start: usize,
    pub end: usize,
    pub page: Option<i64>,
}

/// Splits with the embedding model's splitter and locates each chunk in the text.
pub fn chunk_document(
    embedder: &dyn KnowledgeEmbedder,
    document: &Converted,
    max_chunks: usize,
) -> ToolResult<(Vec<Chunk>, bool)> {
    let pieces = embedder.split(&document.text)?;
    let mut chunks = Vec::new();
    let mut cursor = 0;
    let mut dropped = false;
    for piece in pieces.into_iter().filter(|piece| !piece.trim().is_empty()) {
        if chunks.len() >= max_chunks {
            dropped = true;
            break;
        }
        let start = locate(&document.text, &piece, cursor);
        let end = start + piece.len();
        cursor = start
            + document.text[start..]
                .chars()
                .next()
                .map_or(0, char::len_utf8);
        chunks.push(Chunk {
            page: document.page_at(start),
            text: piece,
            start,
            end,
        });
    }
    Ok((chunks, dropped))
}

fn locate(text: &str, piece: &str, from: usize) -> usize {
    let mut from = from.min(text.len());
    while !text.is_char_boundary(from) {
        from += 1;
    }
    text[from..]
        .find(piece)
        .map_or(from, |offset| from + offset)
}

pub async fn embed_chunks(
    embedder: &dyn KnowledgeEmbedder,
    chunks: &[Chunk],
    batch: usize,
    dimension: usize,
) -> ToolResult<Vec<Vec<f32>>> {
    let mut vectors = Vec::with_capacity(chunks.len());
    for group in chunks.chunks(batch.max(1)) {
        let texts: Vec<String> = group.iter().map(|chunk| chunk.text.clone()).collect();
        let embedded = embedder.embed_documents(&texts).await?;
        if embedded.len() != texts.len() || embedded.iter().any(|vector| vector.len() != dimension)
        {
            return Err(ToolError::new(
                code::FAILED,
                format!(
                    "Embedding model '{}' returned {} vectors for {} texts, or vectors that are not {dimension} values long",
                    embedder.model_id(),
                    embedded.len(),
                    texts.len()
                ),
            ));
        }
        vectors.extend(embedded);
    }
    Ok(vectors)
}

pub fn chunk_rows(
    file: &ResolvedFile,
    chunks: Vec<Chunk>,
    vectors: Vec<Vec<f32>>,
    model: &str,
    now: i64,
) -> ToolResult<Vec<ChunkRow>> {
    let source = file.source_key();
    let handle = to_string(&file.handle()).map_err(|e| ToolError::invalid(e.to_string()))?;
    let classification = file
        .classification
        .map(DataClassification::as_str)
        .unwrap_or_default();
    Ok(chunks
        .into_iter()
        .zip(vectors)
        .enumerate()
        .map(|(ordinal, (chunk, vector))| ChunkRow {
            id: blake3::hash(format!("{source}\0{ordinal}").as_bytes())
                .to_hex()
                .to_string(),
            text: chunk.text,
            source: source.clone(),
            handle: handle.clone(),
            page: chunk.page,
            start_offset: chunk.start as i64,
            end_offset: chunk.end as i64,
            created_at: now,
            classification: classification.to_string(),
            embedding_model: model.to_string(),
            vector,
        })
        .collect())
}

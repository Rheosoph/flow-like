use std::collections::HashMap;

use flow_like_bot_memory::retrieval::{RRF_K, rrf};
use serde_json::{Value, json};

pub struct Ranked {
    pub index: String,
    pub row: Value,
    pub score: f64,
}

fn key(index: &str, id: &str) -> String {
    format!("{index}\u{1f}{id}")
}

fn ids_of(index: &str, hits: &[Value], rows: &mut HashMap<String, (String, Value)>) -> Vec<String> {
    let mut ids = Vec::new();
    for hit in hits {
        if let Some(id) = hit.get("id").and_then(Value::as_str) {
            let key = key(index, id);
            rows.entry(key.clone())
                .or_insert_with(|| (index.to_string(), hit.clone()));
            ids.push(key);
        }
    }
    ids
}

/// Fuses every index's vector and text rankings into one list with reciprocal rank fusion.
pub fn fuse(rankings: &[(String, Vec<Value>, Vec<Value>)], k: usize) -> Vec<Ranked> {
    let mut rows = HashMap::new();
    let mut lists = Vec::new();
    for (index, vector_hits, text_hits) in rankings {
        lists.push(ids_of(index, vector_hits, &mut rows));
        lists.push(ids_of(index, text_hits, &mut rows));
    }
    rrf(&lists, RRF_K)
        .into_iter()
        .take(k)
        .filter_map(|(id, score)| {
            rows.remove(&id)
                .map(|(index, row)| Ranked { index, row, score })
        })
        .collect()
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub fn hit_json(ranked: &Ranked, text_budget: usize) -> Value {
    let row = &ranked.row;
    let text = row.get("text").and_then(Value::as_str).unwrap_or_default();
    let cut = floor_boundary(text, text_budget);
    let shown = if cut < text.len() {
        format!("{}…", &text[..cut])
    } else {
        text.to_string()
    };
    let source: Value = row
        .get("handle")
        .and_then(Value::as_str)
        .and_then(|handle| serde_json::from_str(handle).ok())
        .unwrap_or(Value::Null);
    json!({
        "index": ranked.index,
        "score": (ranked.score * 100_000.0).round() / 100_000.0,
        "text": shown,
        "source": source,
        "page": row.get("page"),
        "start": row.get("start_offset"),
        "end": row.get("end_offset"),
    })
}

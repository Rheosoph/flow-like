//! Host-local `internet_search` tool implementation (SearXNG), shared by every FlowPilot backend:
//! the Copilot SDK / MCP tool handlers call it directly, the rig/Bits bridge intercepts the tool
//! name and dispatches here instead of the frontend.

use std::sync::LazyLock;
use std::time::Duration;

use flow_like::flow::copilot::tool_spec::spec_arg_str;
use serde_json::{Value, json};

static SEARCH_CLIENT: LazyLock<Result<reqwest::blocking::Client, reqwest::Error>> =
    LazyLock::new(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("FlowPilot/1.0")
            .build()
    });

pub fn run_internet_search(args: &Value) -> Value {
    let query = spec_arg_str(args, "query", "query").to_string();
    if query.trim().is_empty() {
        return json!({
            "status": "error",
            "tool": "internet_search",
            "error": "internet_search requires a non-empty query."
        });
    }

    let language = spec_arg_str(args, "language", "language");
    let language = if language.trim().is_empty() {
        "en-US".to_string()
    } else {
        language.to_string()
    };
    let page = args
        .get("page")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .clamp(1, 100);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(8)
        .clamp(1, 20) as usize;
    let url = format!(
        "https://search.flow-like.com/search?q={}&format=json&pageno={}&language={}",
        urlencoding::encode(&query),
        page,
        urlencoding::encode(&language)
    );

    let client = match SEARCH_CLIENT.as_ref() {
        Ok(client) => client,
        Err(error) => {
            return json!({
                "status": "error",
                "tool": "internet_search",
                "error": format!("Failed to create search client: {error}")
            });
        }
    };

    let response = match client.get(&url).send() {
        Ok(response) => response,
        Err(error) => {
            return json!({
                "status": "error",
                "tool": "internet_search",
                "query": query,
                "error": format!("Search request failed: {error}")
            });
        }
    };

    let status = response.status();
    if !status.is_success() {
        return json!({
            "status": "error",
            "tool": "internet_search",
            "query": query,
            "http_status": status.as_u16(),
            "error": format!("Search request failed with HTTP {status}")
        });
    }

    let payload = match response.json::<Value>() {
        Ok(payload) => payload,
        Err(error) => {
            return json!({
                "status": "error",
                "tool": "internet_search",
                "query": query,
                "error": format!("Search response was not valid JSON: {error}")
            });
        }
    };

    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .map(|results| {
            results
                .iter()
                .take(limit)
                .map(compact_search_result)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "status": "ok",
        "query": query,
        "page": page,
        "results": results
    })
}

fn compact_search_result(result: &Value) -> Value {
    let object = result.as_object();
    json!({
        "title": object.and_then(|item| item.get("title")).cloned().unwrap_or(Value::Null),
        "url": object.and_then(|item| item.get("url")).cloned().unwrap_or(Value::Null),
        "content": object.and_then(|item| item.get("content")).cloned().unwrap_or(Value::Null),
        "publishedDate": object.and_then(|item| item.get("publishedDate")).cloned().unwrap_or(Value::Null),
        "engine": object.and_then(|item| item.get("engine")).cloned().unwrap_or(Value::Null),
        "category": object.and_then(|item| item.get("category")).cloned().unwrap_or(Value::Null),
        "score": object.and_then(|item| item.get("score")).cloned().unwrap_or(Value::Null),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_search_result_keeps_only_model_relevant_fields() {
        let result = compact_search_result(&json!({
            "title": "Flow Like",
            "url": "https://flow-like.com",
            "content": "Workflow automation",
            "publishedDate": "2026-06-04",
            "engine": "test",
            "category": "general",
            "score": 1.25,
            "huge": "drop me"
        }));

        assert_eq!(
            result.get("title").and_then(Value::as_str),
            Some("Flow Like")
        );
        assert!(result.get("huge").is_none());
    }
}

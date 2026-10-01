use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Answers one request with `response` and yields the body the Rig client put on the wire.
pub(crate) async fn serve_once(response: String) -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 16 * 1024];
        let (body_start, body_len) = loop {
            let read = socket.read(&mut buffer).await.unwrap();
            assert!(read > 0, "client closed before sending its headers");
            request.extend_from_slice(&buffer[..read]);
            if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let body_len = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                break (end + 4, body_len);
            }
        };
        while request.len() < body_start + body_len {
            let read = socket.read(&mut buffer).await.unwrap();
            assert!(read > 0, "client closed before sending its body");
            request.extend_from_slice(&buffer[..read]);
        }
        socket.write_all(response.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
        String::from_utf8(request[body_start..body_start + body_len].to_vec()).unwrap()
    });
    (endpoint, server)
}

pub(crate) fn json_response(body: Value) -> String {
    let body = body.to_string();
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

pub(crate) fn sse_response(events: &[Value]) -> String {
    let body: String = events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .chain(std::iter::once("data: [DONE]\n\n".to_string()))
        .collect();
    format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}")
}

/// Shaped like FlowPilot's `interact_app_page`: `value` has no `type`, not every property is
/// required, and `payload` is an open object. Strict mode can express none of that.
pub(crate) fn typeless_tool() -> rig::completion::ToolDefinition {
    rig::completion::ToolDefinition {
        name: "interact_app_page".to_string(),
        description: "Interact with components on an app page.".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "actions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": {"type": "string"},
                            "component_id": {"type": "string"},
                            "value": {"description": "Any JSON value the component accepts."},
                        },
                        "required": ["action", "component_id"],
                    },
                },
                "payload": {"type": "object"},
            },
            "required": ["actions"],
        }),
    }
}

/// The request body carries `tool` alone, as written, with an explicit `strict: false`.
pub(crate) fn assert_sends_tool_as_written(body: &Value, tool: &rig::completion::ToolDefinition) {
    let tools = body["tools"].as_array().expect("request has a tools array");
    assert_eq!(tools.len(), 1, "{body}");
    assert_eq!(tools[0]["type"], "function", "{body}");
    assert_eq!(tools[0]["name"], tool.name.as_str(), "{body}");
    assert_eq!(tools[0].get("strict"), Some(&Value::Bool(false)), "{body}");
    assert_eq!(tools[0]["parameters"], tool.parameters, "{body}");
}

/// A completed OpenAI Responses API response with one text message.
pub(crate) fn responses_completion(model: &str) -> Value {
    serde_json::json!({
        "id": "resp-test",
        "object": "response",
        "created_at": 1,
        "status": "completed",
        "model": model,
        "output": [{
            "type": "message",
            "id": "msg-test",
            "role": "assistant",
            "status": "completed",
            "content": [{"type": "output_text", "text": "ok", "annotations": []}],
        }],
        "usage": {"input_tokens": 12, "output_tokens": 1, "total_tokens": 13},
    })
}

/// A one-choice OpenAI-compatible chat completion.
pub(crate) fn chat_completion(model: &str, finish_reason: &str, completion_tokens: u32) -> Value {
    serde_json::json!({
        "id": "gen-test",
        "object": "chat.completion",
        "created": 1,
        "model": model,
        "choices": [{
            "index": 0,
            "finish_reason": finish_reason,
            "message": {"role": "assistant", "content": "ok"},
        }],
        "usage": {
            "prompt_tokens": 12,
            "completion_tokens": completion_tokens,
            "total_tokens": completion_tokens + 12,
        },
    })
}

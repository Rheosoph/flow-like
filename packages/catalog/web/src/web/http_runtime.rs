#[cfg(feature = "execute")]
use ahash::AHashSet;
#[cfg(feature = "execute")]
use flow_like::flow::{
    execution::{
        LogLevel, context::ExecutionContext, internal_node::InternalNode, log::LogMessage,
    },
    pin::PinType,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::sync::Mutex;
#[cfg(feature = "execute")]
use flow_like_types::{Value, anyhow, json};
#[cfg(feature = "execute")]
use std::collections::HashMap;
#[cfg(feature = "execute")]
use std::sync::Arc;
#[cfg(feature = "execute")]
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[cfg(feature = "execute")]
#[derive(Clone, Debug)]
pub(crate) struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub remote_addr: String,
}

#[cfg(feature = "execute")]
#[derive(Clone, Debug)]
pub(crate) struct HttpResponse {
    pub status_code: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

#[cfg(feature = "execute")]
impl HttpResponse {
    pub(crate) fn text(status_code: u16, body: impl Into<String>) -> Self {
        let mut headers = HashMap::new();
        headers.insert(
            "content-type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        );
        Self {
            status_code,
            headers,
            body: body.into().into_bytes(),
        }
    }

    pub(crate) fn json(status_code: u16, value: Value) -> Self {
        let mut headers = HashMap::new();
        headers.insert(
            "content-type".to_string(),
            "application/json; charset=utf-8".to_string(),
        );
        let body = json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
        Self {
            status_code,
            headers,
            body,
        }
    }
}

#[cfg(feature = "execute")]
pub(crate) async fn read_http_request<S>(
    stream: &mut S,
    remote_addr: String,
    max_body_bytes: usize,
) -> flow_like_types::Result<Option<HttpRequest>>
where
    S: AsyncRead + Unpin + ?Sized,
{
    let mut raw = Vec::with_capacity(4096);
    let mut buf = [0_u8; 2048];
    let header_end;

    loop {
        let read = stream.read(&mut buf).await?;
        if read == 0 {
            return Ok(None);
        }
        raw.extend_from_slice(&buf[..read]);
        if raw.len() > 64 * 1024 {
            return Err(anyhow!("HTTP headers exceed 64 KiB"));
        }
        if let Some(pos) = find_header_end(&raw) {
            header_end = pos;
            break;
        }
    }

    let header_bytes = &raw[..header_end];
    let header_text = std::str::from_utf8(header_bytes)
        .map_err(|err| anyhow!("HTTP headers are not valid UTF-8: {}", err))?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| anyhow!("HTTP request line missing"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| anyhow!("HTTP method missing"))?
        .to_uppercase();
    let target = parts
        .next()
        .ok_or_else(|| anyhow!("HTTP target missing"))?
        .to_string();

    let mut headers = HashMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(name.trim().to_lowercase(), value.trim().to_string());
    }

    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > max_body_bytes {
        return Err(anyhow!(
            "HTTP body exceeds max_body_bytes ({} > {})",
            content_length,
            max_body_bytes
        ));
    }

    let body_start = header_end + 4;
    let mut body = raw.get(body_start..).unwrap_or_default().to_vec();
    while body.len() < content_length {
        let needed = content_length - body.len();
        let read_len = needed.min(buf.len());
        let read = stream.read(&mut buf[..read_len]).await?;
        if read == 0 {
            return Err(anyhow!("HTTP body ended before content-length"));
        }
        body.extend_from_slice(&buf[..read]);
    }
    body.truncate(content_length);

    let (path, query) = split_target(&target);
    Ok(Some(HttpRequest {
        method,
        path,
        query,
        headers,
        body,
        remote_addr,
    }))
}

#[cfg(feature = "execute")]
pub(crate) async fn write_http_response<S>(
    stream: &mut S,
    response: HttpResponse,
) -> flow_like_types::Result<()>
where
    S: AsyncWrite + Unpin + ?Sized,
{
    let reason = reason_phrase(response.status_code);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(format!("HTTP/1.1 {} {}\r\n", response.status_code, reason).as_bytes());

    let mut has_content_length = false;
    let mut has_connection = false;
    for (name, value) in &response.headers {
        if name.eq_ignore_ascii_case("content-length") {
            has_content_length = true;
        }
        if name.eq_ignore_ascii_case("connection") {
            has_connection = true;
        }
        bytes.extend_from_slice(
            format!("{}: {}\r\n", canonical_header_name(name), value).as_bytes(),
        );
    }
    if !has_content_length {
        bytes.extend_from_slice(format!("Content-Length: {}\r\n", response.body.len()).as_bytes());
    }
    if !has_connection {
        bytes.extend_from_slice(b"Connection: close\r\n");
    }
    bytes.extend_from_slice(b"\r\n");
    bytes.extend_from_slice(&response.body);
    stream.write_all(&bytes).await?;
    stream.shutdown().await?;
    Ok(())
}

#[cfg(feature = "execute")]
pub(crate) async fn write_sse_response_head<S>(
    stream: &mut S,
    status_code: u16,
    extra_headers: &[(String, String)],
) -> flow_like_types::Result<()>
where
    S: AsyncWrite + Unpin + ?Sized,
{
    let reason = reason_phrase(status_code);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(format!("HTTP/1.1 {} {}\r\n", status_code, reason).as_bytes());
    bytes.extend_from_slice(b"Content-Type: text/event-stream\r\n");
    bytes.extend_from_slice(b"Cache-Control: no-cache, no-transform\r\n");
    bytes.extend_from_slice(b"Connection: keep-alive\r\n");
    bytes.extend_from_slice(b"X-Accel-Buffering: no\r\n");
    for (name, value) in extra_headers {
        bytes.extend_from_slice(
            format!("{}: {}\r\n", canonical_header_name(name), value).as_bytes(),
        );
    }
    bytes.extend_from_slice(b"\r\n");
    stream.write_all(&bytes).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(feature = "execute")]
pub(crate) async fn write_sse_event<S>(
    stream: &mut S,
    event: Option<&str>,
    data: &str,
    id: Option<&str>,
) -> flow_like_types::Result<()>
where
    S: AsyncWrite + Unpin + ?Sized,
{
    let mut out = String::new();
    if let Some(id) = id {
        out.push_str("id: ");
        out.push_str(id);
        out.push('\n');
    }
    if let Some(event) = event {
        out.push_str("event: ");
        out.push_str(event);
        out.push('\n');
    }
    for line in data.split('\n') {
        out.push_str("data: ");
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
    stream.write_all(out.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(feature = "execute")]
pub(crate) async fn write_sse_comment<S>(
    stream: &mut S,
    text: &str,
) -> flow_like_types::Result<()>
where
    S: AsyncWrite + Unpin + ?Sized,
{
    let mut line = String::with_capacity(text.len() + 4);
    line.push_str(": ");
    line.push_str(text);
    line.push_str("\n\n");
    stream.write_all(line.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(feature = "execute")]
pub(crate) fn parse_body_value(request: &HttpRequest) -> Value {
    if request.body.is_empty() {
        return Value::Null;
    }

    let content_type = request
        .headers
        .get("content-type")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    if content_type.contains("application/json") {
        if let Ok(value) = json::from_slice::<Value>(&request.body) {
            return value;
        }
    }

    match String::from_utf8(request.body.clone()) {
        Ok(text) => Value::String(text),
        Err(_) => Value::Array(request.body.iter().map(|byte| Value::from(*byte)).collect()),
    }
}

#[cfg(feature = "execute")]
#[cfg(feature = "execute")]
pub(crate) type SharedFunctionContext = Arc<Mutex<ExecutionContext>>;

#[cfg(feature = "execute")]
pub(crate) async fn create_shared_function_context(
    context: &ExecutionContext,
    referenced_node: &Arc<InternalNode>,
) -> SharedFunctionContext {
    let mut sub = context.create_sub_context(referenced_node).await;
    sub.delegated = true;
    sub.context_pin_overrides = Some(Default::default());
    Arc::new(Mutex::new(sub))
}

#[cfg(feature = "execute")]
pub(crate) async fn trigger_shared_function_context(
    context: &SharedFunctionContext,
    arguments: &Value,
    parent_node_id: &str,
    log_name: &'static str,
) -> flow_like_types::Result<Value> {
    let args = arguments
        .as_object()
        .ok_or_else(|| anyhow!("Registered function arguments must be a JSON object"))?;
    let mut context = context.lock().await;
    reset_function_output_pins(&context).await;

    let pins: Vec<_> = context
        .node
        .pins
        .iter()
        .filter(|(_, pin)| {
            pin.pin_type == PinType::Output && pin.data_type != VariableType::Execution
        })
        .map(|(_, pin)| pin.clone())
        .collect();

    for pin in pins {
        let sanitized = sanitize_identifier(&pin.name);
        if let Some(value) = args.get(&pin.name).or_else(|| args.get(&sanitized)) {
            context.set_pin_ref_value(&pin, value.clone()).await?;
        }
    }

    let mut recursion_guard = AHashSet::new();
    recursion_guard.insert(parent_node_id.to_string());
    let mut recursion_guard = Some(recursion_guard);

    let mut log_message = LogMessage::new(log_name, LogLevel::Debug, None);
    let run = InternalNode::trigger(&mut context, &mut recursion_guard, true).await;
    let result = context.result.clone();
    context.result = None;
    log_message.end();
    context.log(log_message);
    context.end_trace();
    if let Err(err) = context.flush_logs().await {
        tracing::warn!("Failed to flush {} logs: {:?}", log_name, err);
    }

    match run {
        Ok(_) => Ok(result.unwrap_or_else(|| json::json!({"ok": true}))),
        Err(error) => Err(anyhow!("Registered function failed: {:?}", error)),
    }
}

#[cfg(feature = "execute")]
async fn reset_function_output_pins(context: &ExecutionContext) {
    let pins: Vec<_> = context
        .node
        .pins
        .iter()
        .filter(|(_, pin)| {
            pin.pin_type == PinType::Output && pin.data_type != VariableType::Execution
        })
        .map(|(_, pin)| pin.clone())
        .collect();

    for pin in pins {
        pin.reset().await;
    }
}

pub(crate) fn normalize_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        "/".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed)
    }
}

#[cfg(feature = "execute")]
pub(crate) fn sanitize_identifier(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            output.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() {
            output.push('_');
        }
    }
    let output = output.trim_matches('_').to_string();
    if output.is_empty() {
        "function".to_string()
    } else {
        output
    }
}

#[cfg(feature = "execute")]
fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

#[cfg(feature = "execute")]
fn split_target(target: &str) -> (String, HashMap<String, String>) {
    let (path, query_string) = target.split_once('?').unwrap_or((target, ""));
    let mut query = HashMap::new();
    for pair in query_string.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = urlencoding::decode(key)
            .map(|value| value.into_owned())
            .unwrap_or_else(|_| key.to_string());
        let value = urlencoding::decode(value)
            .map(|value| value.into_owned())
            .unwrap_or_else(|_| value.to_string());
        query.insert(key, value);
    }
    (normalize_path(path), query)
}

#[cfg(feature = "execute")]
fn canonical_header_name(name: &str) -> String {
    name.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut out = first.to_uppercase().collect::<String>();
                    out.push_str(&chars.as_str().to_ascii_lowercase());
                    out
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(feature = "execute")]
fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        _ => "OK",
    }
}

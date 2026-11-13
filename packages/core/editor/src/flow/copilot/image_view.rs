//! `image_view`: the model looks at images behind links it received in this run. The links stay
//! links; the media layer decides per provider whether they travel as links or inline.
//!
//! Only a link whose first appearance in the run is in a person's message or a tool result is
//! viewed. A link the model wrote first could carry data it read out to any host.

use std::time::Duration;

use flow_like_types::{reqwest, tokio};
use futures::future::join_all;
use rig::message::{AssistantContent, DocumentSourceKind, Message, ToolResultContent, UserContent};
use serde_json::{Value, json};
use url::Url;

use super::platform::PLATFORM_TOOL_IMAGE_URLS_FIELD;
use super::tool_spec::{PlatformToolSpec, ToolApprovalSpec};
use crate::flow::execution::{ExecutionEnvironment, egress::GuardedHttpClient};

pub const IMAGE_VIEW_TOOL: &str = "image_view";

const MAX_URLS: usize = 6;
/// The largest image link the providers that fetch links themselves accept.
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const SNIFF_BYTES: usize = 16;

const DESCRIPTION: &str = "Look at images behind http(s) links you received in this turn: a signed link an app, connector or MCP tool returned, an image link in a tool or web result, or one the person sent. The images are attached right after this tool's result, so you see them yourself; nothing is saved. Copy each link exactly, query string included. Links you wrote yourself and links seen only in earlier turns are refused: call the tool that produced them again for a fresh link. JPEG, PNG, GIF and WebP.";

pub fn image_view_tool_spec() -> PlatformToolSpec {
    PlatformToolSpec {
        name: IMAGE_VIEW_TOOL,
        description: DESCRIPTION,
        schema: image_view_schema,
        approval: ToolApprovalSpec::None,
        timeout_secs: 60,
    }
}

fn image_view_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "urls": {
                "type": "array",
                "items": { "type": "string" },
                "description": format!("Up to {MAX_URLS} image links, copied exactly, in the order to view them."),
            }
        },
        "required": ["urls"],
        "additionalProperties": false
    })
}

#[derive(Debug, PartialEq, Eq)]
enum Provenance {
    Received,
    Written,
    Unseen,
}

/// Where `url` first appears in `messages`, oldest first.
fn provenance(url: &str, messages: &[&Message]) -> Provenance {
    let unescaped_url = unescaped(url);
    let needle = unescaped_url.as_deref().unwrap_or(url);
    for message in messages {
        match message {
            Message::User { content } => {
                if content.iter().any(|part| user_part_mentions(part, needle)) {
                    return Provenance::Received;
                }
            }
            Message::Assistant { content, .. } => {
                if content
                    .iter()
                    .any(|part| assistant_part_mentions(part, needle))
                {
                    return Provenance::Written;
                }
            }
            Message::System { .. } => {}
        }
    }
    Provenance::Unseen
}

fn user_part_mentions(part: &UserContent, needle: &str) -> bool {
    match part {
        UserContent::Text(text) => mentions(&text.text, needle),
        UserContent::ToolResult(result) => result.content.iter().any(|content| match content {
            ToolResultContent::Text(text) => mentions(&text.text, needle),
            ToolResultContent::Image(image) => source_is(&image.data, needle),
        }),
        UserContent::Image(image) => source_is(&image.data, needle),
        UserContent::Document(document) => source_is(&document.data, needle),
        UserContent::Audio(_) | UserContent::Video(_) => false,
    }
}

fn assistant_part_mentions(part: &AssistantContent, needle: &str) -> bool {
    match part {
        AssistantContent::Text(text) => mentions(&text.text, needle),
        AssistantContent::ToolCall(call) => mentions(&call.function.arguments.to_string(), needle),
        AssistantContent::Reasoning(reasoning) => mentions(
            &serde_json::to_string(reasoning).unwrap_or_default(),
            needle,
        ),
        AssistantContent::Image(image) => source_is(&image.data, needle),
    }
}

fn source_is(source: &DocumentSourceKind, needle: &str) -> bool {
    matches!(source, DocumentSourceKind::Url(url) if unescaped(url).as_deref().unwrap_or(url) == needle)
}

/// The text with the escapes a link picks up on its way through JSON or HTML undone, or `None`
/// when it has none.
fn unescaped(text: &str) -> Option<String> {
    (text.contains('\\') || text.contains("&amp;")).then(|| {
        text.replace("\\\\u0026", "&")
            .replace("\\u0026", "&")
            .replace("\\\\/", "/")
            .replace("\\/", "/")
            .replace("&amp;", "&")
    })
}

/// Whether `haystack` holds `needle` as a whole link, not as the start of a longer one.
fn mentions(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let unescaped_haystack = unescaped(haystack);
    let haystack = unescaped_haystack.as_deref().unwrap_or(haystack);
    haystack.match_indices(needle).any(|(start, _)| {
        let rest = &haystack[start + needle.len()..];
        let mut chars = rest.chars();
        match chars.next() {
            None => true,
            Some(next) if ends_link(next) => true,
            Some('.' | ',' | ';' | ':' | '!' | '?') => chars
                .next()
                .is_none_or(|after| after.is_whitespace() || ends_link(after)),
            Some(_) => false,
        }
    })
}

fn ends_link(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '"' | '\'' | '`' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\'
        )
}

#[derive(Debug, PartialEq, Eq)]
struct Probed {
    media_type: &'static str,
    bytes: Option<u64>,
}

/// The model-readable image type the first bytes show, else the one the response declares.
fn image_type(head: &[u8], declared: Option<&str>) -> Result<&'static str, String> {
    let sniffed = if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    };
    if let Some(sniffed) = sniffed {
        return Ok(sniffed);
    }
    let declared = declared
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    match declared.as_deref() {
        Some("image/jpeg" | "image/jpg") => Ok("image/jpeg"),
        Some("image/png") => Ok("image/png"),
        Some("image/gif") => Ok("image/gif"),
        Some("image/webp") => Ok("image/webp"),
        Some(other) if other.starts_with("image/") => Err(format!(
            "the link serves {other}, which models do not read; only JPEG, PNG, GIF and WebP"
        )),
        Some(other) => Err(format!("the link serves {other}, not an image")),
        None => Err("the link serves no recognizable image".to_string()),
    }
}

/// Checks that the link answers with a model-readable image, reading only its first bytes, so a
/// dead or wrong link fails this call instead of the model call that would carry it.
async fn probe(client: &GuardedHttpClient, url: &str) -> Result<Probed, String> {
    let request = client
        .get(url)
        .map_err(|error| error.to_string())?
        .header(reqwest::header::ACCEPT, "image/*");
    let attempt = async {
        let mut response = request
            .send()
            .await
            .map_err(|error| format!("the request failed: {}", error.without_url()))?;
        let bytes = accepted_length(&response)?;
        let declared = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let head = read_head(&mut response).await?;
        Ok(Probed {
            media_type: image_type(&head, declared.as_deref())?,
            bytes,
        })
    };
    tokio::time::timeout(PROBE_TIMEOUT, attempt)
        .await
        .unwrap_or_else(|_| Err(format!("the link did not answer within {PROBE_TIMEOUT:?}")))
}

/// The declared length of a successful response small enough for models to fetch.
fn accepted_length(response: &reqwest::Response) -> Result<Option<u64>, String> {
    let status = response.status();
    if !status.is_success() {
        let hint = if matches!(status.as_u16(), 401 | 403) {
            "; a signed link may have expired, so get a fresh one"
        } else {
            ""
        };
        return Err(format!("the link answered HTTP {status}{hint}"));
    }
    match response.content_length() {
        Some(bytes) if bytes > MAX_IMAGE_BYTES => Err(format!(
            "the image has {bytes} bytes; models fetch at most {MAX_IMAGE_BYTES}"
        )),
        bytes => Ok(bytes),
    }
}

/// The first bytes of the body, enough to recognize an image.
async fn read_head(response: &mut reqwest::Response) -> Result<Vec<u8>, String> {
    let mut head = Vec::with_capacity(SNIFF_BYTES);
    while head.len() < SNIFF_BYTES {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| format!("reading the image failed: {}", error.without_url()))?;
        let Some(chunk) = chunk else { break };
        head.extend_from_slice(&chunk);
    }
    if head.is_empty() {
        return Err("the link answered with an empty body".to_string());
    }
    Ok(head)
}

fn requested_urls(arguments: &Value) -> Result<Vec<String>, String> {
    let urls = match arguments.get("urls") {
        Some(Value::Array(urls)) => urls
            .iter()
            .map(|url| url.as_str().map(|url| url.trim().to_string()))
            .collect::<Option<Vec<_>>>()
            .ok_or("urls must hold only strings")?,
        Some(Value::String(url)) => vec![url.trim().to_string()],
        _ => return Err("urls must be a list of image links".to_string()),
    };
    if urls.is_empty() {
        return Err("urls is empty; give at least one image link".to_string());
    }
    if urls.len() > MAX_URLS {
        return Err(format!(
            "{} links given; view at most {MAX_URLS} per call",
            urls.len()
        ));
    }
    Ok(urls)
}

/// Why `url` may not be viewed, before anything is fetched.
fn refusal(url: &str, messages: &[&Message]) -> Option<String> {
    match Url::parse(url) {
        Ok(parsed) if matches!(parsed.scheme(), "https" | "http") => {}
        _ => return Some("not an http(s) link".to_string()),
    }
    match provenance(url, messages) {
        Provenance::Received => None,
        Provenance::Written => Some(
            "you wrote this link before any tool or person gave it to you; only received links are viewed"
                .to_string(),
        ),
        Provenance::Unseen => Some(
            "this exact link is in no tool result or message of this turn; copy it exactly, or call the tool that produced it again for a fresh link"
                .to_string(),
        ),
    }
}

/// The `image_view` result for `arguments`, given the run's messages oldest first. Viewable links
/// travel in the tool loop's image field, which attaches them after the result.
pub(super) async fn run_image_view(
    arguments: &Value,
    messages: &[&Message],
    environment: ExecutionEnvironment,
) -> String {
    let urls = match requested_urls(arguments) {
        Ok(urls) => urls,
        Err(message) => {
            return json!({
                "status": "error",
                "tool": IMAGE_VIEW_TOOL,
                "code": "invalid_arguments",
                "retryable": true,
                "message": message,
            })
            .to_string();
        }
    };
    let client = match GuardedHttpClient::new(environment) {
        Ok(client) => client,
        Err(error) => {
            return json!({
                "status": "error",
                "tool": IMAGE_VIEW_TOOL,
                "code": "image_view_unavailable",
                "retryable": false,
                "message": format!("No HTTP client for image links: {error}"),
            })
            .to_string();
        }
    };
    let outcomes = join_all(urls.iter().map(|url| {
        let refused = refusal(url, messages);
        let client = &client;
        async move {
            match refused {
                Some(reason) => Err(reason),
                None => probe(client, url).await,
            }
        }
    }))
    .await;
    image_view_result(&urls, outcomes)
}

fn image_view_result(urls: &[String], outcomes: Vec<Result<Probed, String>>) -> String {
    let mut attached = Vec::new();
    let mut images = Vec::new();
    let mut refused = Vec::new();
    for (index, (url, outcome)) in urls.iter().zip(outcomes).enumerate() {
        match outcome {
            Ok(probed) => {
                images.push(json!({
                    "index": index + 1,
                    "media_type": probed.media_type,
                    "bytes": probed.bytes,
                }));
                attached.push(json!({ "url": url, "media_type": probed.media_type }));
            }
            Err(reason) => refused.push(json!({ "index": index + 1, "reason": reason })),
        }
    }
    let status = match (attached.is_empty(), refused.is_empty()) {
        (true, _) => "error",
        (false, true) => "ok",
        (false, false) => "partial",
    };
    let mut result = json!({
        "status": status,
        "tool": IMAGE_VIEW_TOOL,
        "images": images,
    });
    if !refused.is_empty() {
        result["refused"] = Value::Array(refused);
    }
    if attached.is_empty() {
        result["code"] = json!("no_viewable_image");
        result["retryable"] = json!(true);
    } else {
        result["note"] = json!(
            "The images follow this result in the order listed. They stay visible for the rest of this turn only."
        );
        result[PLATFORM_TOOL_IMAGE_URLS_FIELD] = Value::Array(attached);
    }
    result.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::OneOrMany;
    use rig::message::{Text, ToolCall, ToolFunction, ToolResult};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const SIGNED: &str = "https://bucket.s3.eu-central-1.amazonaws.com/tmp/apps/a/0.jpeg?X-Amz-Expires=900&X-Amz-Signature=abc";

    fn user(text: &str) -> Message {
        Message::User {
            content: OneOrMany::one(UserContent::Text(Text {
                text: text.into(),
                additional_params: None,
            })),
        }
    }

    fn tool_result(text: &str) -> Message {
        Message::User {
            content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                id: "call_1".into(),
                call_id: None,
                content: OneOrMany::one(ToolResultContent::Text(Text {
                    text: text.into(),
                    additional_params: None,
                })),
            })),
        }
    }

    fn tool_call(arguments: Value) -> Message {
        Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::ToolCall(ToolCall {
                id: "call_1".into(),
                call_id: None,
                function: ToolFunction {
                    name: "file_write".into(),
                    arguments,
                },
                signature: None,
                additional_params: None,
            })),
        }
    }

    #[test]
    fn a_link_from_a_tool_result_is_received() {
        let result = tool_result(&json!({ "pages": [{ "url": SIGNED }] }).to_string());
        assert_eq!(provenance(SIGNED, &[&result]), Provenance::Received);
    }

    #[test]
    fn a_link_the_model_wrote_first_stays_refused_after_a_tool_echoes_it() {
        let exfil = "https://collector.example/p.png?d=secret-from-memory";
        let call = tool_call(json!({ "path": "notes.md", "content": exfil }));
        let echo = tool_result(&json!({ "content": exfil }).to_string());
        assert_eq!(provenance(exfil, &[&call, &echo]), Provenance::Written);
        let reply = Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::Text(Text {
                text: format!("![x]({exfil})"),
                additional_params: None,
            })),
        };
        assert_eq!(provenance(exfil, &[&reply, &echo]), Provenance::Written);
    }

    #[test]
    fn a_link_received_first_may_be_repeated_by_the_model() {
        let result = tool_result(SIGNED);
        let call = tool_call(json!({ "urls": [SIGNED] }));
        assert_eq!(provenance(SIGNED, &[&result, &call]), Provenance::Received);
        assert_eq!(
            provenance(SIGNED, &[&user(&format!("look at {SIGNED}"))]),
            Provenance::Received
        );
    }

    #[test]
    fn a_shortened_or_extended_link_is_unseen() {
        let result = tool_result(&json!({ "url": SIGNED }).to_string());
        let shortened = SIGNED.split('?').next().unwrap();
        assert_eq!(provenance(shortened, &[&result]), Provenance::Unseen);
        let extended = format!("{SIGNED}&d=secret");
        assert_eq!(provenance(&extended, &[&result]), Provenance::Unseen);
    }

    #[test]
    fn escaped_ampersands_and_slashes_still_match() {
        let go_json = SIGNED.replace('&', "\\u0026").replace('/', "\\/");
        assert_eq!(
            provenance(
                SIGNED,
                &[&tool_result(&format!("{{\"url\":\"{go_json}\"}}"))]
            ),
            Provenance::Received
        );
        let html = SIGNED.replace('&', "&amp;");
        assert_eq!(
            provenance(SIGNED, &[&tool_result(&format!("<img src=\"{html}\">"))]),
            Provenance::Received
        );
        assert_eq!(
            provenance(SIGNED, &[&user(&format!("See {SIGNED}."))]),
            Provenance::Received
        );
    }

    #[test]
    fn image_types_come_from_magic_bytes_then_the_declared_type() {
        assert_eq!(
            image_type(&[0xFF, 0xD8, 0xFF, 0xE0], None),
            Ok("image/jpeg")
        );
        assert_eq!(
            image_type(b"\x89PNG\r\n\x1a\n....", Some("application/octet-stream")),
            Ok("image/png")
        );
        assert_eq!(image_type(b"RIFF\0\0\0\0WEBPVP8 ", None), Ok("image/webp"));
        assert_eq!(
            image_type(b"????", Some("Image/JPG; q=1")),
            Ok("image/jpeg")
        );
        assert!(image_type(b"<svg", Some("image/svg+xml")).is_err());
        assert!(image_type(b"<!doctype html>", Some("text/html")).is_err());
        assert!(image_type(b"????", None).is_err());
    }

    #[test]
    fn requested_urls_are_bounded() {
        assert_eq!(
            requested_urls(&json!({ "urls": [" https://a.example/x.png "] })).unwrap(),
            ["https://a.example/x.png"]
        );
        assert!(requested_urls(&json!({ "urls": [] })).is_err());
        assert!(requested_urls(&json!({ "urls": [1] })).is_err());
        let many: Vec<_> = (0..=MAX_URLS)
            .map(|i| format!("https://a.example/{i}.png"))
            .collect();
        assert!(requested_urls(&json!({ "urls": many })).is_err());
    }

    #[test]
    fn refusals_name_the_reason_before_any_fetch() {
        assert!(refusal("file:///etc/passwd", &[]).is_some());
        assert!(
            refusal("https://a.example/x.png", &[])
                .unwrap()
                .contains("fresh link")
        );
    }

    #[test]
    fn viewable_links_become_the_tool_loop_image_field() {
        let urls = vec![SIGNED.to_string(), "https://a.example/b.png".to_string()];
        let output = image_view_result(
            &urls,
            vec![
                Ok(Probed {
                    media_type: "image/jpeg",
                    bytes: Some(1234),
                }),
                Err("the link answered HTTP 403 Forbidden".into()),
            ],
        );
        let mut value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["status"], "partial");
        assert_eq!(value["refused"][0]["index"], 2);
        let images = super::super::platform::take_platform_tool_image_urls(&mut value);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].url.as_deref(), Some(SIGNED));
        assert_eq!(images[0].media_type, "image/jpeg");

        let none = image_view_result(&urls[..1], vec![Err("gone".into())]);
        let none: Value = serde_json::from_str(&none).unwrap();
        assert_eq!(none["status"], "error");
        assert!(none.get(PLATFORM_TOOL_IMAGE_URLS_FIELD).is_none());
    }

    async fn serve_once(head: &'static str, body: &'static [u8]) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            let _ = socket.read(&mut request).await;
            let _ = socket
                .write_all(format!("{head}\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes())
                .await;
            let _ = socket.write_all(body).await;
        });
        format!("http://{address}/tmp/0.jpeg?X-Amz-Signature=abc")
    }

    #[tokio::test]
    async fn a_received_link_is_probed_and_attached() {
        let url = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Type: binary/octet-stream",
            b"\xFF\xD8\xFF\xE0\0\x10JFIF\0\x01\x01\0\0\x01",
        )
        .await;
        let result = tool_result(&json!({ "pages": [url] }).to_string());
        let output = run_image_view(
            &json!({ "urls": [url] }),
            &[&result],
            ExecutionEnvironment::Desktop,
        )
        .await;
        let mut value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["status"], "ok", "{output}");
        let images = super::super::platform::take_platform_tool_image_urls(&mut value);
        assert_eq!(images[0].url.as_deref(), Some(url.as_str()));
        assert_eq!(images[0].media_type, "image/jpeg");
    }

    #[tokio::test]
    async fn an_expired_link_fails_the_call_with_a_hint() {
        let url = serve_once("HTTP/1.1 403 Forbidden", b"denied").await;
        let result = tool_result(&url);
        let output = run_image_view(
            &json!({ "urls": [url] }),
            &[&result],
            ExecutionEnvironment::Desktop,
        )
        .await;
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["status"], "error");
        assert!(
            value["refused"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("expired")
        );
    }

    #[tokio::test]
    async fn an_unreceived_link_is_never_fetched() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/x.png", listener.local_addr().unwrap());
        let output = run_image_view(
            &json!({ "urls": [url] }),
            &[],
            ExecutionEnvironment::Desktop,
        )
        .await;
        let value: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["status"], "error");
        let accepted = tokio::time::timeout(Duration::from_millis(100), listener.accept()).await;
        assert!(accepted.is_err(), "the link was fetched");
    }
}

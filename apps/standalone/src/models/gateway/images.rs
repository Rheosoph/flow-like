//! Images in chat requests. An engine resolves an image reference itself: llama-server
//! downloads http(s) URLs, and the MLX helper also reads absolute paths and `file:` URLs. So
//! only inline `data:` images reach an engine. The gateway fetches a public HTTPS image under
//! the agent's address policy and puts it inline; it refuses every other reference.

use super::error;
use crate::models::fetch::{FetchError, Fetcher};
use axum::{http::StatusCode, response::Response};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map, Value};
use std::time::Duration;
use url::Url;

const MAX_LINKED: usize = 8;
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_LINKED_BYTES: u64 = 32 * 1024 * 1024;
const FETCH_DEADLINE: Duration = Duration::from_secs(60);
const MIB: u64 = 1024 * 1024;

/// A public HTTPS image of the request, by its position among the request's images.
pub(super) struct Linked {
    position: usize,
    url: Url,
}

/// The image references of a chat request's message parts, in order.
fn references(request: &mut Map<String, Value>) -> Vec<&mut Value> {
    let Some(Value::Array(messages)) = request.get_mut("messages") else {
        return Vec::new();
    };
    messages
        .iter_mut()
        .filter_map(|message| message.get_mut("content")?.as_array_mut())
        .flatten()
        .filter_map(|part| {
            let image = part.get_mut("image_url")?;
            if image.is_object() {
                image.get_mut("url")
            } else {
                Some(image)
            }
        })
        .collect()
}

fn refused(message: &str) -> Response {
    error(StatusCode::BAD_REQUEST, "invalid_request_error", message)
}

/// Checks every image reference before the request queues: inline `data:` images pass, public
/// HTTPS images are fetched once the request has a slot, and anything else is refused.
pub(super) fn linked(request: &mut Map<String, Value>) -> Result<Vec<Linked>, Response> {
    let mut linked = Vec::new();
    for (position, reference) in references(request).into_iter().enumerate() {
        let text = reference.as_str().unwrap_or_default();
        if text.starts_with("data:") {
            continue;
        }
        match Url::parse(text) {
            Ok(url) if url.scheme() == "https" => linked.push(Linked { position, url }),
            _ => {
                return Err(refused(&format!(
                    "Image {} must be a data: URL or a public https URL",
                    position + 1
                )));
            }
        }
    }
    if linked.len() > MAX_LINKED {
        return Err(refused(&format!(
            "A request may link at most {MAX_LINKED} images; send the others as data: URLs"
        )));
    }
    Ok(linked)
}

/// Fetches the linked images and puts each into the request as a `data:` URL.
pub(super) async fn inline(
    fetcher: &Fetcher,
    request: &mut Map<String, Value>,
    linked: Vec<Linked>,
) -> Result<(), Response> {
    if linked.is_empty() {
        return Ok(());
    }
    let fetched = tokio::time::timeout(FETCH_DEADLINE, fetch_all(fetcher, &linked))
        .await
        .map_err(|_| {
            refused(&format!(
                "The linked images could not be fetched within {} seconds",
                FETCH_DEADLINE.as_secs()
            ))
        })??;
    let mut references = references(request);
    for (link, data) in linked.iter().zip(fetched) {
        if let Some(reference) = references.get_mut(link.position) {
            **reference = Value::String(data);
        }
    }
    Ok(())
}

async fn fetch_all(fetcher: &Fetcher, linked: &[Linked]) -> Result<Vec<String>, Response> {
    let mut fetched = Vec::with_capacity(linked.len());
    let mut total = 0;
    for link in linked {
        let number = link.position + 1;
        let (content_type, bytes) = fetcher
            .fetch_small(
                link.url.as_str(),
                &format!("image {number}"),
                MAX_IMAGE_BYTES,
            )
            .await
            .map_err(|failure| unreachable(number, &failure))?;
        total += bytes.len() as u64;
        if total > MAX_LINKED_BYTES {
            return Err(refused(&format!(
                "The linked images exceed {} MiB together",
                MAX_LINKED_BYTES / MIB
            )));
        }
        let media_type = image_type(content_type.as_deref())
            .ok_or_else(|| refused(&format!("Image {number} is not an image")))?;
        fetched.push(format!(
            "data:{media_type};base64,{}",
            STANDARD.encode(&bytes)
        ));
    }
    Ok(fetched)
}

/// Why a linked image is missing. Only a public server's answer is named: telling which names
/// resolve or which addresses the policy refuses would map the device's network for the caller.
fn unreachable(number: usize, failure: &FetchError) -> Response {
    tracing::debug!("Fetch image {number} of a chat request: {failure}");
    refused(&match failure {
        FetchError::Status { status, .. } => {
            format!("Image {number} could not be fetched: its server answered {status}")
        }
        FetchError::SizeMismatch(_) => {
            format!(
                "Image {number} is larger than {} MiB",
                MAX_IMAGE_BYTES / MIB
            )
        }
        _ => format!("Image {number} could not be fetched from a public HTTPS address"),
    })
}

/// The bare `image/*` media type of a response, if it has one.
fn image_type(content_type: Option<&str>) -> Option<String> {
    let media_type = content_type?.split(';').next()?.trim().to_ascii_lowercase();
    let subtype = media_type.strip_prefix("image/")?;
    let token =
        |character: char| character.is_ascii_alphanumeric() || "!#$&^_.+-".contains(character);
    (!subtype.is_empty() && subtype.chars().all(token)).then_some(media_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chat(image_url: Value) -> Map<String, Value> {
        let request = json!({"messages": [
            {"role": "system", "content": "Describe images."},
            {"role": "user", "content": [
                {"type": "text", "text": "Two images:"},
                {"type": "image_url", "image_url": "data:image/png;base64,AAAA"},
                {"type": "image_url", "image_url": image_url}]}]});
        request.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn inline_and_https_images_pass_and_nothing_else() {
        let mut request = chat(json!({"url": "https://example.com/cat.png"}));
        let found = linked(&mut request).map_err(|_| "refused").expect("passes");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].position, 1);
        assert_eq!(found[0].url.as_str(), "https://example.com/cat.png");
        let local = [
            json!("file:///etc/passwd"),
            json!({"url": "/Users/owner/scan.png"}),
            json!("http://10.0.0.5/x.png"),
            json!({"url": "ftp://example.com/x.png"}),
            json!({"url": 7}),
        ];
        for reference in local {
            let refusal = linked(&mut chat(reference.clone())).err();
            assert!(refusal.is_some_and(|response| response.status() == StatusCode::BAD_REQUEST));
        }
    }

    #[test]
    fn fetched_images_keep_their_bare_image_type() {
        let typed = |value| image_type(Some(value));
        assert_eq!(
            typed("image/PNG; charset=binary").as_deref(),
            Some("image/png")
        );
        assert_eq!(typed("image/svg+xml").as_deref(), Some("image/svg+xml"));
        assert_eq!(typed("text/html"), None);
        assert_eq!(typed("image/"), None);
        assert_eq!(typed("image/png,<x>"), None);
        assert_eq!(image_type(None), None);
    }
}

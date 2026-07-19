use flow_like_model_provider::history::{Content, ContentType, ImageUrl, MessageContent};
use serde_json::Value;

/// Convert a WASM SDK message's content into Flow-Like's provider-neutral wire format.
///
/// The Rust SDK flattens multimodal content to a top-level `parts` property, while the
/// Python SDK uses OpenAI-style array-valued `content`. Both forms contain the same
/// SDK content-part objects and must behave identically at the host boundary.
pub(crate) fn sdk_message_content(message: &Value) -> MessageContent {
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        return MessageContent::String(text.to_string());
    }

    let parts = message
        .get("parts")
        .and_then(Value::as_array)
        .or_else(|| message.get("content").and_then(Value::as_array));

    let Some(parts) = parts else {
        return MessageContent::String(String::new());
    };

    MessageContent::Contents(parts.iter().filter_map(sdk_content_part).collect())
}

fn sdk_content_part(part: &Value) -> Option<Content> {
    if let Some(text) = part.get("text").and_then(Value::as_str) {
        return Some(Content::Text {
            content_type: ContentType::Text,
            text: text.to_string(),
        });
    }

    if let Some(text) = reasoning_text(part) {
        return Some(Content::Text {
            content_type: ContentType::Text,
            text,
        });
    }

    if let Some(value) = part.get("image").or_else(|| part.get("image_url")) {
        return Some(Content::Image {
            content_type: ContentType::ImageUrl,
            image_url: ImageUrl {
                url: media_url(value)?.to_string(),
                detail: media_property(part, value, "detail")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                media_type: media_property(part, value, "media_type")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                additional_params: media_property(part, value, "additional_params").cloned(),
            },
        });
    }

    if let Some(value) = part.get("audio").or_else(|| part.get("audio_url")) {
        return Some(Content::Audio {
            content_type: ContentType::AudioUrl,
            audio_url: media_url(value)?.to_string(),
            media_type: media_property(part, value, "media_type")
                .and_then(Value::as_str)
                .map(str::to_string),
            additional_params: media_property(part, value, "additional_params").cloned(),
        });
    }

    if let Some(value) = part.get("video").or_else(|| part.get("video_url")) {
        return Some(Content::Video {
            content_type: ContentType::VideoUrl,
            video_url: media_url(value)?.to_string(),
            media_type: media_property(part, value, "media_type")
                .and_then(Value::as_str)
                .map(str::to_string),
            additional_params: media_property(part, value, "additional_params").cloned(),
        });
    }

    if let Some(value) = part.get("document").or_else(|| part.get("document_url")) {
        return Some(Content::Document {
            content_type: ContentType::DocumentUrl,
            document_url: media_url(value)?.to_string(),
            media_type: media_property(part, value, "media_type")
                .and_then(Value::as_str)
                .map(str::to_string),
            additional_params: media_property(part, value, "additional_params").cloned(),
        });
    }

    None
}

fn media_url(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.get("url").and_then(Value::as_str))
}

fn media_property<'a>(part: &'a Value, media: &'a Value, property: &str) -> Option<&'a Value> {
    media.get(property).or_else(|| part.get(property))
}

fn reasoning_text(part: &Value) -> Option<String> {
    let text = part.get("reasoning")?.get("text")?;
    let joined = if let Some(entries) = text.as_array() {
        entries
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        text.as_str()?.to_string()
    };

    (!joined.is_empty()).then_some(joined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_rust_sdk_multimodal_parts() {
        let content = sdk_message_content(&json!({
            "role": "user",
            "parts": [
                { "type": "text", "text": "Inspect these" },
                { "type": "image", "image": {
                    "url": "data:image/png;base64,aW1hZ2U=",
                    "media_type": "image/png",
                    "detail": "high"
                }},
                { "type": "audio", "audio": {
                    "url": "https://example.com/audio.mp3",
                    "media_type": "audio/mp3"
                }},
                { "type": "video", "video": {
                    "url": "file_id:video-1",
                    "media_type": "video/mp4"
                }},
                { "type": "document", "document": {
                    "url": "https://example.com/report.pdf",
                    "media_type": "application/pdf",
                    "additional_params": { "provider": "value" }
                }},
                { "type": "reasoning", "reasoning": { "text": ["first", "second"] }}
            ]
        }));

        let MessageContent::Contents(parts) = content else {
            panic!("expected structured content")
        };
        assert_eq!(parts.len(), 6);
        assert!(matches!(
            &parts[1],
            Content::Image { image_url, .. }
                if image_url.media_type.as_deref() == Some("image/png")
                    && image_url.detail.as_deref() == Some("high")
        ));
        assert!(matches!(
            &parts[2],
            Content::Audio { media_type, .. }
                if media_type.as_deref() == Some("audio/mp3")
        ));
        assert!(matches!(
            &parts[3],
            Content::Video { video_url, .. } if video_url == "file_id:video-1"
        ));
        assert!(matches!(
            &parts[4],
            Content::Document { additional_params: Some(params), .. }
                if params == &json!({ "provider": "value" })
        ));
        assert!(matches!(&parts[5], Content::Text { text, .. } if text == "first\nsecond"));
    }

    #[test]
    fn parses_python_array_valued_content_and_wire_aliases() {
        let content = sdk_message_content(&json!({
            "role": "user",
            "content": [
                { "type": "image_url", "image_url": {
                    "url": "https://example.com/image.webp",
                    "media_type": "image/webp"
                }},
                {
                    "type": "audio_url",
                    "audio_url": "data:audio/wav;base64,YXVkaW8=",
                    "media_type": "audio/wav"
                },
                {
                    "type": "video_url",
                    "video_url": "https://example.com/video.webm",
                    "media_type": "video/webm"
                },
                {
                    "type": "document_url",
                    "document_url": "https://example.com/data.csv",
                    "media_type": "text/csv"
                }
            ]
        }));

        let MessageContent::Contents(parts) = content else {
            panic!("expected structured content")
        };
        assert_eq!(parts.len(), 4);
        assert!(matches!(&parts[0], Content::Image { .. }));
        assert!(matches!(&parts[1], Content::Audio { .. }));
        assert!(matches!(&parts[2], Content::Video { .. }));
        assert!(matches!(&parts[3], Content::Document { .. }));
    }

    #[test]
    fn preserves_string_content() {
        assert_eq!(
            sdk_message_content(&json!({ "content": "hello" })),
            MessageContent::String("hello".to_string())
        );
    }
}

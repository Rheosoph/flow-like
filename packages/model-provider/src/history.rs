// Implementation according to
// https://modelcontextprotocol.io/docs/concepts/sampling/

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use flow_like_types::{Value, anyhow, json};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
use std::fmt;

use crate::response::{Annotation, Response};
use flow_like_types::Result;
use rig::OneOrMany;
use rig::completion::{Message as RigMessage, ToolDefinition};
use rig::message::{
    AssistantContent as RigAssistantContent, Audio as RigAudio, AudioMediaType,
    Document as RigDocument, DocumentMediaType, DocumentSourceKind, Image as RigImage, ImageDetail,
    ImageMediaType, MimeType, Text as RigText, ToolCall as RigToolCall,
    ToolChoice as RigToolChoice, ToolFunction as RigToolFunction,
    ToolResultContent as RigToolResultContent, UserContent as RigUserContent, Video as RigVideo,
    VideoMediaType,
};

/// Recursively normalize string values in a JSON Value tree,
/// removing escaped quotes (\" → ") that cause OpenAI strict mode to reject schemas.
pub fn normalize_json_schema_strings(value: &mut Value) {
    match value {
        Value::String(s) => {
            while s.contains("\\\"") {
                *s = s.replace("\\\"", "\"");
            }
        }
        Value::Array(arr) => {
            for item in arr {
                normalize_json_schema_strings(item);
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                normalize_json_schema_strings(v);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct ToolCall {
    pub id: String,
    pub r#type: String,
    pub function: ToolCallFunction,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema, Clone)]
pub struct ToolCallFunction {
    //#[serde(skip_serializing_if = "Option::is_none")]
    pub name: String,
    #[serde(deserialize_with = "arguments_as_str")]
    pub arguments: String,
}

/// Handles arguments incoming as str (e.g. for cloud-based LLM providers) or map (local LLM providers)
fn arguments_as_str<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let v = Value::deserialize(deserializer)?;
    match v {
        Value::String(s) => Ok(s), // already a string
        other => json::to_string(&other).map_err(serde::de::Error::custom), // object/array/number → stringified JSON
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum MessageContent {
    String(String),
    Contents(Vec<Content>),
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub struct HistoryMessage {
    pub role: Role,
    pub content: MessageContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<Annotation>>,
}

impl HistoryMessage {
    pub fn from_string(role: Role, content: &str) -> Self {
        Self {
            role,
            content: MessageContent::Contents(vec![Content::Text {
                content_type: ContentType::Text,
                text: content.to_string(),
            }]),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            annotations: None,
        }
    }

    pub fn from_response(response: Response) -> Self {
        let first_choice = response.choices.first();

        let content = first_choice
            .map(|choice| MessageContent::Contents(choice.message.ordered_content_parts()));
        let annotations = match first_choice {
            Some(choice) => choice.message.annotations.clone(),
            None => None,
        };
        let tool_calls = first_choice
            .map(|choice| {
                choice
                    .message
                    .tool_calls
                    .iter()
                    .map(|tool_call| ToolCall {
                        id: tool_call.id.clone(),
                        r#type: tool_call
                            .tool_type
                            .clone()
                            .unwrap_or_else(|| "function".to_string()),
                        function: ToolCallFunction {
                            name: tool_call.function.name.clone(),
                            arguments: tool_call.function.arguments.clone(),
                        },
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|tool_calls| !tool_calls.is_empty());

        let role: Role = match first_choice {
            Some(choice) => match choice.message.role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "system" => Role::System,
                _ => Role::Assistant,
            },
            None => Role::Assistant,
        };

        Self {
            role,
            content: content.unwrap_or_else(|| MessageContent::Contents(Vec::new())),
            name: None,
            tool_call_id: None,
            tool_calls,
            annotations,
        }
    }
}

impl HistoryMessage {
    /// Returns a copy of the entire text-related content as single String
    pub fn as_str(&self) -> String {
        match &self.content {
            MessageContent::String(s) => s.clone(),
            MessageContent::Contents(contents) => contents
                .iter()
                .filter_map(|content| {
                    if let Content::Text { text, .. } = content {
                        Some(text.as_str())
                    } else {
                        None
                    }
                })
                .collect::<Vec<&str>>()
                .join("\n"),
        }
    }
}

impl From<RigMessage> for HistoryMessage {
    fn from(msg: RigMessage) -> Self {
        match msg {
            RigMessage::System { content } => HistoryMessage {
                role: Role::System,
                content: MessageContent::String(content),
                name: None,
                tool_call_id: None,
                tool_calls: None,
                annotations: None,
            },
            RigMessage::User { content } => {
                let is_single_tool_result =
                    content.len() == 1 && matches!(content.first(), RigUserContent::ToolResult(_));

                if is_single_tool_result && let RigUserContent::ToolResult(tr) = content.first() {
                    let contents = tr
                        .content
                        .iter()
                        .map(|item| match item {
                            RigToolResultContent::Text(text) => Content::Text {
                                content_type: ContentType::Text,
                                text: text.text.clone(),
                            },
                            RigToolResultContent::Image(image) => {
                                Content::from_rig_image(image.clone())
                            }
                        })
                        .collect();
                    return HistoryMessage {
                        role: Role::Tool,
                        content: MessageContent::Contents(contents),
                        name: None,
                        tool_call_id: Some(tr.id.clone()),
                        tool_calls: None,
                        annotations: None,
                    };
                }

                let contents: Vec<Content> = content.iter().map(|c| c.clone().into()).collect();

                HistoryMessage {
                    role: Role::User,
                    content: if contents.len() == 1 && matches!(contents[0], Content::Text { .. }) {
                        if let Content::Text { text, .. } = &contents[0] {
                            MessageContent::String(text.clone())
                        } else {
                            MessageContent::Contents(contents)
                        }
                    } else {
                        MessageContent::Contents(contents)
                    },
                    name: None,
                    tool_calls: None,
                    tool_call_id: None,
                    annotations: None,
                }
            }
            RigMessage::Assistant { id, content } => {
                let mut tool_calls = Vec::new();
                let mut contents = Vec::new();

                for item in content.iter() {
                    match item {
                        RigAssistantContent::Text(text) => {
                            contents.push(Content::Text {
                                content_type: ContentType::Text,
                                text: text.text.clone(),
                            });
                        }
                        RigAssistantContent::ToolCall(tool_call) => {
                            tool_calls.push(ToolCall {
                                id: tool_call.id.clone(),
                                r#type: "function".to_string(),
                                function: ToolCallFunction {
                                    name: tool_call.function.name.clone(),
                                    arguments: tool_call.function.arguments.to_string(),
                                },
                            });
                        }
                        RigAssistantContent::Image(image) => {
                            contents.push(Content::from_rig_image(image.clone()));
                        }
                        RigAssistantContent::Reasoning(_) => {}
                    }
                }

                let message_content = if contents.len() == 1 {
                    match contents.pop().expect("length checked") {
                        Content::Text { text, .. } => MessageContent::String(text),
                        content => MessageContent::Contents(vec![content]),
                    }
                } else {
                    MessageContent::Contents(contents)
                };

                HistoryMessage {
                    role: Role::Assistant,
                    content: message_content,
                    name: id,
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                    tool_call_id: None,
                    annotations: None,
                }
            }
        }
    }
}

fn one_or_many_or_default<T: Clone>(
    mut contents: Vec<T>,
    default: impl FnOnce() -> T,
) -> Result<OneOrMany<T>> {
    match contents.len() {
        0 => Ok(OneOrMany::one(default())),
        1 => Ok(OneOrMany::one(contents.pop().expect("one content item"))),
        _ => OneOrMany::many(contents).map_err(|error| anyhow!(error.to_string())),
    }
}

impl TryFrom<HistoryMessage> for RigMessage {
    type Error = flow_like_types::Error;

    fn try_from(msg: HistoryMessage) -> Result<Self> {
        match msg.role {
            Role::User => {
                let contents: Vec<RigUserContent> = match msg.content {
                    MessageContent::String(s) => {
                        vec![RigUserContent::Text(RigText {
                            text: s,
                            additional_params: None,
                        })]
                    }
                    MessageContent::Contents(contents) => {
                        contents.into_iter().map(|c| c.into()).collect()
                    }
                };

                let content = if contents.is_empty() {
                    OneOrMany::one(RigUserContent::Text(RigText {
                        text: String::new(),
                        additional_params: None,
                    }))
                } else if contents.len() == 1 {
                    OneOrMany::one(contents.into_iter().next().unwrap())
                } else {
                    OneOrMany::many(contents)
                        .map_err(|e| flow_like_types::Error::msg(e.to_string()))?
                };

                Ok(RigMessage::User { content })
            }
            Role::Assistant => {
                let mut rig_contents = Vec::new();

                match msg.content {
                    MessageContent::String(s) if !s.is_empty() => {
                        rig_contents.push(RigAssistantContent::Text(RigText {
                            text: s,
                            additional_params: None,
                        }));
                    }
                    MessageContent::Contents(contents) => {
                        for content in contents {
                            match content {
                                Content::Text { text, .. } if !text.is_empty() => {
                                    rig_contents.push(RigAssistantContent::Text(RigText {
                                        text,
                                        additional_params: None,
                                    }));
                                }
                                Content::Image { .. } => {
                                    rig_contents.push(content.try_into_rig_assistant()?);
                                }
                                Content::Audio { .. }
                                | Content::Video { .. }
                                | Content::Document { .. } => {
                                    // Rig's generic assistant history only accepts text and images.
                                    // Keep richer response parts in Flow-Like's history/UI, but do
                                    // not fail the next provider turn when replaying that history.
                                }
                                Content::Text { .. } => {}
                            }
                        }
                    }
                    _ => {}
                }

                if let Some(tool_calls) = msg.tool_calls {
                    for tool_call in tool_calls {
                        rig_contents.push(RigAssistantContent::ToolCall(RigToolCall {
                            id: tool_call.id,
                            call_id: None,
                            function: RigToolFunction {
                                name: tool_call.function.name,
                                arguments: json::from_str(&tool_call.function.arguments)
                                    .unwrap_or(json::json!({})),
                            },
                            signature: None,
                            additional_params: None,
                        }));
                    }
                }

                let content = if rig_contents.is_empty() {
                    OneOrMany::one(RigAssistantContent::Text(RigText {
                        text: String::new(),
                        additional_params: None,
                    }))
                } else if rig_contents.len() == 1 {
                    OneOrMany::one(rig_contents.into_iter().next().unwrap())
                } else {
                    OneOrMany::many(rig_contents)
                        .map_err(|e| flow_like_types::Error::msg(e.to_string()))?
                };

                Ok(RigMessage::Assistant {
                    id: msg.name,
                    content,
                })
            }
            Role::Tool | Role::Function => {
                use rig::message::ToolResult;
                let tool_call_id = msg.tool_call_id.or(msg.name.clone()).unwrap_or_default();
                let mut result_contents = Vec::new();
                match msg.content {
                    MessageContent::String(text) => {
                        result_contents.push(RigToolResultContent::text(text));
                    }
                    MessageContent::Contents(contents) => {
                        for content in contents {
                            match content {
                                Content::Text { text, .. } => {
                                    result_contents.push(RigToolResultContent::text(text));
                                }
                                Content::Image { .. } => {
                                    let RigAssistantContent::Image(image) =
                                        content.try_into_rig_assistant()?
                                    else {
                                        unreachable!("image content converts to an image")
                                    };
                                    result_contents.push(RigToolResultContent::Image(image));
                                }
                                Content::Audio { .. }
                                | Content::Video { .. }
                                | Content::Document { .. } => {
                                    return Err(anyhow!(
                                        "Rig tool results support text and images, but not audio, video, or documents"
                                    ));
                                }
                            }
                        }
                    }
                }
                let content = one_or_many_or_default(result_contents, || {
                    RigToolResultContent::text(String::new())
                })?;
                Ok(RigMessage::User {
                    content: OneOrMany::one(RigUserContent::ToolResult(ToolResult {
                        id: tool_call_id,
                        call_id: None,
                        content,
                    })),
                })
            }
            Role::System => {
                let text = msg.as_str();
                Ok(RigMessage::User {
                    content: OneOrMany::one(RigUserContent::Text(RigText {
                        text,
                        additional_params: None,
                    })),
                })
            }
        }
    }
}

impl fmt::Display for History {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.messages.is_empty() {
            let mut history_str = String::from("| ");
            for message in self.messages.iter() {
                let m = match message.role {
                    Role::Assistant => " A |",
                    Role::System => " S |",
                    Role::Tool => " T |",
                    Role::User => " H |",
                    Role::Function => " F |",
                };
                history_str.push_str(m);
            }
            write!(f, "{}", history_str)
        } else {
            write!(f, "[]")
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Function,
    Tool,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct ImageUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_params: Option<Value>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
#[serde(untagged)]
pub enum Content {
    Text {
        #[serde(rename = "type")]
        content_type: ContentType,
        text: String,
    },
    Image {
        #[serde(rename = "type")]
        content_type: ContentType,
        image_url: ImageUrl,
    },
    Audio {
        #[serde(rename = "type")]
        content_type: ContentType,
        audio_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        additional_params: Option<Value>,
    },
    Video {
        #[serde(rename = "type")]
        content_type: ContentType,
        video_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        additional_params: Option<Value>,
    },
    Document {
        #[serde(rename = "type")]
        content_type: ContentType,
        document_url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        additional_params: Option<Value>,
    },
}

impl Content {
    /// Converts a Rig image into Flow-Like's wire-compatible image content without losing
    /// base64/raw payloads or their MIME type.
    pub fn from_rig_image(image: RigImage) -> Self {
        let detail = image
            .detail
            .map(|detail| format!("{detail:?}").to_lowercase());
        let media_type = image
            .media_type
            .as_ref()
            .map(MimeType::to_mime_type)
            .map(ToOwned::to_owned);
        let mime_type = media_type.as_deref().unwrap_or("application/octet-stream");
        let additional_params = image.additional_params.clone();

        Content::Image {
            content_type: ContentType::ImageUrl,
            image_url: ImageUrl {
                url: source_to_wire_value(image.data, mime_type),
                detail,
                media_type,
                additional_params,
            },
        }
    }

    /// Converts content that Rig permits in assistant messages. Rig 0.38 only supports text
    /// and images here; audio, video, and documents are user-input content types.
    pub fn try_into_rig_assistant(self) -> Result<RigAssistantContent> {
        match self {
            Content::Text { text, .. } => Ok(RigAssistantContent::Text(RigText {
                text,
                additional_params: None,
            })),
            Content::Image { image_url, .. } => Ok(RigAssistantContent::Image(
                rig_image_from_wire_value(image_url),
            )),
            Content::Audio { .. } | Content::Video { .. } | Content::Document { .. } => {
                Err(anyhow!(
                    "Rig assistant messages support text and images, but not audio, video, or documents"
                ))
            }
        }
    }

    /// Returns the URL/data URI carried by a media content part.
    pub fn media_url(&self) -> Option<&str> {
        match self {
            Content::Text { .. } => None,
            Content::Image { image_url, .. } => Some(&image_url.url),
            Content::Audio { audio_url, .. } => Some(audio_url),
            Content::Video { video_url, .. } => Some(video_url),
            Content::Document { document_url, .. } => Some(document_url),
        }
    }
}

impl From<RigUserContent> for Content {
    fn from(rig_content: RigUserContent) -> Self {
        match rig_content {
            RigUserContent::Text(text) => Content::Text {
                content_type: ContentType::Text,
                text: text.text,
            },
            RigUserContent::Image(image) => Content::from_rig_image(image),
            RigUserContent::Audio(audio) => {
                let media_type = audio
                    .media_type
                    .as_ref()
                    .map(MimeType::to_mime_type)
                    .map(ToOwned::to_owned);
                Content::Audio {
                    content_type: ContentType::AudioUrl,
                    audio_url: source_to_wire_value(
                        audio.data,
                        media_type.as_deref().unwrap_or("application/octet-stream"),
                    ),
                    media_type,
                    additional_params: audio.additional_params,
                }
            }
            RigUserContent::Video(video) => {
                let media_type = video
                    .media_type
                    .as_ref()
                    .map(MimeType::to_mime_type)
                    .map(ToOwned::to_owned);
                Content::Video {
                    content_type: ContentType::VideoUrl,
                    video_url: source_to_wire_value(
                        video.data,
                        media_type.as_deref().unwrap_or("application/octet-stream"),
                    ),
                    media_type,
                    additional_params: video.additional_params,
                }
            }
            RigUserContent::Document(doc) => {
                let media_type = doc
                    .media_type
                    .as_ref()
                    .map(MimeType::to_mime_type)
                    .map(ToOwned::to_owned);
                Content::Document {
                    content_type: ContentType::DocumentUrl,
                    document_url: source_to_wire_value(
                        doc.data,
                        media_type.as_deref().unwrap_or("application/octet-stream"),
                    ),
                    media_type,
                    additional_params: doc.additional_params,
                }
            }
            RigUserContent::ToolResult(tool_result) => {
                if tool_result.content.len() == 1
                    && let RigToolResultContent::Image(image) = tool_result.content.first()
                {
                    return Content::from_rig_image(image.clone());
                }
                let text = tool_result
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        RigToolResultContent::Text(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Content::Text {
                    content_type: ContentType::Text,
                    text,
                }
            }
        }
    }
}

impl From<Content> for RigUserContent {
    fn from(content: Content) -> Self {
        match content {
            Content::Text { text, .. } => RigUserContent::Text(RigText {
                text,
                additional_params: None,
            }),
            Content::Image { image_url, .. } => {
                RigUserContent::Image(rig_image_from_wire_value(image_url))
            }
            Content::Audio {
                audio_url,
                media_type,
                additional_params,
                ..
            } => {
                let (data, wire_media_type) = source_from_wire_value(&audio_url);
                RigUserContent::Audio(RigAudio {
                    data,
                    media_type: wire_media_type
                        .and_then(audio_media_type_from_mime)
                        .or_else(|| media_type.as_deref().and_then(audio_media_type_from_mime))
                        .or_else(|| detect_audio_media_type(&audio_url)),
                    additional_params,
                })
            }
            Content::Video {
                video_url,
                media_type,
                additional_params,
                ..
            } => {
                let (data, wire_media_type) = source_from_wire_value(&video_url);
                RigUserContent::Video(RigVideo {
                    data,
                    media_type: wire_media_type
                        .and_then(video_media_type_from_mime)
                        .or_else(|| media_type.as_deref().and_then(video_media_type_from_mime))
                        .or_else(|| detect_video_media_type(&video_url)),
                    additional_params,
                })
            }
            Content::Document {
                document_url,
                media_type,
                additional_params,
                ..
            } => {
                let (data, wire_media_type) = source_from_wire_value(&document_url);
                let media_type = wire_media_type
                    .and_then(document_media_type_from_mime)
                    .or_else(|| {
                        media_type
                            .as_deref()
                            .and_then(document_media_type_from_mime)
                    })
                    .or_else(|| detect_document_media_type(&document_url));
                RigUserContent::Document(RigDocument {
                    data: decode_textual_document_source(data, media_type.as_ref()),
                    media_type,
                    additional_params,
                })
            }
        }
    }
}

fn rig_image_from_wire_value(image_url: ImageUrl) -> RigImage {
    let (data, wire_media_type) = source_from_wire_value(&image_url.url);
    RigImage {
        data,
        media_type: wire_media_type
            .and_then(image_media_type_from_mime)
            .or_else(|| {
                image_url
                    .media_type
                    .as_deref()
                    .and_then(image_media_type_from_mime)
            })
            .or_else(|| detect_image_media_type(&image_url.url)),
        detail: Some(parse_image_detail(image_url.detail.as_deref())),
        additional_params: image_url.additional_params,
    }
}

/// Converts every Rig source kind to a stable string representation. Binary/string payloads are
/// emitted as valid data URIs; a private parameter preserves Rig's otherwise ambiguous source kind.
fn source_to_wire_value(source: DocumentSourceKind, mime_type: &str) -> String {
    match source {
        DocumentSourceKind::Url(url) => url,
        DocumentSourceKind::Base64(data) => format!("data:{mime_type};base64,{data}"),
        DocumentSourceKind::FileId(file_id) => format!("file_id:{file_id}"),
        DocumentSourceKind::Raw(bytes) => format!(
            "data:{mime_type};flow-like-source=raw;base64,{}",
            BASE64_STANDARD.encode(bytes)
        ),
        DocumentSourceKind::String(value) => format!(
            "data:{mime_type};flow-like-source=string;base64,{}",
            BASE64_STANDARD.encode(value.as_bytes())
        ),
        DocumentSourceKind::Unknown => String::new(),
        _ => String::new(),
    }
}

fn source_from_wire_value(value: &str) -> (DocumentSourceKind, Option<&str>) {
    if value.is_empty() {
        return (DocumentSourceKind::Unknown, None);
    }

    if let Some(data_uri) = value.strip_prefix("data:")
        && let Some((metadata, payload)) = data_uri.split_once(',')
        && let Some(metadata) = metadata.strip_suffix(";base64")
    {
        let mut metadata_parts = metadata.split(';');
        let mime_type = metadata_parts.next().filter(|mime| !mime.is_empty());
        let source_kind = metadata_parts.find_map(|part| {
            part.strip_prefix("flow-like-source=")
                .map(str::to_ascii_lowercase)
        });
        let source = match source_kind.as_deref() {
            Some("raw") => BASE64_STANDARD
                .decode(payload)
                .map(DocumentSourceKind::Raw)
                .unwrap_or_else(|_| DocumentSourceKind::Base64(payload.to_string())),
            Some("string") => BASE64_STANDARD
                .decode(payload)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .map(DocumentSourceKind::String)
                .unwrap_or_else(|| DocumentSourceKind::Base64(payload.to_string())),
            _ => DocumentSourceKind::Base64(payload.to_string()),
        };
        return (source, mime_type);
    }

    if let Some(file_id) = value.strip_prefix("file_id:") {
        return (DocumentSourceKind::FileId(file_id.to_string()), None);
    }

    (DocumentSourceKind::url(value), None)
}

/// Every [`DocumentMediaType`] except PDF is text-based, and providers forward such documents as
/// plain text. A base64 payload would reach the model verbatim, so decode it back into a string.
fn decode_textual_document_source(
    source: DocumentSourceKind,
    media_type: Option<&DocumentMediaType>,
) -> DocumentSourceKind {
    if matches!(media_type, None | Some(DocumentMediaType::PDF)) {
        return source;
    }

    let DocumentSourceKind::Base64(payload) = &source else {
        return source;
    };

    BASE64_STANDARD
        .decode(payload)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map_or(source, DocumentSourceKind::String)
}

fn extension(value: &str) -> Option<&str> {
    let path = value.split(['?', '#']).next()?;
    path.rsplit_once('.').map(|(_, extension)| extension)
}

fn image_media_type_from_mime(value: &str) -> Option<ImageMediaType> {
    match value.to_ascii_lowercase().as_str() {
        "image/jpg" => Some(ImageMediaType::JPEG),
        mime_type => ImageMediaType::from_mime_type(mime_type),
    }
}

fn audio_media_type_from_mime(value: &str) -> Option<AudioMediaType> {
    match value.to_ascii_lowercase().as_str() {
        "audio/mpeg" | "audio/mpeg3" => Some(AudioMediaType::MP3),
        "audio/x-wav" | "audio/wave" => Some(AudioMediaType::WAV),
        "audio/x-aiff" => Some(AudioMediaType::AIFF),
        "audio/mp4" | "audio/x-m4a" => Some(AudioMediaType::M4A),
        mime_type => AudioMediaType::from_mime_type(mime_type),
    }
}

fn video_media_type_from_mime(value: &str) -> Option<VideoMediaType> {
    match value.to_ascii_lowercase().as_str() {
        "video/x-msvideo" => Some(VideoMediaType::AVI),
        "video/quicktime" => Some(VideoMediaType::MOV),
        mime_type => VideoMediaType::from_mime_type(mime_type),
    }
}

fn document_media_type_from_mime(value: &str) -> Option<DocumentMediaType> {
    match value.to_ascii_lowercase().as_str() {
        "application/rtf" => Some(DocumentMediaType::RTF),
        "application/javascript" | "text/javascript" | "text/x-javascript" => {
            Some(DocumentMediaType::Javascript)
        }
        "text/md" | "text/x-markdown" => Some(DocumentMediaType::MARKDOWN),
        "text/x-python" => Some(DocumentMediaType::Python),
        "application/xml" => Some(DocumentMediaType::XML),
        mime_type => DocumentMediaType::from_mime_type(mime_type),
    }
}

fn detect_image_media_type(value: &str) -> Option<ImageMediaType> {
    match extension(value)?.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some(ImageMediaType::JPEG),
        "png" => Some(ImageMediaType::PNG),
        "gif" => Some(ImageMediaType::GIF),
        "webp" => Some(ImageMediaType::WEBP),
        "heic" => Some(ImageMediaType::HEIC),
        "heif" => Some(ImageMediaType::HEIF),
        "svg" => Some(ImageMediaType::SVG),
        _ => None,
    }
}

fn detect_audio_media_type(value: &str) -> Option<AudioMediaType> {
    match extension(value)?.to_ascii_lowercase().as_str() {
        "wav" => Some(AudioMediaType::WAV),
        "mp3" => Some(AudioMediaType::MP3),
        "aif" | "aiff" => Some(AudioMediaType::AIFF),
        "aac" => Some(AudioMediaType::AAC),
        "ogg" | "oga" => Some(AudioMediaType::OGG),
        "flac" => Some(AudioMediaType::FLAC),
        "m4a" => Some(AudioMediaType::M4A),
        "pcm16" => Some(AudioMediaType::PCM16),
        "pcm24" => Some(AudioMediaType::PCM24),
        _ => None,
    }
}

fn detect_video_media_type(value: &str) -> Option<VideoMediaType> {
    match extension(value)?.to_ascii_lowercase().as_str() {
        "avi" => Some(VideoMediaType::AVI),
        "mp4" => Some(VideoMediaType::MP4),
        "mpeg" | "mpg" => Some(VideoMediaType::MPEG),
        "mov" => Some(VideoMediaType::MOV),
        "webm" => Some(VideoMediaType::WEBM),
        _ => None,
    }
}

fn detect_document_media_type(value: &str) -> Option<DocumentMediaType> {
    match extension(value)?.to_ascii_lowercase().as_str() {
        "pdf" => Some(DocumentMediaType::PDF),
        "txt" => Some(DocumentMediaType::TXT),
        "rtf" => Some(DocumentMediaType::RTF),
        "html" | "htm" => Some(DocumentMediaType::HTML),
        "css" => Some(DocumentMediaType::CSS),
        "md" | "markdown" => Some(DocumentMediaType::MARKDOWN),
        "csv" => Some(DocumentMediaType::CSV),
        "xml" => Some(DocumentMediaType::XML),
        "js" | "mjs" | "cjs" => Some(DocumentMediaType::Javascript),
        "py" => Some(DocumentMediaType::Python),
        _ => None,
    }
}

/// Parses image detail string to ImageDetail enum, defaulting to Auto
fn parse_image_detail(detail: Option<&str>) -> ImageDetail {
    match detail {
        Some("low") => ImageDetail::Low,
        Some("high") => ImageDetail::High,
        Some("auto") => ImageDetail::Auto,
        _ => ImageDetail::Auto, // Default to Auto if not specified or unknown
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    Text,
    #[serde(rename = "image_url")]
    ImageUrl,
    #[serde(rename = "audio_url")]
    AudioUrl,
    #[serde(rename = "video_url")]
    VideoUrl,
    #[serde(rename = "document_url")]
    DocumentUrl,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(untagged)]
pub enum ResponseFormat {
    String(String),
    Object(flow_like_types::Value),
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct StreamOptions {
    pub include_usage: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Usage {
    pub include: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryThinking {
    Off,
    Low,
    Mid,
    High,
}

impl HistoryThinking {
    pub fn openai_reasoning_effort(self) -> &'static str {
        match self {
            Self::Off => "none",
            Self::Low => "low",
            Self::Mid => "medium",
            Self::High => "high",
        }
    }

    pub fn xai_reasoning_effort(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::Low => Some("low"),
            Self::Mid | Self::High => Some("high"),
        }
    }
}

impl std::str::FromStr for HistoryThinking {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Ok(Self::Off),
            "low" => Ok(Self::Low),
            "mid" | "medium" => Ok(Self::Mid),
            "high" => Ok(Self::High),
            other => Err(format!("Unknown thinking mode: {other}")),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct History {
    pub model: String,
    pub messages: Vec<HistoryMessage>,

    pub preset: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<StreamOptions>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<HistoryThinking>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<ResponseFormat>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

impl History {
    pub fn new(model: String, messages: Vec<HistoryMessage>) -> Self {
        Self {
            model,
            messages,
            preset: None,
            stream: Some(true),
            stream_options: None,
            max_completion_tokens: None,
            top_p: None,
            temperature: None,
            thinking: None,
            seed: None,
            presence_penalty: None,
            frequency_penalty: None,
            user: None,
            stop: None,
            response_format: None,
            n: None,
            tools: None,
            tool_choice: None,
            usage: None,
        }
    }

    pub fn push_message(&mut self, message: HistoryMessage) {
        self.messages.push(message);
    }

    pub fn get_system_prompt_index(&self) -> Option<usize> {
        self.messages
            .iter()
            .position(|message| message.role == Role::System)
    }

    pub fn get_system_prompt(&self) -> Option<String> {
        if let Some(index) = self.get_system_prompt_index() {
            match &self.messages[index].content {
                MessageContent::Contents(contents) => {
                    let mut prompt = String::new();
                    for content in contents {
                        if let Content::Text {
                            content_type: _,
                            text,
                        } = content
                        {
                            prompt.push_str(text);
                        }
                    }
                    return Some(prompt);
                }
                MessageContent::String(content) => return Some(content.to_string()),
            }
        }
        None
    }

    /// Extracts and removes the system prompt from messages, returning its text.
    /// Use this to move the system prompt into preamble before rig conversion,
    /// preventing System→User conversion that breaks role alternation.
    pub fn take_system_prompt(&mut self) -> Option<String> {
        let prompt = self.get_system_prompt();
        if prompt.is_some() {
            self.messages.retain(|msg| msg.role != Role::System);
        }
        prompt
    }

    pub fn set_system_prompt(&mut self, prompt: String) {
        if let Some(index) = self.get_system_prompt_index() {
            self.messages[index].content = MessageContent::Contents(vec![Content::Text {
                content_type: ContentType::Text,
                text: prompt,
            }]);
            return;
        }

        self.messages.insert(
            0,
            HistoryMessage {
                role: Role::System,
                content: MessageContent::Contents(vec![Content::Text {
                    content_type: ContentType::Text,
                    text: prompt,
                }]),
                name: None,
                tool_call_id: None,
                tool_calls: None,
                annotations: None,
            },
        );
    }

    pub fn set_stream(&mut self, stream: bool) {
        self.stream = Some(stream);
    }

    /// Merges adjacent messages that share the same role into a single message.
    /// This ensures strict role alternation (user/assistant/user/assistant/...)
    /// required by models like Gemma loaded in LM Studio.
    ///
    /// Tool and Function messages are left untouched since they carry
    /// tool_call_id metadata that must not be merged.
    pub fn normalize_for_alternation(&mut self) {
        if self.messages.len() <= 1 {
            return;
        }

        let mut normalized: Vec<HistoryMessage> = Vec::with_capacity(self.messages.len());

        for msg in self.messages.drain(..) {
            let dominated_by_tool_meta =
                matches!(msg.role, Role::Tool | Role::Function) || msg.tool_call_id.is_some();

            if let Some(prev) = normalized.last_mut()
                && prev.role == msg.role
                && !dominated_by_tool_meta
                && !matches!(prev.role, Role::Tool | Role::Function)
                && prev.tool_call_id.is_none()
            {
                let prev_parts = match std::mem::replace(
                    &mut prev.content,
                    MessageContent::Contents(Vec::new()),
                ) {
                    MessageContent::String(s) => vec![Content::Text {
                        content_type: ContentType::Text,
                        text: s,
                    }],
                    MessageContent::Contents(c) => c,
                };

                let next_parts = match msg.content {
                    MessageContent::String(s) => vec![Content::Text {
                        content_type: ContentType::Text,
                        text: s,
                    }],
                    MessageContent::Contents(c) => c,
                };

                let mut merged = Vec::with_capacity(prev_parts.len() + next_parts.len());
                merged.extend(prev_parts);
                merged.extend(next_parts);
                prev.content = MessageContent::Contents(merged);

                if let Some(next_calls) = msg.tool_calls {
                    prev.tool_calls
                        .get_or_insert_with(Vec::new)
                        .extend(next_calls);
                }
            } else {
                normalized.push(msg);
            }
        }

        self.messages = normalized;
    }

    /// Extracts prompt and history messages suitable for rig completion
    /// Returns (prompt_message, history_messages) where prompt_message is the last user message
    /// and history_messages are all previous messages
    ///
    /// This is the preferred method as it preserves all content types (images, tools, etc.)
    pub fn extract_prompt_and_history(&self) -> Result<(RigMessage, Vec<RigMessage>)> {
        let mut messages: Vec<RigMessage> = Vec::new();
        let mut prompt: Option<RigMessage> = None;

        for (idx, msg) in self.messages.iter().enumerate() {
            if idx == self.messages.len() - 1 && msg.role == Role::User {
                prompt = Some(msg.clone().try_into()?);
            } else {
                messages.push(msg.clone().try_into()?);
            }
        }

        // If no user message at the end, try to pop one from history
        // But never take a ToolResult message as the prompt
        if prompt.is_none()
            && !messages.is_empty()
            && let Some(last_msg) = messages.last()
            && matches!(last_msg, RigMessage::User { .. })
        {
            let is_tool_result = if let RigMessage::User { content } = last_msg {
                content
                    .iter()
                    .any(|c| matches!(c, RigUserContent::ToolResult(_)))
            } else {
                false
            };
            if !is_tool_result {
                prompt = messages.pop();
            }
        }

        // If still no prompt, create a default empty user message
        let prompt = prompt.unwrap_or_else(|| RigMessage::User {
            content: OneOrMany::one(RigUserContent::Text(RigText {
                text: String::new(),
                additional_params: None,
            })),
        });

        Ok((prompt, messages))
    }

    /// Extracts text-only prompt and history messages for simple text completion
    /// Returns (prompt_text, history_messages) where prompt_text is the text from the last user message
    ///
    /// Note: This method only extracts text content and discards images, audio, etc.
    /// Use `extract_prompt_and_history()` if you need to preserve all content types.
    pub fn extract_text_prompt_and_history(&self) -> Result<(String, Vec<RigMessage>)> {
        let (prompt_msg, history) = self.extract_prompt_and_history()?;

        let prompt_text = match prompt_msg {
            RigMessage::User { content } => {
                let first = content.first();
                let rest = content.rest();

                let mut texts = Vec::new();
                if let RigUserContent::Text(t) = &first {
                    texts.push(t.text.clone());
                }

                for c in rest {
                    if let RigUserContent::Text(t) = c {
                        texts.push(t.text.clone());
                    }
                }

                texts.join("\n")
            }
            _ => String::new(),
        };

        Ok((prompt_text, history))
    }

    /// Converts to rig messages vector
    pub fn to_rig_messages(&self) -> Result<Vec<RigMessage>> {
        self.messages
            .iter()
            .map(|msg| msg.clone().try_into())
            .collect()
    }

    /// Creates History from rig messages
    pub fn from_rig_messages(messages: Vec<RigMessage>, model: String) -> Self {
        let history_messages: Vec<HistoryMessage> =
            messages.into_iter().map(|m| m.into()).collect();
        Self::new(model, history_messages)
    }

    /// Converts tools to rig ToolDefinition
    pub fn tools_to_rig(&self) -> Result<Vec<ToolDefinition>> {
        let Some(tools) = self.tools.as_ref() else {
            return Ok(Vec::new());
        };

        let mut definitions = Vec::with_capacity(tools.len());
        for tool in tools {
            let mut parameters = json::to_value(&tool.function.parameters).map_err(|e| {
                anyhow!(
                    "Failed to serialize tool parameters for '{}': {e}",
                    tool.function.name
                )
            })?;

            // Normalize schema strings to remove escaped quotes (\" → ")
            // that cause OpenAI strict mode validation failures
            normalize_json_schema_strings(&mut parameters);

            definitions.push(ToolDefinition {
                name: tool.function.name.clone(),
                description: tool.function.description.clone().unwrap_or_default(),
                parameters,
            });
        }

        Ok(definitions)
    }

    /// Converts tool choice to rig ToolChoice
    pub fn tool_choice_to_rig(&self) -> Option<RigToolChoice> {
        self.tool_choice.as_ref().map(|choice| match choice {
            ToolChoice::None => RigToolChoice::None,
            ToolChoice::Auto => RigToolChoice::Auto,
            ToolChoice::Required => RigToolChoice::Required,
            ToolChoice::Specific { function, .. } => RigToolChoice::Specific {
                function_names: vec![function.name.clone()],
            },
        })
    }

    /// Builds additional parameters for the request
    pub fn build_additional_params(&self) -> Result<Option<Value>> {
        let mut map = json::Map::new();

        if let Some(stream) = self.stream {
            map.insert("stream".to_string(), Value::Bool(stream));
        }

        if let Some(top_p) = self.top_p {
            map.insert("top_p".to_string(), json::json!(top_p));
        }

        if let Some(presence_penalty) = self.presence_penalty {
            map.insert(
                "presence_penalty".to_string(),
                json::json!(presence_penalty),
            );
        }

        if let Some(frequency_penalty) = self.frequency_penalty {
            map.insert(
                "frequency_penalty".to_string(),
                json::json!(frequency_penalty),
            );
        }

        if let Some(stop) = self.stop.as_ref() {
            map.insert("stop".to_string(), json::json!(stop));
        }

        if let Some(user) = self.user.as_ref() {
            map.insert("user".to_string(), json::json!(user));
        }

        if let Some(seed) = self.seed {
            map.insert("seed".to_string(), json::json!(seed));
        }

        if let Some(response_format) = self.response_format.as_ref() {
            let value = match response_format {
                ResponseFormat::String(s) => json::json!(s),
                ResponseFormat::Object(v) => json::to_value(v)?,
            };
            map.insert("response_format".to_string(), value);
        }

        if let Some(n) = self.n {
            map.insert("n".to_string(), json::json!(n));
        }

        if let Some(options) = self.stream_options.as_ref() {
            map.insert("stream_options".to_string(), json::to_value(options)?);
        }

        if let Some(usage) = self.usage.as_ref() {
            map.insert("usage".to_string(), json::to_value(usage)?);
        }

        if let Some(preset) = self.preset.as_ref() {
            map.insert("preset".to_string(), json::json!(preset));
        }

        if map.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Value::Object(map)))
        }
    }
}

impl From<Vec<RigMessage>> for History {
    fn from(messages: Vec<RigMessage>) -> Self {
        let mut history_messages: Vec<HistoryMessage> = Vec::new();
        for msg in messages {
            if let RigMessage::User { ref content } = msg {
                let tool_results: Vec<_> = content
                    .iter()
                    .filter(|c| matches!(c, RigUserContent::ToolResult(_)))
                    .collect();
                let has_non_tool = content
                    .iter()
                    .any(|c| !matches!(c, RigUserContent::ToolResult(_)));

                if tool_results.len() > 1 || (tool_results.len() == 1 && has_non_tool) {
                    for c in content.iter() {
                        if let RigUserContent::ToolResult(tr) = c {
                            let contents = tr
                                .content
                                .iter()
                                .map(|item| match item {
                                    RigToolResultContent::Text(text) => Content::Text {
                                        content_type: ContentType::Text,
                                        text: text.text.clone(),
                                    },
                                    RigToolResultContent::Image(image) => {
                                        Content::from_rig_image(image.clone())
                                    }
                                })
                                .collect();
                            history_messages.push(HistoryMessage {
                                role: Role::Tool,
                                content: MessageContent::Contents(contents),
                                name: None,
                                tool_call_id: Some(tr.id.clone()),
                                tool_calls: None,
                                annotations: None,
                            });
                        }
                    }
                    continue;
                }
            }
            history_messages.push(msg.into());
        }
        Self::new("".to_string(), history_messages)
    }
}

impl TryFrom<History> for Vec<RigMessage> {
    type Error = flow_like_types::Error;

    fn try_from(history: History) -> Result<Self> {
        history.to_rig_messages()
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: ToolType,
    pub function: HistoryFunction,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub enum ToolType {
    Function,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct HistoryFunction {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: HistoryFunctionParameters,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct HistoryFunctionParameters {
    #[serde(rename = "type")]
    pub schema_type: HistoryJSONSchemaType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, Box<HistoryJSONSchemaDefine>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub enum HistoryJSONSchemaType {
    Object,
    Number,
    String,
    Array,
    Null,
    Boolean,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct HistoryJSONSchemaDefine {
    #[serde(rename = "type")]
    pub schema_type: Option<HistoryJSONSchemaType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "enum")]
    pub enum_values: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<HashMap<String, Box<HistoryJSONSchemaDefine>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<HistoryJSONSchemaDefine>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(rename_all = "lowercase", untagged)]
pub enum ToolChoice {
    None,
    Auto,
    Required,
    Specific {
        r#type: ToolType,
        function: HistoryFunction,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_data_uris_become_typed_rig_inputs() {
        let audio: RigUserContent = Content::Audio {
            content_type: ContentType::AudioUrl,
            audio_url: "data:audio/mpeg;base64,YXVkaW8=".to_string(),
            media_type: None,
            additional_params: None,
        }
        .into();
        assert!(matches!(
            audio,
            RigUserContent::Audio(RigAudio {
                data: DocumentSourceKind::Base64(data),
                media_type: Some(AudioMediaType::MP3),
                ..
            }) if data == "YXVkaW8="
        ));

        let video: RigUserContent = Content::Video {
            content_type: ContentType::VideoUrl,
            video_url: "data:video/webm;base64,dmlkZW8=".to_string(),
            media_type: None,
            additional_params: None,
        }
        .into();
        assert!(matches!(
            video,
            RigUserContent::Video(RigVideo {
                data: DocumentSourceKind::Base64(data),
                media_type: Some(VideoMediaType::WEBM),
                ..
            }) if data == "dmlkZW8="
        ));

        let document: RigUserContent = Content::Document {
            content_type: ContentType::DocumentUrl,
            document_url: "data:application/pdf;base64,JVBERg==".to_string(),
            media_type: None,
            additional_params: None,
        }
        .into();
        assert!(matches!(
            document,
            RigUserContent::Document(RigDocument {
                data: DocumentSourceKind::Base64(data),
                media_type: Some(DocumentMediaType::PDF),
                ..
            }) if data == "JVBERg=="
        ));
    }

    #[test]
    fn explicit_mime_fills_generic_data_uris_without_guessing_unknown_images() {
        let audio: RigUserContent = Content::Audio {
            content_type: ContentType::AudioUrl,
            audio_url: "data:application/octet-stream;base64,YXVkaW8=".to_string(),
            media_type: Some("audio/mpeg".to_string()),
            additional_params: None,
        }
        .into();
        assert!(matches!(
            audio,
            RigUserContent::Audio(RigAudio {
                media_type: Some(AudioMediaType::MP3),
                ..
            })
        ));

        let image = Content::Image {
            content_type: ContentType::ImageUrl,
            image_url: ImageUrl {
                url: "https://example.com/signed-download".to_string(),
                detail: None,
                media_type: None,
                additional_params: None,
            },
        }
        .try_into_rig_assistant()
        .expect("valid image");
        assert!(matches!(
            image,
            RigAssistantContent::Image(RigImage {
                media_type: None,
                ..
            })
        ));
    }

    #[test]
    fn chat_upload_mime_aliases_map_to_rig_types() {
        assert_eq!(
            audio_media_type_from_mime("audio/mpeg3"),
            Some(AudioMediaType::MP3)
        );
        assert_eq!(
            document_media_type_from_mime("text/md"),
            Some(DocumentMediaType::MARKDOWN)
        );
        assert_eq!(
            document_media_type_from_mime("text/x-javascript"),
            Some(DocumentMediaType::Javascript)
        );
        assert_eq!(
            document_media_type_from_mime("text/x-python"),
            Some(DocumentMediaType::Python)
        );
    }

    #[test]
    fn rig_sources_round_trip_without_becoming_fake_urls() {
        let raw_audio = RigUserContent::Audio(RigAudio {
            data: DocumentSourceKind::Raw(b"audio".to_vec()),
            media_type: Some(AudioMediaType::WAV),
            additional_params: None,
        });
        let flow_audio: Content = raw_audio.into();
        assert!(matches!(
            &flow_audio,
            Content::Audio { audio_url, .. }
                if audio_url == "data:audio/wav;flow-like-source=raw;base64,YXVkaW8="
        ));
        let RigUserContent::Audio(round_trip) = flow_audio.into() else {
            panic!("expected audio")
        };
        assert_eq!(round_trip.data, DocumentSourceKind::Raw(b"audio".to_vec()));
        assert_eq!(round_trip.media_type, Some(AudioMediaType::WAV));

        let file_image = Content::from_rig_image(RigImage {
            data: DocumentSourceKind::FileId("file-123".to_string()),
            media_type: Some(ImageMediaType::PNG),
            detail: None,
            additional_params: None,
        });
        assert!(matches!(
            &file_image,
            Content::Image { image_url, .. } if image_url.url == "file_id:file-123"
        ));
        let RigAssistantContent::Image(round_trip) = file_image
            .try_into_rig_assistant()
            .expect("image round trip")
        else {
            panic!("expected image")
        };
        assert_eq!(
            round_trip.data,
            DocumentSourceKind::FileId("file-123".to_string())
        );

        let string_document = RigUserContent::Document(RigDocument {
            data: DocumentSourceKind::String("literal document text".to_string()),
            media_type: Some(DocumentMediaType::TXT),
            additional_params: None,
        });
        let flow_document: Content = string_document.into();
        assert!(matches!(
            &flow_document,
            Content::Document { document_url, .. }
                if document_url.starts_with("data:text/plain;flow-like-source=string;base64,")
        ));
        let RigUserContent::Document(round_trip) = flow_document.into() else {
            panic!("expected document")
        };
        assert_eq!(
            round_trip.data,
            DocumentSourceKind::String("literal document text".to_string())
        );
    }

    #[test]
    fn textual_documents_are_decoded_instead_of_reaching_the_model_as_base64() {
        let document = Content::Document {
            content_type: ContentType::DocumentUrl,
            document_url: "data:text/markdown;base64,SGVsbG8gd29ybGQ=".to_string(),
            media_type: None,
            additional_params: None,
        };
        let RigUserContent::Document(converted) = document.into() else {
            panic!("expected document")
        };
        assert_eq!(converted.media_type, Some(DocumentMediaType::MARKDOWN));
        assert_eq!(
            converted.data,
            DocumentSourceKind::String("Hello world".to_string())
        );

        let pdf = Content::Document {
            content_type: ContentType::DocumentUrl,
            document_url: "data:application/pdf;base64,SGVsbG8gd29ybGQ=".to_string(),
            media_type: None,
            additional_params: None,
        };
        let RigUserContent::Document(converted) = pdf.into() else {
            panic!("expected document")
        };
        assert_eq!(
            converted.data,
            DocumentSourceKind::Base64("SGVsbG8gd29ybGQ=".to_string()),
            "binary documents must stay base64"
        );
    }

    #[test]
    fn non_base64_data_urls_are_not_split_on_an_embedded_base64_marker() {
        let svg =
            "data:image/svg+xml,<svg><image href=\"data:image/png;base64,iVBORw0KGgo=\"/></svg>";
        let image = Content::Image {
            content_type: ContentType::ImageUrl,
            image_url: ImageUrl {
                url: svg.to_string(),
                detail: None,
                media_type: None,
                additional_params: None,
            },
        };
        let RigUserContent::Image(converted) = image.into() else {
            panic!("expected image")
        };
        assert_eq!(
            converted.data,
            DocumentSourceKind::Url(svg.to_string()),
            "an inline SVG carrying a nested base64 image must survive intact"
        );
    }

    #[test]
    fn assistant_images_survive_history_conversion() {
        let message = RigMessage::Assistant {
            id: Some("assistant-1".to_string()),
            content: OneOrMany::many(vec![
                RigAssistantContent::Text(RigText::new("caption")),
                RigAssistantContent::Image(RigImage {
                    data: DocumentSourceKind::Url("https://example.com/generated.webp".to_string()),
                    media_type: Some(ImageMediaType::WEBP),
                    detail: Some(ImageDetail::Auto),
                    additional_params: Some(json::json!({
                        "openrouter": {
                            "response_only": true,
                            "source": "assistant.images"
                        }
                    })),
                }),
            ])
            .expect("multiple contents"),
        };

        let history_message: HistoryMessage = message.into();
        let MessageContent::Contents(parts) = &history_message.content else {
            panic!("multimodal assistant message should use content parts")
        };
        assert_eq!(parts.len(), 2);

        let round_trip: RigMessage = history_message.try_into().expect("Rig round trip");
        let RigMessage::Assistant { content, .. } = round_trip else {
            panic!("expected assistant message")
        };
        let round_trip_image = content.iter().find_map(|part| match part {
            RigAssistantContent::Image(image) => Some(image),
            _ => None,
        });
        assert_eq!(
            round_trip_image.and_then(|image| image.additional_params.as_ref()),
            Some(&json::json!({
                "openrouter": {
                    "response_only": true,
                    "source": "assistant.images"
                }
            }))
        );
    }

    #[test]
    fn unsupported_assistant_media_is_kept_in_history_but_skipped_on_replay() {
        let history_message = HistoryMessage {
            role: Role::Assistant,
            content: MessageContent::Contents(vec![
                Content::Text {
                    content_type: ContentType::Text,
                    text: "listen".to_string(),
                },
                Content::Audio {
                    content_type: ContentType::AudioUrl,
                    audio_url: "https://example.com/generated.mp3".to_string(),
                    media_type: Some("audio/mpeg".to_string()),
                    additional_params: None,
                },
                Content::Video {
                    content_type: ContentType::VideoUrl,
                    video_url: "https://example.com/generated.mp4".to_string(),
                    media_type: Some("video/mp4".to_string()),
                    additional_params: None,
                },
                Content::Document {
                    content_type: ContentType::DocumentUrl,
                    document_url: "https://example.com/generated.pdf".to_string(),
                    media_type: Some("application/pdf".to_string()),
                    additional_params: None,
                },
            ]),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            annotations: None,
        };

        let RigMessage::Assistant { content, .. } = history_message
            .try_into()
            .expect("unsupported response-only media must not break continuation")
        else {
            panic!("expected assistant message")
        };
        let parts: Vec<_> = content.iter().collect();
        assert_eq!(parts.len(), 1);
        assert!(matches!(
            parts[0],
            RigAssistantContent::Text(text) if text.text == "listen"
        ));
    }

    #[test]
    fn tool_result_images_survive_history_conversion() {
        let message = RigMessage::User {
            content: OneOrMany::one(RigUserContent::ToolResult(rig::message::ToolResult {
                id: "tool-1".to_string(),
                call_id: None,
                content: OneOrMany::many(vec![
                    RigToolResultContent::text("chart"),
                    RigToolResultContent::Image(RigImage {
                        data: DocumentSourceKind::Base64("aW1hZ2U=".to_string()),
                        media_type: Some(ImageMediaType::PNG),
                        detail: None,
                        additional_params: None,
                    }),
                ])
                .expect("multiple tool result parts"),
            })),
        };

        let history_message: HistoryMessage = message.into();
        assert_eq!(history_message.role, Role::Tool);
        let MessageContent::Contents(parts) = &history_message.content else {
            panic!("tool result should keep content parts")
        };
        assert_eq!(parts.len(), 2);

        let round_trip: RigMessage = history_message.try_into().expect("Rig round trip");
        let RigMessage::User { content } = round_trip else {
            panic!("expected Rig tool result")
        };
        let RigUserContent::ToolResult(result) = content.first() else {
            panic!("expected tool result content")
        };
        assert!(
            result
                .content
                .iter()
                .any(|part| matches!(part, RigToolResultContent::Image(_)))
        );
    }
}

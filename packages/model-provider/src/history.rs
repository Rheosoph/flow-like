// Implementation according to
// https://modelcontextprotocol.io/docs/concepts/sampling/

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
    AssistantContent as RigAssistantContent, Audio as RigAudio, Document as RigDocument,
    DocumentSourceKind, Image as RigImage, ImageDetail, ImageMediaType, Text as RigText,
    ToolCall as RigToolCall, ToolChoice as RigToolChoice, ToolFunction as RigToolFunction,
    UserContent as RigUserContent, Video as RigVideo,
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

        let content = match first_choice {
            Some(choice) => choice.message.content.clone(),
            None => None,
        };
        let annotations = match first_choice {
            Some(choice) => choice.message.annotations.clone(),
            None => None,
        };

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
            content: MessageContent::Contents(vec![Content::Text {
                content_type: ContentType::Text,
                text: content.unwrap_or_default(),
            }]),
            name: None,
            tool_call_id: None,
            tool_calls: None,
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
            RigMessage::User { content } => {
                let is_single_tool_result =
                    content.len() == 1 && matches!(content.first(), RigUserContent::ToolResult(_));

                if is_single_tool_result && let RigUserContent::ToolResult(tr) = content.first() {
                    let text = tr
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            rig::message::ToolResultContent::Text(t) => Some(t.text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    return HistoryMessage {
                        role: Role::Tool,
                        content: MessageContent::Contents(vec![Content::Text {
                            content_type: ContentType::Text,
                            text,
                        }]),
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
                let mut text_content = String::new();

                for item in content.iter() {
                    match item {
                        RigAssistantContent::Text(text) => {
                            if !text_content.is_empty() {
                                text_content.push('\n');
                            }
                            text_content.push_str(&text.text);
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
                        RigAssistantContent::Reasoning(_) | RigAssistantContent::Image(_) => {}
                    }
                }

                HistoryMessage {
                    role: Role::Assistant,
                    content: MessageContent::String(text_content),
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

impl TryFrom<HistoryMessage> for RigMessage {
    type Error = flow_like_types::Error;

    fn try_from(msg: HistoryMessage) -> Result<Self> {
        match msg.role {
            Role::User => {
                let contents: Vec<RigUserContent> = match msg.content {
                    MessageContent::String(s) => {
                        vec![RigUserContent::Text(RigText { text: s })]
                    }
                    MessageContent::Contents(contents) => {
                        contents.into_iter().map(|c| c.into()).collect()
                    }
                };

                let content = if contents.is_empty() {
                    OneOrMany::one(RigUserContent::Text(RigText {
                        text: String::new(),
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
                        rig_contents.push(RigAssistantContent::Text(RigText { text: s }));
                    }
                    MessageContent::Contents(contents) => {
                        for content in contents {
                            if let Content::Text { text, .. } = content
                                && !text.is_empty()
                            {
                                rig_contents.push(RigAssistantContent::Text(RigText { text }));
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
                use rig::message::{ToolResult, ToolResultContent};
                let text = msg.as_str();
                let tool_call_id = msg.tool_call_id.or(msg.name.clone()).unwrap_or_default();
                Ok(RigMessage::User {
                    content: OneOrMany::one(RigUserContent::ToolResult(ToolResult {
                        id: tool_call_id,
                        call_id: None,
                        content: OneOrMany::one(ToolResultContent::text(text)),
                    })),
                })
            }
            Role::System => {
                let text = msg.as_str();
                Ok(RigMessage::User {
                    content: OneOrMany::one(RigUserContent::Text(RigText { text })),
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
    },
    Video {
        #[serde(rename = "type")]
        content_type: ContentType,
        video_url: String,
    },
    Document {
        #[serde(rename = "type")]
        content_type: ContentType,
        document_url: String,
    },
}

impl From<RigUserContent> for Content {
    fn from(rig_content: RigUserContent) -> Self {
        match rig_content {
            RigUserContent::Text(text) => Content::Text {
                content_type: ContentType::Text,
                text: text.text,
            },
            RigUserContent::Image(image) => Content::Image {
                content_type: ContentType::ImageUrl,
                image_url: ImageUrl {
                    url: image.data.to_string(),
                    detail: image.detail.map(|d| format!("{:?}", d).to_lowercase()),
                },
            },
            RigUserContent::Audio(audio) => Content::Audio {
                content_type: ContentType::AudioUrl,
                audio_url: audio.data.to_string(),
            },
            RigUserContent::Video(video) => Content::Video {
                content_type: ContentType::VideoUrl,
                video_url: video.data.to_string(),
            },
            RigUserContent::Document(doc) => Content::Document {
                content_type: ContentType::DocumentUrl,
                document_url: doc.data.to_string(),
            },
            RigUserContent::ToolResult(tool_result) => {
                let text = tool_result
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        rig::message::ToolResultContent::Text(t) => Some(t.text.as_str()),
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
            Content::Text { text, .. } => RigUserContent::Text(RigText { text }),
            Content::Image { image_url, .. } => {
                // Detect media type from URL or default to PNG
                let media_type = detect_image_media_type(&image_url.url);

                // Prefer passing raw base64 payloads to rig providers when the input is a data URL.
                // Some providers (notably OpenAI-compatible ones) are more reliable with base64 than
                // with large `data:` URLs.
                let data = if image_url.url.starts_with("data:") {
                    // Expected shape: data:<mime>;base64,<payload>
                    image_url
                        .url
                        .find(",")
                        .and_then(|comma_pos| {
                            let prefix = &image_url.url[..comma_pos];
                            if prefix.contains(";base64") {
                                Some(DocumentSourceKind::Base64(
                                    image_url.url[(comma_pos + 1)..].to_string(),
                                ))
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| DocumentSourceKind::url(&image_url.url))
                } else {
                    DocumentSourceKind::url(&image_url.url)
                };

                RigUserContent::Image(RigImage {
                    data,
                    media_type: Some(media_type),
                    detail: Some(parse_image_detail(image_url.detail.as_deref())),
                    additional_params: None,
                })
            }
            Content::Audio { audio_url, .. } => RigUserContent::Audio(RigAudio {
                data: DocumentSourceKind::url(&audio_url),
                media_type: None,
                additional_params: None,
            }),
            Content::Video { video_url, .. } => RigUserContent::Video(RigVideo {
                data: DocumentSourceKind::url(&video_url),
                media_type: None,
                additional_params: None,
            }),
            Content::Document { document_url, .. } => RigUserContent::Document(RigDocument {
                data: DocumentSourceKind::url(&document_url),
                media_type: None,
                additional_params: None,
            }),
        }
    }
}

/// Detects image media type from URL extension or data URL MIME type
fn detect_image_media_type(url: &str) -> ImageMediaType {
    // Check if it's a data URL with MIME type
    if url.starts_with("data:")
        && let Some(mime_start) = url.strip_prefix("data:")
        && let Some(mime_end) = mime_start.find(';')
    {
        let mime_type = &mime_start[..mime_end];
        return match mime_type {
            "image/jpeg" | "image/jpg" => ImageMediaType::JPEG,
            "image/png" => ImageMediaType::PNG,
            "image/gif" => ImageMediaType::GIF,
            "image/webp" => ImageMediaType::WEBP,
            "image/heic" => ImageMediaType::HEIC,
            "image/heif" => ImageMediaType::HEIF,
            _ => ImageMediaType::PNG, // default fallback
        };
    }

    // Check file extension
    let lower_url = url.to_lowercase();
    if lower_url.ends_with(".jpg") || lower_url.ends_with(".jpeg") {
        ImageMediaType::JPEG
    } else if lower_url.ends_with(".png") {
        ImageMediaType::PNG
    } else if lower_url.ends_with(".gif") {
        ImageMediaType::GIF
    } else if lower_url.ends_with(".webp") {
        ImageMediaType::WEBP
    } else if lower_url.ends_with(".heic") {
        ImageMediaType::HEIC
    } else if lower_url.ends_with(".heif") {
        ImageMediaType::HEIF
    } else {
        // Default to PNG if we can't detect
        ImageMediaType::PNG
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
                            let text = tr
                                .content
                                .iter()
                                .filter_map(|trc| match trc {
                                    rig::message::ToolResultContent::Text(t) => {
                                        Some(t.text.as_str())
                                    }
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            history_messages.push(HistoryMessage {
                                role: Role::Tool,
                                content: MessageContent::Contents(vec![Content::Text {
                                    content_type: ContentType::Text,
                                    text,
                                }]),
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

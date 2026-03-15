//! FlowLike rig provider — allows using a `Bit` as a `rig::completion::CompletionModel`.
//!
//! This module bridges the WASM SDK's synchronous host-call–based LLM interface with rig's
//! async `CompletionModel` trait, letting WASM node authors use rig agents, chains and
//! extractors backed by models running on the host.

use crate::interop::{
    AudioData, Bit, ChatContent, ChatMessage, ContentPart, DocumentData, FlowPath, ImageData,
    ReasoningData, ToolCallData, ToolResultData, VideoData,
};
use crate::Context;
use futures::stream;
use rig::client::FinalCompletionResponse;
use rig::completion::{
    CompletionError, CompletionModel, CompletionRequest, CompletionResponse, Message, Usage,
};
use rig::message::{
    AssistantContent, Audio, Document, DocumentSourceKind, Image, MimeType, Reasoning, Text,
    ToolCall, ToolFunction, ToolResult, ToolResultContent, UserContent, Video,
};
use rig::streaming::{RawStreamingChoice, StreamingCompletionResponse, StreamingResult};
use rig::tool::Tool;
use rig::OneOrMany;

// =============================================================================
// Conversion: rig Message → SDK ChatMessage
// =============================================================================

fn rig_user_content_to_parts(content: &OneOrMany<UserContent>) -> Vec<ContentPart> {
    content
        .iter()
        .map(|uc| match uc {
            UserContent::Text(t) => ContentPart::Text {
                text: t.text.clone(),
            },
            UserContent::Image(img) => ContentPart::Image {
                image: ImageData {
                    url: img.data.to_string(),
                    media_type: img.media_type.as_ref().map(|m| m.to_mime_type().into()),
                    detail: img.detail.as_ref().map(|d| format!("{d:?}").to_lowercase()),
                },
            },
            UserContent::Audio(aud) => ContentPart::Audio {
                audio: AudioData {
                    url: aud.data.to_string(),
                    media_type: aud.media_type.as_ref().map(|m| m.to_mime_type().into()),
                },
            },
            UserContent::Video(vid) => ContentPart::Video {
                video: VideoData {
                    url: vid.data.to_string(),
                    media_type: vid.media_type.as_ref().map(|m| m.to_mime_type().into()),
                },
            },
            UserContent::Document(doc) => ContentPart::Document {
                document: DocumentData {
                    url: doc.data.to_string(),
                    media_type: doc.media_type.as_ref().map(|m| m.to_mime_type().into()),
                },
            },
            UserContent::ToolResult(tr) => {
                let text = tr
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        ToolResultContent::Text(t) => Some(t.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                ContentPart::ToolResult {
                    tool_result: ToolResultData {
                        id: tr.id.clone(),
                        content: text,
                    },
                }
            }
        })
        .collect()
}

fn rig_assistant_content_to_parts(
    content: &OneOrMany<AssistantContent>,
) -> (Vec<ContentPart>, Vec<ToolCallData>) {
    let mut parts = Vec::new();
    let mut tool_calls = Vec::new();

    for ac in content.iter() {
        match ac {
            AssistantContent::Text(t) => {
                parts.push(ContentPart::Text {
                    text: t.text.clone(),
                });
            }
            AssistantContent::ToolCall(tc) => {
                tool_calls.push(ToolCallData {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: tc.function.arguments.clone(),
                });
            }
            AssistantContent::Reasoning(r) => {
                parts.push(ContentPart::Reasoning {
                    reasoning: ReasoningData {
                        id: r.id.clone(),
                        text: r.reasoning.clone(),
                        signature: r.signature.clone(),
                    },
                });
            }
            AssistantContent::Image(img) => {
                parts.push(ContentPart::Image {
                    image: ImageData {
                        url: img.data.to_string(),
                        media_type: img.media_type.as_ref().map(|m| m.to_mime_type().into()),
                        detail: img.detail.as_ref().map(|d| format!("{d:?}").to_lowercase()),
                    },
                });
            }
        }
    }

    (parts, tool_calls)
}

fn rig_message_to_chat(msg: &Message) -> ChatMessage {
    match msg {
        Message::User { content } => {
            let parts = rig_user_content_to_parts(content);
            if parts.len() == 1 {
                if let Some(ContentPart::Text { text }) = parts.first() {
                    return ChatMessage::user(text.clone());
                }
            }
            ChatMessage {
                role: "user".into(),
                content: ChatContent::Parts { parts },
                tool_calls: None,
                tool_call_id: None,
            }
        }
        Message::Assistant { content, .. } => {
            let (parts, tool_calls) = rig_assistant_content_to_parts(content);

            let text = parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            let has_non_text = parts.iter().any(|p| !matches!(p, ContentPart::Text { .. }));

            if !has_non_text && tool_calls.is_empty() {
                ChatMessage::assistant(text)
            } else {
                ChatMessage {
                    role: "assistant".into(),
                    content: if has_non_text {
                        ChatContent::Parts { parts }
                    } else {
                        ChatContent::Text { content: text }
                    },
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                    tool_call_id: None,
                }
            }
        }
    }
}

fn completion_request_to_messages(request: &CompletionRequest) -> Vec<ChatMessage> {
    let mut messages = Vec::new();

    if let Some(preamble) = &request.preamble {
        messages.push(ChatMessage::system(preamble.clone()));
    }

    for msg in request.chat_history.iter() {
        messages.push(rig_message_to_chat(msg));
    }

    messages
}

/// Serialize a CompletionRequest as the JSON payload for the host llm-prompt call.
/// If tools are present, wraps messages+tools in an object; otherwise sends a plain array.
fn serialize_llm_request(request: &CompletionRequest) -> Option<String> {
    use serde_json::json;

    let messages = completion_request_to_messages(request);

    if request.tools.is_empty() {
        serde_json::to_string(&messages).ok()
    } else {
        let tools: Vec<serde_json::Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            })
            .collect();
        serde_json::to_string(&json!({
            "messages": messages,
            "tools": tools,
        }))
        .ok()
    }
}

/// Parse the host response JSON into rig AssistantContent items.
/// The host may return a JSON ChatMessage (with optional tool_calls) or plain text.
fn parse_llm_response(text: &str) -> OneOrMany<AssistantContent> {
    #[derive(serde::Deserialize)]
    struct HostResponse {
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        tool_calls: Option<Vec<HostToolCall>>,
    }

    #[derive(serde::Deserialize)]
    struct HostToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    }

    if let Ok(resp) = serde_json::from_str::<HostResponse>(text) {
        let mut items: Vec<AssistantContent> = Vec::new();

        if let Some(content) = resp.content {
            if !content.is_empty() {
                items.push(AssistantContent::Text(Text { text: content }));
            }
        }

        if let Some(tool_calls) = resp.tool_calls {
            for tc in tool_calls {
                items.push(AssistantContent::ToolCall(ToolCall::new(
                    tc.id,
                    ToolFunction::new(tc.name, tc.arguments),
                )));
            }
        }

        if !items.is_empty() {
            return OneOrMany::many(items)
                .unwrap_or_else(|_| OneOrMany::one(AssistantContent::Text(Text { text: String::new() })));
        }
    }

    // Fallback: treat the entire string as plain text
    OneOrMany::one(AssistantContent::Text(Text {
        text: text.to_string(),
    }))
}

// =============================================================================
// Conversion: SDK ChatMessage → rig Message
// =============================================================================

fn content_part_to_user_content(part: &ContentPart) -> UserContent {
    match part {
        ContentPart::Text { text } => UserContent::Text(Text { text: text.clone() }),
        ContentPart::Image { image } => UserContent::Image(Image {
            data: DocumentSourceKind::url(&image.url),
            media_type: None,
            detail: None,
            additional_params: None,
        }),
        ContentPart::Audio { audio } => UserContent::Audio(Audio {
            data: DocumentSourceKind::url(&audio.url),
            media_type: None,
            additional_params: None,
        }),
        ContentPart::Video { video } => UserContent::Video(Video {
            data: DocumentSourceKind::url(&video.url),
            media_type: None,
            additional_params: None,
        }),
        ContentPart::Document { document } => UserContent::Document(Document {
            data: DocumentSourceKind::url(&document.url),
            media_type: None,
            additional_params: None,
        }),
        ContentPart::ToolResult { tool_result } => UserContent::ToolResult(ToolResult {
            id: tool_result.id.clone(),
            call_id: None,
            content: OneOrMany::one(ToolResultContent::Text(Text {
                text: tool_result.content.clone(),
            })),
        }),
        ContentPart::ToolCall { tool_call } => UserContent::Text(Text {
            text: format!(
                "[tool_call: {} {}({})]",
                tool_call.id, tool_call.name, tool_call.arguments
            ),
        }),
        ContentPart::Reasoning { reasoning } => UserContent::Text(Text {
            text: reasoning.text.join("\n"),
        }),
    }
}

/// Convert SDK `ChatMessage` slice to rig `Message` list + optional preamble.
pub fn chat_messages_to_rig(messages: &[ChatMessage]) -> (Option<String>, Vec<Message>) {
    let mut preamble = None;
    let mut rig_messages = Vec::new();

    for msg in messages {
        match msg.role.as_str() {
            "system" => {
                preamble = Some(msg.text_content());
            }
            "user" => match &msg.content {
                ChatContent::Text { content } => {
                    rig_messages.push(Message::User {
                        content: OneOrMany::one(UserContent::Text(Text {
                            text: content.clone(),
                        })),
                    });
                }
                ChatContent::Parts { parts } => {
                    let user_contents: Vec<UserContent> =
                        parts.iter().map(content_part_to_user_content).collect();
                    if let Ok(many) = OneOrMany::many(user_contents) {
                        rig_messages.push(Message::User { content: many });
                    }
                }
            },
            "assistant" => {
                let mut assistant_contents: Vec<AssistantContent> = Vec::new();

                // Extract text content
                match &msg.content {
                    ChatContent::Text { content } => {
                        if !content.is_empty() {
                            assistant_contents.push(AssistantContent::Text(Text {
                                text: content.clone(),
                            }));
                        }
                    }
                    ChatContent::Parts { parts } => {
                        for part in parts {
                            match part {
                                ContentPart::Text { text } => {
                                    assistant_contents.push(AssistantContent::Text(Text {
                                        text: text.clone(),
                                    }));
                                }
                                ContentPart::Image { image } => {
                                    assistant_contents.push(AssistantContent::Image(Image {
                                        data: DocumentSourceKind::url(&image.url),
                                        media_type: None,
                                        detail: None,
                                        additional_params: None,
                                    }));
                                }
                                ContentPart::Reasoning { reasoning } => {
                                    assistant_contents.push(AssistantContent::Reasoning(
                                        Reasoning::multi(reasoning.text.clone())
                                            .optional_id(reasoning.id.clone())
                                            .with_signature(reasoning.signature.clone()),
                                    ));
                                }
                                _ => {}
                            }
                        }
                    }
                }

                // Extract tool calls from the dedicated field
                if let Some(tool_calls) = &msg.tool_calls {
                    for tc in tool_calls {
                        assistant_contents.push(AssistantContent::ToolCall(
                            ToolCall::new(tc.id.clone(), ToolFunction::new(
                                tc.name.clone(),
                                tc.arguments.clone(),
                            )),
                        ));
                    }
                }

                if assistant_contents.is_empty() {
                    assistant_contents.push(AssistantContent::Text(Text {
                        text: String::new(),
                    }));
                }

                let content = OneOrMany::many(assistant_contents)
                    .unwrap_or_else(|_| OneOrMany::one(AssistantContent::Text(Text {
                        text: String::new(),
                    })));

                rig_messages.push(Message::Assistant {
                    id: None,
                    content,
                });
            }
            "tool" => {
                let tool_call_id = msg
                    .tool_call_id
                    .clone()
                    .unwrap_or_default();

                rig_messages.push(Message::User {
                    content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                        id: tool_call_id,
                        call_id: None,
                        content: OneOrMany::one(ToolResultContent::Text(Text {
                            text: msg.text_content(),
                        })),
                    })),
                });
            }
            _ => {}
        }
    }

    (preamble, rig_messages)
}

// =============================================================================
// FlowLikeCompletionModel
// =============================================================================

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FlowLikeResponse;

/// A rig `CompletionModel` backed by the FlowLike WASM host.
///
/// Use this to plug a `Bit` into rig agents, extractors, or pipelines inside a WASM node.
/// The actual inference runs on the host; this adapter converts rig's request format into
/// the host-call protocol and returns the result.
#[derive(Clone)]
pub struct FlowLikeCompletionModel {
    bit: Bit,
    ctx: *const Context,
}

unsafe impl Send for FlowLikeCompletionModel {}
unsafe impl Sync for FlowLikeCompletionModel {}

impl FlowLikeCompletionModel {
    /// Wrap a `Bit` and execution `Context` into a rig-compatible completion model.
    ///
    /// # Safety
    /// The `Context` reference must remain valid for the lifetime of this model. In practice
    /// this is always the case inside a WASM node's `run` function.
    pub fn new(bit: Bit, ctx: &Context) -> Self {
        Self {
            bit,
            ctx: ctx as *const Context,
        }
    }

    fn ctx(&self) -> &Context {
        unsafe { &*self.ctx }
    }
}

impl CompletionModel for FlowLikeCompletionModel {
    type Response = FlowLikeResponse;
    type StreamingResponse = FinalCompletionResponse;
    type Client = ();

    fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
        panic!("FlowLikeCompletionModel must be created via FlowLikeCompletionModel::new()")
    }

    fn completion(
        &self,
        request: CompletionRequest,
    ) -> impl std::future::Future<
        Output = Result<CompletionResponse<Self::Response>, CompletionError>,
    > + Send {
        crate::host::info(&format!(
            "FlowLikeCompletionModel::completion: tools={}, history={}",
            request.tools.len(),
            request.chat_history.len()
        ));

        let request_json = serialize_llm_request(&request);
        let result = request_json.and_then(|json| {
            crate::host::debug(&format!("llm_prompt request len={}", json.len()));
            let bit_json = serde_json::to_string(&self.bit).ok()?;
            let resp = crate::host::llm_prompt(&bit_json, &json, false);
            if let Some(ref r) = resp {
                crate::host::debug(&format!("llm_prompt response len={}", r.len()));
            } else {
                crate::host::warn("llm_prompt returned None");
            }
            resp
        });

        async move {
            let text = result.ok_or_else(|| {
                CompletionError::ProviderError("FlowLike host LLM prompt returned None".into())
            })?;

            let choice = parse_llm_response(&text);

            Ok(CompletionResponse {
                choice,
                usage: Usage::new(),
                raw_response: FlowLikeResponse,
            })
        }
    }

    fn stream(
        &self,
        request: CompletionRequest,
    ) -> impl std::future::Future<
        Output = Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError>,
    > + Send {
        let request_json = serialize_llm_request(&request);
        let result = request_json.and_then(|json| {
            let bit_json = serde_json::to_string(&self.bit).ok()?;
            crate::host::llm_prompt(&bit_json, &json, true)
        });

        async move {
            let text = result.ok_or_else(|| {
                CompletionError::ProviderError(
                    "FlowLike host LLM streaming prompt returned None".into(),
                )
            })?;

            let items: Vec<Result<RawStreamingChoice<FinalCompletionResponse>, CompletionError>> =
                vec![
                    Ok(RawStreamingChoice::Message(text)),
                    Ok(RawStreamingChoice::FinalResponse(
                        FinalCompletionResponse { usage: None },
                    )),
                ];
            let raw_stream: StreamingResult<FinalCompletionResponse> =
                Box::pin(stream::iter(items));

            Ok(StreamingCompletionResponse::stream(raw_stream))
        }
    }
}

// =============================================================================
// FlowPath rig tools — let rig agents interact with storage
// =============================================================================

/// Rig tool that reads a file from a FlowPath base directory.
/// The agent supplies a relative `path` argument and gets back the file contents as a string.
pub struct FlowPathReadTool {
    base: FlowPath,
    ctx: *const Context,
}

unsafe impl Send for FlowPathReadTool {}
unsafe impl Sync for FlowPathReadTool {}

impl FlowPathReadTool {
    pub fn new(base: FlowPath, ctx: &Context) -> Self {
        Self {
            base,
            ctx: ctx as *const Context,
        }
    }

    fn ctx(&self) -> &Context {
        unsafe { &*self.ctx }
    }
}

#[derive(Debug)]
pub struct FlowPathToolError(String);

impl std::fmt::Display for FlowPathToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for FlowPathToolError {}

impl Tool for FlowPathReadTool {
    const NAME: &'static str = "read_file";
    type Error = FlowPathToolError;
    type Args = serde_json::Value;
    type Output = String;

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Read the contents of a file at the given relative path.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative path to the file" }
                },
                "required": ["path"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| FlowPathToolError("Missing 'path' argument".into()))?;

        let target = self.base.child(path);
        target
            .get_string(self.ctx())
            .ok_or_else(|| FlowPathToolError(format!("File not found: {path}")))
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

/// Rig tool that writes text content to a file at a relative path.
pub struct FlowPathWriteTool {
    base: FlowPath,
    ctx: *const Context,
}

unsafe impl Send for FlowPathWriteTool {}
unsafe impl Sync for FlowPathWriteTool {}

impl FlowPathWriteTool {
    pub fn new(base: FlowPath, ctx: &Context) -> Self {
        Self {
            base,
            ctx: ctx as *const Context,
        }
    }

    fn ctx(&self) -> &Context {
        unsafe { &*self.ctx }
    }
}

impl Tool for FlowPathWriteTool {
    const NAME: &'static str = "write_file";
    type Error = FlowPathToolError;
    type Args = serde_json::Value;
    type Output = String;

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Write text content to a file at the given relative path.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Relative path to the file" },
                    "content": { "type": "string", "description": "Text content to write" }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| FlowPathToolError("Missing 'path' argument".into()))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| FlowPathToolError("Missing 'content' argument".into()))?;

        let target = self.base.child(path);
        if target.put_string(self.ctx(), content) {
            Ok(format!("Written to {path}"))
        } else {
            Err(FlowPathToolError(format!("Failed to write to {path}")))
        }
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

/// Rig tool that lists files under a FlowPath prefix.
pub struct FlowPathListTool {
    base: FlowPath,
    ctx: *const Context,
}

unsafe impl Send for FlowPathListTool {}
unsafe impl Sync for FlowPathListTool {}

impl FlowPathListTool {
    pub fn new(base: FlowPath, ctx: &Context) -> Self {
        Self {
            base,
            ctx: ctx as *const Context,
        }
    }

    fn ctx(&self) -> &Context {
        unsafe { &*self.ctx }
    }
}

impl Tool for FlowPathListTool {
    const NAME: &'static str = "list_files";
    type Error = FlowPathToolError;
    type Args = serde_json::Value;
    type Output = String;

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: Self::NAME.to_string(),
            description: "List files under the given relative directory path.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Relative directory path (empty string for root)",
                        "default": ""
                    }
                },
                "required": []
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let target = if path.is_empty() {
            self.base.clone()
        } else {
            self.base.child(path)
        };

        let entries = target
            .list(self.ctx())
            .ok_or_else(|| FlowPathToolError("Failed to list directory".into()))?;

        let names: Vec<String> = entries.iter().filter_map(|e| e.file_name()).collect();
        Ok(serde_json::to_string(&names).unwrap_or_default())
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interop::{ChatContent, ContentPart, ToolCallData};
    use serde_json::json;

    #[test]
    fn test_text_message_roundtrip() {
        let sdk_msgs = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi!"),
        ];
        let (preamble, rig_msgs) = chat_messages_to_rig(&sdk_msgs);
        assert_eq!(preamble.as_deref(), Some("You are helpful."));
        assert_eq!(rig_msgs.len(), 2);

        let back: Vec<ChatMessage> = rig_msgs.iter().map(rig_message_to_chat).collect();
        assert_eq!(back[0].text_content(), "Hello");
        assert_eq!(back[1].text_content(), "Hi!");
    }

    #[test]
    fn test_multimodal_to_rig() {
        let msg = ChatMessage::user_multimodal(vec![
            ContentPart::text("Describe this"),
            ContentPart::image_url("https://example.com/img.png"),
        ]);
        let (_, rig_msgs) = chat_messages_to_rig(&[msg]);
        assert_eq!(rig_msgs.len(), 1);
        if let Message::User { content } = &rig_msgs[0] {
            assert_eq!(content.len(), 2);
        } else {
            panic!("Expected user message");
        }
    }

    #[test]
    fn test_rig_to_sdk_preserves_multimodal() {
        let rig_msg = Message::User {
            content: OneOrMany::many(vec![
                UserContent::Text(Text {
                    text: "Look at this".into(),
                }),
                UserContent::Image(Image {
                    data: DocumentSourceKind::url("https://example.com/img.png"),
                    media_type: None,
                    detail: None,
                    additional_params: None,
                }),
            ])
            .unwrap(),
        };

        let sdk_msg = rig_message_to_chat(&rig_msg);
        assert_eq!(sdk_msg.role, "user");
        if let ChatContent::Parts { parts } = &sdk_msg.content {
            assert_eq!(parts.len(), 2);
            assert!(matches!(parts[0], ContentPart::Text { .. }));
            assert!(matches!(parts[1], ContentPart::Image { .. }));
        } else {
            panic!("Expected multimodal parts");
        }
    }

    #[test]
    fn test_completion_request_to_messages() {
        let request = CompletionRequest {
            preamble: Some("Be concise.".into()),
            chat_history: OneOrMany::one(Message::User {
                content: OneOrMany::one(UserContent::Text(Text {
                    text: "What is 2+2?".into(),
                })),
            }),
            documents: vec![],
            tools: vec![],
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
        };

        let msgs = completion_request_to_messages(&request);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[0].text_content(), "Be concise.");
        assert_eq!(msgs[1].role, "user");
        assert_eq!(msgs[1].text_content(), "What is 2+2?");
    }

    #[test]
    fn test_tool_call_rig_to_sdk() {
        let rig_msg = Message::Assistant {
            id: None,
            content: OneOrMany::many(vec![
                AssistantContent::Text(Text {
                    text: "Let me check.".into(),
                }),
                AssistantContent::ToolCall(ToolCall::new(
                    "call_1".into(),
                    ToolFunction::new("get_weather".into(), json!({"city": "Berlin"})),
                )),
            ])
            .unwrap(),
        };

        let sdk_msg = rig_message_to_chat(&rig_msg);
        assert_eq!(sdk_msg.role, "assistant");
        assert_eq!(sdk_msg.text_content(), "Let me check.");
        let tc = sdk_msg.tool_calls.as_ref().expect("should have tool_calls");
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0].id, "call_1");
        assert_eq!(tc[0].name, "get_weather");
        assert_eq!(tc[0].arguments, json!({"city": "Berlin"}));
    }

    #[test]
    fn test_tool_call_sdk_to_rig() {
        let sdk_msg = ChatMessage::assistant_with_tool_calls(
            "Let me check.",
            vec![ToolCallData {
                id: "call_1".into(),
                name: "get_weather".into(),
                arguments: json!({"city": "Berlin"}),
            }],
        );

        let (_, rig_msgs) = chat_messages_to_rig(&[sdk_msg]);
        assert_eq!(rig_msgs.len(), 1);
        if let Message::Assistant { content, .. } = &rig_msgs[0] {
            assert_eq!(content.len(), 2);
            assert!(matches!(content.first(), AssistantContent::Text(_)));
            let items: Vec<_> = content.iter().collect();
            if let AssistantContent::ToolCall(tc) = items[1] {
                assert_eq!(tc.id, "call_1");
                assert_eq!(tc.function.name, "get_weather");
            } else {
                panic!("Expected ToolCall");
            }
        } else {
            panic!("Expected assistant message");
        }
    }

    #[test]
    fn test_tool_result_rig_to_sdk() {
        let rig_msg = Message::User {
            content: OneOrMany::one(UserContent::ToolResult(ToolResult {
                id: "call_1".into(),
                call_id: None,
                content: OneOrMany::one(ToolResultContent::Text(Text {
                    text: "25°C, sunny".into(),
                })),
            })),
        };

        let sdk_msg = rig_message_to_chat(&rig_msg);
        assert_eq!(sdk_msg.role, "user");
        if let ChatContent::Parts { parts } = &sdk_msg.content {
            assert_eq!(parts.len(), 1);
            if let ContentPart::ToolResult { tool_result } = &parts[0] {
                assert_eq!(tool_result.id, "call_1");
                assert_eq!(tool_result.content, "25°C, sunny");
            } else {
                panic!("Expected ToolResult part");
            }
        } else {
            panic!("Expected Parts content");
        }
    }

    #[test]
    fn test_tool_role_sdk_to_rig() {
        let sdk_msg = ChatMessage::tool_result("call_1", "25°C, sunny");

        let (_, rig_msgs) = chat_messages_to_rig(&[sdk_msg]);
        assert_eq!(rig_msgs.len(), 1);
        if let Message::User { content } = &rig_msgs[0] {
            if let UserContent::ToolResult(tr) = content.first() {
                assert_eq!(tr.id, "call_1");
            } else {
                panic!("Expected ToolResult");
            }
        } else {
            panic!("Expected User message with ToolResult");
        }
    }

    #[test]
    fn test_reasoning_rig_to_sdk() {
        let rig_msg = Message::Assistant {
            id: None,
            content: OneOrMany::many(vec![
                AssistantContent::Reasoning(
                    Reasoning::multi(vec!["Step 1".into(), "Step 2".into()])
                        .with_id("r1".into()),
                ),
                AssistantContent::Text(Text {
                    text: "The answer is 4.".into(),
                }),
            ])
            .unwrap(),
        };

        let sdk_msg = rig_message_to_chat(&rig_msg);
        assert_eq!(sdk_msg.role, "assistant");
        if let ChatContent::Parts { parts } = &sdk_msg.content {
            assert!(parts.iter().any(|p| matches!(p, ContentPart::Reasoning { .. })));
            assert!(parts.iter().any(|p| matches!(p, ContentPart::Text { .. })));
        } else {
            panic!("Expected Parts content with reasoning");
        }
    }

    #[test]
    fn test_assistant_image_rig_to_sdk() {
        let rig_msg = Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::Image(Image {
                data: DocumentSourceKind::url("https://example.com/gen.png"),
                media_type: None,
                detail: None,
                additional_params: None,
            })),
        };

        let sdk_msg = rig_message_to_chat(&rig_msg);
        assert_eq!(sdk_msg.role, "assistant");
        if let ChatContent::Parts { parts } = &sdk_msg.content {
            assert_eq!(parts.len(), 1);
            assert!(matches!(parts[0], ContentPart::Image { .. }));
        } else {
            panic!("Expected Parts content with image");
        }
    }

    #[test]
    fn test_full_tool_use_roundtrip() {
        let sdk_conversation = vec![
            ChatMessage::system("You are a weather assistant."),
            ChatMessage::user("What's the weather in Berlin?"),
            ChatMessage::assistant_with_tool_calls(
                "",
                vec![ToolCallData {
                    id: "call_1".into(),
                    name: "get_weather".into(),
                    arguments: json!({"city": "Berlin"}),
                }],
            ),
            ChatMessage::tool_result("call_1", "25°C, sunny"),
            ChatMessage::assistant("The weather in Berlin is 25°C and sunny."),
        ];

        let (preamble, rig_msgs) = chat_messages_to_rig(&sdk_conversation);
        assert_eq!(preamble.as_deref(), Some("You are a weather assistant."));
        assert_eq!(rig_msgs.len(), 4);

        // user message
        assert!(matches!(&rig_msgs[0], Message::User { .. }));
        // assistant with tool call
        if let Message::Assistant { content, .. } = &rig_msgs[1] {
            let has_tool_call = content
                .iter()
                .any(|ac| matches!(ac, AssistantContent::ToolCall(_)));
            assert!(has_tool_call, "Expected tool call in assistant message");
        } else {
            panic!("Expected assistant message");
        }
        // tool result (mapped to user with ToolResult)
        if let Message::User { content } = &rig_msgs[2] {
            assert!(matches!(content.first(), UserContent::ToolResult(_)));
        } else {
            panic!("Expected user message with tool result");
        }
        // final assistant
        assert!(matches!(&rig_msgs[3], Message::Assistant { .. }));
    }
}

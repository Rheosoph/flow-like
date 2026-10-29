//! A Rig completion request as the body of Vertex AI's `generateContent`. Text-only requests are
//! shaped as rig-vertexai 0.38.2 shaped them.

use std::collections::HashMap;

use futures::{StreamExt, TryStreamExt, stream};
use rig::OneOrMany;
use rig::completion::{CompletionError, CompletionRequest, Message, ToolDefinition};
use rig::message::{
    AssistantContent, DocumentSourceKind, MimeType, Reasoning, ToolChoice, ToolResult,
    ToolResultContent, UserContent,
};
use serde_json::{Map, Value, json};

use super::media::{Kind, LinkQuota, MediaItem, MediaResolver};
use super::wire::{
    Content, FunctionCall, FunctionResponse, FunctionResponsePart, GenerateContentRequest, Part,
};

const CONCURRENT_MEDIA: usize = 4;
const USER: &str = "user";
const MODEL: &str = "model";

pub async fn build(
    request: CompletionRequest,
    media: &MediaResolver,
) -> Result<GenerateContentRequest, CompletionError> {
    let CompletionRequest {
        preamble,
        chat_history,
        documents,
        tools,
        temperature,
        max_tokens,
        tool_choice,
        additional_params,
        output_schema,
        ..
    } = request;

    let mut system: Vec<String> = preamble
        .into_iter()
        .filter(|text| !text.is_empty())
        .collect();
    let mut messages = Vec::new();
    for message in chat_history {
        match message {
            Message::System { content } if !content.is_empty() => system.push(content),
            Message::System { .. } => {}
            message => messages.push(message),
        }
    }

    let mut plan = Plan::default();
    if !documents.is_empty() {
        let slots = documents
            .iter()
            .map(|document| Slot::Ready(Box::new(Part::text(document.to_string()))))
            .collect();
        plan.contents.push(Planned { role: USER, slots });
    }
    let tool_names = tool_names(&messages);
    for message in messages {
        match message {
            Message::User { content } => plan.user(content, &tool_names),
            Message::Assistant { content, .. } => plan.assistant(content)?,
            Message::System { .. } => {}
        }
    }
    let contents = plan.resolve(media).await?;
    if contents.is_empty() {
        return Err(CompletionError::RequestError(
            "Vertex AI needs at least one message with content".into(),
        ));
    }

    let params = Params::from(additional_params);
    let output_schema = output_schema
        .map(|schema| serde_json::to_value(&schema))
        .transpose()?;
    let tool_config = (!tools.is_empty()).then(|| tool_config(tool_choice.as_ref()));
    let mut tool_values: Vec<Value> = function_declarations(tools).into_iter().collect();
    tool_values.extend(params.tools);

    Ok(GenerateContentRequest {
        contents,
        system_instruction: (!system.is_empty()).then(|| Content {
            role: USER.to_string(),
            parts: vec![Part::text(system.join("\n\n"))],
        }),
        tools: tool_values,
        tool_config,
        generation_config: generation_config(
            params.generation,
            params.thinking,
            temperature,
            max_tokens,
            output_schema,
        ),
        extra: params.extra,
    })
}

/// The function name of each tool call, by the ids a tool result may refer to it with.
fn tool_names(messages: &[Message]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for message in messages {
        let Message::Assistant { content, .. } = message else {
            continue;
        };
        for item in content.iter() {
            if let AssistantContent::ToolCall(call) = item {
                for id in std::iter::once(&call.id).chain(call.call_id.as_ref()) {
                    names.insert(id.clone(), call.function.name.clone());
                }
            }
        }
    }
    names
}

enum Slot {
    Ready(Box<Part>),
    Media,
    ToolResult {
        name: String,
        outputs: Vec<String>,
        images: usize,
    },
}

struct Planned {
    role: &'static str,
    slots: Vec<Slot>,
}

/// The contents of a request with their media parts still to resolve, in the order the media
/// items were queued.
#[derive(Default)]
struct Plan {
    contents: Vec<Planned>,
    media: Vec<MediaItem>,
    quota: LinkQuota,
}

fn mime<T: MimeType>(media_type: Option<T>) -> Option<String> {
    media_type.map(|media_type| media_type.to_mime_type().to_string())
}

fn text_slot(text: String) -> Option<Slot> {
    (!text.is_empty()).then(|| Slot::Ready(Box::new(Part::text(text))))
}

impl Plan {
    fn queue(&mut self, kind: Kind, declared: Option<String>, source: DocumentSourceKind) {
        let may_link = self.quota.take(kind, &source);
        self.media.push(MediaItem {
            kind,
            declared,
            source,
            may_link,
        });
    }

    fn media(&mut self, kind: Kind, declared: Option<String>, source: DocumentSourceKind) -> Slot {
        self.queue(kind, declared, source);
        Slot::Media
    }

    fn user(&mut self, content: OneOrMany<UserContent>, tool_names: &HashMap<String, String>) {
        let slots = content
            .into_iter()
            .filter_map(|part| match part {
                UserContent::Text(text) => text_slot(text.text),
                UserContent::ToolResult(result) => Some(self.tool_result(result, tool_names)),
                UserContent::Image(image) => {
                    Some(self.media(Kind::Image, mime(image.media_type), image.data))
                }
                UserContent::Audio(audio) => {
                    Some(self.media(Kind::Audio, mime(audio.media_type), audio.data))
                }
                UserContent::Video(video) => {
                    Some(self.media(Kind::Video, mime(video.media_type), video.data))
                }
                UserContent::Document(document) => {
                    Some(self.media(Kind::Document, mime(document.media_type), document.data))
                }
            })
            .collect();
        self.contents.push(Planned { role: USER, slots });
    }

    fn tool_result(&mut self, result: ToolResult, tool_names: &HashMap<String, String>) -> Slot {
        let name = tool_names
            .get(&result.id)
            .or_else(|| {
                result
                    .call_id
                    .as_ref()
                    .and_then(|call_id| tool_names.get(call_id))
            })
            .cloned()
            .unwrap_or(result.id);
        let mut outputs = Vec::new();
        let mut images = 0;
        for item in result.content {
            match item {
                ToolResultContent::Text(text) => outputs.push(text.text),
                ToolResultContent::Image(image) => {
                    self.queue(Kind::Image, mime(image.media_type), image.data);
                    images += 1;
                }
            }
        }
        Slot::ToolResult {
            name,
            outputs,
            images,
        }
    }

    fn assistant(&mut self, content: OneOrMany<AssistantContent>) -> Result<(), CompletionError> {
        let mut slots = Vec::new();
        for part in content {
            let slot = match part {
                AssistantContent::Text(text) => text_slot(text.text),
                AssistantContent::ToolCall(call) => Some(Slot::Ready(Box::new(Part {
                    function_call: Some(FunctionCall {
                        args: arguments(&call.function.name, call.function.arguments)?,
                        name: call.function.name,
                    }),
                    thought_signature: call.signature,
                    ..Part::default()
                }))),
                AssistantContent::Reasoning(reasoning) => thought(&reasoning),
                AssistantContent::Image(image) => {
                    Some(self.media(Kind::Image, mime(image.media_type), image.data))
                }
            };
            slots.extend(slot);
        }
        self.contents.push(Planned { role: MODEL, slots });
        Ok(())
    }

    async fn resolve(self, media: &MediaResolver) -> Result<Vec<Content>, CompletionError> {
        let pending: Vec<_> = self.media.iter().map(|item| media.resolve(item)).collect();
        let resolved: Vec<Part> = stream::iter(pending)
            .buffered(CONCURRENT_MEDIA)
            .try_collect()
            .await?;
        let mut resolved = resolved.into_iter();
        let mut contents: Vec<Content> = Vec::new();
        for planned in self.contents {
            let mut parts = Vec::new();
            for slot in planned.slots {
                match slot {
                    Slot::Ready(part) => parts.push(*part),
                    Slot::Media => parts.extend(resolved.next()),
                    Slot::ToolResult {
                        name,
                        outputs,
                        images,
                    } => parts.push(function_response(
                        name,
                        outputs,
                        resolved.by_ref().take(images).collect(),
                    )),
                }
            }
            if parts.is_empty() {
                continue;
            }
            match contents.last_mut() {
                Some(last) if last.role == planned.role => last.parts.extend(parts),
                _ => contents.push(Content {
                    role: planned.role.to_string(),
                    parts,
                }),
            }
        }
        Ok(contents)
    }
}

/// Vertex AI takes function arguments as a JSON object.
fn arguments(name: &str, arguments: Value) -> Result<Value, CompletionError> {
    match arguments {
        Value::Object(_) => Ok(arguments),
        Value::Null => Ok(json!({})),
        Value::String(text) => match serde_json::from_str::<Value>(&text) {
            Ok(object @ Value::Object(_)) => Ok(object),
            _ => Err(non_object_arguments(name)),
        },
        _ => Err(non_object_arguments(name)),
    }
}

fn non_object_arguments(name: &str) -> CompletionError {
    CompletionError::RequestError(
        format!("Vertex AI takes tool call arguments as a JSON object, and the call to '{name}' has none")
            .into(),
    )
}

fn thought(reasoning: &Reasoning) -> Option<Slot> {
    let signature = reasoning.first_signature().map(str::to_owned);
    let text = reasoning.display_text();
    (!text.is_empty() || signature.is_some()).then(|| {
        Slot::Ready(Box::new(Part {
            text: Some(text),
            thought: true,
            thought_signature: signature,
            ..Part::default()
        }))
    })
}

/// A tool result: its text as `response.output`, rig-vertexai's shape, and its images as parts.
/// An image Vertex AI cannot read leaves its note in the output.
fn function_response(name: String, mut outputs: Vec<String>, images: Vec<Part>) -> Part {
    let mut parts = Vec::new();
    for image in images {
        match image {
            Part {
                inline_data: Some(blob),
                ..
            } => parts.push(FunctionResponsePart {
                inline_data: Some(blob),
                file_data: None,
            }),
            Part {
                file_data: Some(file),
                ..
            } => parts.push(FunctionResponsePart {
                inline_data: None,
                file_data: Some(file),
            }),
            Part {
                text: Some(note), ..
            } => outputs.push(note),
            _ => {}
        }
    }
    let output = match outputs.len() {
        0 => Value::String(String::new()),
        1 => Value::String(outputs.remove(0)),
        _ => Value::Array(outputs.into_iter().map(Value::String).collect()),
    };
    Part {
        function_response: Some(FunctionResponse {
            name,
            response: json!({ "output": output }),
            parts,
        }),
        ..Part::default()
    }
}

fn function_declarations(tools: Vec<ToolDefinition>) -> Option<Value> {
    if tools.is_empty() {
        return None;
    }
    let declarations: Vec<Value> = tools
        .into_iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parametersJsonSchema": tool.parameters,
            })
        })
        .collect();
    Some(json!({ "functionDeclarations": declarations }))
}

fn tool_config(choice: Option<&ToolChoice>) -> Value {
    let config = match choice {
        None | Some(ToolChoice::Auto) => json!({ "mode": "AUTO" }),
        Some(ToolChoice::None) => json!({ "mode": "NONE" }),
        Some(ToolChoice::Specific { function_names }) if !function_names.is_empty() => {
            json!({ "mode": "ANY", "allowedFunctionNames": function_names })
        }
        Some(ToolChoice::Required | ToolChoice::Specific { .. }) => json!({ "mode": "ANY" }),
    };
    json!({ "functionCallingConfig": config })
}

/// The caller's request parameters Vertex AI has a place for. The OpenAI-style History fields
/// (`stream`, `top_p`, `stop`, ...) have none here, as with rig-vertexai, and are left out.
#[derive(Default)]
struct Params {
    generation: Map<String, Value>,
    thinking: Option<Value>,
    tools: Vec<Value>,
    extra: Map<String, Value>,
}

impl From<Option<Value>> for Params {
    fn from(additional: Option<Value>) -> Self {
        let mut params = Self::default();
        let Some(Value::Object(object)) = additional else {
            return params;
        };
        let mut ignored = Vec::new();
        for (key, value) in object {
            match (key.as_str(), value) {
                ("generationConfig" | "generation_config", Value::Object(config)) => {
                    params.generation.extend(config)
                }
                ("thinkingConfig" | "thinking_config", thinking) => {
                    params.thinking = Some(thinking)
                }
                ("tools", Value::Array(tools)) => params.tools.extend(tools),
                ("safetySettings" | "safety_settings", value) => {
                    params.extra.insert("safetySettings".into(), value);
                }
                ("cachedContent" | "cached_content", value) => {
                    params.extra.insert("cachedContent".into(), value);
                }
                ("labels", value) => {
                    params.extra.insert("labels".into(), value);
                }
                _ => ignored.push(key),
            }
        }
        if !ignored.is_empty() {
            tracing::debug!(
                ?ignored,
                "Vertex AI has no such request parameters; leaving them out"
            );
        }
        params
    }
}

fn set(config: &mut Map<String, Value>, camel_case: &str, snake_case: &str, value: Value) {
    config.remove(snake_case);
    config.insert(camel_case.to_string(), value);
}

fn generation_config(
    mut config: Map<String, Value>,
    thinking: Option<Value>,
    temperature: Option<f64>,
    max_tokens: Option<u64>,
    output_schema: Option<Value>,
) -> Map<String, Value> {
    set(&mut config, "candidateCount", "candidate_count", json!(1));
    if let Some(temperature) = temperature {
        set(
            &mut config,
            "temperature",
            "temperature",
            json!(temperature),
        );
    }
    if let Some(max_tokens) = max_tokens {
        set(
            &mut config,
            "maxOutputTokens",
            "max_output_tokens",
            json!(max_tokens),
        );
    }
    if let Some(schema) = output_schema {
        set(
            &mut config,
            "responseMimeType",
            "response_mime_type",
            json!("application/json"),
        );
        set(
            &mut config,
            "responseJsonSchema",
            "response_json_schema",
            schema,
        );
    }
    if let Some(thinking) = thinking {
        set(&mut config, "thinkingConfig", "thinking_config", thinking);
    }
    config
}

#[cfg(test)]
mod tests {
    use super::super::media::fake::FakeNetwork;
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use rig::completion::Document;
    use rig::message::{Image, ImageMediaType, Text, ToolCall, ToolFunction};
    use std::sync::Arc;

    fn resolver() -> MediaResolver {
        MediaResolver::with_network(Arc::new(FakeNetwork::default()))
    }

    fn request(history: Vec<Message>) -> CompletionRequest {
        CompletionRequest {
            model: None,
            preamble: None,
            chat_history: OneOrMany::many(history).unwrap(),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        }
    }

    async fn body(request: CompletionRequest) -> Value {
        serde_json::to_value(build(request, &resolver()).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn text_requests_keep_the_rig_vertexai_shape() {
        let mut text = request(vec![
            Message::system("You are terse."),
            Message::user("Hi"),
            Message::assistant("Hello."),
            Message::system("Answer in English."),
            Message::user("Name a colour."),
        ]);
        text.temperature = Some(0.7);
        text.max_tokens = Some(256);
        text.additional_params = Some(json!({"stream": true, "top_p": 0.9, "user": "u-1"}));
        assert_eq!(
            body(text).await,
            json!({
                "contents": [
                    {"role": "user", "parts": [{"text": "Hi"}]},
                    {"role": "model", "parts": [{"text": "Hello."}]},
                    {"role": "user", "parts": [{"text": "Name a colour."}]},
                ],
                "systemInstruction": {
                    "role": "user",
                    "parts": [{"text": "You are terse.\n\nAnswer in English."}],
                },
                "generationConfig": {"candidateCount": 1, "temperature": 0.7, "maxOutputTokens": 256},
            })
        );
    }

    #[tokio::test]
    async fn tools_and_tool_turns_carry_names_arguments_and_signatures() {
        let call = ToolCall::new(
            "call_1".into(),
            ToolFunction::new("get_weather".into(), json!({"city": "Berlin"})),
        )
        .with_signature(Some("sig-1".into()));
        let mut tools = request(vec![
            Message::user("Weather in Berlin?"),
            Message::Assistant {
                id: None,
                content: OneOrMany::many(vec![
                    AssistantContent::Reasoning(Reasoning::new_with_signature(
                        "Look it up.",
                        Some("sig-0".into()),
                    )),
                    AssistantContent::ToolCall(call),
                ])
                .unwrap(),
            },
            Message::User {
                content: OneOrMany::many(vec![
                    UserContent::ToolResult(ToolResult {
                        id: "call_1".into(),
                        call_id: None,
                        content: OneOrMany::many(vec![
                            ToolResultContent::Text(Text::new("12°C".to_string())),
                            ToolResultContent::Image(Image {
                                data: DocumentSourceKind::Base64("aW1n".into()),
                                media_type: Some(ImageMediaType::PNG),
                                detail: None,
                                additional_params: None,
                            }),
                        ])
                        .unwrap(),
                    }),
                    UserContent::text(""),
                ])
                .unwrap(),
            },
        ]);
        tools.tools = vec![ToolDefinition {
            name: "get_weather".into(),
            description: "Weather for a city".into(),
            parameters: json!({"type": "object", "properties": {"city": {"type": "string"}}}),
        }];
        tools.tool_choice = Some(ToolChoice::Specific {
            function_names: vec!["get_weather".into()],
        });
        let body = body(tools).await;
        assert_eq!(
            body["tools"],
            json!([{"functionDeclarations": [{
                "name": "get_weather",
                "description": "Weather for a city",
                "parametersJsonSchema": {"type": "object", "properties": {"city": {"type": "string"}}},
            }]}])
        );
        assert_eq!(
            body["toolConfig"],
            json!({"functionCallingConfig": {"mode": "ANY", "allowedFunctionNames": ["get_weather"]}})
        );
        assert_eq!(
            body["contents"][1],
            json!({"role": "model", "parts": [
                {"text": "Look it up.", "thought": true, "thoughtSignature": "sig-0"},
                {"functionCall": {"name": "get_weather", "args": {"city": "Berlin"}}, "thoughtSignature": "sig-1"},
            ]})
        );
        assert_eq!(
            body["contents"][2],
            json!({"role": "user", "parts": [{"functionResponse": {
                "name": "get_weather",
                "response": {"output": "12°C"},
                "parts": [{"inlineData": {"mimeType": "image/png", "data": "aW1n"}}],
            }}]})
        );
        assert_eq!(body["contents"].as_array().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn tool_choices_map_to_function_calling_modes() {
        for (choice, expected) in [
            (None, json!({"mode": "AUTO"})),
            (Some(ToolChoice::Auto), json!({"mode": "AUTO"})),
            (Some(ToolChoice::None), json!({"mode": "NONE"})),
            (Some(ToolChoice::Required), json!({"mode": "ANY"})),
        ] {
            assert_eq!(
                tool_config(choice.as_ref()),
                json!({"functionCallingConfig": expected})
            );
        }
    }

    #[tokio::test]
    async fn parameters_documents_and_schemas_reach_their_fields() {
        let mut full = request(vec![Message::user("Go")]);
        full.documents = vec![Document {
            id: "doc-1".into(),
            text: "Context.".into(),
            additional_props: HashMap::new(),
        }];
        full.max_tokens = Some(64);
        full.additional_params = Some(json!({
            "generation_config": {"top_k": 5, "max_output_tokens": 9},
            "thinkingConfig": {"thinkingBudget": 1024, "includeThoughts": true},
            "safetySettings": [{"category": "HARM_CATEGORY_HARASSMENT", "threshold": "BLOCK_NONE"}],
            "tools": [{"googleSearch": {}}],
            "stream": false,
        }));
        full.output_schema = Some(
            serde_json::from_value(
                json!({"type": "object", "properties": {"a": {"type": "string"}}}),
            )
            .unwrap(),
        );
        let body = body(full).await;
        assert_eq!(
            body["contents"],
            json!([{"role": "user", "parts": [
                {"text": "<file id: doc-1>\nContext.\n</file>\n"},
                {"text": "Go"},
            ]}])
        );
        assert_eq!(
            body["generationConfig"],
            json!({
                "top_k": 5,
                "candidateCount": 1,
                "maxOutputTokens": 64,
                "responseMimeType": "application/json",
                "responseJsonSchema": {"type": "object", "properties": {"a": {"type": "string"}}},
                "thinkingConfig": {"thinkingBudget": 1024, "includeThoughts": true},
            })
        );
        assert_eq!(body["tools"], json!([{"googleSearch": {}}]));
        assert_eq!(body["safetySettings"][0]["threshold"], "BLOCK_NONE");
        assert!(body.get("stream").is_none() && body.get("toolConfig").is_none());
    }

    #[tokio::test]
    async fn user_media_resolves_in_place() {
        let png = STANDARD.encode(b"\x89PNG\r\n\x1a\n");
        let media = request(vec![Message::User {
            content: OneOrMany::many(vec![
                UserContent::text("Compare"),
                UserContent::image_base64(png.clone(), Some(ImageMediaType::PNG), None),
                UserContent::Video(rig::message::Video {
                    data: DocumentSourceKind::Url("gs://bucket/clip.mp4".into()),
                    media_type: None,
                    additional_params: None,
                }),
                UserContent::image_base64("R0lGODlh", Some(ImageMediaType::GIF), None),
            ])
            .unwrap(),
        }]);
        assert_eq!(
            body(media).await["contents"],
            json!([{"role": "user", "parts": [
                {"text": "Compare"},
                {"inlineData": {"mimeType": "image/png", "data": png}},
                {"fileData": {"mimeType": "video/mp4", "fileUri": "gs://bucket/clip.mp4"}},
                {"text": "[attachment omitted: unsupported file type image/gif]"},
            ]}])
        );
    }

    #[tokio::test]
    async fn requests_without_content_and_non_object_arguments_are_refused() {
        let empty = build(request(vec![Message::user("")]), &resolver()).await;
        assert!(empty.is_err());
        let call = ToolCall::new("f".into(), ToolFunction::new("f".into(), json!([1, 2])));
        let refused = build(
            request(vec![
                Message::user("Go"),
                Message::Assistant {
                    id: None,
                    content: OneOrMany::one(AssistantContent::ToolCall(call)),
                },
            ]),
            &resolver(),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(refused.contains("'f'"), "{refused}");
    }
}

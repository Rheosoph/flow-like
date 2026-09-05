use std::sync::Arc;

use flow_like::flow::{
    execution::{EventTrigger, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::{
    history::{Content, History, HistoryMessage, ImageUrl, MessageContent},
    response::Response,
    response_chunk::ResponseChunk,
};
use flow_like_types::{
    Cacheable, Value, anyhow, async_trait,
    intercom::InterComEvent,
    json::{from_str, json},
    sync::Mutex,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod attachment_from_path;
pub mod attachment_from_url;
pub mod attachment_to_url;
pub mod push_attachment;
pub mod push_attachments;
pub mod push_chunk;
pub mod push_global_session;
pub mod push_local_session;
pub mod push_reasoning;
pub mod push_response;
pub mod push_stat;
pub mod push_stats;
pub mod push_step;
pub mod push_text_to_step;
pub mod push_widget;
pub mod push_widgets;
pub mod remove_step;

pub use flow_like_catalog_data_support::events::chat_event::{
    Attachment, ComplexAttachment, url_processing,
};

#[crate::register_node]
#[derive(Default)]
pub struct ChatEventNode {}

impl ChatEventNode {
    pub fn new() -> Self {
        ChatEventNode {}
    }

    fn create_chat_history(messages: Vec<HistoryMessage>) -> History {
        let mut history = History::new("".to_string(), messages);
        // Rig's streaming response contract has no media event. Chat histories therefore
        // default to a non-stream request so generated images reach the UI; the invoke layer
        // still replays the completed response through the normal chunk callback.
        history.stream = Some(false);
        history
    }

    async fn process_history_messages(
        messages: Vec<HistoryMessage>,
        mut context: Option<&mut ExecutionContext>,
    ) -> Vec<HistoryMessage> {
        let mut processed = Vec::with_capacity(messages.len());

        for mut message in messages {
            if let MessageContent::Contents(contents) = &message.content {
                let mut processed_contents = Vec::new();

                for content in contents {
                    match content {
                        Content::Image {
                            content_type,
                            image_url,
                        } => {
                            let processed_url =
                                url_processing::process_url(&image_url.url, context.as_deref_mut())
                                    .await;
                            // Only include the image if URL processing succeeded (not empty)
                            if !processed_url.is_empty() {
                                processed_contents.push(Content::Image {
                                    content_type: content_type.clone(),
                                    image_url: ImageUrl {
                                        url: processed_url,
                                        detail: image_url.detail.clone(),
                                        media_type: image_url.media_type.clone(),
                                        additional_params: image_url.additional_params.clone(),
                                    },
                                });
                            }
                        }
                        Content::Audio {
                            content_type,
                            audio_url,
                            media_type,
                            additional_params,
                        } => {
                            let processed_url =
                                url_processing::process_url(audio_url, context.as_deref_mut())
                                    .await;
                            if !processed_url.is_empty() {
                                processed_contents.push(Content::Audio {
                                    content_type: content_type.clone(),
                                    audio_url: processed_url,
                                    media_type: media_type.clone(),
                                    additional_params: additional_params.clone(),
                                });
                            }
                        }
                        Content::Video {
                            content_type,
                            video_url,
                            media_type,
                            additional_params,
                        } => {
                            let processed_url =
                                url_processing::process_url(video_url, context.as_deref_mut())
                                    .await;
                            if !processed_url.is_empty() {
                                processed_contents.push(Content::Video {
                                    content_type: content_type.clone(),
                                    video_url: processed_url,
                                    media_type: media_type.clone(),
                                    additional_params: additional_params.clone(),
                                });
                            }
                        }
                        Content::Document {
                            content_type,
                            document_url,
                            media_type,
                            additional_params,
                        } => {
                            let processed_url =
                                url_processing::process_url(document_url, context.as_deref_mut())
                                    .await;
                            if !processed_url.is_empty() {
                                processed_contents.push(Content::Document {
                                    content_type: content_type.clone(),
                                    document_url: processed_url,
                                    media_type: media_type.clone(),
                                    additional_params: additional_params.clone(),
                                });
                            }
                        }
                        other => processed_contents.push(other.clone()),
                    }
                }

                message.content = MessageContent::Contents(processed_contents);
            }

            processed.push(message);
        }

        processed
    }
}

#[async_trait]
impl NodeLogic for ChatEventNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new("events_chat", "Chat Event", "A simple Chat event", "Events");
        node.set_flowscript_name("events", "chat");
        node.add_icon("/flow/icons/event.svg");
        node.set_version(1);
        node.set_start(true);
        node.set_can_be_referenced_by_fns(true);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Starting an event",
            VariableType::Execution,
        );

        node.add_output_pin("history", "History", "Chat History", VariableType::Struct)
            .set_schema::<History>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "local_session",
            "Local Session",
            "Local to the Chat",
            VariableType::Struct,
        )
        .set_open_schema();

        node.add_output_pin(
            "global_session",
            "Global Session",
            "Global to the User",
            VariableType::Struct,
        )
        .set_open_schema();

        node.add_output_pin(
            "tools",
            "Tools",
            "Tools requested by the user",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin("actions", "Actions", "User Actions", VariableType::Struct)
            .set_schema::<ChatAction>()
            .set_value_type(flow_like::flow::pin::ValueType::Array)
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "attachments",
            "Attachments",
            "User Attachments or References",
            VariableType::Struct,
        )
        .set_schema::<Attachment>()
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("user", "User", "User Information", VariableType::Struct)
            .set_schema::<User>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let exec_out_pin = context.get_pin_by_name("exec_out").await?;

        if context.delegated {
            context.activate_exec_pin_ref(&exec_out_pin).await?;
            return Ok(());
        }

        let payload = context.get_payload().await?;
        let chat = payload
            .payload
            .clone()
            .ok_or(anyhow!("Failed to get payload"))?;
        let chat: Chat = flow_like_types::json::from_value(chat)
            .map_err(|e| anyhow!("Failed to deserialize payload: {}", e))?;

        // Process attachments to convert Tauri URLs to data URLs
        let processed_attachments = if let Some(attachments) = chat.attachments {
            Attachment::process_vec(attachments, Some(context)).await
        } else {
            vec![]
        };

        // Process history messages to convert Tauri URLs in image_url fields to data URLs
        let processed_messages = Self::process_history_messages(chat.messages, Some(context)).await;

        context
            .set_pin_value(
                "history",
                json!(Self::create_chat_history(processed_messages)),
            )
            .await?;
        context
            .set_pin_value(
                "local_session",
                chat.local_session.unwrap_or(from_str("{}")?),
            )
            .await?;
        context
            .set_pin_value(
                "global_session",
                chat.global_session.unwrap_or(from_str("{}")?),
            )
            .await?;
        context
            .set_pin_value("tools", json!(chat.tools.unwrap_or_default()))
            .await?;
        context
            .set_pin_value("actions", json!(chat.actions.unwrap_or_default()))
            .await?;
        context
            .set_pin_value("attachments", json!(processed_attachments))
            .await?;
        context
            .set_pin_value("user", json!(chat.user.unwrap_or_default()))
            .await?;
        context.activate_exec_pin_ref(&exec_out_pin).await?;

        let completion_event: EventTrigger = Arc::new(|run| {
            Box::pin(async move {
                if let Some(cached_response) = run.cache.read().await.get("chat_response") {
                    let cached_response = cached_response.clone();
                    let response = cached_response
                        .as_any()
                        .downcast_ref::<CachedChatResponse>()
                        .ok_or(anyhow!("Failed to downcast cached response"))?;

                    let event = {
                        let response = response.response.lock().await;
                        InterComEvent::with_type("chat_out", response.clone())
                    };
                    event.call(&run.callback).await?;
                }
                Ok(())
            })
        });

        context.hook_completion_event(completion_event).await;

        return Ok(());
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub enum ButtonType {
    Outline,
    Primary,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub enum ChatAction {
    Button(String, ButtonType),
    Form(String, Value),
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
pub struct User {
    pub sub: String,
    pub name: String,
    pub bot: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Chat {
    pub chat_id: Option<String>,
    pub messages: Vec<HistoryMessage>,
    pub local_session: Option<Value>,
    pub global_session: Option<Value>,
    pub actions: Option<Vec<ChatAction>>,
    pub tools: Option<Vec<String>>,
    pub user: Option<User>,
    pub attachments: Option<Vec<Attachment>>,
}

/// An a2ui widget instance embedded inside a chat message. `component` is the
/// self-contained `widgetInstance` component (with `inlineWidgetDef` and
/// `actionBindings`) produced by the Instantiate Widget node, so the chat can
/// render it without touching the a2ui surface channel.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ChatWidget {
    pub instance_id: String,
    pub widget_id: String,
    pub surface_id: String,
    pub component: Value,
    /// Ordered a2ui update messages targeting this widget that were streamed
    /// earlier in the run (before the push). The frontend replays them over the
    /// snapshot so element nodes (Set Element Value, Update GeoMap, Push CSV To
    /// Chart, …) work in the same run that pushes the widget.
    #[serde(default)]
    pub updates: Vec<Value>,
}

impl ChatWidget {
    /// Build a `ChatWidget` from an `element_ref` produced by the Instantiate
    /// Widget node. The ref carries the self-contained `widgetInstance` component
    /// under `component`, which the chat renders directly.
    pub fn from_element_ref(value: &Value) -> flow_like_types::Result<Self> {
        let instance_id = value
            .get("instanceId")
            .or_else(|| value.get("id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Widget reference is missing 'instanceId'"))?
            .to_string();

        let widget_id = value
            .get("widgetId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let surface_id = value
            .get("surfaceId")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| instance_id.clone());

        let component = value.get("component").cloned().ok_or_else(|| {
            anyhow!(
                "Widget reference is missing 'component'. Re-add the Instantiate Widget node (requires version 4 or newer)."
            )
        })?;

        Ok(ChatWidget {
            instance_id,
            widget_id,
            surface_id,
            component,
            updates: vec![],
        })
    }

    fn inline_child_ids(&self) -> Vec<String> {
        Self::component_inline_child_ids(&self.component)
    }

    fn component_inline_child_ids(component: &Value) -> Vec<String> {
        component
            .get("inlineWidgetDef")
            .and_then(|def| def.get("components"))
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| c.get("id").and_then(|id| id.as_str()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Walks a component tree for `{path, defaultValue?}` binding objects and
    /// collects their data-model paths.
    fn collect_bound_paths(value: &Value, paths: &mut std::collections::BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                if let Some(path) = map.get("path").and_then(|p| p.as_str())
                    && map.keys().all(|k| k == "path" || k == "defaultValue")
                {
                    paths.insert(path.to_string());
                }
                for v in map.values() {
                    Self::collect_bound_paths(v, paths);
                }
            }
            Value::Array(arr) => {
                for v in arr {
                    Self::collect_bound_paths(v, paths);
                }
            }
            _ => {}
        }
    }

    /// Segment-aware match: a data update touching `updated` is relevant to a
    /// binding on `bound` when either path is the other or one of its parents.
    fn paths_overlap(updated: &str, bound: &str) -> bool {
        updated == bound
            || bound
                .strip_prefix(updated)
                .is_some_and(|rest| rest.starts_with('/'))
            || updated
                .strip_prefix(bound)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    /// Collects the run's a2ui updates that target this widget so the frontend
    /// can replay them over the pushed snapshot. All matching entries are kept
    /// — including full re-registrations of the instance — because the replay
    /// must end at exactly the state the emission order produces.
    ///
    /// Relevance is a fixpoint, not a single instance check: pushing another
    /// widget instance into a container inside this one makes that instance
    /// (and its own children and pushes) part of this widget's replay, so
    /// `pushChild`/`insertChildAt` references pull the referenced instances
    /// into the kept set transitively. Data-model updates are matched by path
    /// against the widget's surviving bindings — `Data Update` targets a
    /// board-chosen surface id that never equals the chat instance id — and
    /// are rewritten to this widget's surface id so the frontend reducer
    /// applies them to the chat surface.
    pub fn attach_update_log(&mut self, log: &[flow_like::a2ui::A2UIServerMessage]) {
        use flow_like::a2ui::A2UIServerMessage as Msg;
        use std::collections::BTreeSet;

        let mut instance_ids: BTreeSet<String> = BTreeSet::new();
        instance_ids.insert(self.instance_id.clone());
        let mut child_ids: BTreeSet<String> = self.inline_child_ids().into_iter().collect();
        let mut bound_paths = BTreeSet::new();
        Self::collect_bound_paths(&self.component, &mut bound_paths);

        let element_matches =
            |element_id: &str, instance_ids: &BTreeSet<String>, child_ids: &BTreeSet<String>| {
                if let Some((scope, _)) = element_id.split_once('/') {
                    return instance_ids.contains(scope);
                }
                instance_ids.contains(element_id) || {
                    let suffix = format!("-{element_id}");
                    child_ids
                        .iter()
                        .any(|c| c == element_id || c.ends_with(&suffix))
                }
            };

        loop {
            let mut changed = false;
            for message in log {
                let Msg::UpsertElement { element_id, value } = message else {
                    continue;
                };
                if !element_matches(element_id, &instance_ids, &child_ids) {
                    continue;
                }
                match value.get("type").and_then(|t| t.as_str()) {
                    Some("pushChild") | Some("insertChildAt") => {
                        if let Some(child) = value.get("childId").and_then(|c| c.as_str())
                            && !child_ids.contains(child)
                        {
                            changed |= instance_ids.insert(child.to_string());
                        }
                    }
                    Some("createComponent") if instance_ids.contains(element_id) => {
                        if let Some(component) = value.get("component") {
                            for id in Self::component_inline_child_ids(component) {
                                changed |= child_ids.insert(id);
                            }
                            Self::collect_bound_paths(component, &mut bound_paths);
                        }
                    }
                    _ => {}
                }
            }
            if !changed {
                break;
            }
        }

        for message in log {
            let kept = match message {
                Msg::UpsertElement { element_id, .. } => {
                    element_matches(element_id, &instance_ids, &child_ids).then(|| message.clone())
                }
                Msg::CreateElement { surface_id, .. } | Msg::RemoveElement { surface_id, .. } => {
                    (surface_id == &self.surface_id || instance_ids.contains(surface_id))
                        .then(|| message.clone())
                }
                Msg::DataModelUpdate {
                    surface_id,
                    path,
                    contents,
                } => {
                    let targeted =
                        surface_id == &self.surface_id || instance_ids.contains(surface_id);
                    let bound = !targeted
                        && (path.as_deref().is_some_and(|p| {
                            bound_paths.iter().any(|b| Self::paths_overlap(p, b))
                        }) || (path.is_none()
                            && contents.iter().any(|entry| {
                                bound_paths
                                    .iter()
                                    .any(|b| Self::paths_overlap(&entry.key, b))
                            })));
                    (targeted || bound).then(|| Msg::DataModelUpdate {
                        surface_id: self.surface_id.clone(),
                        path: path.clone(),
                        contents: contents.clone(),
                    })
                }
                _ => None,
            };

            if let Some(message) = kept
                && let Ok(value) = flow_like_types::json::to_value(&message)
            {
                self.updates.push(value);
            }
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ChatResponse {
    pub response: Response,
    pub local_session: Option<Value>,
    pub global_session: Option<Value>,
    pub actions: Vec<ChatAction>,
    pub attachments: Vec<Attachment>,
    pub model_id: Option<String>,
    #[serde(default)]
    pub widgets: Vec<ChatWidget>,
}

#[derive(Clone)]
pub struct CachedChatResponse {
    response: Arc<Mutex<ChatResponse>>,
    reasoning: Arc<Mutex<Reasoning>>,
}

impl CachedChatResponse {
    pub async fn load(context: &mut ExecutionContext) -> flow_like_types::Result<Self> {
        if let Some(cached_response) = context.get_cache("chat_response").await {
            let response = cached_response
                .as_any()
                .downcast_ref::<CachedChatResponse>()
                .ok_or(anyhow!("Failed to downcast cached response"))?;
            return Ok(response.clone());
        }

        let response = ChatResponse {
            response: Response::new(),
            actions: vec![],
            attachments: vec![],
            widgets: vec![],
            global_session: flow_like_types::json::from_str("{}")?,
            local_session: flow_like_types::json::from_str("{}")?,
            model_id: None,
        };

        let reasoning = Reasoning {
            current_message: "".to_string(),
            current_step: 0,
            plan: vec![],
        };

        let cached_response = CachedChatResponse {
            response: Arc::new(Mutex::new(response)),
            reasoning: Arc::new(Mutex::new(reasoning)),
        };

        let cacheable = Arc::new(cached_response.clone()) as Arc<dyn Cacheable>;
        context.set_cache("chat_response", cacheable).await;
        Ok(cached_response)
    }
}

impl Cacheable for CachedChatResponse {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Reasoning {
    pub plan: Vec<(u32, String)>,
    pub current_step: u32,
    pub current_message: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ChatStreamingResponse {
    pub chunk: Option<ResponseChunk>,
    pub actions: Vec<ChatAction>,
    pub attachments: Vec<Attachment>,
    pub plan: Option<Reasoning>,
    #[serde(default)]
    pub widgets: Vec<ChatWidget>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ChatUsageStat {
    pub step_name: String,
    pub stats: flow_like_model_provider::response::LLMUsageStats,
}

#[cfg(test)]
mod tests {
    use super::url_processing::*;
    use super::*;

    #[test]
    fn test_is_remote_url() {
        // Valid remote URLs
        assert!(is_remote_url("https://example.com/file.png"));
        assert!(is_remote_url("https://s3.amazonaws.com/bucket/file.pdf"));
        assert!(is_remote_url("http://example.com/image.jpg"));

        // Tauri asset URLs should not be considered remote
        assert!(!is_remote_url("http://asset.localhost/path/to/file.png"));
        assert!(!is_remote_url("asset://localhost/file.png"));

        // Data URLs should not be considered remote
        assert!(!is_remote_url("data:image/png;base64,iVBORw0KG..."));
    }

    #[test]
    fn test_is_tauri_asset_url() {
        // Valid Tauri asset URLs
        assert!(is_tauri_asset_url("asset://localhost/chat/file.png"));
        assert!(is_tauri_asset_url("http://asset.localhost/storage/doc.pdf"));

        // Non-Tauri URLs
        assert!(!is_tauri_asset_url("https://example.com/file.png"));
        assert!(!is_tauri_asset_url("http://example.com/file.png"));
        assert!(!is_tauri_asset_url("data:image/png;base64,iVBORw0KG..."));
    }

    #[tokio::test]
    async fn process_history_keeps_every_remote_media_type() {
        let message = HistoryMessage {
            role: flow_like_model_provider::history::Role::User,
            content: MessageContent::Contents(vec![
                Content::Audio {
                    content_type: flow_like_model_provider::history::ContentType::AudioUrl,
                    audio_url: "https://example.com/input.mp3".to_string(),
                    media_type: Some("audio/mpeg".to_string()),
                    additional_params: None,
                },
                Content::Video {
                    content_type: flow_like_model_provider::history::ContentType::VideoUrl,
                    video_url: "https://example.com/input.mp4".to_string(),
                    media_type: Some("video/mp4".to_string()),
                    additional_params: None,
                },
                Content::Document {
                    content_type: flow_like_model_provider::history::ContentType::DocumentUrl,
                    document_url: "https://example.com/input.pdf".to_string(),
                    media_type: Some("application/pdf".to_string()),
                    additional_params: None,
                },
            ]),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            annotations: None,
        };

        let processed = ChatEventNode::process_history_messages(vec![message], None).await;
        let MessageContent::Contents(parts) = &processed[0].content else {
            panic!("expected content parts")
        };
        assert_eq!(parts.len(), 3);
        assert!(matches!(
            &parts[0],
            Content::Audio { audio_url, .. } if audio_url.ends_with("input.mp3")
        ));
        assert!(matches!(
            &parts[1],
            Content::Video { video_url, .. } if video_url.ends_with("input.mp4")
        ));
        assert!(matches!(
            &parts[2],
            Content::Document { document_url, .. } if document_url.ends_with("input.pdf")
        ));
    }

    #[test]
    fn chat_history_defaults_to_media_safe_non_streaming() {
        let history = ChatEventNode::create_chat_history(Vec::new());
        assert_eq!(history.stream, Some(false));
    }

    #[test]
    fn test_is_blake3_hash() {
        // Valid Blake3 hash (64 hex characters)
        assert!(is_blake3_hash(
            "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadce"
        ));
        assert!(is_blake3_hash(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(is_blake3_hash(
            "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789"
        ));

        // Invalid: too short
        assert!(!is_blake3_hash("3d65ddd83e92b1e3fffee47d8e209802"));
        assert!(!is_blake3_hash("abc123"));

        // Invalid: too long
        assert!(!is_blake3_hash(
            "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadce00"
        ));

        // Invalid: non-hex characters
        assert!(!is_blake3_hash(
            "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadcg"
        ));
        assert!(!is_blake3_hash(
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
        ));

        // Invalid: path traversal attempts
        assert!(!is_blake3_hash("../etc/passwd"));
        assert!(!is_blake3_hash("../../sensitive_file"));

        // Invalid: special characters
        assert!(!is_blake3_hash("file@name"));
        assert!(!is_blake3_hash("my-file-name"));

        // Empty string
        assert!(!is_blake3_hash(""));
    }

    #[test]
    fn test_extract_tauri_path_valid_blake3() {
        let valid_hash = "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadce";

        // Test asset:// URL
        let url = format!("asset://localhost/chat/{}.png", valid_hash);
        let result = extract_tauri_path(&url);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert_eq!(path.file_stem().unwrap().to_string_lossy(), valid_hash);

        // Test http://asset.localhost/ URL
        let url = format!("http://asset.localhost/storage/{}.pdf", valid_hash);
        let result = extract_tauri_path(&url);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert_eq!(path.file_stem().unwrap().to_string_lossy(), valid_hash);
    }

    #[test]
    fn test_extract_tauri_path_invalid_hash() {
        // Non-Blake3 hash filenames should fail
        let invalid_urls = vec![
            "asset://localhost/chat/myfile.png",
            "http://asset.localhost/storage/document.pdf",
            "asset://localhost/../etc/passwd",
            "http://asset.localhost/../../sensitive.txt",
        ];

        for url in invalid_urls {
            let result = extract_tauri_path(url);
            assert!(result.is_err(), "Expected error for URL: {}", url);
            let err = result.unwrap_err();
            assert!(
                err.to_string().contains("Security") || err.to_string().contains("Blake3"),
                "Error should mention security or Blake3 validation, got: {}",
                err
            );
        }
    }

    #[test]
    fn test_extract_tauri_path_url_encoded() {
        let valid_hash = "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadce";

        // Test URL-encoded path
        let url = format!(
            "http://asset.localhost/path%20with%20spaces/{}.png",
            valid_hash
        );
        let result = extract_tauri_path(&url);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.to_string_lossy().contains("path with spaces"));
        assert_eq!(path.file_stem().unwrap().to_string_lossy(), valid_hash);
    }

    #[test]
    fn test_has_safe_path_components() {
        use std::path::Path;

        // Safe paths - normal directory structures
        assert!(has_safe_path_components(Path::new("chat/file.png")).is_ok());
        assert!(has_safe_path_components(Path::new("storage/documents/file.pdf")).is_ok());
        assert!(has_safe_path_components(Path::new("media/videos/file.mp4")).is_ok());
        assert!(has_safe_path_components(Path::new("file.txt")).is_ok());

        // Hidden files should be rejected
        assert!(has_safe_path_components(Path::new(".hidden/file.txt")).is_err());
        assert!(has_safe_path_components(Path::new("chat/.config")).is_err());
        assert!(has_safe_path_components(Path::new(".ssh/id_rsa")).is_err());

        // Absolute paths are allowed (Tauri always uses absolute paths)
        #[cfg(unix)]
        {
            assert!(
                has_safe_path_components(Path::new("/Users/felix/Library/Caches/file.txt")).is_ok()
            );
            assert!(has_safe_path_components(Path::new("/tmp/file.log")).is_ok());
        }

        #[cfg(windows)]
        {
            assert!(
                has_safe_path_components(Path::new("C:\\Users\\felix\\AppData\\file.txt")).is_ok()
            );
        }
    }

    #[test]
    fn test_has_safe_path_components_traversal() {
        use std::path::Path;

        // Path traversal attempts should fail
        assert!(has_safe_path_components(Path::new("chat/../sensitive.txt")).is_err());
        assert!(has_safe_path_components(Path::new("../etc/passwd")).is_err());
        assert!(has_safe_path_components(Path::new("../../root/.ssh")).is_err());
        assert!(has_safe_path_components(Path::new("dir1/../dir2/../../../etc")).is_err());

        // Current directory references
        assert!(has_safe_path_components(Path::new("./file.txt")).is_err());
        assert!(has_safe_path_components(Path::new("chat/./file.txt")).is_err());
    }

    #[test]
    fn test_extract_tauri_path_with_safe_paths() {
        let valid_hash = "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadce";

        // Test various safe path structures with any extension
        let safe_urls = vec![
            format!("asset://localhost/chat/{}.png", valid_hash),
            format!("http://asset.localhost/storage/{}.pdf", valid_hash),
            format!("asset://localhost/media/videos/{}.mp4", valid_hash),
            format!("http://asset.localhost/documents/{}.docx", valid_hash),
            format!("asset://localhost/archives/{}.zip", valid_hash),
            format!("http://asset.localhost/{}.xyz", valid_hash), // Any extension works
        ];

        for url in safe_urls {
            let result = extract_tauri_path(&url);
            assert!(result.is_ok(), "Expected success for safe URL: {}", url);
            let path = result.unwrap();
            assert_eq!(path.file_stem().unwrap().to_string_lossy(), valid_hash);
        }
    }

    #[test]
    fn test_extract_tauri_path_path_traversal() {
        let valid_hash = "3d65ddd83e92b1e3fffee47d8e209802d64e8cf74241b9e6355aa19b9f3dadce";

        // Test path traversal attempts - even with valid hash, should fail on path components
        let traversal_urls = vec![
            format!("asset://localhost/../etc/{}.txt", valid_hash),
            format!(
                "http://asset.localhost/chat/../../sensitive/{}.pdf",
                valid_hash
            ),
            format!("asset://localhost/.ssh/{}.key", valid_hash),
            format!("http://asset.localhost/.config/{}.conf", valid_hash),
        ];

        for url in traversal_urls {
            let result = extract_tauri_path(&url);
            assert!(result.is_err(), "Expected error for traversal URL: {}", url);
            let err = result.unwrap_err();
            assert!(
                err.to_string().contains("Security"),
                "Error should mention security, got: {}",
                err
            );
        }
    }

    #[tokio::test]
    async fn test_process_url_remote_https() {
        let url = "https://example.com/file.png";
        let processed = process_url(url, None).await;
        // Remote HTTPS URLs should be returned unchanged
        assert_eq!(processed, url);
    }

    #[tokio::test]
    async fn test_process_url_data_url() {
        let url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
        let processed = process_url(url, None).await;
        // Data URLs should be returned unchanged
        assert_eq!(processed, url);
    }

    #[tokio::test]
    async fn test_process_url_invalid_tauri_hash() {
        // File with non-Blake3 hash should be rejected and return empty string
        let url = "asset://localhost/chat/myfile.png";
        let processed = process_url(url, None).await;
        // Should return empty string due to security validation failure
        assert_eq!(processed, "");
    }

    #[tokio::test]
    async fn test_attachment_process_url() {
        let url = "https://example.com/file.png";
        let attachment = Attachment::Url(url.to_string());
        let processed = attachment.process(None).await;

        assert!(processed.is_some());
        match processed.unwrap() {
            Attachment::Url(processed_url) => {
                assert_eq!(processed_url, url);
            }
            _ => panic!("Expected Url variant"),
        }
    }

    #[tokio::test]
    async fn test_attachment_process_invalid_url() {
        // Invalid Tauri URL should be filtered out
        let url = "asset://localhost/chat/invalid.png";
        let attachment = Attachment::Url(url.to_string());
        let processed = attachment.process(None).await;

        assert!(
            processed.is_none(),
            "Invalid Tauri URL should be filtered out"
        );
    }

    #[tokio::test]
    async fn test_attachment_process_complex() {
        let complex = ComplexAttachment {
            url: "https://example.com/file.pdf".to_string(),
            preview_text: Some("Preview".to_string()),
            thumbnail_url: Some("https://example.com/thumb.jpg".to_string()),
            name: Some("document.pdf".to_string()),
            size: Some(1024),
            r#type: Some("application/pdf".to_string()),
            anchor: None,
            page: None,
        };

        let attachment = Attachment::Complex(complex.clone());
        let processed = attachment.process(None).await;

        assert!(processed.is_some());
        match processed.unwrap() {
            Attachment::Complex(processed_complex) => {
                assert_eq!(processed_complex.url, complex.url);
                assert_eq!(processed_complex.thumbnail_url, complex.thumbnail_url);
                assert_eq!(processed_complex.name, complex.name);
            }
            _ => panic!("Expected Complex variant"),
        }
    }

    #[tokio::test]
    async fn test_attachment_process_vec() {
        let attachments = vec![
            Attachment::Url("https://example.com/file1.png".to_string()),
            Attachment::Url("https://example.com/file2.jpg".to_string()),
            Attachment::Complex(ComplexAttachment {
                url: "https://example.com/doc.pdf".to_string(),
                preview_text: None,
                thumbnail_url: Some("https://example.com/thumb.jpg".to_string()),
                name: Some("document.pdf".to_string()),
                size: Some(2048),
                r#type: Some("application/pdf".to_string()),
                anchor: None,
                page: None,
            }),
        ];

        let processed = Attachment::process_vec(attachments.clone(), None).await;

        assert_eq!(processed.len(), 3);

        // Verify URLs are processed (in this case, remote URLs stay unchanged)
        match &processed[0] {
            Attachment::Url(url) => assert!(url.starts_with("https://")),
            _ => panic!("Expected Url variant"),
        }

        match &processed[2] {
            Attachment::Complex(complex) => {
                assert!(complex.url.starts_with("https://"));
                assert!(
                    complex
                        .thumbnail_url
                        .as_ref()
                        .unwrap()
                        .starts_with("https://")
                );
            }
            _ => panic!("Expected Complex variant"),
        }
    }

    #[tokio::test]
    async fn test_attachment_process_vec_filters_invalid() {
        // Test that invalid Tauri URLs are filtered out from the vec
        let attachments = vec![
            Attachment::Url("https://example.com/valid.png".to_string()),
            Attachment::Url("asset://localhost/chat/invalid.png".to_string()), // Should be filtered
            Attachment::Url("https://example.com/another-valid.jpg".to_string()),
        ];

        let processed = Attachment::process_vec(attachments, None).await;

        // Should only have 2 attachments (the 2 valid HTTPS URLs)
        assert_eq!(processed.len(), 2);

        // All remaining should be valid HTTPS URLs
        for attachment in processed {
            if let Attachment::Url(url) = attachment {
                assert!(url.starts_with("https://"))
            }
        }
    }

    #[tokio::test]
    async fn test_complex_attachment_process() {
        let complex = ComplexAttachment {
            url: "https://s3.amazonaws.com/bucket/file.pdf".to_string(),
            preview_text: Some("A preview".to_string()),
            thumbnail_url: Some("https://s3.amazonaws.com/bucket/thumb.jpg".to_string()),
            name: Some("report.pdf".to_string()),
            size: Some(4096),
            r#type: Some("application/pdf".to_string()),
            anchor: Some("#section-2".to_string()),
            page: Some(3),
        };

        let processed = complex.process(None).await;

        assert!(processed.is_some());
        let processed = processed.unwrap();

        // Remote URLs should remain unchanged
        assert_eq!(processed.url, complex.url);
        assert_eq!(processed.thumbnail_url, complex.thumbnail_url);

        // Other fields should be preserved
        assert_eq!(processed.preview_text, complex.preview_text);
        assert_eq!(processed.name, complex.name);
        assert_eq!(processed.size, complex.size);
        assert_eq!(processed.r#type, complex.r#type);
        assert_eq!(processed.anchor, complex.anchor);
        assert_eq!(processed.page, complex.page);
    }

    #[test]
    fn test_blake3_hash_edge_cases() {
        // Test case sensitivity (both should be valid)
        let lowercase = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let uppercase = "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789";
        let mixed = "aBcDeF0123456789aBcDeF0123456789aBcDeF0123456789aBcDeF0123456789";

        assert!(is_blake3_hash(lowercase));
        assert!(is_blake3_hash(uppercase));
        assert!(is_blake3_hash(mixed));

        // All zeros and all f's should be valid
        let all_zeros = "0000000000000000000000000000000000000000000000000000000000000000";
        let all_fs = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

        assert!(is_blake3_hash(all_zeros));
        assert!(is_blake3_hash(all_fs));
    }

    #[test]
    fn attach_update_log_follows_nested_widgets_and_bound_paths() {
        use flow_like::a2ui::A2UIServerMessage as Msg;

        let mut widget = ChatWidget {
            instance_id: "a".to_string(),
            widget_id: "w1".to_string(),
            surface_id: "a".to_string(),
            component: flow_like_types::json::json!({
                "type": "widgetInstance",
                "inlineWidgetDef": {
                    "components": [
                        { "id": "container" },
                        { "id": "label", "component": { "text": { "path": "sales", "defaultValue": "0" } } }
                    ]
                }
            }),
            updates: vec![],
        };

        let nested_component = flow_like_types::json::json!({
            "type": "widgetInstance",
            "inlineWidgetDef": { "components": [{ "id": "badge" }] }
        });
        let log = vec![
            Msg::UpsertElement {
                element_id: "a".to_string(),
                value: flow_like_types::json::json!({ "type": "createComponent", "component": widget.component.clone() }),
            },
            Msg::UpsertElement {
                element_id: "b".to_string(),
                value: flow_like_types::json::json!({ "type": "createComponent", "component": nested_component }),
            },
            Msg::UpsertElement {
                element_id: "a/container".to_string(),
                value: flow_like_types::json::json!({ "type": "pushChild", "childId": "b" }),
            },
            Msg::DataModelUpdate {
                surface_id: "main".to_string(),
                path: Some("sales".to_string()),
                contents: vec![],
            },
            Msg::DataModelUpdate {
                surface_id: "main".to_string(),
                path: Some("unrelated".to_string()),
                contents: vec![],
            },
            Msg::UpsertElement {
                element_id: "other-widget".to_string(),
                value: flow_like_types::json::json!({ "type": "createComponent", "component": {} }),
            },
        ];

        widget.attach_update_log(&log);

        let kept: Vec<(&str, &str)> = widget
            .updates
            .iter()
            .map(|value| {
                let kind = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
                let target = value
                    .get("element_id")
                    .or_else(|| value.get("surface_id"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                (kind, target)
            })
            .collect();

        assert_eq!(
            kept,
            vec![
                ("upsertElement", "a"),
                // Pushed nested instance travels with the widget (fixpoint over pushChild refs).
                ("upsertElement", "b"),
                ("upsertElement", "a/container"),
                // Bound-path data update is kept and re-addressed to the widget surface.
                ("dataModelUpdate", "a"),
            ]
        );
    }
}

//! Folds the Bot Framework activities of one Copilot Studio turn into node outputs, and streams
//! partial text to the nodes connected to `on_stream`.

use super::MODEL_NAME;
use crate::events::chat_event::{Attachment, ComplexAttachment};
use ahash::{AHashMap, AHashSet};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
    node::Node,
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_model_provider::{
    response::{Annotation, Response},
    response_chunk::ResponseChunk,
};
use flow_like_types::{
    JsonSchema, Value,
    json::{self, json},
};
use serde::{Deserialize, Serialize};

/// A Bot Framework activity as exchanged with Copilot Studio; fields not listed are kept as-is.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
pub struct BotActivity {
    #[serde(rename = "type", default)]
    pub activity_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(flatten)]
    pub extra: json::Map<String, Value>,
}

/// An attachment the agent sent with a message, e.g. an Adaptive Card.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default, PartialEq)]
pub struct CopilotStudioAttachment {
    pub content_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_url: Option<String>,
    #[serde(default)]
    pub content: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamUpdate {
    Text(String),
    Progress(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Citation {
    title: String,
    url: String,
    snippet: Option<String>,
}

/// Messages Copilot Studio sends instead of an answer when the environment's capacity or billing
/// blocks the agent.
const UNAVAILABLE_NOTICES: [&str; 2] = [
    "there is a billing issue.",
    "this agent is currently unavailable. it has reached its usage limit.",
];

#[derive(Default)]
pub(crate) struct TurnCollector {
    user_id: Option<String>,
    conversation_id: Option<String>,
    activities: Vec<Value>,
    messages: Vec<String>,
    streams: AHashMap<String, String>,
    seen_sequences: AHashSet<(String, i64)>,
    active_segment: Option<String>,
    text_emitted: bool,
    last_progress: Option<String>,
    suggested_actions: Vec<String>,
    attachments: Vec<CopilotStudioAttachment>,
    citations: Vec<Citation>,
    expecting_input: bool,
    ended: bool,
}

pub(crate) struct TurnOutput {
    pub conversation_id: String,
    pub text: String,
    pub suggested_actions: Vec<String>,
    pub cards: Vec<CopilotStudioAttachment>,
    pub expecting_input: bool,
    pub conversation_ended: bool,
    pub activities: Vec<Value>,
    citations: Vec<Citation>,
}

struct StreamInfo {
    id: Option<String>,
    kind: Option<String>,
    sequence: Option<i64>,
}

impl TurnCollector {
    /// `user_id` filters out echoes of the user's own activities (Direct Line returns them).
    pub fn new(user_id: Option<String>) -> Self {
        Self {
            user_id,
            ..Self::default()
        }
    }

    pub fn conversation_id(&self) -> Option<&str> {
        self.conversation_id.as_deref()
    }

    pub fn set_conversation_id(&mut self, conversation_id: &str) {
        if !conversation_id.is_empty() {
            self.conversation_id = Some(conversation_id.to_string());
        }
    }

    pub fn has_bot_message(&self) -> bool {
        !self.messages.is_empty()
    }

    pub fn expecting_input(&self) -> bool {
        self.expecting_input
    }

    pub fn ended(&self) -> bool {
        self.ended
    }

    pub fn ingest(&mut self, activity: Value) -> Vec<StreamUpdate> {
        if self.conversation_id.is_none()
            && let Some(id) = activity["conversation"]["id"].as_str()
        {
            self.set_conversation_id(id);
        }
        let updates = if self.is_from_user(&activity) {
            Vec::new()
        } else {
            match activity["type"].as_str().unwrap_or_default() {
                "typing" => self.ingest_typing(&activity),
                "message" => self.ingest_message(&activity),
                "endOfConversation" => {
                    self.ended = true;
                    Vec::new()
                }
                _ => Vec::new(),
            }
        };
        self.activities.push(activity);
        updates
    }

    fn is_from_user(&self, activity: &Value) -> bool {
        let from = &activity["from"];
        if from["role"].as_str() == Some("user") {
            return true;
        }
        match (&self.user_id, from["id"].as_str()) {
            (Some(user_id), Some(from_id)) => user_id == from_id,
            _ => false,
        }
    }

    fn ingest_typing(&mut self, activity: &Value) -> Vec<StreamUpdate> {
        let text = activity["text"].as_str().unwrap_or_default();
        if text.is_empty() {
            return Vec::new();
        }
        let info = stream_info(activity);
        if info.kind.as_deref() == Some("informative") {
            if self.last_progress.as_deref() == Some(text) {
                return Vec::new();
            }
            self.last_progress = Some(text.to_string());
            return vec![StreamUpdate::Progress(format!("{text}\n"))];
        }
        let stream_id = info.id.unwrap_or_default();
        if let Some(sequence) = info.sequence
            && !self.seen_sequences.insert((stream_id.clone(), sequence))
        {
            return Vec::new();
        }
        let accumulated = self.streams.entry(stream_id.clone()).or_default();
        let delta = merge_stream_text(accumulated, text);
        self.segment_text(Some(stream_id), delta)
    }

    fn ingest_message(&mut self, activity: &Value) -> Vec<StreamUpdate> {
        let text = activity["text"].as_str().unwrap_or_default().to_string();
        let info = stream_info(activity);

        let stream_key = info.id.unwrap_or_default();
        let delta = match self.streams.remove(&stream_key) {
            Some(streamed) => text
                .strip_prefix(streamed.as_str())
                .map(str::to_string)
                .unwrap_or_default(),
            None => text.clone(),
        };
        let updates = self.segment_text(Some(stream_key), delta);

        if !text.is_empty() {
            self.messages.push(text);
        }
        let actions = suggested_actions(activity);
        if !actions.is_empty() {
            self.suggested_actions = actions;
        }
        self.attachments.extend(message_attachments(activity));
        for citation in message_citations(activity) {
            if !self.citations.iter().any(|known| known.url == citation.url) {
                self.citations.push(citation);
            }
        }
        self.expecting_input = activity["inputHint"].as_str() == Some("expectingInput");
        updates
    }

    /// Emits `delta`, separating it from earlier messages of the same turn by a blank line. A
    /// message without stream id gets a fresh segment key so it never continues a stream.
    fn segment_text(&mut self, segment: Option<String>, delta: String) -> Vec<StreamUpdate> {
        if delta.is_empty() {
            return Vec::new();
        }
        let segment = match segment {
            Some(id) if !id.is_empty() => id,
            _ => format!("message-{}", self.messages.len()),
        };
        let continues = self.active_segment.as_deref() == Some(segment.as_str());
        self.active_segment = Some(segment);
        let text = if self.text_emitted && !continues {
            format!("\n\n{delta}")
        } else {
            delta
        };
        self.text_emitted = true;
        vec![StreamUpdate::Text(text)]
    }

    pub fn finish(self) -> TurnOutput {
        TurnOutput {
            conversation_id: self.conversation_id.unwrap_or_default(),
            text: self.messages.join("\n\n"),
            suggested_actions: self.suggested_actions,
            cards: self.attachments,
            expecting_input: self.expecting_input,
            conversation_ended: self.ended,
            activities: self.activities,
            citations: self.citations,
        }
    }
}

/// Copilot Studio may stream deltas or cumulative snapshots. A chunk that extends everything seen
/// so far is a snapshot; anything else is a delta.
fn merge_stream_text(accumulated: &mut String, incoming: &str) -> String {
    if !accumulated.is_empty()
        && let Some(rest) = incoming.strip_prefix(accumulated.as_str())
    {
        let delta = rest.to_string();
        accumulated.push_str(&delta);
        return delta;
    }
    accumulated.push_str(incoming);
    incoming.to_string()
}

fn stream_info(activity: &Value) -> StreamInfo {
    let entity = activity["entities"].as_array().and_then(|entities| {
        entities
            .iter()
            .find(|entity| entity["type"].as_str() == Some("streaminfo"))
    });
    let source = entity.unwrap_or(&activity["channelData"]);
    StreamInfo {
        id: source["streamId"].as_str().map(str::to_string),
        kind: source["streamType"].as_str().map(str::to_string),
        sequence: source["streamSequence"].as_i64(),
    }
}

fn suggested_actions(activity: &Value) -> Vec<String> {
    activity["suggestedActions"]["actions"]
        .as_array()
        .map(|actions| {
            actions
                .iter()
                .filter_map(|action| {
                    action["value"]
                        .as_str()
                        .or_else(|| action["title"].as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn message_attachments(activity: &Value) -> Vec<CopilotStudioAttachment> {
    activity["attachments"]
        .as_array()
        .map(|attachments| {
            attachments
                .iter()
                .filter_map(|attachment| {
                    Some(CopilotStudioAttachment {
                        content_type: attachment["contentType"].as_str()?.to_string(),
                        name: attachment["name"].as_str().map(str::to_string),
                        content_url: attachment["contentUrl"].as_str().map(str::to_string),
                        content: attachment["content"].clone(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Generative answers carry their sources as a schema.org `Message` entity with `citation` claims.
fn message_citations(activity: &Value) -> Vec<Citation> {
    let Some(entities) = activity["entities"].as_array() else {
        return Vec::new();
    };
    entities
        .iter()
        .filter(|entity| entity["type"].as_str() == Some("https://schema.org/Message"))
        .filter_map(|entity| entity["citation"].as_array())
        .flatten()
        .filter_map(|claim| {
            let appearance = &claim["appearance"];
            let url = appearance["url"].as_str().filter(|url| !url.is_empty())?;
            Some(Citation {
                title: appearance["name"].as_str().unwrap_or(url).to_string(),
                url: url.to_string(),
                snippet: appearance["abstract"].as_str().map(str::to_string),
            })
        })
        .collect()
}

impl TurnOutput {
    /// The agent's text when Copilot Studio refused the turn for capacity or billing reasons.
    pub fn unavailable_notice(&self) -> Option<&str> {
        let text = self.text.trim();
        let lowered = text.to_ascii_lowercase();
        UNAVAILABLE_NOTICES
            .iter()
            .any(|notice| lowered == *notice)
            .then_some(text)
    }

    fn response(&self) -> Response {
        let mut response = Response::from_text(&self.text, MODEL_NAME);
        let annotations: Vec<Annotation> = self
            .citations
            .iter()
            .filter_map(|citation| {
                json::from_value(json!({
                    "type": "url_citation",
                    "url_citation": {
                        "start_index": 0,
                        "end_index": 0,
                        "title": citation.title,
                        "url": citation.url,
                        "content": citation.snippet,
                    }
                }))
                .ok()
            })
            .collect();
        if let Some(choice) = response.choices.first_mut()
            && !annotations.is_empty()
        {
            choice.message.annotations = Some(annotations);
        }
        response
    }

    fn chat_attachments(&self) -> Vec<Attachment> {
        self.citations
            .iter()
            .map(|citation| {
                Attachment::Complex(ComplexAttachment {
                    url: citation.url.clone(),
                    preview_text: citation.snippet.clone(),
                    thumbnail_url: None,
                    name: Some(citation.title.clone()),
                    size: None,
                    r#type: Some("citation".to_string()),
                    anchor: None,
                    page: None,
                })
            })
            .collect()
    }
}

/// Output pins shared by every Copilot Studio conversation node.
pub(crate) fn add_turn_output_pins(node: &mut Node) {
    add_flow_output_pins(node);
    add_reply_output_pins(node);
    add_state_output_pins(node);
}

fn add_flow_output_pins(node: &mut Node) {
    node.add_output_pin(
        "on_stream",
        "On Stream",
        "Fires for every streamed text or progress chunk",
        VariableType::Execution,
    );
    node.add_output_pin(
        "chunk",
        "Chunk",
        "Streamed chunk: answer text as content, progress updates as reasoning",
        VariableType::Struct,
    )
    .set_schema::<ResponseChunk>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_output_pin(
        "done",
        "Done",
        "The agent finished its turn",
        VariableType::Execution,
    );
    node.add_output_pin("error", "Error", "The call failed", VariableType::Execution);
}

fn add_reply_output_pins(node: &mut Node) {
    node.add_output_pin(
        "result",
        "Result",
        "The agent's reply with citations as annotations",
        VariableType::Struct,
    )
    .set_schema::<Response>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_output_pin(
        "new_conversation_id",
        "Conversation ID",
        "Pass this into the next call to continue the conversation",
        VariableType::String,
    );
    node.add_output_pin(
        "suggested_actions",
        "Suggested Actions",
        "Quick replies the agent offered; send one back as the next message",
        VariableType::String,
    )
    .set_value_type(ValueType::Array);
    node.add_output_pin(
        "cards",
        "Cards",
        "Adaptive Cards and other attachments the agent sent",
        VariableType::Struct,
    )
    .set_value_type(ValueType::Array)
    .set_schema::<CopilotStudioAttachment>();
    node.add_output_pin(
        "attachments",
        "Attachments",
        "Citations from generative answers, as chat attachments",
        VariableType::Struct,
    )
    .set_value_type(ValueType::Array)
    .set_schema::<Attachment>();
}

fn add_state_output_pins(node: &mut Node) {
    node.add_output_pin(
        "expecting_input",
        "Expecting Input",
        "The agent asked a question and waits for the user's answer",
        VariableType::Boolean,
    );
    node.add_output_pin(
        "conversation_ended",
        "Conversation Ended",
        "The agent ended the conversation; start a new one for the next message",
        VariableType::Boolean,
    );
    node.add_output_pin(
        "activities",
        "Activities",
        "Every raw Bot Framework activity received in this call",
        VariableType::Struct,
    )
    .set_value_type(ValueType::Array)
    .set_schema::<BotActivity>();
    node.add_output_pin(
        "error_message",
        "Error Message",
        "Why the call failed",
        VariableType::String,
    );
}

async fn write_turn_outputs(
    context: &mut ExecutionContext,
    output: &TurnOutput,
) -> flow_like_types::Result<()> {
    let values = [
        ("result", json!(output.response())),
        ("new_conversation_id", json!(output.conversation_id)),
        ("suggested_actions", json!(output.suggested_actions)),
        ("cards", json!(output.cards)),
        ("attachments", json!(output.chat_attachments())),
        ("expecting_input", json!(output.expecting_input)),
        ("conversation_ended", json!(output.conversation_ended)),
        ("activities", json!(output.activities)),
        ("error_message", json!("")),
    ];
    for (pin, value) in values {
        context.set_pin_value(pin, value).await?;
    }
    Ok(())
}

async fn fail_turn(context: &mut ExecutionContext, message: &str) -> flow_like_types::Result<()> {
    context.log_message(message, LogLevel::Error);
    context
        .set_pin_value("error_message", json!(message))
        .await?;
    context.activate_exec_pin("error").await?;
    Ok(())
}

/// One conversation node run: deactivates the outcome pins, streams while the node talks to the
/// agent, then writes the outputs or routes the failure to the error pin.
pub(crate) struct Turn {
    pub collector: TurnCollector,
    pub emitter: StreamEmitter,
}

impl Turn {
    pub async fn begin(
        context: &mut ExecutionContext,
        user_id: Option<String>,
    ) -> flow_like_types::Result<Self> {
        context.deactivate_exec_pin("done").await?;
        context.deactivate_exec_pin("error").await?;
        Ok(Self {
            collector: TurnCollector::new(user_id),
            emitter: StreamEmitter::attach(context).await?,
        })
    }

    pub async fn complete(
        self,
        context: &mut ExecutionContext,
        outcome: flow_like_types::Result<()>,
    ) -> flow_like_types::Result<()> {
        self.emitter.finish(context).await?;
        if let Err(error) = outcome {
            return fail_turn(context, &format!("{error:#}")).await;
        }
        let output = self.collector.finish();
        if let Some(notice) = output.unavailable_notice() {
            let message = format!(
                "Copilot Studio refused the turn: \"{notice}\". The environment's Copilot Credits capacity is exhausted or its billing is misconfigured."
            );
            return fail_turn(context, &message).await;
        }
        write_turn_outputs(context, &output).await?;
        context.activate_exec_pin("done").await?;
        Ok(())
    }
}

struct StreamConsumer {
    node_id: String,
    context: ExecutionContext,
}

/// Runs the nodes connected to `on_stream` once per chunk, the same way the LLM invoke nodes do.
pub(crate) struct StreamEmitter {
    parent_node_id: String,
    consumers: Vec<StreamConsumer>,
}

impl StreamEmitter {
    pub async fn attach(context: &mut ExecutionContext) -> flow_like_types::Result<Self> {
        let on_stream = context.get_pin_by_name("on_stream").await?;
        context.activate_exec_pin_ref(&on_stream).await?;
        let parent_node_id = context.node.node.lock().await.id.clone();
        let mut consumers = Vec::new();
        for node in on_stream.get_connected_nodes() {
            let node_id = node.node.lock().await.id.clone();
            consumers.push(StreamConsumer {
                node_id,
                context: context.create_sub_context(&node).await,
            });
        }
        Ok(Self {
            parent_node_id,
            consumers,
        })
    }

    pub async fn emit_all(
        &mut self,
        context: &mut ExecutionContext,
        updates: Vec<StreamUpdate>,
    ) -> flow_like_types::Result<()> {
        for update in updates {
            self.emit(context, update).await?;
        }
        Ok(())
    }

    async fn emit(
        &mut self,
        context: &mut ExecutionContext,
        update: StreamUpdate,
    ) -> flow_like_types::Result<()> {
        let chunk = match &update {
            StreamUpdate::Text(text) => ResponseChunk::from_text(text, MODEL_NAME),
            StreamUpdate::Progress(text) => ResponseChunk::from_reasoning(text, MODEL_NAME),
        };
        context.set_pin_value("chunk", json!(chunk)).await?;

        let mut recursion_guard = AHashSet::new();
        recursion_guard.insert(self.parent_node_id.clone());
        for consumer in &mut self.consumers {
            let mut guard = Some(recursion_guard.clone());
            let result = InternalNode::trigger(&mut consumer.context, &mut guard, true).await;
            consumer.context.end_trace();
            if let Err(error) = result {
                context.log_message(
                    &format!(
                        "Copilot Studio stream consumer {} failed: {error:?}",
                        consumer.node_id
                    ),
                    LogLevel::Error,
                );
            }
        }
        Ok(())
    }

    pub async fn finish(mut self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("on_stream").await?;
        for consumer in &mut self.consumers {
            consumer.context.end_trace();
            context.push_sub_context(&mut consumer.context);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(updates: Vec<StreamUpdate>) -> String {
        updates
            .into_iter()
            .map(|update| match update {
                StreamUpdate::Text(text) => text,
                StreamUpdate::Progress(text) => format!("<{text}>"),
            })
            .collect()
    }

    fn typing(stream: &str, sequence: i64, kind: &str, text: &str) -> Value {
        json!({
            "type": "typing",
            "from": { "id": "bot", "role": "bot" },
            "conversation": { "id": "conv-1" },
            "text": text,
            "entities": [{
                "type": "streaminfo",
                "streamId": stream,
                "streamSequence": sequence,
                "streamType": kind
            }]
        })
    }

    fn final_message(stream: &str, text: &str) -> Value {
        json!({
            "type": "message",
            "from": { "id": "bot", "role": "bot" },
            "text": text,
            "entities": [{ "type": "streaminfo", "streamId": stream, "streamType": "final" }]
        })
    }

    #[test]
    fn delta_chunks_stream_once_and_the_final_message_adds_only_the_tail() {
        let mut collector = TurnCollector::new(None);
        let mut streamed = String::new();
        streamed += &texts(collector.ingest(typing("s1", 1, "streaming", "Hel")));
        streamed += &texts(collector.ingest(typing("s1", 2, "streaming", "lo wor")));
        streamed += &texts(collector.ingest(typing("s1", 2, "streaming", "lo wor")));
        streamed += &texts(collector.ingest(final_message("s1", "Hello world!")));
        assert_eq!(streamed, "Hello world!");
        let output = collector.finish();
        assert_eq!(output.text, "Hello world!");
        assert_eq!(output.conversation_id, "conv-1");
    }

    #[test]
    fn cumulative_snapshots_are_reduced_to_deltas() {
        let mut collector = TurnCollector::new(None);
        let mut streamed = String::new();
        streamed += &texts(collector.ingest(typing("s1", 1, "streaming", "The")));
        streamed += &texts(collector.ingest(typing("s1", 2, "streaming", "The answer")));
        streamed += &texts(collector.ingest(typing("s1", 3, "streaming", "The answer is 42")));
        streamed += &texts(collector.ingest(final_message("s1", "The answer is 42.")));
        assert_eq!(streamed, "The answer is 42.");
    }

    #[test]
    fn informative_updates_become_progress_and_never_touch_the_answer() {
        let mut collector = TurnCollector::new(None);
        let progress = texts(collector.ingest(typing("s1", 1, "informative", "Searching…")));
        assert_eq!(progress, "<Searching…\n>");
        assert!(
            collector
                .ingest(typing("s1", 2, "informative", "Searching…"))
                .is_empty()
        );
        let answer = texts(collector.ingest(final_message("s1", "Found it.")));
        assert_eq!(answer, "Found it.");
    }

    #[test]
    fn separate_messages_are_streamed_whole_with_a_blank_line_between() {
        let mut collector = TurnCollector::new(None);
        let first = texts(collector.ingest(json!({"type": "message", "text": "Hi!"})));
        let second = texts(collector.ingest(json!({"type": "message", "text": "How can I help?"})));
        assert_eq!(format!("{first}{second}"), "Hi!\n\nHow can I help?");
        assert_eq!(collector.finish().text, "Hi!\n\nHow can I help?");
    }

    #[test]
    fn user_echoes_are_ignored_but_kept_as_raw_activities() {
        let mut collector = TurnCollector::new(Some("user-1".to_string()));
        let updates = collector.ingest(json!({
            "type": "message", "text": "question", "from": { "id": "user-1" }
        }));
        assert!(updates.is_empty());
        assert!(!collector.has_bot_message());
        assert_eq!(collector.finish().activities.len(), 1);
    }

    #[test]
    fn questions_actions_cards_citations_and_end_are_collected() {
        let mut collector = TurnCollector::new(None);
        collector.ingest(json!({
            "type": "message",
            "text": "Which region?",
            "inputHint": "expectingInput",
            "suggestedActions": { "actions": [
                { "type": "imBack", "title": "Europe", "value": "Europe" },
                { "type": "imBack", "title": "Asia" }
            ]},
            "attachments": [{
                "contentType": "application/vnd.microsoft.card.adaptive",
                "content": { "type": "AdaptiveCard" }
            }],
            "entities": [{
                "type": "https://schema.org/Message",
                "citation": [
                    { "appearance": { "name": "Policy", "url": "https://contoso.com/p", "abstract": "Travel" } },
                    { "appearance": { "name": "Duplicate", "url": "https://contoso.com/p" } },
                    { "appearance": { "name": "No URL" } }
                ]
            }]
        }));
        collector.ingest(json!({ "type": "endOfConversation", "code": "completedSuccessfully" }));
        let output = collector.finish();
        assert!(output.expecting_input);
        assert!(output.conversation_ended);
        assert_eq!(output.suggested_actions, vec!["Europe", "Asia"]);
        assert_eq!(output.cards.len(), 1);
        assert_eq!(output.chat_attachments().len(), 1);
        let response = json!(output.response());
        assert_eq!(
            response["choices"][0]["message"]["annotations"][0]["url_citation"]["url"],
            "https://contoso.com/p"
        );
    }

    #[test]
    fn legacy_channel_data_stream_fields_are_understood() {
        let mut collector = TurnCollector::new(None);
        let chunk = json!({
            "type": "typing",
            "text": "Par",
            "channelData": { "streamId": "s9", "streamType": "streaming", "streamSequence": 1 }
        });
        let mut streamed = texts(collector.ingest(chunk));
        streamed += &texts(collector.ingest(json!({
            "type": "message",
            "text": "Partial",
            "channelData": { "streamId": "s9", "streamType": "final" }
        })));
        assert_eq!(streamed, "Partial");
    }

    #[test]
    fn chunks_without_stream_id_are_not_repeated_by_the_final_message() {
        let mut collector = TurnCollector::new(None);
        let mut streamed = String::new();
        streamed += &texts(collector.ingest(json!({"type": "typing", "text": "Hel"})));
        streamed += &texts(collector.ingest(json!({"type": "typing", "text": "lo"})));
        streamed += &texts(collector.ingest(json!({"type": "message", "text": "Hello"})));
        streamed += &texts(collector.ingest(json!({"type": "message", "text": "Bye"})));
        assert_eq!(streamed, "Hello\n\nBye");
    }

    #[test]
    fn capacity_notices_are_detected_verbatim_only() {
        let mut collector = TurnCollector::new(None);
        collector.ingest(json!({"type": "message", "text": "There is a billing issue."}));
        assert!(collector.finish().unavailable_notice().is_some());

        let mut collector = TurnCollector::new(None);
        collector.ingest(json!({"type": "message", "text": "There is a billing issue. Here is how to fix it: …"}));
        assert!(collector.finish().unavailable_notice().is_none());
    }
}

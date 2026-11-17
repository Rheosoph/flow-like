//! What an app run pushed into the chat: text, files and widgets. A run called as a tool
//! (`call_app_chat`, `call_app_event`, an MCP tool) streams these as chat events; hosts fold them
//! for the model and hand them to a live chat.

use std::collections::HashSet;

use serde_json::{Value, json};

/// Query flag of the member `mcp-operation` route that asks for a `tools/call` run's chat events.
pub const CHAT_EVENTS_QUERY: &str = "chat_events";
/// Key under an MCP result's `_meta` that carries those events. Public MCP clients never get it.
pub const CHAT_EVENTS_META: &str = "flow-like.com/chat_events";
/// Captured events stop here: they travel in one buffered JSON response.
pub const MAX_CAPTURED_CHAT_BYTES: usize = 4 * 1024 * 1024;
/// A fold stops reading pushes beyond this many payload bytes.
pub const MAX_FOLDED_CHAT_BYTES: usize = 16 * 1024 * 1024;

/// An event an app run sends to the chat it answers.
pub fn is_chat_event(kind: &str) -> bool {
    matches!(
        kind,
        "chat_stream_partial" | "chat_stream" | "chat_out" | "chat_usage_stat" | "a2ui"
    )
}

/// Calls `visit` for each envelope; a batch arrives as an array of envelopes.
pub fn each_envelope(event: &Value, visit: &mut impl FnMut(&Value)) {
    match event {
        Value::Array(events) => {
            for event in events {
                visit(event);
            }
        }
        event => visit(event),
    }
}

fn kind(envelope: &Value) -> &str {
    envelope["event_type"].as_str().unwrap_or_default()
}

/// A run's chat events in order, for a caller that receives them after the run.
#[derive(Debug, Default)]
pub struct ChatEventLog {
    events: Vec<Value>,
    bytes: usize,
    truncated: bool,
}

impl ChatEventLog {
    pub fn push(&mut self, event: &Value) {
        each_envelope(event, &mut |envelope| self.keep(envelope));
    }

    fn keep(&mut self, envelope: &Value) {
        if self.truncated || !is_chat_event(kind(envelope)) {
            return;
        }
        let kept = json!({ "event_type": kind(envelope), "payload": envelope["payload"] });
        let size = kept.to_string().len();
        if self.bytes.saturating_add(size) > MAX_CAPTURED_CHAT_BYTES {
            self.truncated = true;
            return;
        }
        self.bytes += size;
        self.events.push(kept);
    }

    /// Puts the events under `result._meta`. An object result only; nothing when nothing was pushed.
    pub fn attach(self, result: &mut Value) {
        if self.events.is_empty() && !self.truncated {
            return;
        }
        let Some(object) = result.as_object_mut() else {
            return;
        };
        let meta = object.entry("_meta").or_insert_with(|| json!({}));
        if let Some(meta) = meta.as_object_mut() {
            meta.insert(
                CHAT_EVENTS_META.to_string(),
                json!({ "events": self.events, "truncated": self.truncated }),
            );
        }
    }
}

/// The events [`ChatEventLog::attach`] put on a result.
#[derive(Debug, Default, PartialEq)]
pub struct CapturedChatEvents {
    pub events: Vec<Value>,
    pub truncated: bool,
}

/// Removes the captured chat events from an MCP result, and `_meta` when nothing else is in it.
pub fn take_chat_events(result: &mut Value) -> CapturedChatEvents {
    let Some(object) = result.as_object_mut() else {
        return CapturedChatEvents::default();
    };
    let Some(meta) = object.get_mut("_meta").and_then(Value::as_object_mut) else {
        return CapturedChatEvents::default();
    };
    let taken = meta.remove(CHAT_EVENTS_META);
    if meta.is_empty() {
        object.remove("_meta");
    }
    let Some(taken) = taken else {
        return CapturedChatEvents::default();
    };
    CapturedChatEvents {
        truncated: taken["truncated"].as_bool().unwrap_or(false),
        events: match taken.get("events") {
            Some(Value::Array(events)) => events.clone(),
            _ => Vec::new(),
        },
    }
}

/// One answer of the run: a message id starts a new one.
#[derive(Debug, Default)]
pub struct ChatMessage {
    pub id: Option<String>,
    pub text: String,
    pub attachments: Vec<Value>,
    pub widgets: Vec<Value>,
    choices: Vec<Value>,
    /// How many [`ChatFold::a2ui`] payloads had arrived when each widget was pushed.
    pushed_at: Vec<usize>,
}

/// What a run's events pushed to the chat, folded as they arrive.
#[derive(Debug, Default)]
pub struct ChatFold {
    pub run_id: Option<String>,
    pub messages: Vec<ChatMessage>,
    /// `chat_out` arrived; later pushes are ignored.
    pub done: bool,
    pub interactions: usize,
    pub surfaces: HashSet<String>,
    /// Every a2ui payload in order, so widgets can take the updates sent after their push.
    pub a2ui: Vec<Value>,
    /// The pushes passed [`MAX_FOLDED_CHAT_BYTES`]; later ones are ignored.
    pub overflow: bool,
    bytes: usize,
}

impl ChatFold {
    pub fn push(&mut self, event: &Value) {
        each_envelope(event, &mut |envelope| self.fold(envelope));
    }

    fn fold(&mut self, envelope: &Value) {
        let kind = kind(envelope);
        let payload = &envelope["payload"];
        match kind {
            "run_initiated" if self.run_id.is_none() => {
                self.run_id = payload["run_id"]
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned);
            }
            "interaction_request" => self.interactions += 1,
            "a2ui" => {
                self.track_surface(payload);
                if self.counts(payload) {
                    self.a2ui.push(payload.clone());
                }
            }
            "chat_stream_partial" | "chat_stream" | "chat_out" | "chat_usage_stat"
                if !self.done && !self.overflow =>
            {
                self.fold_chat(kind, payload);
            }
            _ => {}
        }
    }

    fn track_surface(&mut self, payload: &Value) {
        let Some(id) = payload["surfaceId"].as_str() else {
            return;
        };
        match payload["type"].as_str() {
            Some("beginRendering") => {
                self.surfaces.insert(id.to_string());
            }
            Some("deleteSurface") => {
                self.surfaces.remove(id);
            }
            _ => {}
        }
    }

    fn current(&mut self, message_id: Option<&str>) -> &mut ChatMessage {
        let starts_new = match (self.messages.last(), message_id) {
            (None, _) => true,
            (Some(last), Some(id)) => last.id.as_deref().is_some_and(|known| known != id),
            (Some(_), None) => false,
        };
        if starts_new {
            self.messages.push(ChatMessage::default());
        }
        let current = self
            .messages
            .last_mut()
            .expect("a message was just ensured");
        if current.id.is_none() {
            current.id = message_id.map(str::to_owned);
        }
        current
    }

    /// Adds the payload to the folded size; false once the fold is over its limit.
    fn counts(&mut self, payload: &Value) -> bool {
        if self.overflow {
            return false;
        }
        self.bytes = self.bytes.saturating_add(payload.to_string().len());
        if self.bytes > MAX_FOLDED_CHAT_BYTES {
            self.overflow = true;
        }
        !self.overflow
    }

    fn fold_chat(&mut self, kind: &str, payload: &Value) {
        if !self.counts(payload) {
            return;
        }
        let pushed_at = self.a2ui.len();
        let message_id = payload["message_id"].as_str().filter(|id| !id.is_empty());
        let current = self.current(message_id);
        if kind == "chat_stream_partial" {
            for choice in items(&payload["chunk"]["choices"]) {
                let index = &choice["index"];
                let position = current
                    .choices
                    .iter()
                    .position(|existing| existing["index"] == *index)
                    .unwrap_or_else(|| {
                        current.choices.push(json!({
                            "index": index,
                            "message": { "role": "", "content": "", "content_parts": [] },
                        }));
                        current.choices.len() - 1
                    });
                apply_delta(&mut current.choices[position]["message"], &choice["delta"]);
            }
        } else if payload["response"].is_object() {
            current.choices = items(&payload["response"]["choices"]).to_vec();
        }
        if let Some(message) = current
            .choices
            .iter()
            .rev()
            .map(|choice| &choice["message"])
            .find(|message| message["role"] == "assistant")
        {
            current.text = visible_text(message);
        }
        for attachment in items(&payload["attachments"]) {
            if !current
                .attachments
                .iter()
                .any(|existing| url_of(existing) == url_of(attachment))
            {
                current.attachments.push(attachment.clone());
            }
        }
        for widget in items(&payload["widgets"]) {
            let id = &widget["instance_id"];
            if id.is_null()
                || !current
                    .widgets
                    .iter()
                    .any(|known| known["instance_id"] == *id)
            {
                current.widgets.push(widget.clone());
                current.pushed_at.push(pushed_at);
            }
        }
        if kind == "chat_out" {
            self.done = true;
        }
    }

    /// The answers' text, one paragraph per message.
    pub fn text(&self) -> String {
        self.messages
            .iter()
            .map(|message| message.text.as_str())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Every pushed file as `{url, name, type}`, or `{url}` for a bare link.
    pub fn attachments(&self) -> Vec<Value> {
        self.messages
            .iter()
            .flat_map(|message| &message.attachments)
            .map(|file| match file.as_str() {
                Some(url) => json!({ "url": url }),
                None => json!({ "url": file["url"], "name": file["name"], "type": file["type"] }),
            })
            .collect()
    }

    /// Every pushed widget with the number of a2ui payloads that had arrived before its push.
    pub fn widgets(&self) -> impl Iterator<Item = (&Value, usize)> {
        self.messages.iter().flat_map(|message| {
            message
                .widgets
                .iter()
                .zip(message.pushed_at.iter().copied())
        })
    }

    pub fn widget_count(&self) -> usize {
        self.messages
            .iter()
            .map(|message| message.widgets.len())
            .sum()
    }

    /// Adds what the run pushed besides text: its files, and how many widgets and surfaces.
    pub fn describe_pushed(&self, outcome: &mut Value) {
        let attachments = self.attachments();
        if !attachments.is_empty() {
            outcome["attachments"] = json!(attachments);
        }
        let widgets = self.widget_count();
        if widgets > 0 {
            outcome["pushed_widget_count"] = json!(widgets);
        }
        if !self.surfaces.is_empty() {
            outcome["pushed_surface_count"] = json!(self.surfaces.len());
        }
        if self.overflow {
            outcome["pushed_truncated"] = json!(true);
        }
    }
}

fn items(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn url_of(value: &Value) -> Option<&str> {
    value.as_str().or_else(|| value["url"].as_str())
}

fn append_part(parts: &mut Vec<Value>, part: Value) {
    match parts.last_mut() {
        Some(last) if part["type"] == "text" && last["type"] == "text" => {
            last["text"] = json!(format!(
                "{}{}",
                last["text"].as_str().unwrap_or_default(),
                part["text"].as_str().unwrap_or_default()
            ));
        }
        _ => parts.push(part),
    }
}

fn apply_delta(message: &mut Value, incoming: &Value) {
    let content = incoming["content"].as_str().unwrap_or_default();
    let new_parts = items(&incoming["content_parts"]);
    let carries_text = new_parts.iter().any(|part| part["type"] == "text");
    let mut parts = items(&message["content_parts"]).to_vec();
    if !content.is_empty() {
        if !parts.is_empty() && !carries_text {
            append_part(&mut parts, json!({ "type": "text", "text": content }));
        }
        let joined = format!(
            "{}{content}",
            message["content"].as_str().unwrap_or_default()
        );
        message["content"] = json!(joined);
    }
    if !new_parts.is_empty() {
        let content = message["content"].as_str().unwrap_or_default();
        if parts.is_empty() && !content.is_empty() && !carries_text {
            parts.push(json!({ "type": "text", "text": content }));
        }
        for part in new_parts {
            append_part(&mut parts, part.clone());
        }
    }
    message["content_parts"] = json!(parts);
    if let Some(role) = incoming["role"].as_str().filter(|role| !role.is_empty()) {
        message["role"] = json!(role);
    }
}

/// The text a reader sees: the text parts when there are any, else the plain content.
pub fn visible_text(message: &Value) -> String {
    let content = message["content"].as_str().unwrap_or_default();
    let parts = items(&message["content_parts"]);
    if parts.is_empty() {
        return content.to_string();
    }
    let mut visible = if parts.iter().any(|part| part["type"] == "text") {
        String::new()
    } else {
        content.to_string()
    };
    for part in parts.iter().filter(|part| part["type"] == "text") {
        visible.push_str(part["text"].as_str().unwrap_or_default());
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(event_type: &str, payload: Value) -> Value {
        json!({ "event_type": event_type, "payload": payload })
    }

    fn partial(message_id: &str, content: &str) -> Value {
        envelope(
            "chat_stream_partial",
            json!({
                "message_id": message_id,
                "chunk": { "choices": [{ "index": 0, "delta": { "role": "assistant", "content": content } }] },
            }),
        )
    }

    fn pushed(attachments: Value, widgets: Value) -> Value {
        envelope(
            "chat_stream_partial",
            json!({ "message_id": "m1", "chunk": null, "attachments": attachments, "widgets": widgets }),
        )
    }

    #[test]
    fn streamed_chunks_fold_into_one_answer_per_message() {
        let mut fold = ChatFold::default();
        for event in [
            partial("m1", "Hel"),
            partial("m1", "lo"),
            partial("m2", "Second"),
        ] {
            fold.push(&event);
        }
        assert_eq!(fold.text(), "Hello\n\nSecond");
    }

    #[test]
    fn a_final_response_replaces_the_streamed_text_and_ends_the_fold() {
        let mut fold = ChatFold::default();
        fold.push(&partial("m1", "draft"));
        fold.push(&envelope(
            "chat_out",
            json!({
                "message_id": "m1",
                "response": { "choices": [{ "index": 0, "message": { "role": "assistant", "content": "final" } }] },
                "attachments": [{ "url": "https://f/a.png", "name": "a.png", "type": "image/png" }, "https://f/a.png"],
            }),
        ));
        fold.push(&partial("m1", " ignored"));
        assert!(fold.done);
        assert_eq!(fold.text(), "final");
        assert_eq!(fold.attachments().len(), 1);
    }

    #[test]
    fn snapshots_replace_text_and_message_ids_start_new_responses() {
        let mut fold = ChatFold::default();
        for (id, content) in [
            ("first", "old"),
            ("first", "replacement"),
            ("second", "next"),
        ] {
            fold.push(&envelope(
                "chat_stream",
                json!({ "message_id": id, "response": { "choices": [{ "index": 0, "message": { "role": "assistant", "content": content } }] } }),
            ));
        }
        let texts: Vec<_> = fold.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["replacement", "next"]);
    }

    #[test]
    fn a_long_stream_folds_completely() {
        let mut fold = ChatFold::default();
        for index in 0..1000 {
            fold.push(&envelope(
                "chat_stream_partial",
                json!({ "chunk": { "choices": [{ "index": 0, "delta": { "role": if index == 0 { "assistant" } else { "" }, "content": "x" } }] } }),
            ));
        }
        assert_eq!(fold.messages[0].text.len(), 1000);
    }

    #[test]
    fn structured_chunks_do_not_duplicate_plain_text() {
        let mut message = json!({ "role": "assistant", "content": "", "content_parts": [] });
        apply_delta(
            &mut message,
            &json!({ "content": "hello", "content_parts": [{ "type": "text", "text": "hello" }] }),
        );
        apply_delta(&mut message, &json!({ "content": " world" }));
        assert_eq!(visible_text(&message), "hello world");
    }

    #[test]
    fn plain_text_before_the_first_parts_is_kept_whole() {
        let mut message = json!({ "role": "assistant", "content": "", "content_parts": [] });
        apply_delta(&mut message, &json!({ "content": "a" }));
        apply_delta(
            &mut message,
            &json!({ "content": "b", "content_parts": [{ "type": "image" }] }),
        );
        assert_eq!(visible_text(&message), "ab");
    }

    #[test]
    fn text_parts_win_over_plain_content_when_both_exist() {
        let message = json!({
            "content": "plain",
            "content_parts": [{ "type": "text", "text": "a" }, { "type": "image" }, { "type": "text", "text": "b" }],
        });
        assert_eq!(visible_text(&message), "ab");
    }

    #[test]
    fn interactions_surfaces_and_the_run_id_are_tracked_not_folded_into_text() {
        let mut fold = ChatFold::default();
        fold.push(&json!([
            envelope("run_initiated", json!({ "run_id": "r1" })),
            envelope("interaction_request", json!({ "id": "i1" })),
            envelope(
                "a2ui",
                json!({ "surfaceId": "s1", "type": "beginRendering" })
            ),
            envelope(
                "a2ui",
                json!({ "surfaceId": "s2", "type": "beginRendering" })
            ),
            envelope(
                "a2ui",
                json!({ "surfaceId": "s2", "type": "deleteSurface" })
            ),
        ]));
        assert_eq!(fold.run_id.as_deref(), Some("r1"));
        assert_eq!(fold.interactions, 1);
        assert_eq!(fold.surfaces.len(), 1);
        assert!(fold.messages.is_empty());
    }

    #[test]
    fn pushed_files_and_widgets_are_described_without_text() {
        let mut fold = ChatFold::default();
        fold.push(&pushed(
            json!([{ "url": "https://f/r.pdf", "name": "r.pdf", "type": "application/pdf" }]),
            json!([{ "instance_id": "w1" }]),
        ));
        fold.push(&pushed(
            json!(["https://f/r.pdf"]),
            json!([{ "instance_id": "w1" }, { "instance_id": "w2" }]),
        ));
        let mut outcome = json!({ "status": "ok" });
        fold.describe_pushed(&mut outcome);
        assert_eq!(
            outcome["attachments"],
            json!([{ "url": "https://f/r.pdf", "name": "r.pdf", "type": "application/pdf" }])
        );
        assert_eq!(outcome["pushed_widget_count"], 2);
        assert!(outcome.get("response").is_none());

        let mut quiet = json!({ "status": "ok" });
        ChatFold::default().describe_pushed(&mut quiet);
        assert_eq!(quiet, json!({ "status": "ok" }));
    }

    #[test]
    fn the_log_keeps_only_chat_events_in_order() {
        let mut log = ChatEventLog::default();
        log.push(&envelope("log", json!({ "message": "x" })));
        log.push(&json!([
            pushed(json!(["https://f/a"]), json!([])),
            envelope("generic_result", json!(1)),
            envelope(
                "a2ui",
                json!({ "surfaceId": "s1", "type": "beginRendering" })
            ),
        ]));
        let mut result = json!({ "content": [], "isError": false });
        log.attach(&mut result);
        let captured = take_chat_events(&mut result);
        assert_eq!(result, json!({ "content": [], "isError": false }));
        assert!(!captured.truncated);
        let kinds: Vec<_> = captured
            .events
            .iter()
            .map(|e| e["event_type"].clone())
            .collect();
        assert_eq!(kinds, [json!("chat_stream_partial"), json!("a2ui")]);
    }

    #[test]
    fn a_run_that_pushed_nothing_leaves_the_result_alone() {
        let mut log = ChatEventLog::default();
        log.push(&envelope("generic_result", json!("done")));
        let mut result = json!({ "content": [] });
        log.attach(&mut result);
        assert_eq!(result, json!({ "content": [] }));
        assert_eq!(take_chat_events(&mut result), CapturedChatEvents::default());
    }

    #[test]
    fn other_meta_survives_taking_the_events() {
        let mut log = ChatEventLog::default();
        log.push(&pushed(json!(["https://f/a"]), json!([])));
        let mut result = json!({ "content": [], "_meta": { "other": 1 } });
        log.attach(&mut result);
        assert_eq!(take_chat_events(&mut result).events.len(), 1);
        assert_eq!(result, json!({ "content": [], "_meta": { "other": 1 } }));
    }

    #[test]
    fn an_oversized_log_is_cut_and_says_so() {
        let mut log = ChatEventLog::default();
        let big = "x".repeat(MAX_CAPTURED_CHAT_BYTES / 3);
        for _ in 0..4 {
            log.push(&pushed(json!([big]), json!([])));
        }
        log.push(&pushed(json!(["small"]), json!([])));
        let mut result = json!({});
        log.attach(&mut result);
        let captured = take_chat_events(&mut result);
        assert!(captured.truncated);
        assert_eq!(captured.events.len(), 2);
    }
}

//! Files a bot attaches to its reply (`file_attach`): stored in its workspace or linked. Hosts put
//! them on the reply body as attachment parts before the tool trail, so the trail shrinks to fit.

use crate::bot_message::{AttachmentPart, MAX_PARTS, MessagePart, UiPart, WorkspaceHandle};
use crate::bot_tool_trail::MAX_BODY_BYTES;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// At most this many files go with one reply.
pub const MAX_REPLY_FILES: usize = 8;
/// At most this many app widgets go with one reply.
pub const MAX_REPLY_WIDGETS: usize = 8;
/// A widget document larger than this is kept as a file, not inside the reply body.
pub const MAX_INLINE_WIDGET_BYTES: usize = 24 * 1024;
/// `type` of the document a `ui` part holds for a widget an app pushed.
pub const APP_WIDGET_TYPE: &str = "app_widget";

/// The document of an app widget on a reply: the widget as the app pushed it, its updates
/// included, and the app event whose board runs the widget's actions.
pub fn app_widget_doc(origin: &Value, widget: &Value) -> Value {
    json!({ "type": APP_WIDGET_TYPE, "origin": origin, "widget": widget })
}

/// An app widget of a reply. `handle` is the document as a file, for when it does not fit the
/// body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplyWidget {
    pub summary: String,
    pub doc: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<WorkspaceHandle>,
}

impl ReplyWidget {
    fn part(&self, id: String, inline: bool) -> UiPart {
        UiPart {
            id,
            summary: Some(self.summary.clone()),
            a2ui: inline.then(|| self.doc.clone()),
            handle: (!inline).then(|| self.handle.clone()).flatten(),
        }
    }
}

/// Appends `widgets` to a reply `body` as `ui` parts after the parts it holds: inline when the
/// document is small and fits [`MAX_BODY_BYTES`], else pointing at its file, else left out.
pub fn attach_reply_widgets(body: &mut Value, widgets: &[ReplyWidget]) {
    if widgets.is_empty() || !body.is_object() {
        return;
    }
    let mut parts: Vec<Value> = body
        .get("parts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut numbered = parts
        .iter()
        .filter(|part| part.get("kind").and_then(Value::as_str) == Some("ui"))
        .count();
    let mut used = serde_json::to_string(body).map_or(usize::MAX, |text| text.len());
    for widget in widgets.iter().take(MAX_REPLY_WIDGETS) {
        if parts.len() >= MAX_PARTS {
            break;
        }
        let id = format!("ui{}", numbered + 1);
        let inline = widget.doc.to_string().len() <= MAX_INLINE_WIDGET_BYTES;
        let candidates = [
            inline.then(|| widget.part(id.clone(), true)),
            widget
                .handle
                .is_some()
                .then(|| widget.part(id.clone(), false)),
        ];
        let Some((value, size)) = candidates
            .into_iter()
            .flatten()
            .map(MessagePart::Ui)
            .filter(part_passes)
            .filter_map(|part| serde_json::to_value(&part).ok())
            .map(|value| {
                let size = value.to_string().len() + 1;
                (value, size)
            })
            .find(|(_, size)| used.saturating_add(*size) <= MAX_BODY_BYTES)
        else {
            continue;
        };
        used += size;
        numbered += 1;
        parts.push(value);
    }
    body["parts"] = Value::Array(parts);
}

/// One file of a reply. Exactly one of `handle` and `url`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyFile {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<WorkspaceHandle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

impl ReplyFile {
    pub fn stored(name: String, handle: WorkspaceHandle) -> Self {
        Self {
            name,
            handle: Some(handle),
            url: None,
            mime: None,
        }
    }

    pub fn linked(name: String, url: String, mime: Option<String>) -> Self {
        Self {
            name,
            handle: None,
            url: Some(url),
            mime,
        }
    }

    /// The same file: one stored object or one address.
    pub fn same_file(&self, other: &Self) -> bool {
        match (&self.handle, &other.handle) {
            (Some(left), Some(right)) => left.app_id == right.app_id && left.path == right.path,
            (None, None) => self.url == other.url,
            _ => false,
        }
    }

    fn part(&self, id: String) -> AttachmentPart {
        match &self.handle {
            Some(handle) => AttachmentPart::stored(id, self.name.clone(), handle.clone()),
            None => AttachmentPart::linked(
                id,
                self.name.clone(),
                self.url.clone().unwrap_or_default(),
                self.mime.clone(),
            ),
        }
    }
}

/// Appends `files` to a reply `body` as attachment parts numbered after the ones it holds: at
/// most [`MAX_REPLY_FILES`], within [`MAX_PARTS`] and [`MAX_BODY_BYTES`]. A file that would not
/// pass the body check is left out; a body that is no object stays as it is.
pub fn attach_reply_files(body: &mut Value, files: &[ReplyFile]) {
    if files.is_empty() || !body.is_object() {
        return;
    }
    let mut parts: Vec<Value> = body
        .get("parts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut numbered = parts
        .iter()
        .filter(|part| part.get("kind").and_then(Value::as_str) == Some("attachment"))
        .count();
    let mut used = serde_json::to_string(body).map_or(usize::MAX, |text| text.len());
    for file in files.iter().take(MAX_REPLY_FILES) {
        if parts.len() >= MAX_PARTS {
            break;
        }
        let part = MessagePart::Attachment(file.part(format!("a{}", numbered + 1)));
        if !part_passes(&part) {
            continue;
        }
        let Ok(value) = serde_json::to_value(&part) else {
            continue;
        };
        let size = value.to_string().len() + 1;
        if used.saturating_add(size) > MAX_BODY_BYTES {
            break;
        }
        used += size;
        numbered += 1;
        parts.push(value);
    }
    body["parts"] = Value::Array(parts);
}

fn part_passes(part: &MessagePart) -> bool {
    let body = crate::bot_message::MessageBody {
        parts: vec![renumbered(part)],
        ..Default::default()
    };
    body.check().is_ok()
}

fn renumbered(part: &MessagePart) -> MessagePart {
    match part {
        MessagePart::Attachment(attachment) => MessagePart::Attachment(AttachmentPart {
            id: "a1".to_string(),
            ..attachment.clone()
        }),
        MessagePart::Ui(ui) => MessagePart::Ui(UiPart {
            id: "ui1".to_string(),
            ..ui.clone()
        }),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot_message::MessageBody;
    use serde_json::json;

    fn handle(path: &str) -> WorkspaceHandle {
        WorkspaceHandle {
            app_id: "ws1".to_string(),
            path: path.to_string(),
            bytes: 12,
            sha256: "a".repeat(64),
            mime: "application/pdf".to_string(),
        }
    }

    #[test]
    fn stored_and_linked_files_become_numbered_attachment_parts() {
        let mut body = json!({
            "text": "Hier ist das PDF",
            "parts": [{ "kind": "attachment", "id": "a1", "name": "in.png", "handle": handle("bot/in.png") }]
        });
        attach_reply_files(
            &mut body,
            &[
                ReplyFile::stored("Angebot.pdf".into(), handle("bot/files/Angebot.pdf")),
                ReplyFile::linked(
                    "Offering.pdf".into(),
                    "https://bucket.s3.amazonaws.com/o.pdf?X-Amz-Signature=abc".into(),
                    Some("application/pdf".into()),
                ),
            ],
        );
        let parsed = MessageBody::parse(&body).expect("valid body");
        let names: Vec<_> = parsed
            .parts
            .iter()
            .map(|part| match part {
                MessagePart::Attachment(file) => (file.id.clone(), file.name.clone()),
                _ => panic!("only attachments"),
            })
            .collect();
        assert_eq!(
            names,
            [
                ("a1".to_string(), "in.png".to_string()),
                ("a2".to_string(), "Angebot.pdf".to_string()),
                ("a3".to_string(), "Offering.pdf".to_string())
            ]
        );
        assert_eq!(parsed.handles().count(), 2);
    }

    #[test]
    fn a_file_the_body_check_refuses_is_left_out() {
        let mut body = json!({ "text": "x" });
        attach_reply_files(
            &mut body,
            &[
                ReplyFile::linked("a.pdf".into(), "ftp://host/a.pdf".into(), None),
                ReplyFile::linked("b.pdf".into(), "https://user:pw@host/b.pdf".into(), None),
                ReplyFile::linked("c.pdf".into(), "https://host/c.pdf".into(), None),
            ],
        );
        let parsed = MessageBody::parse(&body).expect("valid body");
        assert_eq!(parsed.parts.len(), 1);
        assert!(
            matches!(&parsed.parts[0], MessagePart::Attachment(file) if file.id == "a1" && file.name == "c.pdf")
        );
    }

    #[test]
    fn a_reply_keeps_at_most_eight_files_and_the_body_limit() {
        let mut body = json!({ "text": "x" });
        let files: Vec<_> = (0..12)
            .map(|index| {
                ReplyFile::linked(
                    format!("{index}.pdf"),
                    format!("https://h/{index}.pdf"),
                    None,
                )
            })
            .collect();
        attach_reply_files(&mut body, &files);
        assert_eq!(
            body["parts"].as_array().map(Vec::len),
            Some(MAX_REPLY_FILES)
        );

        let mut big = json!({ "text": "y".repeat(MAX_BODY_BYTES - 3_000) });
        let long = format!("https://h/{}", "p".repeat(2_000));
        attach_reply_files(
            &mut big,
            &[
                ReplyFile::linked("1.pdf".into(), long.clone(), None),
                ReplyFile::linked("2.pdf".into(), long, None),
            ],
        );
        assert_eq!(big["parts"].as_array().map(Vec::len), Some(1));
        assert!(serde_json::to_string(&big).unwrap().len() <= MAX_BODY_BYTES);
    }

    #[test]
    fn a_link_and_a_handle_together_are_refused() {
        let body = json!({
            "text": "x",
            "parts": [{ "kind": "attachment", "id": "a1", "name": "a.pdf", "handle": handle("bot/a.pdf"), "url": "https://h/a.pdf" }]
        });
        assert!(MessageBody::parse(&body).is_err());
        let neither = json!({ "text": "x", "parts": [{ "kind": "attachment", "id": "a1", "name": "a.pdf" }] });
        assert!(MessageBody::parse(&neither).is_err());
    }

    fn widget(bytes: usize, handle: Option<WorkspaceHandle>) -> ReplyWidget {
        ReplyWidget {
            summary: "Widget from Orders".into(),
            doc: app_widget_doc(
                &json!({ "app_id": "app", "board_id": "board", "event_id": "event" }),
                &json!({ "instance_id": "w", "component": { "blob": "x".repeat(bytes) } }),
            ),
            handle,
        }
    }

    fn ui_parts(body: &Value) -> Vec<(String, bool, bool)> {
        MessageBody::parse(body)
            .expect("valid body")
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Ui(ui) => {
                    Some((ui.id.clone(), ui.a2ui.is_some(), ui.handle.is_some()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn small_widgets_go_inline_and_large_ones_point_at_their_file() {
        let mut body = json!({ "text": "Here is the chart" });
        let file = WorkspaceHandle {
            mime: "application/json".into(),
            ..handle("chat-widgets/w.json")
        };
        attach_reply_widgets(
            &mut body,
            &[
                widget(100, None),
                widget(MAX_INLINE_WIDGET_BYTES, Some(file)),
                widget(MAX_INLINE_WIDGET_BYTES, None),
            ],
        );
        assert_eq!(
            ui_parts(&body),
            [("ui1".into(), true, false), ("ui2".into(), false, true)]
        );
        assert_eq!(body["parts"][0]["a2ui"]["type"], APP_WIDGET_TYPE);
        assert_eq!(body["parts"][0]["a2ui"]["origin"]["board_id"], "board");
    }

    #[test]
    fn widgets_stop_where_the_body_is_full() {
        let mut body = json!({ "text": "x".repeat(MAX_BODY_BYTES - 30_000) });
        attach_reply_widgets(&mut body, &[widget(20_000, None), widget(20_000, None)]);
        assert_eq!(ui_parts(&body).len(), 1);
        assert!(serde_json::to_string(&body).unwrap().len() <= MAX_BODY_BYTES);
    }

    #[test]
    fn a_widget_part_may_point_at_a_conversation_file() {
        let body = json!({ "text": "x", "parts": [{
            "kind": "ui", "id": "ui1", "summary": "Widget",
            "handle": { "app_id": "conversation_c1", "path": "f1.json", "bytes": 3, "sha256": "a".repeat(64), "mime": "application/json" },
        }] });
        assert!(MessageBody::parse(&body).is_ok());
    }
}

//! Files a bot attaches to its reply (`file_attach`): stored in its workspace or linked. Hosts put
//! them on the reply body as attachment parts before the tool trail, so the trail shrinks to fit.

use crate::bot_message::{AttachmentPart, MAX_PARTS, MessagePart, WorkspaceHandle};
use crate::bot_tool_trail::MAX_BODY_BYTES;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// At most this many files go with one reply.
pub const MAX_REPLY_FILES: usize = 8;

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
        assert!(matches!(&parsed.parts[0], MessagePart::Attachment(file) if file.id == "a1" && file.name == "c.pdf"));
    }

    #[test]
    fn a_reply_keeps_at_most_eight_files_and_the_body_limit() {
        let mut body = json!({ "text": "x" });
        let files: Vec<_> = (0..12)
            .map(|index| ReplyFile::linked(format!("{index}.pdf"), format!("https://h/{index}.pdf"), None))
            .collect();
        attach_reply_files(&mut body, &files);
        assert_eq!(body["parts"].as_array().map(Vec::len), Some(MAX_REPLY_FILES));

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
}

//! A bot shares a workspace file in its text as a Markdown link to
//! `storage://apps/{app}/app/{path}` (the bot tools' `markdown_url`). Only those who can read that
//! app can open it, so where others take part the host turns each link into an attachment:
//! [`workspace_links`] finds them and [`without_links`] takes them out of the text again.

use crate::bot_message::WorkspaceHandle;
use crate::bot_reply_files::{ReplyFile, attach_reply_files};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

const SCHEME: &str = "storage://apps/";
const DESTINATION: &str = "](storage://apps/";

/// A Markdown link or image whose destination is a file in an app's storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceLink {
    /// Where the whole `[label](url)` or `![alt](url)` sits in the text, in bytes.
    pub range: Range<usize>,
    pub image: bool,
    pub label: String,
    pub app_id: String,
    /// The path below the app's upload root, as a handle names it: segments decoded, joined
    /// with `/`.
    pub path: String,
}

impl WorkspaceLink {
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// The links and images in `text` that point into an app's storage, in order. Links inside code
/// are text, not links, and are left out.
pub fn workspace_links(text: &str) -> Vec<WorkspaceLink> {
    let code = code_ranges(text);
    let mut links = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find(DESTINATION) {
        let close = from + found;
        let url_start = close + 2;
        from = url_start;
        let Some(url_len) = text[url_start..].find(')') else {
            break;
        };
        let url_end = url_start + url_len;
        let Some(open) = label_start(text, close) else {
            continue;
        };
        let image = open > 0 && text.as_bytes()[open - 1] == b'!';
        let range = if image { open - 1 } else { open }..url_end + 1;
        if code
            .iter()
            .any(|code| code.start < range.end && range.start < code.end)
        {
            continue;
        }
        let Some((app_id, path)) = parse_url(&text[url_start..url_end]) else {
            continue;
        };
        links.push(WorkspaceLink {
            range,
            image,
            label: text[open + 1..close].to_string(),
            app_id,
            path,
        });
        from = url_end + 1;
    }
    links
}

/// The links in a message body's text that point into `workspace`.
pub fn body_links(body: &Value, workspace: &str) -> Vec<WorkspaceLink> {
    body.get("text")
        .and_then(Value::as_str)
        .map(workspace_links)
        .unwrap_or_default()
        .into_iter()
        .filter(|link| link.app_id == workspace)
        .collect()
}

/// Attaches the file of every link that `handles` (keyed by path) resolves, once per file, and
/// takes those links out of the body's text. A link whose file is missing or no longer fits the
/// body stays in the text. Returns how many files were attached.
pub fn attach_links(
    body: &mut Value,
    links: &[WorkspaceLink],
    handles: &BTreeMap<String, WorkspaceHandle>,
) -> usize {
    let mut attached = BTreeSet::new();
    for link in links {
        if attached.contains(&link.path) {
            continue;
        }
        let Some(handle) = handles.get(&link.path) else {
            continue;
        };
        let before = attachment_count(body);
        attach_reply_files(
            body,
            &[ReplyFile::stored(
                link.file_name().to_string(),
                handle.clone(),
            )],
        );
        if attachment_count(body) > before {
            attached.insert(link.path.clone());
        }
    }
    if attached.is_empty() {
        return 0;
    }
    let taken: Vec<WorkspaceLink> = links
        .iter()
        .filter(|link| attached.contains(&link.path))
        .cloned()
        .collect();
    if let Some(text) = body.get("text").and_then(Value::as_str) {
        body["text"] = Value::String(without_links(text, &taken));
    }
    attached.len()
}

fn attachment_count(body: &Value) -> usize {
    body.get("parts")
        .and_then(Value::as_array)
        .map_or(0, |parts| {
            parts
                .iter()
                .filter(|part| part.get("kind").and_then(Value::as_str) == Some("attachment"))
                .count()
        })
}

/// `text` with every link of `links` (found in this `text`) replaced by its label, or by the file
/// name when the label is empty. An image goes away entirely, with its line when it stood alone.
pub fn without_links(text: &str, links: &[WorkspaceLink]) -> String {
    let mut sorted: Vec<&WorkspaceLink> = links.iter().collect();
    sorted.sort_by_key(|link| link.range.start);
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for link in sorted {
        if link.range.start < cursor || link.range.end > text.len() {
            continue;
        }
        let mut start = link.range.start;
        let mut end = link.range.end;
        if link.image {
            let line_start = text[..start].rfind('\n').map_or(0, |at| at + 1);
            let alone_before = text[line_start.max(cursor)..start].trim().is_empty();
            let rest = &text[end..];
            let line_end = rest.find('\n').unwrap_or(rest.len());
            if alone_before && rest[..line_end].trim().is_empty() {
                start = line_start.max(cursor);
                end += (line_end + 1).min(rest.len());
            }
        }
        out.push_str(&text[cursor..start]);
        if !link.image {
            let label = link.label.trim();
            out.push_str(if label.is_empty() {
                link.file_name()
            } else {
                label
            });
        }
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    let trimmed = out.trim_end().len();
    out.truncate(trimmed);
    out
}

/// The `[` that opens the label closed at `close`, on the same line.
fn label_start(text: &str, close: usize) -> Option<usize> {
    let mut depth = 0usize;
    for at in (0..close).rev() {
        match text.as_bytes()[at] {
            b'\n' => return None,
            b']' => depth += 1,
            b'[' if depth == 0 => return Some(at),
            b'[' => depth -= 1,
            _ => {}
        }
    }
    None
}

fn parse_url(url: &str) -> Option<(String, String)> {
    let url = url.split_whitespace().next()?;
    let rest = url.strip_prefix(SCHEME)?;
    let (app, rest) = rest.split_once('/')?;
    let rest = rest.strip_prefix("app/")?;
    let app_id = decode(app).filter(|app| !app.is_empty() && !app.contains('/'))?;
    let segments = rest
        .split('/')
        .map(|segment| {
            decode(segment).filter(|segment| {
                !segment.is_empty()
                    && segment != "."
                    && segment != ".."
                    && !segment.contains('/')
                    && !segment.chars().any(char::is_control)
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some((app_id, segments.join("/")))
}

/// Percent-decodes `text`; `None` when the bytes are not UTF-8.
fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |offset: usize| {
            bytes
                .get(at + offset)
                .and_then(|byte| (*byte as char).to_digit(16))
        };
        match (bytes[at], hex(1), hex(2)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                at += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Fenced blocks and inline code spans, in bytes.
fn code_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut fence: Option<(usize, &str)> = None;
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let marker = line.trim_start();
        let opens = ["```", "~~~"]
            .into_iter()
            .find(|fence| marker.starts_with(fence));
        match (fence, opens) {
            (None, Some(marker)) => fence = Some((line_start, marker)),
            (Some((start, open)), Some(marker)) if marker == open => {
                ranges.push(start..line_start + line.len());
                fence = None;
            }
            _ => {}
        }
        line_start += line.len();
    }
    if let Some((start, _)) = fence {
        ranges.push(start..text.len());
    }
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if let Some(fenced) = ranges.iter().find(|range| range.contains(&at)) {
            at = fenced.end;
            continue;
        }
        if bytes[at] != b'`' {
            at += 1;
            continue;
        }
        let run = bytes[at..].iter().take_while(|byte| **byte == b'`').count();
        let open = at;
        at += run;
        let mut search = at;
        let mut closed = None;
        while search < bytes.len() {
            if bytes[search] == b'`' {
                let length = bytes[search..]
                    .iter()
                    .take_while(|byte| **byte == b'`')
                    .count();
                if length == run {
                    closed = Some(search + length);
                    break;
                }
                search += length;
            } else {
                search += 1;
            }
        }
        if let Some(end) = closed {
            ranges.push(open..end);
            at = end;
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links_and_images_with_decoded_paths() {
        let text = "Report: [Q3 report](storage://apps/ws1/app/bot/files/q3.pdf) and\n![chart](storage://apps/ws1/app/bot/files/%C3%9Cbersicht%20%282%29.png)";
        let links = workspace_links(text);
        assert_eq!(links.len(), 2);
        assert!(!links[0].image);
        assert_eq!(links[0].label, "Q3 report");
        assert_eq!(links[0].app_id, "ws1");
        assert_eq!(links[0].path, "bot/files/q3.pdf");
        assert_eq!(
            &text[links[0].range.clone()],
            "[Q3 report](storage://apps/ws1/app/bot/files/q3.pdf)"
        );
        assert!(links[1].image);
        assert_eq!(links[1].path, "bot/files/Übersicht (2).png");
        assert_eq!(links[1].file_name(), "Übersicht (2).png");
    }

    #[test]
    fn other_destinations_code_and_bad_paths_are_not_links() {
        let text = "[web](https://example.com/a.pdf) `[x](storage://apps/ws1/app/a.pdf)`\n```\n[y](storage://apps/ws1/app/b.pdf)\n```\n[up](storage://apps/ws1/app/bot/../secret.pdf) [raw](storage://apps/ws1/upload/a.pdf)";
        assert!(workspace_links(text).is_empty());
    }

    #[test]
    fn a_label_may_hold_brackets_and_a_destination_a_title() {
        let text = "See [the [final] deck](storage://apps/ws1/app/deck.pptx \"Deck\").";
        let links = workspace_links(text);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].label, "the [final] deck");
        assert_eq!(links[0].path, "deck.pptx");
    }

    #[test]
    fn links_become_their_labels_and_lone_images_leave_with_their_line() {
        let text = "Here it is: [the offer](storage://apps/ws1/app/offer.pdf).\n\n![](storage://apps/ws1/app/chart.png)\n\nInline ![c](storage://apps/ws1/app/c.png) too, [](storage://apps/ws1/app/notes.md)\n";
        let links = workspace_links(text);
        assert_eq!(links.len(), 4);
        assert_eq!(
            without_links(text, &links),
            "Here it is: the offer.\n\n\nInline  too, notes.md"
        );
    }

    #[test]
    fn resolved_links_become_attachments_once_and_leave_the_text() {
        let mut body = serde_json::json!({
            "text": "[Offer](storage://apps/ws1/app/bot/files/offer.pdf), again [here](storage://apps/ws1/app/bot/files/offer.pdf), [gone](storage://apps/ws1/app/bot/files/gone.pdf), [other](storage://apps/ws2/app/x.pdf)",
            "parts": [{ "kind": "attachment", "id": "a1", "name": "a.txt", "url": "https://files.example.com/a.txt" }]
        });
        let links = body_links(&body, "ws1");
        assert_eq!(links.len(), 3);
        let handle = WorkspaceHandle {
            app_id: "ws1".to_string(),
            path: "bot/files/offer.pdf".to_string(),
            bytes: 2048,
            sha256: "a".repeat(64),
            mime: "application/pdf".to_string(),
        };
        let handles = BTreeMap::from([(handle.path.clone(), handle.clone())]);
        assert_eq!(attach_links(&mut body, &links, &handles), 1);
        assert_eq!(
            body["text"],
            "Offer, again here, [gone](storage://apps/ws1/app/bot/files/gone.pdf), [other](storage://apps/ws2/app/x.pdf)"
        );
        let parts = body["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1]["id"], "a2");
        assert_eq!(parts[1]["name"], "offer.pdf");
        assert_eq!(parts[1]["handle"]["path"], "bot/files/offer.pdf");
    }

    #[test]
    fn only_the_given_links_change() {
        let text = "[a](storage://apps/ws1/app/a.pdf) [b](storage://apps/ws2/app/b.pdf)";
        let links: Vec<_> = workspace_links(text)
            .into_iter()
            .filter(|link| link.app_id == "ws1")
            .collect();
        assert_eq!(
            without_links(text, &links),
            "a [b](storage://apps/ws2/app/b.pdf)"
        );
    }
}

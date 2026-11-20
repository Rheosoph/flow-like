//! WebDAV and CalDAV XML: the bodies this client sends and a reader for `multistatus` answers
//! (RFC 4918 §13, RFC 4791 §7–9). Read-only DOM parsing; documents with a DTD are refused.

use chrono::{DateTime, Utc};
use roxmltree::{Document, Node};

pub const DAV: &str = "DAV:";
pub const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
pub const CALENDARSERVER: &str = "http://calendarserver.org/ns/";
pub const APPLE: &str = "http://apple.com/ns/ical/";
const SABRE: &str = "http://sabredav.org/ns";

/// Depth 0 on a context path or principal: who signed in and where their calendars are.
pub const PRINCIPAL_QUERY: &str = concat!(
    r#"<?xml version="1.0" encoding="utf-8"?>"#,
    r#"<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">"#,
    r#"<d:prop><d:current-user-principal/><d:resourcetype/><c:calendar-home-set/></d:prop>"#,
    r#"</d:propfind>"#,
);

/// Depth 1 on a calendar home (or Depth 0 on one calendar): what each collection is.
pub const CALENDARS_QUERY: &str = concat!(
    r#"<?xml version="1.0" encoding="utf-8"?>"#,
    r#"<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" "#,
    r#"xmlns:cs="http://calendarserver.org/ns/" xmlns:a="http://apple.com/ns/ical/">"#,
    r#"<d:prop><d:resourcetype/><d:displayname/><c:supported-calendar-component-set/>"#,
    r#"<a:calendar-color/><cs:getctag/><d:sync-token/><d:current-user-privilege-set/>"#,
    r#"<c:calendar-timezone/></d:prop>"#,
    r#"</d:propfind>"#,
);

fn stamp(time: DateTime<Utc>) -> String {
    time.format("%Y%m%dT%H%M%SZ").to_string()
}

/// `calendar-query` for the VEVENTs overlapping `[from, to)`. With `expand` the server answers
/// each occurrence of a repeating event as its own VEVENT with a RECURRENCE-ID, in UTC.
pub fn events_query(from: DateTime<Utc>, to: DateTime<Utc>, expand: bool) -> String {
    let (start, end) = (stamp(from), stamp(to));
    let data = if expand {
        format!(r#"<c:calendar-data><c:expand start="{start}" end="{end}"/></c:calendar-data>"#)
    } else {
        "<c:calendar-data/>".to_string()
    };
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">"#,
            r#"<d:prop><d:getetag/>{data}</d:prop>"#,
            r#"<c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT">"#,
            r#"<c:time-range start="{start}" end="{end}"/>"#,
            r#"</c:comp-filter></c:comp-filter></c:filter>"#,
            r#"</c:calendar-query>"#,
        ),
        data = data,
        start = start,
        end = end,
    )
}

/// An element somewhere inside a property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub ns: String,
    pub name: String,
    /// The `name` attribute (`<c:comp name="VEVENT"/>`).
    pub name_attr: Option<String>,
}

/// One property of a `propstat` that answered 2xx.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prop {
    pub ns: String,
    pub name: String,
    /// All text inside, as sent.
    pub text: String,
    /// Every `DAV:href` inside.
    pub hrefs: Vec<String>,
    pub elements: Vec<Element>,
}

impl Prop {
    pub fn has(&self, ns: &str, name: &str) -> bool {
        self.elements
            .iter()
            .any(|element| element.ns == ns && element.name == name)
    }

    pub fn trimmed(&self) -> Option<&str> {
        Some(self.text.trim()).filter(|text| !text.is_empty())
    }
}

/// One `response` of a multistatus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub href: String,
    /// The response's own status, set when it carries no properties (a 404 for a gone resource).
    pub status: Option<u16>,
    pub props: Vec<Prop>,
}

impl Entry {
    pub fn prop(&self, ns: &str, name: &str) -> Option<&Prop> {
        self.props
            .iter()
            .find(|prop| prop.ns == ns && prop.name == name)
    }
}

fn is(node: &Node, ns: &str, name: &str) -> bool {
    node.is_element() && node.tag_name().namespace() == Some(ns) && node.tag_name().name() == name
}

fn child<'a, 'input>(node: &Node<'a, 'input>, ns: &str, name: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|child| is(child, ns, name))
}

fn text_of(node: &Node) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|text| text.text())
        .collect()
}

/// The code of a status line such as `HTTP/1.1 200 OK`.
pub fn status_code(line: &str) -> Option<u16> {
    line.split_whitespace().nth(1)?.parse().ok()
}

fn prop_of(node: Node) -> Prop {
    Prop {
        ns: node.tag_name().namespace().unwrap_or_default().to_string(),
        name: node.tag_name().name().to_string(),
        text: text_of(&node),
        hrefs: node
            .descendants()
            .filter(|inner| is(inner, DAV, "href"))
            .map(|href| text_of(&href).trim().to_string())
            .filter(|href| !href.is_empty())
            .collect(),
        elements: node
            .descendants()
            .skip(1)
            .filter(Node::is_element)
            .map(|inner| Element {
                ns: inner.tag_name().namespace().unwrap_or_default().to_string(),
                name: inner.tag_name().name().to_string(),
                name_attr: inner.attribute("name").map(str::to_string),
            })
            .collect(),
    }
}

fn entry_of(response: Node) -> Option<Entry> {
    let href = text_of(&child(&response, DAV, "href")?).trim().to_string();
    let status = child(&response, DAV, "status").and_then(|node| status_code(&text_of(&node)));
    let mut props = Vec::new();
    for propstat in response.children().filter(|node| is(node, DAV, "propstat")) {
        let ok = child(&propstat, DAV, "status")
            .and_then(|node| status_code(&text_of(&node)))
            .is_none_or(|code| (200..300).contains(&code));
        if !ok {
            continue;
        }
        if let Some(prop) = child(&propstat, DAV, "prop") {
            props.extend(prop.children().filter(Node::is_element).map(prop_of));
        }
    }
    Some(Entry {
        href,
        status,
        props,
    })
}

/// The responses of a `207 Multi-Status` body.
pub fn multistatus(text: &str) -> Result<Vec<Entry>, String> {
    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("the answer is not valid XML: {error}"))?;
    let root = document.root_element();
    if !is(&root, DAV, "multistatus") {
        return Err(format!(
            "the answer is a '{}' document, not a WebDAV multistatus",
            root.tag_name().name()
        ));
    }
    Ok(root
        .children()
        .filter(|node| is(node, DAV, "response"))
        .filter_map(entry_of)
        .collect())
}

/// What a `DAV:error` body says: SabreDAV's message, else the failed preconditions.
pub fn error_message(text: &str) -> Option<String> {
    let document = Document::parse(text.trim()).ok()?;
    let root = document.root_element();
    if !is(&root, DAV, "error") {
        return None;
    }
    if let Some(message) = child(&root, SABRE, "message") {
        let message = text_of(&message).trim().to_string();
        if !message.is_empty() {
            return Some(message);
        }
    }
    let conditions: Vec<&str> = root
        .children()
        .filter(Node::is_element)
        .filter(|node| node.tag_name().namespace() != Some(SABRE))
        .map(|node| node.tag_name().name())
        .collect();
    (!conditions.is_empty()).then(|| conditions.join(", "))
}

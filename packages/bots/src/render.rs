//! Text for the chat: Markdown → Telegram HTML, the plan as a Telegram text box or a Discord
//! embed, links, and cutting and splitting on character boundaries.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::reply::{Link, Plan};

/// Embed fields of a plan beyond the current step.
const UPCOMING_SHOWN: usize = 2;
const DONE_SHOWN: usize = 2;
const STEP_CHARS: usize = 200;
const SHORT_STEP_CHARS: usize = 60;
const MESSAGE_CHARS: usize = 150;
const EMBED_FIELD_CHARS: usize = 1_024;

/// At most `limit` characters of `text`, cut on a character boundary.
pub fn cut(text: &str, limit: usize) -> &str {
    match text.char_indices().nth(limit) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

/// `text` in at most `limit` characters, ending in "…" when it was longer.
pub fn shorten(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut short = cut(text, limit.saturating_sub(1)).to_string();
    short.push('…');
    short
}

/// The length the providers count: UTF-16 code units, at least the number of characters.
pub fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// Splits `text` into parts of at most `limit` UTF-16 code units, never inside a character. A
/// part ends after a line break or a space when one is in its second half. The parts joined
/// are `text`.
pub fn split(text: &str, limit: usize) -> Vec<String> {
    let limit = limit.max(2);
    let mut parts = Vec::new();
    let mut rest = text;
    while utf16_len(rest) > limit {
        let mut end = 0;
        let mut units = 0;
        for (index, character) in rest.char_indices() {
            if units + character.len_utf16() > limit {
                break;
            }
            units += character.len_utf16();
            end = index + character.len_utf8();
        }
        let window = &rest[..end];
        let half = window.len() / 2;
        let at = window
            .rfind('\n')
            .filter(|at| *at >= half)
            .or_else(|| window.rfind(' ').filter(|at| *at >= half))
            .map_or(end, |at| at + 1);
        parts.push(rest[..at].to_string());
        rest = &rest[at..];
    }
    if !rest.is_empty() || parts.is_empty() {
        parts.push(rest.to_string());
    }
    parts
}

/// Escapes text for Telegram HTML.
pub fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// Markdown as the HTML subset Telegram reads.
pub fn telegram_html(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let mut html = String::with_capacity(markdown.len() + 32);
    for event in Parser::new_ext(markdown, options) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { .. } | Tag::Strong => html.push_str("<b>"),
                Tag::CodeBlock(CodeBlockKind::Fenced(_) | CodeBlockKind::Indented) => {
                    html.push_str("<pre>")
                }
                Tag::Item => html.push_str("• "),
                Tag::Emphasis => html.push_str("<i>"),
                Tag::Strikethrough => html.push_str("<s>"),
                Tag::BlockQuote(_) => html.push_str("<blockquote>"),
                Tag::Link { dest_url, .. } => {
                    html.push_str("<a href=\"");
                    html.push_str(&escape_html(&dest_url));
                    html.push_str("\">");
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph | TagEnd::List(_) | TagEnd::Item => html.push('\n'),
                TagEnd::Heading(_) => html.push_str("</b>\n"),
                TagEnd::CodeBlock => html.push_str("</pre>\n"),
                TagEnd::Emphasis => html.push_str("</i>"),
                TagEnd::Strong => html.push_str("</b>"),
                TagEnd::Strikethrough => html.push_str("</s>"),
                TagEnd::BlockQuote(_) => html.push_str("</blockquote>\n"),
                TagEnd::Link => html.push_str("</a>"),
                _ => {}
            },
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                html.push_str(&escape_html(&text))
            }
            Event::Code(code) => {
                html.push_str("<code>");
                html.push_str(&escape_html(&code));
                html.push_str("</code>");
            }
            Event::SoftBreak => html.push(' '),
            Event::HardBreak | Event::Rule => html.push('\n'),
            _ => {}
        }
    }
    html.trim().to_string()
}

fn mark(plan: &Plan, step: u32) -> &'static str {
    if step < plan.current_step {
        "✅"
    } else if step == plan.current_step {
        "🔄"
    } else {
        "⏳"
    }
}

/// The plan as a Telegram HTML text box; empty when there is no plan.
pub fn plan_telegram(plan: &Plan) -> String {
    if plan.is_empty() {
        return String::new();
    }
    let mut text = String::from("┌─ 🧠 <b>Thinking</b> ─────────\n");
    for (step, step_text) in &plan.steps {
        text.push_str(&format!(
            "│ {} <b>{step}</b>: {}\n",
            mark(plan, *step),
            escape_html(&shorten(step_text, STEP_CHARS))
        ));
        if *step == plan.current_step && !plan.current_message.is_empty() {
            text.push_str(&format!(
                "│    <i>{}</i>\n",
                escape_html(&shorten(&plan.current_message, MESSAGE_CHARS))
            ));
        }
    }
    text.push_str("└────────────────────");
    text
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbedField {
    pub name: String,
    pub value: String,
}

/// A Discord embed in the provider's terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Embed {
    pub title: String,
    /// `0xRRGGBB`.
    pub colour: u32,
    pub fields: Vec<EmbedField>,
}

fn field(name: String, value: String) -> EmbedField {
    EmbedField {
        name,
        value: shorten(&value, EMBED_FIELD_CHARS),
    }
}

/// The plan as a Discord embed within Discord's limits; `None` when there is no plan.
pub fn plan_discord(plan: &Plan) -> Option<Embed> {
    if plan.is_empty() {
        return None;
    }
    let mut fields = Vec::new();
    let done: Vec<&(u32, String)> = plan
        .steps
        .iter()
        .filter(|(step, _)| *step < plan.current_step)
        .collect();
    if done.len() > DONE_SHOWN {
        fields.push(field(
            format!("✅ {} steps completed", done.len()),
            "─".repeat(20),
        ));
    } else {
        for (step, text) in done {
            fields.push(field(
                format!("✅ Step {step}"),
                shorten(text, SHORT_STEP_CHARS),
            ));
        }
    }
    for (step, text) in plan
        .steps
        .iter()
        .filter(|(step, _)| *step == plan.current_step)
    {
        let mut value = shorten(text, STEP_CHARS);
        if !plan.current_message.is_empty() {
            value.push_str(&format!(
                "\n> *{}*",
                shorten(&plan.current_message, MESSAGE_CHARS)
            ));
        }
        fields.push(field(format!("🔄 Step {step}"), value));
    }
    let upcoming: Vec<&(u32, String)> = plan
        .steps
        .iter()
        .filter(|(step, _)| *step > plan.current_step)
        .collect();
    for (step, text) in upcoming.iter().take(UPCOMING_SHOWN) {
        fields.push(field(
            format!("⏳ Step {step}"),
            shorten(text, SHORT_STEP_CHARS),
        ));
    }
    if upcoming.len() > UPCOMING_SHOWN {
        fields.push(field(
            format!("⏳ +{} more steps", upcoming.len() - UPCOMING_SHOWN),
            "─".repeat(10),
        ));
    }
    Some(Embed {
        title: "🧠 Thinking...".to_string(),
        colour: 0xFF_BF_00,
        fields,
    })
}

/// The answer's files as a Markdown list; empty without links.
pub fn links_markdown(links: &[Link]) -> String {
    if links.is_empty() {
        return String::new();
    }
    let mut text = String::from("📎 Attachments:");
    for link in links {
        let url = link.url.replace(' ', "%20").replace(')', "%29");
        match &link.name {
            Some(name) => text.push_str(&format!("\n- [{}]({url})", name.replace(['[', ']'], ""))),
            None => text.push_str(&format!("\n- {url}")),
        }
    }
    text
}

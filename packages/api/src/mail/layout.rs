//! The one look of every email the platform sends. An email is a list of blocks, and the same
//! blocks render its HTML part and its plain-text part, so a client without HTML reads all of
//! it. The HTML is a light card that turns dark where the client follows the system theme
//! (Apple Mail, Outlook for Mac and iOS, Outlook.com), built from tables and inline styles so
//! Outlook for Windows and Gmail keep it intact, with no images and no web fonts: nothing is
//! fetched when it opens, and nothing breaks when images are blocked.

const FONT: &str =
    "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif";
const MONO: &str = "'SFMono-Regular', Menlo, Consolas, 'Liberation Mono', monospace";
const CONTACT: &str = "help@great-co.de";
const COMPANY: &str = "Flow-Like by Rheosoph GmbH";

const PAGE: &str = "#f3f3f7";
const CARD: &str = "#ffffff";
const BORDER: &str = "#e4e4ea";
const PANEL: &str = "#f7f7fa";
const PANEL_BORDER: &str = "#e9e9f0";
const INK: &str = "#16161d";
const BODY: &str = "#33333d";
const MUTED: &str = "#5c5c68";
const SUBTLE: &str = "#7c7c8a";
const ACCENT: &str = "#4f46e5";
const BRAND: &str = "#6366f1";

/// Light colors are inline; the stylesheet swaps in the dark ones for clients that follow the
/// system theme. `[data-ogsc]`/`[data-ogsb]` are the selectors Outlook.com sets in dark mode.
const STYLE: &str = r#"
:root { color-scheme: light dark; supported-color-schemes: light dark; }
body, table, td, a { -webkit-text-size-adjust: 100%; -ms-text-size-adjust: 100%; }
table, td { mso-table-lspace: 0pt; mso-table-rspace: 0pt; }
a[x-apple-data-detectors] { color: inherit !important; text-decoration: none !important; }
@media only screen and (max-width: 620px) {
  .fl-pad { padding-left: 24px !important; padding-right: 24px !important; }
  .fl-title { font-size: 24px !important; line-height: 30px !important; }
  .fl-label, .fl-value { display: block !important; width: 100% !important; box-sizing: border-box; }
  .fl-label { padding: 10px 0 0 0 !important; }
  .fl-value { padding: 2px 0 10px 0 !important; text-align: left !important; border-top: 0 !important; }
}
@media (prefers-color-scheme: dark) {
  .fl-page { background-color: #0c0c11 !important; }
  .fl-card { background-color: #16161e !important; border-color: #2a2a35 !important; }
  .fl-panel { background-color: #1e1e28 !important; border-top-color: #2e2e3a !important; border-right-color: #2e2e3a !important; border-bottom-color: #2e2e3a !important; }
  .fl-ink { color: #f2f2f6 !important; }
  .fl-body { color: #d6d6de !important; }
  .fl-muted { color: #a7a7b5 !important; }
  .fl-subtle { color: #868696 !important; }
  .fl-rule { border-color: #2e2e3a !important; }
  .fl-link { color: #a5b4fc !important; }
  .fl-code { background-color: #23233a !important; color: #c7d2fe !important; }
  .fl-outline { border-color: #3a3a48 !important; }
}
[data-ogsc] .fl-ink { color: #f2f2f6 !important; }
[data-ogsc] .fl-body { color: #d6d6de !important; }
[data-ogsc] .fl-muted { color: #a7a7b5 !important; }
[data-ogsc] .fl-subtle { color: #868696 !important; }
[data-ogsc] .fl-link { color: #a5b4fc !important; }
[data-ogsb] .fl-page { background-color: #0c0c11 !important; }
[data-ogsb] .fl-card { background-color: #16161e !important; }
[data-ogsb] .fl-panel { background-color: #1e1e28 !important; }
"#;

/// Keeps the inbox preview to the preview line instead of the start of the body.
const PREVIEW_FILLER: &str = "&#847;&zwnj;&nbsp;";

/// The color of a badge or a callout's edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

impl Tone {
    /// Badge background, badge text, edge.
    fn colors(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Neutral => ("#eeeef3", "#3f3f4a", "#a1a1ae"),
            Self::Info => ("#e9ebff", "#3730a3", BRAND),
            Self::Success => ("#e1f6ec", "#0f6a45", "#10b981"),
            Self::Warning => ("#fff3d6", "#7a4f00", "#f59e0b"),
            Self::Danger => ("#fde7e7", "#991b1b", "#ef4444"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Badge {
    pub tone: Tone,
    pub label: String,
}

impl Badge {
    pub(crate) fn new(tone: Tone, label: impl Into<String>) -> Self {
        Self {
            tone,
            label: label.into(),
        }
    }

    fn html(&self) -> String {
        let (background, color, _) = self.tone.colors();
        format!(
            r#"<span style="display:inline-block;padding:3px 10px;border-radius:999px;background-color:{background};color:{color};font-family:{FONT};font-size:12px;line-height:18px;font-weight:600;white-space:nowrap;">{}</span>"#,
            escape(&self.label)
        )
    }
}

/// A piece of a paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Span {
    Plain(String),
    Strong(String),
}

pub(crate) fn plain(text: impl Into<String>) -> Span {
    Span::Plain(text.into())
}

pub(crate) fn strong(text: impl Into<String>) -> Span {
    Span::Strong(text.into())
}

/// The value of a row in [`Block::Details`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Value {
    Text(String),
    Code(String),
    Badge(Badge),
}

impl Value {
    fn html(&self) -> String {
        match self {
            Self::Text(text) => escape(text),
            Self::Code(code) => format!(
                r#"<span class="fl-code" style="display:inline-block;padding:2px 8px;border-radius:6px;background-color:#eef0ff;color:#312e81;font-family:{MONO};font-size:13px;line-height:20px;">{}</span>"#,
                escape(code)
            ),
            Self::Badge(badge) => badge.html(),
        }
    }

    fn text(&self) -> &str {
        match self {
            Self::Text(text) | Self::Code(text) => text,
            Self::Badge(badge) => &badge.label,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Block {
    /// The opening: a short label, the headline and a line under it.
    Hero {
        eyebrow: Option<String>,
        title: String,
        subtitle: Option<String>,
    },
    Text(Vec<Span>),
    /// Words set apart: a reason, a reviewer's comment, a notice. Line breaks are kept.
    Callout {
        tone: Tone,
        label: String,
        body: String,
    },
    Details {
        title: Option<String>,
        rows: Vec<(String, Value)>,
    },
    /// A state that changed, from one badge to the other.
    Change {
        label: String,
        from: Badge,
        to: Badge,
    },
    /// Numbered steps, each a title and a line.
    Steps {
        title: String,
        steps: Vec<(String, String)>,
    },
    /// The main action, with its link written out below for clients that drop the button.
    Button {
        label: String,
        url: String,
    },
    SecondaryButton {
        label: String,
        url: String,
    },
    /// A one-time code, large and easy to copy.
    Code(String),
    Note(String),
}

/// An email ready to render: every block in order, the inbox preview line and the footer line
/// saying why the reader gets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Email {
    pub preview: String,
    pub footer: String,
    pub blocks: Vec<Block>,
}

pub(crate) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Escaped text whose line breaks survive clients that ignore `white-space`.
fn escape_lines(value: &str) -> String {
    escape(value.trim())
        .replace("\r\n", "\n")
        .replace('\n', "<br>")
}

/// `text` on one line of at most `max_chars` characters: control characters and runs of
/// whitespace become single spaces, and an overlong line ends in an ellipsis.
pub(crate) fn one_line(text: &str, max_chars: usize) -> String {
    let collapsed = text
        .split(|c: char| c.is_whitespace() || c.is_control())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let kept: String = collapsed
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect();
    format!("{}…", kept.trim_end())
}

fn font(size: u32, line: u32, weight: u32) -> String {
    format!("font-family:{FONT};font-size:{size}px;line-height:{line}px;font-weight:{weight};")
}

/// One row of the card; `top` and `bottom` are its vertical padding.
fn row(top: u32, bottom: u32, content: &str) -> String {
    format!(
        r#"<tr><td class="fl-pad" style="padding:{top}px 40px {bottom}px 40px;">{content}</td></tr>"#
    )
}

/// A small uppercase label; `class` picks its dark-mode color.
fn label_html(label: &str, color: &str, class: &str) -> String {
    format!(
        r#"<p class="{class}" style="margin:0 0 6px 0;{}letter-spacing:0.6px;text-transform:uppercase;color:{color};">{}</p>"#,
        font(12, 16, 600),
        escape(label)
    )
}

fn spans_html(spans: &[Span]) -> String {
    spans
        .iter()
        .map(|span| match span {
            Span::Plain(text) => escape(text),
            Span::Strong(text) => format!(
                r#"<strong class="fl-ink" style="font-weight:600;color:{INK};">{}</strong>"#,
                escape(text)
            ),
        })
        .collect()
}

fn spans_text(spans: &[Span]) -> String {
    spans
        .iter()
        .map(|span| match span {
            Span::Plain(text) | Span::Strong(text) => text.as_str(),
        })
        .collect()
}

/// A tinted box; an `edge` color marks its left side and keeps it in dark mode.
fn panel(edge: Option<&str>, content: &str) -> String {
    let (class, edge) = match edge {
        Some(color) => ("fl-panel", format!("border-left:3px solid {color};")),
        None => ("fl-panel fl-rule", String::new()),
    };
    format!(
        r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="width:100%;"><tr><td class="{class}" bgcolor="{PANEL}" style="background-color:{PANEL};border:1px solid {PANEL_BORDER};{edge}border-radius:10px;padding:16px 18px;">{content}</td></tr></table>"#
    )
}

fn button_html(label: &str, url: &str, primary: bool) -> String {
    let url = escape(url);
    let label = escape(label);
    if primary {
        format!(
            r#"<table role="presentation" cellpadding="0" cellspacing="0" border="0"><tr><td align="center" bgcolor="{ACCENT}" style="border-radius:10px;background-color:{ACCENT};background-image:linear-gradient(135deg,#2563eb,#7c3aed);mso-padding-alt:14px 28px;"><a href="{url}" target="_blank" rel="noopener" style="display:inline-block;padding:14px 28px;{}color:#ffffff;text-decoration:none;border-radius:10px;">{label}</a></td></tr></table>"#,
            font(15, 20, 600)
        )
    } else {
        format!(
            r#"<table role="presentation" cellpadding="0" cellspacing="0" border="0"><tr><td class="fl-outline" align="center" style="border:1px solid #d4d4de;border-radius:10px;mso-padding-alt:11px 22px;"><a class="fl-ink" href="{url}" target="_blank" rel="noopener" style="display:inline-block;padding:11px 22px;{}color:{INK};text-decoration:none;border-radius:10px;">{label}</a></td></tr></table>"#,
            font(14, 20, 600)
        )
    }
}

impl Block {
    fn html(&self) -> String {
        match self {
            Self::Hero {
                eyebrow,
                title,
                subtitle,
            } => {
                let eyebrow = eyebrow
                    .as_deref()
                    .map(|eyebrow| label_html(eyebrow, ACCENT, "fl-link"))
                    .unwrap_or_default();
                let subtitle = subtitle
                    .as_deref()
                    .map(|subtitle| {
                        format!(
                            r#"<p class="fl-muted" style="margin:8px 0 0 0;{}color:{MUTED};">{}</p>"#,
                            font(16, 24, 400),
                            escape(subtitle)
                        )
                    })
                    .unwrap_or_default();
                row(
                    36,
                    24,
                    &format!(
                        r#"{eyebrow}<h1 class="fl-ink fl-title" style="margin:0;{}letter-spacing:-0.4px;color:{INK};">{}</h1>{subtitle}"#,
                        font(26, 32, 700),
                        escape(title)
                    ),
                )
            }
            Self::Text(spans) => row(
                0,
                20,
                &format!(
                    r#"<p class="fl-body" style="margin:0;{}color:{BODY};">{}</p>"#,
                    font(15, 24, 400),
                    spans_html(spans)
                ),
            ),
            Self::Callout { tone, label, body } => {
                let (_, label_color, edge) = tone.colors();
                row(
                    0,
                    20,
                    &panel(
                        Some(edge),
                        &format!(
                            r#"{}<p class="fl-body" style="margin:0;{}color:{BODY};">{}</p>"#,
                            label_html(label, label_color, "fl-subtle"),
                            font(15, 24, 400),
                            escape_lines(body)
                        ),
                    ),
                )
            }
            Self::Details { title, rows } => {
                let title = title
                    .as_deref()
                    .map(|title| {
                        format!(
                            r#"<tr><td colspan="2" style="padding:0 0 4px 0;">{}</td></tr>"#,
                            label_html(title, SUBTLE, "fl-subtle")
                        )
                    })
                    .unwrap_or_default();
                let lines: String = rows
                    .iter()
                    .enumerate()
                    .map(|(index, (label, value))| {
                        let rule = if index == 0 {
                            String::new()
                        } else {
                            format!("border-top:1px solid {PANEL_BORDER};")
                        };
                        format!(
                            r#"<tr><td class="fl-label fl-muted fl-rule" valign="top" style="padding:8px 12px 8px 0;{rule}{}color:{MUTED};">{}</td><td class="fl-value fl-ink fl-rule" align="right" valign="top" style="padding:8px 0;{rule}{}color:{INK};text-align:right;word-break:break-word;">{}</td></tr>"#,
                            font(14, 20, 400),
                            escape(label),
                            font(14, 20, 600),
                            value.html()
                        )
                    })
                    .collect();
                row(
                    0,
                    20,
                    &panel(
                        None,
                        &format!(
                            r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="width:100%;">{title}{lines}</table>"#
                        ),
                    ),
                )
            }
            Self::Change { label, from, to } => row(
                0,
                20,
                &panel(
                    None,
                    &format!(
                        r#"{}<p style="margin:0;{}">{} <span class="fl-subtle" style="color:{SUBTLE};">&rarr;</span> {}</p>"#,
                        label_html(label, SUBTLE, "fl-subtle"),
                        font(14, 24, 400),
                        from.html(),
                        to.html()
                    ),
                ),
            ),
            Self::Steps { title, steps } => {
                let items: String = steps
                    .iter()
                    .enumerate()
                    .map(|(index, (title, line))| {
                        format!(
                            r#"<tr><td width="36" valign="top" style="width:36px;padding:0 0 14px 0;"><span class="fl-code" style="display:inline-block;width:24px;height:24px;border-radius:12px;background-color:#eef0ff;color:{ACCENT};{}text-align:center;">{}</span></td><td valign="top" style="padding:0 0 14px 0;"><p class="fl-ink" style="margin:0;{}color:{INK};">{}</p><p class="fl-muted" style="margin:2px 0 0 0;{}color:{MUTED};">{}</p></td></tr>"#,
                            font(12, 24, 700),
                            index + 1,
                            font(14, 20, 600),
                            escape(title),
                            font(14, 20, 400),
                            escape(line)
                        )
                    })
                    .collect();
                row(
                    0,
                    6,
                    &format!(
                        r#"{}<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="width:100%;">{items}</table>"#,
                        label_html(title, SUBTLE, "fl-subtle")
                    ),
                )
            }
            Self::Button { label, url } => row(
                4,
                24,
                &format!(
                    r#"{}<p class="fl-subtle" style="margin:12px 0 0 0;{}color:{SUBTLE};word-break:break-all;">If the button doesn’t work, open this link:<br><a class="fl-link" href="{url}" target="_blank" rel="noopener" style="color:{ACCENT};text-decoration:underline;">{url}</a></p>"#,
                    button_html(label, url, true),
                    font(12, 18, 400),
                    url = escape(url)
                ),
            ),
            Self::SecondaryButton { label, url } => row(0, 24, &button_html(label, url, false)),
            Self::Code(code) => row(
                4,
                24,
                &format!(
                    r##"<table role="presentation" cellpadding="0" cellspacing="0" border="0"><tr><td class="fl-code" bgcolor="#eef0ff" style="background-color:#eef0ff;border-radius:12px;padding:16px 24px;"><span class="fl-code" style="font-family:{MONO};font-size:30px;line-height:36px;font-weight:700;letter-spacing:6px;color:#312e81;">{}</span></td></tr></table>"##,
                    escape(code)
                ),
            ),
            Self::Note(note) => row(
                0,
                16,
                &format!(
                    r#"<p class="fl-muted" style="margin:0;{}color:{MUTED};">{}</p>"#,
                    font(13, 20, 400),
                    escape_lines(note)
                ),
            ),
        }
    }

    fn text(&self) -> String {
        match self {
            Self::Hero {
                eyebrow,
                title,
                subtitle,
            } => [
                eyebrow.as_deref().map(str::to_uppercase),
                Some(title.clone()),
                subtitle.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n"),
            Self::Text(spans) => spans_text(spans),
            Self::Callout { label, body, .. } => {
                format!("{}\n{}", label.to_uppercase(), body.trim())
            }
            Self::Details { title, rows } => title
                .as_deref()
                .map(str::to_uppercase)
                .into_iter()
                .chain(
                    rows.iter()
                        .map(|(label, value)| format!("{label}: {}", value.text())),
                )
                .collect::<Vec<_>>()
                .join("\n"),
            Self::Change { label, from, to } => {
                format!("{label}: {} → {}", from.label, to.label)
            }
            Self::Steps { title, steps } => std::iter::once(title.to_uppercase())
                .chain(
                    steps
                        .iter()
                        .enumerate()
                        .map(|(index, (title, line))| format!("{}. {title}: {line}", index + 1)),
                )
                .collect::<Vec<_>>()
                .join("\n"),
            Self::Button { label, url } | Self::SecondaryButton { label, url } => {
                format!("{label}:\n{url}")
            }
            Self::Code(code) => code.clone(),
            Self::Note(note) => note.trim().to_string(),
        }
    }
}

impl Email {
    fn title(&self) -> &str {
        self.blocks
            .iter()
            .find_map(|block| match block {
                Block::Hero { title, .. } => Some(title.as_str()),
                _ => None,
            })
            .unwrap_or("Flow-Like")
    }

    pub(crate) fn html(&self) -> String {
        let title = escape(self.title());
        let preview = format!("{}{}", escape(&self.preview), PREVIEW_FILLER.repeat(40));
        let body: String = self.blocks.iter().map(Block::html).collect();
        let wordmark = format!(
            r#"<span class="fl-ink" style="{}letter-spacing:-0.2px;color:{INK};"><span style="color:{BRAND};">&#10022;</span>&nbsp;Flow-Like</span>"#,
            font(17, 24, 700)
        );
        let footer = format!(
            r#"<p class="fl-subtle" style="margin:0 0 6px 0;{small}color:{SUBTLE};">{}</p><p class="fl-subtle" style="margin:0 0 6px 0;{small}color:{SUBTLE};">Questions? Write to <a class="fl-link" href="mailto:{CONTACT}" style="color:{ACCENT};text-decoration:underline;">{CONTACT}</a></p><p class="fl-subtle" style="margin:0;{small}color:{SUBTLE};">&copy; {COMPANY}</p>"#,
            escape(&self.footer),
            small = font(12, 18, 400)
        );
        format!(
            r#"<!DOCTYPE html>
<html lang="en" xmlns="http://www.w3.org/1999/xhtml" xmlns:o="urn:schemas-microsoft-com:office:office">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="X-UA-Compatible" content="IE=edge">
<meta name="x-apple-disable-message-reformatting">
<meta name="format-detection" content="telephone=no, date=no, address=no, email=no, url=no">
<meta name="color-scheme" content="light dark">
<meta name="supported-color-schemes" content="light dark">
<title>{title}</title>
<!--[if mso]><noscript><xml><o:OfficeDocumentSettings><o:PixelsPerInch>96</o:PixelsPerInch></o:OfficeDocumentSettings></xml></noscript><style>table, td, p, a, span, h1 {{ font-family: Arial, Helvetica, sans-serif !important; }}</style><![endif]-->
<style>{STYLE}</style>
</head>
<body class="fl-page" style="margin:0;padding:0;width:100%;background-color:{PAGE};">
<div style="display:none;max-height:0;max-width:0;overflow:hidden;opacity:0;mso-hide:all;font-size:1px;line-height:1px;color:{PAGE};">{preview}</div>
<table role="presentation" class="fl-page" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="{PAGE}" style="width:100%;background-color:{PAGE};">
<tr><td align="center" style="padding:32px 12px 40px 12px;">
<!--[if mso]><table role="presentation" width="600" align="center" cellpadding="0" cellspacing="0" border="0"><tr><td><![endif]-->
<table role="presentation" class="fl-shell" width="600" cellpadding="0" cellspacing="0" border="0" style="width:100%;max-width:600px;">
<tr><td style="padding:0 4px 18px 4px;">{wordmark}</td></tr>
<tr><td class="fl-card" bgcolor="{CARD}" style="background-color:{CARD};border:1px solid {BORDER};border-radius:16px;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="width:100%;">
<tr><td height="4" bgcolor="{BRAND}" style="height:4px;line-height:4px;font-size:4px;background-color:{BRAND};background-image:linear-gradient(90deg,#3b82f6,#8b5cf6,#ec4899);border-radius:16px 16px 0 0;">&nbsp;</td></tr>
{body}
<tr><td height="12" style="height:12px;line-height:12px;font-size:12px;">&nbsp;</td></tr>
</table>
</td></tr>
<tr><td class="fl-pad" align="center" style="padding:24px 24px 0 24px;text-align:center;">{footer}</td></tr>
</table>
<!--[if mso]></td></tr></table><![endif]-->
</td></tr>
</table>
</body>
</html>"#
        )
    }

    pub(crate) fn text(&self) -> String {
        let blocks = self.blocks.iter().map(Block::text);
        let footer = format!(
            "-- \n{}\nQuestions? Write to {CONTACT}\n© {COMPANY}",
            self.footer.trim()
        );
        let mut text = blocks
            .chain(std::iter::once(footer))
            .collect::<Vec<_>>()
            .join("\n\n");
        text.push('\n');
        text
    }

    /// The HTML part and the plain-text part.
    pub(crate) fn render(&self) -> (String, String) {
        (self.html(), self.text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Email {
        Email {
            preview: "Ana <ana@example.org> & co".into(),
            footer: "You get this because <you> asked.".into(),
            blocks: vec![
                Block::Hero {
                    eyebrow: Some("Group chat".into()),
                    title: "Join <b>Launch</b>".into(),
                    subtitle: Some("Flux invited you".into()),
                },
                Block::Text(vec![plain("Hello "), strong("<Ana>"), plain("!")]),
                Block::Callout {
                    tone: Tone::Info,
                    label: "Why".into(),
                    body: "First line\nsecond <line>".into(),
                },
                Block::Details {
                    title: Some("Order".into()),
                    rows: vec![
                        ("Amount".into(), Value::Text("9.99 EUR".into())),
                        ("Token".into(), Value::Code("<t>".into())),
                        (
                            "Status".into(),
                            Value::Badge(Badge::new(Tone::Success, "Paid")),
                        ),
                    ],
                },
                Block::Change {
                    label: "Status".into(),
                    from: Badge::new(Tone::Neutral, "Queued"),
                    to: Badge::new(Tone::Success, "Done"),
                },
                Block::Steps {
                    title: "Next".into(),
                    steps: vec![("Review".into(), "We look at it".into())],
                },
                Block::Button {
                    label: "Open".into(),
                    url: "https://example.com/a?b=1&c=\"2\"".into(),
                },
                Block::SecondaryButton {
                    label: "Receipt".into(),
                    url: "https://example.com/r".into(),
                },
                Block::Code("{####}".into()),
                Block::Note("Ignore it if <unsure>.".into()),
            ],
        }
    }

    #[test]
    fn every_value_is_escaped_in_html_and_kept_as_is_in_text() {
        let (html, text) = sample().render();
        for raw in [
            "<b>Launch</b>",
            "<Ana>",
            "<line>",
            "<t>",
            "<unsure>",
            "<you>",
            "c=\"2\"",
        ] {
            assert!(!html.contains(raw), "{raw} leaked into the HTML");
        }
        assert!(html.contains("Join &lt;b&gt;Launch&lt;/b&gt;"));
        assert!(html.contains("https://example.com/a?b=1&amp;c=&quot;2&quot;"));
        assert!(html.contains("First line<br>second &lt;line&gt;"));
        for kept in [
            "GROUP CHAT\nJoin <b>Launch</b>\nFlux invited you",
            "Hello <Ana>!",
            "WHY\nFirst line\nsecond <line>",
            "ORDER\nAmount: 9.99 EUR\nToken: <t>\nStatus: Paid",
            "Status: Queued → Done",
            "NEXT\n1. Review: We look at it",
            "Open:\nhttps://example.com/a?b=1&c=\"2\"",
            "Receipt:\nhttps://example.com/r",
            "{####}",
            "Ignore it if <unsure>.",
            "-- \nYou get this because <you> asked.\nQuestions? Write to help@great-co.de",
        ] {
            assert!(
                text.contains(kept),
                "the text part misses {kept:?}:\n{text}"
            );
        }
    }

    #[test]
    fn the_html_follows_the_theme_survives_outlook_and_previews_its_line() {
        let html = sample().html();
        assert!(html.contains(r#"<meta name="color-scheme" content="light dark">"#));
        assert!(html.contains("@media (prefers-color-scheme: dark)"));
        assert!(html.contains("[data-ogsc]"));
        assert!(html.contains("<!--[if mso]>"));
        assert!(html.contains("mso-padding-alt"));
        assert!(html.contains("<title>Join &lt;b&gt;Launch&lt;/b&gt;</title>"));
        assert!(html.contains("Ana &lt;ana@example.org&gt; &amp; co&#847;"));
        assert!(!html.contains("fonts.googleapis.com"));
        assert!(!html.contains("<img"));
        assert!(html.contains("{####}"));
    }

    #[test]
    fn one_line_collapses_breaks_and_shortens_with_an_ellipsis() {
        assert_eq!(one_line("  Launch\r\n\tplanning  ", 80), "Launch planning");
        assert_eq!(one_line("abcdefghij", 5), "abcd…");
        assert_eq!(one_line("abcde", 5), "abcde");
        assert_eq!(one_line("\u{7}", 5), "");
    }
}

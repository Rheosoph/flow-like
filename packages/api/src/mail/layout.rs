//! The one look of every email the platform sends. An email is a list of blocks, and the same
//! blocks render its HTML part and its plain-text part, so a client without HTML reads all of
//! it. The HTML wears the website's dark theme (`packages/ui/global.css`, `.dark`) in every
//! client: clients that darken light emails, like Outlook, find nothing light to darken, and the
//! orange button also carries its color as a gradient image, which they leave alone. It is built
//! from tables and inline styles so Outlook for Windows and Gmail keep it intact, with no web
//! fonts. Its one image is the app icon from the website; with images blocked the wordmark stays.

const SANS: &str =
    "Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif";
const MONO: &str =
    "'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace";
const CONTACT: &str = "info@flow-like.com";
const COMPANY: &str = "Flow-Like by Rheosoph GmbH";
const PRIVACY_POLICY: &str = "https://flow-like.com/privacy-policy";
const LEGAL_NOTICE: &str = "https://flow-like.com/legal-notice";
const LOGO: &str = "https://flow-like.com/android-chrome-192x192.png";

const PAGE: &str = "#0C0E13";
const CARD: &str = "#13161B";
const RAISED: &str = "#1B1E23";
const BORDER: &str = "#282C33";
const HAIRLINE: &str = "#20232A";
const GLOW_BORDER: &str = "#4A231D";
const INK: &str = "#F3EFEC";
const BODY: &str = "#C9CBD1";
const MUTED: &str = "#999EA8";
const FAINT: &str = "#80858F";
/// The website's primary and accent; its buttons run from one to the other.
const PRIMARY: &str = "#FB562D";
const ACCENT: &str = "#F6472C";
const LINK: &str = "#FF7350";
const BUTTON_FILL: &str = "linear-gradient(100deg,#FB562D 0%,#F6472C 100%)";
const AVATAR_FILL: &str = "linear-gradient(135deg,#FF8A3D 0%,#F6472C 100%)";
const GLOW: &str = "radial-gradient(ellipse 80% 60% at 30% 0%, rgba(246,71,44,0.32) 0%, rgba(246,71,44,0.11) 45%, rgba(12,14,19,0) 100%)";
const DOT_GLOW: &str = "rgba(246,71,44,0.75)";

/// Layout rules every client with a stylesheet keeps, Gmail included.
const BASE_STYLE: &str = r#"
:root { color-scheme: light dark; supported-color-schemes: light dark; }
body, table, td, a { -webkit-text-size-adjust: 100%; -ms-text-size-adjust: 100%; }
table, td { mso-table-lspace: 0pt; mso-table-rspace: 0pt; }
img { -ms-interpolation-mode: bicubic; }
a[x-apple-data-detectors] { color: inherit !important; text-decoration: none !important; }
@media only screen and (max-width: 620px) {
  .fl-outer { padding: 8px 0 28px 0 !important; }
  .fl-pad { padding-left: 20px !important; padding-right: 20px !important; }
  .fl-title { font-size: 31px !important; line-height: 34px !important; letter-spacing: -1.3px !important; }
  .fl-lead { font-size: 16px !important; line-height: 25px !important; }
  .fl-code-text { font-size: 28px !important; letter-spacing: 7px !important; }
  .fl-stack-label { display: block !important; width: 100% !important; box-sizing: border-box; padding: 12px 0 2px 0 !important; }
  .fl-stack-value { display: block !important; width: 100% !important; box-sizing: border-box; padding: 0 0 12px 0 !important; text-align: left !important; border-top: 0 !important; }
  .fl-actions { width: 100% !important; }
  .fl-action { display: block !important; width: 100% !important; }
  .fl-action-gap { display: block !important; width: 100% !important; height: 10px !important; }
  .fl-btn, .fl-btn td { width: 100% !important; }
  .fl-btn a { display: block !important; }
}
"#;

/// Outlook.com marks the cells it darkens, and this gives the button its orange back; printing
/// drops dark backgrounds, so print turns the text dark. Apart from [`BASE_STYLE`]: Gmail drops
/// a whole style block it cannot parse.
fn guard_style() -> String {
    format!(
        r#"
[data-ogsb] .fl-fill {{ background-color: {PRIMARY} !important; background-image: {BUTTON_FILL} !important; }}
@media print {{
  body, table, td, div, p, h1, span, a, strong {{ color: #111111 !important; background: #FFFFFF !important; border-color: #DDDDDD !important; box-shadow: none !important; }}
}}
"#
    )
}

/// Keeps the inbox preview to the preview line instead of the start of the body.
const PREVIEW_FILLER: &str = "&#847;&zwnj;&nbsp;";

/// Detail rows longer than this stack label over value on a phone.
const SIDE_BY_SIDE_LABEL_CHARS: usize = 24;
const SIDE_BY_SIDE_VALUE_CHARS: usize = 28;

/// The color of a badge or a callout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

struct ToneColors {
    dot: &'static str,
    ink: &'static str,
    border: &'static str,
}

impl Tone {
    fn colors(self) -> ToneColors {
        let (dot, ink, border) = match self {
            Self::Neutral => (MUTED, BODY, "#30343C"),
            Self::Info => ("#60A5FA", "#A5C8FF", "#24364F"),
            Self::Success => ("#22C55E", "#86E5AC", "#1D3D2C"),
            Self::Warning => ("#F5A524", "#F9CB72", "#483818"),
            Self::Danger => ("#F04848", "#FF9A9A", "#4B2024"),
        };
        ToneColors { dot, ink, border }
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
        let colors = self.tone.colors();
        format!(
            r#"<span style="display:inline-block;padding:3px 10px 3px 9px;border:1px solid {border};border-radius:999px;{font}color:{ink};white-space:nowrap;"><span style="color:{dot};font-size:9px;">&#9679;</span>&nbsp;{label}</span>"#,
            border = colors.border,
            font = mono(11, 16, 600, "0.9px"),
            ink = colors.ink,
            dot = colors.dot,
            label = escape(&self.label)
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
                r#"<span style="display:inline-block;padding:2px 8px;border-radius:6px;background-color:{RAISED};border:1px solid {BORDER};font-family:{MONO};font-size:13px;line-height:20px;font-weight:500;color:{INK};">{}</span>"#,
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

/// Who said the words of a [`Block::Quote`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Speaker {
    /// Flow-Like's own people, shown with the logo.
    Team(String),
    /// Someone named, shown with their initial; the role reads after the name.
    Named { name: String, role: Option<String> },
}

impl Speaker {
    fn heading(&self) -> String {
        match self {
            Self::Team(name)
            | Self::Named {
                name, role: None, ..
            } => name.clone(),
            Self::Named {
                name,
                role: Some(role),
            } => format!("{name}, {role}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Block {
    /// The opening below the logo: a short label, the headline and a line under it.
    Hero {
        eyebrow: Option<String>,
        title: String,
        subtitle: Option<String>,
    },
    Text(Vec<Span>),
    /// Words someone wrote to the reader, as a chat message. Line breaks are kept.
    Quote {
        speaker: Speaker,
        body: String,
    },
    /// Words set apart: a notice or a record. Line breaks are kept.
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
    /// The main action. Its link is written out in the fine print for clients that drop it.
    Button {
        label: String,
        url: String,
    },
    /// A lesser action; right after a [`Block::Button`] it sits beside it.
    SecondaryButton {
        label: String,
        url: String,
    },
    /// A one-time code, large and easy to copy.
    Code(String),
    /// Small print; the notes that end an email sit below a rule, after everything else.
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

fn sans(size: u32, line: u32, weight: u32) -> String {
    format!("font-family:{SANS};font-size:{size}px;line-height:{line}px;font-weight:{weight};")
}

/// The website's small uppercase monospace type.
fn mono(size: u32, line: u32, weight: u32, spacing: &str) -> String {
    format!(
        "font-family:{MONO};font-size:{size}px;line-height:{line}px;font-weight:{weight};letter-spacing:{spacing};text-transform:uppercase;"
    )
}

/// One row of the email; `top` and `bottom` are its vertical padding.
fn row(top: u32, bottom: u32, content: &str) -> String {
    format!(
        r#"<tr><td class="fl-pad" style="padding:{top}px 32px {bottom}px 32px;">{content}</td></tr>"#
    )
}

fn full_width(rows: &str) -> String {
    format!(
        r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="width:100%;">{rows}</table>"#
    )
}

fn inline_table(class: Option<&str>, cells: &str) -> String {
    let class = class
        .map(|class| format!(r#" class="{class}""#))
        .unwrap_or_default();
    format!(
        r#"<table role="presentation"{class} cellpadding="0" cellspacing="0" border="0"><tr>{cells}</tr></table>"#
    )
}

/// The website's status dot, glowing where `glow` is given.
fn dot(color: &str, glow: Option<&str>) -> String {
    let glow = glow
        .map(|glow| format!("box-shadow:0 0 12px {glow};"))
        .unwrap_or_default();
    format!(
        r#"<span style="display:inline-block;width:6px;height:6px;border-radius:3px;background-color:{color};{glow}vertical-align:middle;margin:-2px 10px 0 0;font-size:0;line-height:0;">&nbsp;</span>"#
    )
}

/// A panel's heading: a dot and a monospace label, given as HTML.
fn tag(label: &str, dot_color: &str, color: &str) -> String {
    format!(
        r#"<p style="margin:0;{}color:{color};">{}{label}</p>"#,
        mono(11, 16, 500, "1.4px"),
        dot(dot_color, None)
    )
}

fn logo_image(size: u32) -> String {
    format!(
        r#"<img src="{LOGO}" width="{size}" height="{size}" alt="" style="display:block;width:{size}px;height:{size}px;border:0;outline:none;text-decoration:none;">"#
    )
}

fn initial_avatar(name: &str) -> String {
    let initial: String = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "?".to_string(), |c| c.to_uppercase().collect());
    inline_table(
        None,
        &format!(
            r#"<td width="32" height="32" align="center" valign="middle" bgcolor="{PRIMARY}" style="width:32px;height:32px;border-radius:16px;background-color:{PRIMARY};background-image:{AVATAR_FILL};{}color:#FFFFFF;text-align:center;">{}</td>"#,
            sans(14, 32, 700),
            escape(&initial)
        ),
    )
}

/// A card like the ones on the website: an optional heading row over `content`; `glow` gives
/// it the orange edge of the website's hero card.
fn panel(heading: Option<&str>, content: &str, padding: &str, glow: bool) -> String {
    let heading = heading
        .map(|heading| {
            format!(
                r#"<tr><td style="padding:13px 20px 12px 20px;border-bottom:1px solid {BORDER};">{heading}</td></tr>"#
            )
        })
        .unwrap_or_default();
    let (border, shadow) = if glow {
        (
            GLOW_BORDER,
            "box-shadow:0 24px 64px -28px rgba(251,86,45,0.45);",
        )
    } else {
        (BORDER, "")
    };
    full_width(&format!(
        r#"<tr><td bgcolor="{CARD}" style="background-color:{CARD};border:1px solid {border};border-radius:14px;{shadow}">{}</td></tr>"#,
        full_width(&format!(
            r#"{heading}<tr><td style="padding:{padding};">{content}</td></tr>"#
        ))
    ))
}

fn button_html(label: &str, url: &str, primary: bool) -> String {
    let url = escape(url);
    let label = escape(label);
    let font = sans(14, 20, 700);
    let cell = if primary {
        format!(
            r#"<td class="fl-fill" align="center" bgcolor="{PRIMARY}" style="border-radius:10px;background-color:{PRIMARY};background-image:{BUTTON_FILL};box-shadow:0 12px 30px -10px rgba(251,86,45,0.55);mso-padding-alt:15px 24px;"><a href="{url}" target="_blank" rel="noopener" style="display:inline-block;padding:15px 24px;{font}color:#FFFFFF;text-decoration:none;border-radius:10px;">{label}&nbsp;&nbsp;&rarr;</a></td>"#
        )
    } else {
        format!(
            r#"<td align="center" bgcolor="{PAGE}" style="border:1px solid {BORDER};border-radius:10px;background-color:{PAGE};mso-padding-alt:14px 22px;"><a href="{url}" target="_blank" rel="noopener" style="display:inline-block;padding:14px 22px;{font}color:{INK};text-decoration:none;border-radius:10px;">{label}</a></td>"#
        )
    };
    inline_table(Some("fl-btn"), &cell)
}

/// A primary button with a secondary one beside it, stacked on a phone.
fn button_pair(primary: (&str, &str), secondary: (&str, &str)) -> String {
    inline_table(
        Some("fl-actions"),
        &format!(
            r#"<td class="fl-action" valign="middle">{}</td><td class="fl-action-gap" width="12" style="width:12px;font-size:0;line-height:0;">&nbsp;</td><td class="fl-action" valign="middle">{}</td>"#,
            button_html(primary.0, primary.1, true),
            button_html(secondary.0, secondary.1, false)
        ),
    )
}

fn hero_html(eyebrow: Option<&str>, title: &str, subtitle: Option<&str>) -> String {
    let eyebrow = eyebrow
        .map(|eyebrow| {
            format!(
                r#"<p style="margin:0 0 22px 0;{}color:{ACCENT};">{}{}</p>"#,
                mono(11, 16, 500, "1.8px"),
                dot(ACCENT, Some(DOT_GLOW)),
                escape(eyebrow)
            )
        })
        .unwrap_or_default();
    let subtitle = subtitle
        .map(|subtitle| {
            format!(
                r#"<p class="fl-lead" style="margin:18px 0 0 0;{}color:{MUTED};">{}</p>"#,
                sans(17, 27, 400),
                escape(subtitle)
            )
        })
        .unwrap_or_default();
    row(
        44,
        32,
        &format!(
            r#"{eyebrow}<h1 class="fl-title" style="margin:0;{}letter-spacing:-1.8px;color:{INK};text-wrap:balance;">{}</h1>{subtitle}"#,
            sans(40, 43, 700),
            escape(title)
        ),
    )
}

fn quote_html(speaker: &Speaker, body: &str) -> String {
    let (picture, heading) = match speaker {
        Speaker::Team(name) => (logo_image(32), escape(name)),
        Speaker::Named { name, role } => (
            initial_avatar(name),
            match role {
                Some(role) => format!(
                    r#"{} <span style="color:{FAINT};">/ {}</span>"#,
                    escape(name),
                    escape(role)
                ),
                None => escape(name),
            },
        ),
    };
    panel(
        Some(&tag(&heading, ACCENT, INK)),
        &full_width(&format!(
            r#"<tr><td width="46" valign="top" style="width:46px;padding-top:1px;">{picture}</td><td valign="top"><p style="margin:0;{}color:{INK};">{}</p></td></tr>"#,
            sans(16, 25, 400),
            escape_lines(body)
        )),
        "18px 20px",
        true,
    )
}

fn details_html(title: Option<&str>, rows: &[(String, Value)]) -> String {
    let stacks = rows.iter().any(|(label, value)| {
        label.chars().count() > SIDE_BY_SIDE_LABEL_CHARS
            || value.text().chars().count() > SIDE_BY_SIDE_VALUE_CHARS
    });
    let (label_class, value_class) = if stacks {
        (r#" class="fl-stack-label""#, r#" class="fl-stack-value""#)
    } else {
        ("", "")
    };
    let lines: String = rows
        .iter()
        .enumerate()
        .map(|(index, (label, value))| {
            let rule = if index == 0 {
                String::new()
            } else {
                format!("border-top:1px solid {HAIRLINE};")
            };
            format!(
                r#"<tr><td{label_class} width="46%" valign="top" style="width:46%;padding:12px 16px 12px 0;{rule}{}color:{MUTED};">{}</td><td{value_class} align="right" valign="top" style="padding:12px 0;{rule}{}color:{INK};text-align:right;word-break:break-word;">{}</td></tr>"#,
                sans(14, 20, 400),
                escape(label),
                sans(14, 20, 500),
                value.html()
            )
        })
        .collect();
    let heading = title.map(|title| tag(&escape(title), ACCENT, MUTED));
    panel(heading.as_deref(), &full_width(&lines), "2px 20px", false)
}

fn steps_html(title: &str, steps: &[(String, String)]) -> String {
    let items: String = steps
        .iter()
        .enumerate()
        .map(|(index, (title, line))| {
            let rule = if index == 0 {
                String::new()
            } else {
                format!("border-top:1px solid {HAIRLINE};")
            };
            format!(
                r#"<tr><td width="44" valign="top" style="width:44px;padding:14px 0;{rule}{}color:{ACCENT};">{:02}</td><td valign="top" style="padding:14px 0;{rule}"><p style="margin:0;{}color:{INK};">{}</p><p style="margin:3px 0 0 0;{}color:{MUTED};">{}</p></td></tr>"#,
                mono(12, 20, 600, "0.6px"),
                index + 1,
                sans(14, 20, 600),
                escape(title),
                sans(14, 20, 400),
                escape(line)
            )
        })
        .collect();
    panel(
        Some(&tag(&escape(title), ACCENT, MUTED)),
        &full_width(&items),
        "0 20px",
        false,
    )
}

fn spans_html(spans: &[Span]) -> String {
    spans
        .iter()
        .map(|span| match span {
            Span::Plain(text) => escape(text),
            Span::Strong(text) => format!(
                r#"<strong style="font-weight:600;color:{INK};">{}</strong>"#,
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

impl Block {
    fn html(&self) -> String {
        match self {
            Self::Hero {
                eyebrow,
                title,
                subtitle,
            } => hero_html(eyebrow.as_deref(), title, subtitle.as_deref()),
            Self::Text(spans) => row(
                0,
                22,
                &format!(
                    r#"<p style="margin:0;{}color:{BODY};">{}</p>"#,
                    sans(15, 25, 400),
                    spans_html(spans)
                ),
            ),
            Self::Quote { speaker, body } => row(0, 24, &quote_html(speaker, body)),
            Self::Callout { tone, label, body } => {
                let colors = tone.colors();
                row(
                    0,
                    24,
                    &panel(
                        Some(&tag(&escape(label), colors.dot, colors.ink)),
                        &format!(
                            r#"<p style="margin:0;{}color:{BODY};">{}</p>"#,
                            sans(15, 24, 400),
                            escape_lines(body)
                        ),
                        "18px 20px",
                        false,
                    ),
                )
            }
            Self::Details { title, rows } => row(0, 24, &details_html(title.as_deref(), rows)),
            Self::Change { label, from, to } => row(
                0,
                24,
                &panel(
                    Some(&tag(&escape(label), ACCENT, MUTED)),
                    &format!(
                        r#"<p style="margin:0;{}">{}&nbsp;&nbsp;<span style="color:{FAINT};">&rarr;</span>&nbsp;&nbsp;{}</p>"#,
                        sans(14, 26, 400),
                        Badge::new(Tone::Neutral, from.label.clone()).html(),
                        to.html()
                    ),
                    "18px 20px",
                    false,
                ),
            ),
            Self::Steps { title, steps } => row(0, 24, &steps_html(title, steps)),
            Self::Button { label, url } => row(4, 32, &button_html(label, url, true)),
            Self::SecondaryButton { label, url } => row(0, 32, &button_html(label, url, false)),
            Self::Code(code) => row(
                0,
                28,
                &panel(
                    None,
                    &format!(
                        r#"<p class="fl-code-text" style="margin:0;font-family:{MONO};font-size:36px;line-height:44px;font-weight:600;letter-spacing:10px;color:{INK};text-align:center;">{}</p>"#,
                        escape(code)
                    ),
                    "28px 16px 28px 24px",
                    true,
                ),
            ),
            Self::Note(note) => row(
                0,
                10,
                &format!(
                    r#"<p style="margin:0;{}color:{MUTED};">{}</p>"#,
                    sans(13, 20, 400),
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
            Self::Quote { speaker, body } => format!("{}:\n{}", speaker.heading(), body.trim()),
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

    /// The rows under the logo: the blocks, a button with the secondary one after it side by
    /// side, then the closing notes and the main button's link below a rule.
    fn body_html(&self) -> String {
        let fine_print_from = self
            .blocks
            .iter()
            .rposition(|block| !matches!(block, Block::Note(_)))
            .map_or(0, |index| index + 1);
        let (main, notes) = self.blocks.split_at(fine_print_from);
        let mut rows = String::new();
        let mut blocks = main.iter().peekable();
        while let Some(block) = blocks.next() {
            if let Block::Button { label, url } = block
                && let Some(Block::SecondaryButton {
                    label: second_label,
                    url: second_url,
                }) = blocks.peek()
            {
                rows.push_str(&row(
                    4,
                    32,
                    &button_pair(
                        (label.as_str(), url.as_str()),
                        (second_label.as_str(), second_url.as_str()),
                    ),
                ));
                blocks.next();
                continue;
            }
            rows.push_str(&block.html());
        }
        let primary = main.iter().find_map(|block| match block {
            Block::Button { url, .. } => Some(url.as_str()),
            _ => None,
        });
        if notes.is_empty() && primary.is_none() {
            return rows;
        }
        rows.push_str(&format!(
            r#"<tr><td class="fl-pad" style="padding:0 32px;">{}</td></tr>"#,
            full_width(&format!(
                r#"<tr><td style="border-top:1px solid {BORDER};font-size:0;line-height:0;height:20px;">&nbsp;</td></tr>"#
            ))
        ));
        for note in notes {
            rows.push_str(&note.html());
        }
        if let Some(url) = primary {
            rows.push_str(&row(
                0,
                10,
                &format!(
                    r#"<p style="margin:0;{}color:{FAINT};word-break:break-all;">Button not working? Paste this link into your browser:<br><a href="{url}" target="_blank" rel="noopener" style="color:{LINK};text-decoration:underline;">{url}</a></p>"#,
                    sans(12, 19, 400),
                    url = escape(url)
                ),
            ));
        }
        rows
    }

    fn footer_html(&self) -> String {
        let small = sans(12, 19, 400);
        let quiet = |url: &str, label: &str| {
            format!(
                r#"<a href="{url}" target="_blank" rel="noopener" style="color:{MUTED};text-decoration:underline;">{label}</a>"#
            )
        };
        let lines = format!(
            r#"<p style="margin:0 0 6px 0;{small}color:{FAINT};">{}</p><p style="margin:0 0 6px 0;{small}color:{FAINT};">Questions? Write to <a href="mailto:{CONTACT}" style="color:{LINK};text-decoration:underline;">{CONTACT}</a></p><p style="margin:0 0 6px 0;{small}color:{FAINT};">{} &nbsp;&middot;&nbsp; {}</p><p style="margin:0;{small}color:{FAINT};">&copy; {COMPANY}</p>"#,
            escape(&self.footer),
            quiet(PRIVACY_POLICY, "Privacy policy"),
            quiet(LEGAL_NOTICE, "Legal notice")
        );
        format!(
            r#"<tr><td class="fl-pad" style="padding:28px 32px 0 32px;">{}</td></tr>"#,
            full_width(&format!(
                r#"<tr><td style="border-top:1px solid {HAIRLINE};padding-top:24px;">{lines}</td></tr>"#
            ))
        )
    }

    pub(crate) fn html(&self) -> String {
        let title = escape(self.title());
        let preview = format!("{}{}", escape(&self.preview), PREVIEW_FILLER.repeat(40));
        let brand = row(
            24,
            0,
            &inline_table(
                None,
                &format!(
                    r#"<td valign="middle" width="28" style="width:28px;">{}</td><td valign="middle" style="padding-left:10px;{}letter-spacing:-0.3px;color:{INK};">Flow Like</td>"#,
                    logo_image(28),
                    sans(18, 28, 600)
                ),
            ),
        );
        let body = self.body_html();
        let footer = self.footer_html();
        let guard = guard_style();
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
<style>{BASE_STYLE}</style>
<style>{guard}</style>
</head>
<body style="margin:0;padding:0;width:100%;background-color:{PAGE};">
<div style="display:none;max-height:0;max-width:0;overflow:hidden;opacity:0;mso-hide:all;font-size:1px;line-height:1px;color:{PAGE};">{preview}</div>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="{PAGE}" style="width:100%;background-color:{PAGE};">
<tr><td class="fl-outer" align="center" style="padding:16px 12px 40px 12px;background-color:{PAGE};background-image:{GLOW};">
<!--[if mso]><table role="presentation" width="600" align="center" cellpadding="0" cellspacing="0" border="0"><tr><td><![endif]-->
<table role="presentation" width="600" cellpadding="0" cellspacing="0" border="0" style="width:100%;max-width:600px;">
{brand}
{body}
{footer}
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
            "-- \n{}\nQuestions? Write to {CONTACT}\nPrivacy policy: {PRIVACY_POLICY}\nLegal notice: {LEGAL_NOTICE}\n© {COMPANY}",
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
                Block::Quote {
                    speaker: Speaker::Named {
                        name: "<Flux>".into(),
                        role: Some("<Felix>’s bot".into()),
                    },
                    body: "Come <in>\nplease".into(),
                },
                Block::Quote {
                    speaker: Speaker::Team("Flow-Like team".into()),
                    body: "Done.".into(),
                },
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
            "<Flux>",
            "<Felix>",
            "<in>",
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
        assert!(html.contains("Come &lt;in&gt;<br>please"));
        for kept in [
            "GROUP CHAT\nJoin <b>Launch</b>\nFlux invited you",
            "Hello <Ana>!",
            "<Flux>, <Felix>’s bot:\nCome <in>\nplease",
            "Flow-Like team:\nDone.",
            "WHY\nFirst line\nsecond <line>",
            "ORDER\nAmount: 9.99 EUR\nToken: <t>\nStatus: Paid",
            "Status: Queued → Done",
            "NEXT\n1. Review: We look at it",
            "Open:\nhttps://example.com/a?b=1&c=\"2\"",
            "Receipt:\nhttps://example.com/r",
            "{####}",
            "Ignore it if <unsure>.",
            "-- \nYou get this because <you> asked.\nQuestions? Write to info@flow-like.com",
            "Legal notice: https://flow-like.com/legal-notice",
        ] {
            assert!(
                text.contains(kept),
                "the text part misses {kept:?}:\n{text}"
            );
        }
    }

    #[test]
    fn the_html_survives_outlook_loads_only_the_logo_and_previews_its_line() {
        let html = sample().html();
        assert!(html.contains(r#"<meta name="color-scheme" content="light dark">"#));
        assert!(html.contains("<!--[if mso]>"));
        assert!(html.contains("mso-padding-alt"));
        assert!(html.contains("<title>Join &lt;b&gt;Launch&lt;/b&gt;</title>"));
        assert!(html.contains("Ana &lt;ana@example.org&gt; &amp; co&#847;"));
        assert!(!html.contains("fonts.googleapis.com"));
        let logo = format!(r#"<img src="{LOGO}""#);
        assert!(html.matches("<img").count() > 0);
        assert_eq!(html.matches("<img").count(), html.matches(&logo).count());
        assert!(html.contains("{####}"));
    }

    #[test]
    fn the_main_button_keeps_its_orange_where_outlook_darkens_backgrounds() {
        let html = sample().html();
        let button = &html[html.find(r#"<td class="fl-fill""#).unwrap()..];
        let cell = &button[..button.find('>').unwrap()];
        assert!(cell.contains(&format!("background-image:{BUTTON_FILL}")));
        assert!(cell.contains(&format!(r#"bgcolor="{PRIMARY}""#)));
        assert!(html.contains("[data-ogsb] .fl-fill"));
    }

    #[test]
    fn gmail_keeps_the_phone_layout_when_it_drops_the_guard_rules() {
        let html = sample().html();
        let base = html.find(".fl-stack-label").unwrap();
        let base_closes = base + html[base..].find("</style>").unwrap();
        let guard = html.find("[data-ogsb]").unwrap();
        assert!(base_closes < guard);
    }

    #[test]
    fn a_secondary_button_sits_beside_the_primary_whose_link_closes_the_body() {
        let html = sample().html();
        assert!(html.contains(r#"class="fl-actions""#));
        let note = html.find("Ignore it if").unwrap();
        let fallback = html.find("Button not working?").unwrap();
        let footer = html.find("You get this because").unwrap();
        assert!(note < fallback && fallback < footer);
        let alone = Email {
            preview: String::new(),
            footer: String::new(),
            blocks: vec![Block::Button {
                label: "Open".into(),
                url: "https://example.com".into(),
            }],
        }
        .html();
        assert!(!alone.contains(r#"class="fl-actions""#));
        assert!(alone.contains("Button not working?"));
    }

    #[test]
    fn long_detail_rows_stack_on_a_phone_and_short_ones_stay_side_by_side() {
        let details = |label: &str, value: &str| {
            Block::Details {
                title: None,
                rows: vec![(label.into(), Value::Text(value.into()))],
            }
            .html()
        };
        assert!(!details("Amount", "9.99 EUR").contains("fl-stack-label"));
        assert!(
            details("Immediate delivery withdrawal waiver accepted", "Yes")
                .contains("fl-stack-label")
        );
        assert!(
            details(
                "Declaration",
                "Withdrawal from the purchase identified below."
            )
            .contains("fl-stack-value")
        );
    }

    #[test]
    fn one_line_collapses_breaks_and_shortens_with_an_ellipsis() {
        assert_eq!(one_line("  Launch\r\n\tplanning  ", 80), "Launch planning");
        assert_eq!(one_line("abcdefghij", 5), "abcd…");
        assert_eq!(one_line("abcde", 5), "abcde");
        assert_eq!(one_line("\u{7}", 5), "");
    }
}

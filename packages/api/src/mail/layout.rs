//! The one look of every email the platform sends. An email is a list of blocks, and the same
//! blocks render its HTML part and its plain-text part, so a client without HTML reads all of
//! it. The HTML is a white card on a warm canvas in the brand's orange, which turns dark where the
//! client follows the system theme (Apple Mail, Outlook for Mac and iOS, Outlook.com). It is built
//! from tables and inline styles so Outlook for Windows and Gmail keep it intact, with no images
//! and no web fonts: nothing is fetched when it opens, and nothing breaks when images are blocked.

const FONT: &str =
    "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif";
const MONO: &str = "'SF Mono', SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace";
const CONTACT: &str = "help@great-co.de";
const COMPANY: &str = "Flow-Like by Rheosoph GmbH";
const PRIVACY_POLICY: &str = "https://flow-like.com/privacy-policy";
const LEGAL_NOTICE: &str = "https://flow-like.com/legal-notice";

const PAGE: &str = "#F5F5F4";
const CARD: &str = "#FFFFFF";
const HERO: &str = "#FFF8F4";
const BORDER: &str = "#E7E5E4";
const PANEL: &str = "#FAFAF9";
const PANEL_BORDER: &str = "#EFEDEB";
const BUBBLE: &str = "#F5F5F4";
const CHIP: &str = "#F0EEEC";
const OUTLINE: &str = "#D6D3D1";
const INK: &str = "#1C1917";
const BODY: &str = "#44403C";
const MUTED: &str = "#78716C";
const FOOTNOTE: &str = "#6B6560";
/// White text on it passes WCAG AA; the logo's brighter orange does not.
const ACCENT: &str = "#D93A0B";
const LINK: &str = "#C2410C";
const TINT: &str = "#FFF1EA";
const TINT_STRONG: &str = "#FFE7DB";
const TINT_BORDER: &str = "#FFDCCB";
const TINT_INK: &str = "#B3360C";
const BRAND: &str = "#FF4C18";
const BRAND_TOP: &str = "#FF8500";
const BRAND_BOTTOM: &str = "#FF002E";

/// Layout rules every client with a stylesheet keeps, Gmail included.
const BASE_STYLE: &str = r#"
:root { color-scheme: light dark; supported-color-schemes: light dark; }
body, table, td, a { -webkit-text-size-adjust: 100%; -ms-text-size-adjust: 100%; }
table, td { mso-table-lspace: 0pt; mso-table-rspace: 0pt; }
a[x-apple-data-detectors] { color: inherit !important; text-decoration: none !important; }
@media only screen and (max-width: 620px) {
  .fl-outer { padding: 24px 10px 32px 10px !important; }
  .fl-pad { padding-left: 24px !important; padding-right: 24px !important; }
  .fl-title { font-size: 24px !important; line-height: 30px !important; }
  .fl-code-text { font-size: 30px !important; letter-spacing: 7px !important; }
  .fl-stack-label { display: block !important; width: 100% !important; box-sizing: border-box; padding: 12px 0 2px 0 !important; }
  .fl-stack-value { display: block !important; width: 100% !important; box-sizing: border-box; padding: 0 0 12px 0 !important; text-align: left !important; border-top: 0 !important; }
  .fl-actions { width: 100% !important; }
  .fl-action { display: block !important; width: 100% !important; }
  .fl-action-gap { display: block !important; width: 100% !important; height: 10px !important; }
  .fl-btn, .fl-btn td { width: 100% !important; }
  .fl-btn a { display: block !important; }
}
"#;

/// Dark colors for clients that follow the system theme, and the selectors Outlook.com sets in
/// dark mode. Apart from [`BASE_STYLE`]: Gmail drops a whole style block it cannot parse.
const THEME_STYLE: &str = r#"
@media (prefers-color-scheme: dark) {
  .fl-page { background-color: #0C0A09 !important; }
  .fl-card { background-color: #1C1917 !important; border-color: #2E2925 !important; }
  .fl-hero { background-color: #221915 !important; background-image: linear-gradient(180deg, #2A1B13 0%, #1C1917 100%) !important; }
  .fl-panel { background-color: #24201D !important; border-color: #342F2B !important; }
  .fl-bubble { background-color: #2A2522 !important; }
  .fl-chip { background-color: #2E2926 !important; border-color: #3D3733 !important; }
  .fl-rule { border-color: #342F2B !important; }
  .fl-ink { color: #FAFAF9 !important; }
  .fl-body { color: #D6D3D1 !important; }
  .fl-muted { color: #A8A29E !important; }
  .fl-link { color: #FF9A6E !important; }
  .fl-outline { border-color: #44403C !important; }
  .fl-pill { background-color: #3A2216 !important; color: #FFB08A !important; }
  .fl-tint { background-color: #2B1C14 !important; border-color: #4A2B1D !important; }
  .fl-tint-ink { color: #FFB08A !important; }
  .fl-tone-neutral { background-color: #292524 !important; border-color: #3A3431 !important; }
  .fl-tone-neutral-ink { color: #D6D3D1 !important; }
  .fl-tone-info { background-color: #2B1C14 !important; border-color: #4A2B1D !important; }
  .fl-tone-info-ink { color: #FFB08A !important; }
  .fl-tone-success { background-color: #12261B !important; border-color: #1E3D2B !important; }
  .fl-tone-success-ink { color: #7DD3A0 !important; }
  .fl-tone-warning { background-color: #2B2210 !important; border-color: #4A3A16 !important; }
  .fl-tone-warning-ink { color: #F5C451 !important; }
  .fl-tone-danger { background-color: #2E1515 !important; border-color: #4D2222 !important; }
  .fl-tone-danger-ink { color: #F59A9A !important; }
}
[data-ogsc] .fl-ink { color: #FAFAF9 !important; }
[data-ogsc] .fl-body { color: #D6D3D1 !important; }
[data-ogsc] .fl-muted { color: #A8A29E !important; }
[data-ogsc] .fl-link { color: #FF9A6E !important; }
[data-ogsb] .fl-page { background-color: #0C0A09 !important; }
[data-ogsb] .fl-card { background-color: #1C1917 !important; }
[data-ogsb] .fl-hero { background-color: #221915 !important; background-image: none !important; }
[data-ogsb] .fl-panel { background-color: #24201D !important; }
[data-ogsb] .fl-bubble { background-color: #2A2522 !important; }
"#;

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
    background: &'static str,
    border: &'static str,
    ink: &'static str,
    /// `fl-tone-<name>` darkens the box, `fl-tone-<name>-ink` its text.
    name: &'static str,
}

impl Tone {
    fn colors(self) -> ToneColors {
        let (background, border, ink, name) = match self {
            Self::Neutral => ("#F5F5F4", "#E7E5E4", "#57534E", "neutral"),
            Self::Info => (TINT, TINT_BORDER, TINT_INK, "info"),
            Self::Success => ("#EAF7EF", "#CBEBD7", "#13693F", "success"),
            Self::Warning => ("#FFF5DD", "#F8E1A6", "#8A5300", "warning"),
            Self::Danger => ("#FDEDED", "#F6CFCF", "#A61B1B", "danger"),
        };
        ToneColors {
            background,
            border,
            ink,
            name,
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
        let colors = self.tone.colors();
        format!(
            r#"<span class="fl-tone-{name} fl-tone-{name}-ink" style="display:inline-block;padding:3px 10px;border-radius:999px;background-color:{background};border:1px solid {border};color:{ink};{font}white-space:nowrap;">{label}</span>"#,
            name = colors.name,
            background = colors.background,
            border = colors.border,
            ink = colors.ink,
            font = font(12, 18, 600),
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
                r#"<span class="fl-chip fl-ink" style="display:inline-block;padding:2px 8px;border-radius:6px;background-color:{CHIP};border:1px solid {BORDER};color:{INK};font-family:{MONO};font-size:13px;line-height:20px;font-weight:500;">{}</span>"#,
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
    /// The opening at the top of the card: a short label, the headline and a line under it.
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

fn font(size: u32, line: u32, weight: u32) -> String {
    format!("font-family:{FONT};font-size:{size}px;line-height:{line}px;font-weight:{weight};")
}

/// One row of the card; `top` and `bottom` are its vertical padding.
fn row(top: u32, bottom: u32, content: &str) -> String {
    format!(
        r#"<tr><td class="fl-pad" style="padding:{top}px 40px {bottom}px 40px;">{content}</td></tr>"#
    )
}

fn full_width(rows: &str) -> String {
    format!(
        r#"<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" style="width:100%;">{rows}</table>"#
    )
}

fn inline_table(cells: &str) -> String {
    format!(
        r#"<table role="presentation" cellpadding="0" cellspacing="0" border="0"><tr>{cells}</tr></table>"#
    )
}

/// A small uppercase label with `gap` pixels below it; `class` picks its dark-mode color.
fn label_html(label: &str, color: &str, class: &str, gap: u32) -> String {
    format!(
        r#"<p class="{class}" style="margin:0 0 {gap}px 0;{}letter-spacing:0.8px;text-transform:uppercase;color:{color};">{}</p>"#,
        font(11, 16, 700),
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

/// A rounded box in `class`'s colors.
fn panel(
    class: &str,
    background: &str,
    border: &str,
    padding: &str,
    radius: u32,
    content: &str,
) -> String {
    full_width(&format!(
        r#"<tr><td class="{class}" bgcolor="{background}" style="background-color:{background};border:1px solid {border};border-radius:{radius}px;padding:{padding};">{content}</td></tr>"#
    ))
}

/// The logo: the app icon's orange square with its F, drawn without an image.
fn logo_mark(size: u32, radius: u32, letter: u32) -> String {
    inline_table(&format!(
        r#"<td width="{size}" height="{size}" align="center" valign="middle" bgcolor="{BRAND}" style="width:{size}px;height:{size}px;border-radius:{radius}px;background-color:{BRAND};background-image:linear-gradient(180deg,{BRAND_TOP} 0%,{BRAND_BOTTOM} 100%);{}color:#FFFFFF;text-align:center;mso-line-height-rule:exactly;">F</td>"#,
        font(letter, size, 800)
    ))
}

fn initial_avatar(name: &str) -> String {
    let initial: String = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "?".to_string(), |c| c.to_uppercase().collect());
    inline_table(&format!(
        r#"<td class="fl-tint fl-tint-ink" width="34" height="34" align="center" valign="middle" bgcolor="{TINT_STRONG}" style="width:34px;height:34px;border-radius:18px;background-color:{TINT_STRONG};border:1px solid {TINT_BORDER};{}color:{TINT_INK};text-align:center;">{}</td>"#,
        font(15, 34, 700),
        escape(&initial)
    ))
}

fn button_html(label: &str, url: &str, primary: bool) -> String {
    let url = escape(url);
    let label = escape(label);
    let body = font(15, 20, 600);
    if primary {
        format!(
            r#"<table role="presentation" class="fl-btn" cellpadding="0" cellspacing="0" border="0"><tr><td align="center" bgcolor="{ACCENT}" style="border-radius:12px;background-color:{ACCENT};mso-padding-alt:14px 28px;"><a href="{url}" target="_blank" rel="noopener" style="display:inline-block;padding:14px 28px;{body}color:#FFFFFF;text-decoration:none;border-radius:12px;">{label}</a></td></tr></table>"#
        )
    } else {
        format!(
            r#"<table role="presentation" class="fl-btn" cellpadding="0" cellspacing="0" border="0"><tr><td class="fl-outline" align="center" style="border:1px solid {OUTLINE};border-radius:12px;mso-padding-alt:13px 24px;"><a class="fl-ink" href="{url}" target="_blank" rel="noopener" style="display:inline-block;padding:13px 24px;{body}color:{INK};text-decoration:none;border-radius:12px;">{label}</a></td></tr></table>"#
        )
    }
}

/// A primary button with a secondary one beside it, stacked on a phone.
fn button_pair(primary: (&str, &str), secondary: (&str, &str)) -> String {
    format!(
        r#"<table role="presentation" class="fl-actions" cellpadding="0" cellspacing="0" border="0"><tr><td class="fl-action" valign="middle">{}</td><td class="fl-action-gap" width="12" style="width:12px;font-size:0;line-height:0;">&nbsp;</td><td class="fl-action" valign="middle">{}</td></tr></table>"#,
        button_html(primary.0, primary.1, true),
        button_html(secondary.0, secondary.1, false)
    )
}

fn hero_html(eyebrow: Option<&str>, title: &str, subtitle: Option<&str>) -> String {
    let pill = eyebrow
        .map(|eyebrow| {
            format!(
                r#"<span class="fl-pill" style="display:inline-block;padding:4px 11px;border-radius:999px;background-color:{TINT_STRONG};{}letter-spacing:0.7px;text-transform:uppercase;color:{TINT_INK};">{}</span>"#,
                font(11, 16, 700),
                escape(eyebrow)
            )
        })
        .unwrap_or_default();
    let gap = if eyebrow.is_some() { 16 } else { 0 };
    let subtitle = subtitle
        .map(|subtitle| {
            format!(
                r#"<p class="fl-muted" style="margin:10px 0 0 0;{}color:{MUTED};">{}</p>"#,
                font(16, 24, 400),
                escape(subtitle)
            )
        })
        .unwrap_or_default();
    format!(
        r#"<tr><td class="fl-pad fl-hero" bgcolor="{HERO}" style="padding:36px 40px 28px 40px;background-color:{HERO};background-image:linear-gradient(180deg,#FFF1E9 0%,{CARD} 100%);border-radius:19px 19px 0 0;">{pill}<h1 class="fl-ink fl-title" style="margin:{gap}px 0 0 0;{}letter-spacing:-0.6px;color:{INK};">{}</h1>{subtitle}</td></tr><tr><td height="4" style="height:4px;line-height:4px;font-size:4px;">&nbsp;</td></tr>"#,
        font(28, 34, 700),
        escape(title)
    )
}

fn quote_html(speaker: &Speaker, body: &str) -> String {
    let (picture, name, role) = match speaker {
        Speaker::Team(name) => (logo_mark(34, 10, 18), name, None),
        Speaker::Named { name, role } => (initial_avatar(name), name, role.as_deref()),
    };
    let role = role
        .map(|role| {
            format!(
                r#" <span class="fl-muted" style="color:{MUTED};font-weight:400;">· {}</span>"#,
                escape(role)
            )
        })
        .unwrap_or_default();
    let bubble = full_width(&format!(
        r#"<tr><td class="fl-bubble" bgcolor="{BUBBLE}" style="background-color:{BUBBLE};border-radius:4px 16px 16px 16px;padding:14px 16px;"><p class="fl-body" style="margin:0;{}color:{BODY};">{}</p></td></tr>"#,
        font(15, 24, 400),
        escape_lines(body)
    ));
    full_width(&format!(
        r#"<tr><td width="48" valign="top" style="width:48px;padding-top:2px;">{picture}</td><td valign="top"><p class="fl-ink" style="margin:0 0 8px 0;{}color:{INK};">{}{role}</p>{bubble}</td></tr>"#,
        font(14, 20, 600),
        escape(name)
    ))
}

fn details_html(title: Option<&str>, rows: &[(String, Value)]) -> String {
    let stacks = rows.iter().any(|(label, value)| {
        label.chars().count() > SIDE_BY_SIDE_LABEL_CHARS
            || value.text().chars().count() > SIDE_BY_SIDE_VALUE_CHARS
    });
    let (label_class, value_class) = if stacks {
        (" fl-stack-label", " fl-stack-value")
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
                format!("border-top:1px solid {PANEL_BORDER};")
            };
            format!(
                r#"<tr><td class="fl-muted fl-rule{label_class}" width="46%" valign="top" style="width:46%;padding:11px 16px 11px 0;{rule}{}color:{MUTED};">{}</td><td class="fl-ink fl-rule{value_class}" align="right" valign="top" style="padding:11px 0;{rule}{}color:{INK};text-align:right;word-break:break-word;">{}</td></tr>"#,
                font(14, 20, 400),
                escape(label),
                font(14, 20, 600),
                value.html()
            )
        })
        .collect();
    let title = title
        .map(|title| label_html(title, MUTED, "fl-muted", 10))
        .unwrap_or_default();
    format!(
        "{title}{}",
        panel(
            "fl-panel",
            PANEL,
            PANEL_BORDER,
            "5px 18px",
            12,
            &full_width(&lines)
        )
    )
}

fn steps_html(title: &str, steps: &[(String, String)]) -> String {
    let items: String = steps
        .iter()
        .enumerate()
        .map(|(index, (title, line))| {
            let number = inline_table(&format!(
                r#"<td class="fl-tint fl-tint-ink" width="26" height="26" align="center" valign="middle" bgcolor="{TINT}" style="width:26px;height:26px;border-radius:13px;background-color:{TINT};border:1px solid {TINT_BORDER};{}color:{TINT_INK};text-align:center;">{}</td>"#,
                font(12, 26, 700),
                index + 1
            ));
            format!(
                r#"<tr><td width="40" valign="top" style="width:40px;padding:0 0 16px 0;">{number}</td><td valign="top" style="padding:3px 0 16px 0;"><p class="fl-ink" style="margin:0;{}color:{INK};">{}</p><p class="fl-muted" style="margin:2px 0 0 0;{}color:{MUTED};">{}</p></td></tr>"#,
                font(14, 20, 600),
                escape(title),
                font(14, 20, 400),
                escape(line)
            )
        })
        .collect();
    format!(
        "{}{}",
        label_html(title, MUTED, "fl-muted", 10),
        full_width(&items)
    )
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
                20,
                &format!(
                    r#"<p class="fl-body" style="margin:0;{}color:{BODY};">{}</p>"#,
                    font(15, 24, 400),
                    spans_html(spans)
                ),
            ),
            Self::Quote { speaker, body } => row(4, 24, &quote_html(speaker, body)),
            Self::Callout { tone, label, body } => {
                let colors = tone.colors();
                let class = format!("fl-tone-{}", colors.name);
                row(
                    0,
                    20,
                    &panel(
                        &class,
                        colors.background,
                        colors.border,
                        "16px 18px",
                        12,
                        &format!(
                            r#"{}<p class="fl-body" style="margin:0;{}color:{BODY};">{}</p>"#,
                            label_html(label, colors.ink, &format!("{class}-ink"), 6),
                            font(14, 22, 400),
                            escape_lines(body)
                        ),
                    ),
                )
            }
            Self::Details { title, rows } => row(0, 24, &details_html(title.as_deref(), rows)),
            Self::Change { label, from, to } => row(
                0,
                24,
                &panel(
                    "fl-panel",
                    PANEL,
                    PANEL_BORDER,
                    "16px 18px",
                    12,
                    &format!(
                        r#"{}<p style="margin:0;{}">{}&nbsp;&nbsp;<span class="fl-muted" style="color:{MUTED};">&rarr;</span>&nbsp;&nbsp;{}</p>"#,
                        label_html(label, MUTED, "fl-muted", 10),
                        font(14, 26, 400),
                        Badge::new(Tone::Neutral, from.label.clone()).html(),
                        to.html()
                    ),
                ),
            ),
            Self::Steps { title, steps } => row(4, 8, &steps_html(title, steps)),
            Self::Button { label, url } => row(6, 28, &button_html(label, url, true)),
            Self::SecondaryButton { label, url } => row(0, 28, &button_html(label, url, false)),
            Self::Code(code) => row(
                6,
                28,
                &panel(
                    "fl-tint",
                    TINT,
                    TINT_BORDER,
                    "22px 16px 22px 26px",
                    14,
                    &format!(
                        r#"<p class="fl-ink fl-code-text" style="margin:0;font-family:{FONT};font-size:36px;line-height:44px;font-weight:700;letter-spacing:10px;color:{INK};text-align:center;">{}</p>"#,
                        escape(code)
                    ),
                ),
            ),
            Self::Note(note) => row(
                0,
                12,
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

    /// The card's rows: the blocks, a button with the secondary one after it side by side, then
    /// the closing notes and the main button's link below a rule.
    fn card_html(&self) -> String {
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
                    6,
                    28,
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
            r#"<tr><td class="fl-pad" style="padding:0 40px;">{}</td></tr>"#,
            full_width(&format!(
                r#"<tr><td class="fl-rule" style="border-top:1px solid {BORDER};font-size:0;line-height:0;height:20px;">&nbsp;</td></tr>"#
            ))
        ));
        for note in notes {
            rows.push_str(&note.html());
        }
        if let Some(url) = primary {
            rows.push_str(&row(
                0,
                12,
                &format!(
                    r#"<p class="fl-muted" style="margin:0;{}color:{MUTED};word-break:break-all;">Button not working? Paste this link into your browser:<br><a class="fl-link" href="{url}" target="_blank" rel="noopener" style="color:{LINK};text-decoration:underline;">{url}</a></p>"#,
                    font(12, 18, 400),
                    url = escape(url)
                ),
            ));
        }
        rows
    }

    fn footer_html(&self) -> String {
        let small = font(12, 18, 400);
        let quiet = |url: &str, label: &str| {
            format!(
                r#"<a class="fl-muted" href="{url}" target="_blank" rel="noopener" style="color:{FOOTNOTE};text-decoration:underline;">{label}</a>"#
            )
        };
        format!(
            r#"<p class="fl-muted" style="margin:0 0 8px 0;{small}color:{FOOTNOTE};">{}</p><p class="fl-muted" style="margin:0 0 8px 0;{small}color:{FOOTNOTE};">Questions? Write to <a class="fl-link" href="mailto:{CONTACT}" style="color:{LINK};text-decoration:underline;">{CONTACT}</a></p><p class="fl-muted" style="margin:0 0 8px 0;{small}color:{FOOTNOTE};">{} &nbsp;&middot;&nbsp; {}</p><p class="fl-muted" style="margin:0;{small}color:{FOOTNOTE};">&copy; {COMPANY}</p>"#,
            escape(&self.footer),
            quiet(PRIVACY_POLICY, "Privacy policy"),
            quiet(LEGAL_NOTICE, "Legal notice")
        )
    }

    pub(crate) fn html(&self) -> String {
        let title = escape(self.title());
        let preview = format!("{}{}", escape(&self.preview), PREVIEW_FILLER.repeat(40));
        let logo = inline_table(&format!(
            r#"<td valign="middle">{}</td><td class="fl-ink" valign="middle" style="padding-left:10px;{}letter-spacing:-0.3px;color:{INK};">Flow-Like</td>"#,
            logo_mark(30, 9, 17),
            font(18, 30, 700)
        ));
        let card = full_width(&format!(
            r#"{}<tr><td height="20" style="height:20px;line-height:20px;font-size:20px;">&nbsp;</td></tr>"#,
            self.card_html()
        ));
        let footer = self.footer_html();
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
<style>{THEME_STYLE}</style>
</head>
<body class="fl-page" style="margin:0;padding:0;width:100%;background-color:{PAGE};">
<div style="display:none;max-height:0;max-width:0;overflow:hidden;opacity:0;mso-hide:all;font-size:1px;line-height:1px;color:{PAGE};">{preview}</div>
<table role="presentation" class="fl-page" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="{PAGE}" style="width:100%;background-color:{PAGE};">
<tr><td class="fl-outer" align="center" style="padding:40px 12px 48px 12px;">
<!--[if mso]><table role="presentation" width="600" align="center" cellpadding="0" cellspacing="0" border="0"><tr><td><![endif]-->
<table role="presentation" width="600" cellpadding="0" cellspacing="0" border="0" style="width:100%;max-width:600px;">
<tr><td style="padding:0 6px 20px 6px;">{logo}</td></tr>
<tr><td class="fl-card" bgcolor="{CARD}" style="background-color:{CARD};border:1px solid {BORDER};border-radius:20px;box-shadow:0 1px 2px rgba(28,25,23,0.04),0 12px 32px -12px rgba(28,25,23,0.10);">{card}</td></tr>
<tr><td align="center" style="padding:28px 24px 0 24px;text-align:center;">{footer}</td></tr>
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
            "-- \nYou get this because <you> asked.\nQuestions? Write to help@great-co.de",
            "Legal notice: https://flow-like.com/legal-notice",
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
    fn gmail_keeps_the_phone_layout_when_it_drops_the_dark_colors() {
        let html = sample().html();
        let base = html.find(".fl-stack-label").unwrap();
        let base_closes = base + html[base..].find("</style>").unwrap();
        let theme = html.find("[data-ogsc]").unwrap();
        assert!(base_closes < theme);
    }

    #[test]
    fn a_secondary_button_sits_beside_the_primary_whose_link_closes_the_card() {
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

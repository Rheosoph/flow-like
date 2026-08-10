//! Markdown → PDF layout engine, styled to the Flow-Like design language.
//!
//! Produces a paginated, text-selectable PDF from GitHub-flavoured Markdown using the base-14
//! fonts. Text is measured with the real Helvetica/Courier AFM widths rather than a fixed
//! character ratio, so wrapping matches what the viewer draws.
//!
//! Deliberately independent of [`crate::document::openxml::markdown_to_runs`]: that flattening
//! loses list depth and ordinal position, which page layout needs.

use flow_like_types::images::image;
use lopdf::{Document, Object, Stream, dictionary};
use std::collections::HashMap;

use crate::document::chart::{
    ChartLayout, ChartType, OfficeChartData, chart_input_to_office_data, parse_chart_block,
};

// ---------------------------------------------------------------------------
// Flow-Like palette
//
// Mirrored from the design tokens in packages/ui/global.css, converted from oklch to sRGB so
// generated documents sit next to the product without a colour shift.
// ---------------------------------------------------------------------------

type Rgb = (f64, f64, f64);

/// `--foreground` #121216
const INK: Rgb = (0.0706, 0.0706, 0.0862);
/// `--muted-foreground` #666670
const MUTED: Rgb = (0.4000, 0.4000, 0.4391);
/// `--primary` #FB562D — the brand ember
const ACCENT: Rgb = (0.9844, 0.3372, 0.1763);
/// `--border` #E2DAD5
const BORDER: Rgb = (0.8862, 0.8549, 0.8354);
/// `--secondary` #F2ECE8
const SURFACE: Rgb = (0.9489, 0.9255, 0.9098);
/// A cooler, lighter tint for code blocks and zebra rows.
const SURFACE_SOFT: Rgb = (0.9725, 0.9647, 0.9608);
const WHITE: Rgb = (1.0, 1.0, 1.0);
/// Hairlines inside chart plots — lighter than `--border` so they recede.
const GRID: Rgb = (0.9255, 0.9059, 0.8941);

/// `--fl-chat-chart-1..8`: the categorical ramp, spaced across the hue wheel so a multi-series
/// chart does not read as one colour. `--chart-1..5` deliberately are NOT used here — they span
/// only ~40° of hue.
const CHART_RAMP: [Rgb; 8] = [
    (0.9844, 0.3372, 0.1763),
    (0.5458, 0.3799, 0.8913),
    (0.0338, 0.6818, 0.6829),
    (0.9224, 0.6447, 0.1724),
    (0.1706, 0.4783, 0.8392),
    (0.2607, 0.6677, 0.3777),
    (0.8538, 0.3228, 0.6107),
    (0.3845, 0.4658, 0.5517),
];

fn fill(color: Rgb) -> String {
    format!("{:.4} {:.4} {:.4} rg", color.0, color.1, color.2)
}

fn stroke(color: Rgb) -> String {
    format!("{:.4} {:.4} {:.4} RG", color.0, color.1, color.2)
}

// ---------------------------------------------------------------------------
// Document model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strikethrough: bool,
}

#[derive(Debug, Clone)]
pub enum Block {
    Heading {
        level: u8,
        spans: Vec<Span>,
    },
    Paragraph(Vec<Span>),
    ListItem {
        depth: usize,
        marker: String,
        spans: Vec<Span>,
    },
    Quote(Vec<Span>),
    Code {
        language: Option<String>,
        lines: Vec<String>,
    },
    Rule,
    Image {
        url: String,
        alt: String,
    },
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Chart(Box<OfficeChartData>),
}

/// Page geometry and styling. All measurements are PDF points.
#[derive(Debug, Clone)]
pub struct PdfLayout {
    pub page_width: f64,
    pub page_height: f64,
    pub margin: f64,
    /// Space reserved above the content column, including the running header band.
    pub top_margin: f64,
    pub bottom_margin: f64,
    pub base_font_size: f64,
}

impl Default for PdfLayout {
    fn default() -> Self {
        Self {
            page_width: 595.276, // A4
            page_height: 841.89,
            margin: 62.0,
            top_margin: 76.0,
            bottom_margin: 64.0,
            base_font_size: 10.5,
        }
    }
}

impl PdfLayout {
    pub fn for_page_size(name: &str) -> Self {
        let (page_width, page_height) = match name.to_ascii_lowercase().as_str() {
            "letter" => (612.0, 792.0),
            "legal" => (612.0, 1008.0),
            "a5" => (419.528, 595.276),
            "a3" => (841.89, 1190.551),
            _ => (595.276, 841.89),
        };
        Self {
            page_width,
            page_height,
            ..Default::default()
        }
    }

    fn content_width(&self) -> f64 {
        self.page_width - self.margin * 2.0
    }

    fn start_y(&self) -> f64 {
        self.page_height - self.top_margin
    }

    fn line_height(&self) -> f64 {
        self.base_font_size * 1.55
    }
}

/// An image decoded into something a PDF XObject can carry.
pub struct EmbeddedImage {
    pub width: u32,
    pub height: u32,
    /// Stream payload, already in the encoding named by `filter`.
    pub data: Vec<u8>,
    /// `DCTDecode` for JPEG passthrough, `FlateDecode` for deflated raw samples.
    pub filter: &'static str,
    /// Deflated 8-bit alpha channel, when the source had transparency.
    pub soft_mask: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct PdfMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub page_numbers: bool,
    /// Draw the accent title block at the top of page one.
    pub cover: bool,
}

// ---------------------------------------------------------------------------
// Font metrics
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Font {
    Regular,
    Bold,
    Italic,
    BoldItalic,
    Mono,
}

impl Font {
    fn resource(self) -> &'static str {
        match self {
            Font::Regular => "/F1",
            Font::Bold => "/F2",
            Font::Italic => "/F3",
            Font::BoldItalic => "/F4",
            Font::Mono => "/F5",
        }
    }

    fn for_span(span: &Span) -> Font {
        if span.code {
            Font::Mono
        } else {
            match (span.bold, span.italic) {
                (true, true) => Font::BoldItalic,
                (true, false) => Font::Bold,
                (false, true) => Font::Italic,
                (false, false) => Font::Regular,
            }
        }
    }

    /// Advance width of `text` at `size`, in points.
    fn width(self, text: &str, size: f64) -> f64 {
        let table = match self {
            Font::Mono => return text.chars().count() as f64 * 0.6 * size,
            Font::Bold | Font::BoldItalic => &HELVETICA_BOLD_WIDTHS,
            _ => &HELVETICA_WIDTHS,
        };
        let units: u32 = text
            .chars()
            .map(|ch| {
                let index = ch as usize;
                if (32..127).contains(&index) {
                    table[index - 32] as u32
                } else {
                    // Non-ASCII collapses to WinAnsi; use the average advance.
                    556
                }
            })
            .sum();
        units as f64 / 1000.0 * size
    }
}

/// Helvetica AFM advance widths for ASCII 32..=126, per 1000 units.
const HELVETICA_WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667,
    611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500,
    222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// Helvetica-Bold AFM advance widths for ASCII 32..=126, per 1000 units.
const HELVETICA_BOLD_WIDTHS: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556,
    556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667,
    611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667,
    667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556,
    278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];

/// Encode text as WinAnsi bytes with PDF string escaping.
///
/// Base-14 fonts are single-byte; anything outside WinAnsi is transliterated so a stray emoji
/// cannot corrupt the content stream.
fn pdf_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        let byte = match ch {
            '\\' => {
                out.push_str("\\\\");
                continue;
            }
            '(' => {
                out.push_str("\\(");
                continue;
            }
            ')' => {
                out.push_str("\\)");
                continue;
            }
            '\n' | '\r' | '\t' => b' ',
            '\u{2018}' | '\u{2019}' => b'\'',
            '\u{201C}' | '\u{201D}' => b'"',
            '\u{2013}' | '\u{2014}' => b'-',
            '\u{2026}' => {
                out.push_str("...");
                continue;
            }
            '\u{00A0}' => b' ',
            // WinAnsi bullet glyphs, so nested list markers survive.
            '\u{2022}' => 0x95,
            '\u{25E6}' | '\u{25AA}' | '\u{25CB}' => 0x95,
            c if (c as u32) < 128 => c as u8,
            c if (c as u32) <= 255 => c as u32 as u8,
            _ => b'?',
        };
        if !(32..=126).contains(&byte) {
            out.push_str(&format!("\\{byte:03o}"));
        } else {
            out.push(byte as char);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Markdown → blocks
// ---------------------------------------------------------------------------

fn is_chart_language(language: &Option<String>) -> bool {
    language
        .as_deref()
        .is_some_and(|l| l == "nivo" || l == "plotly")
}

/// Parse GitHub-flavoured Markdown into the layout engine's block model.
pub fn parse_markdown(markdown: &str) -> Vec<Block> {
    use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(markdown, options);

    let mut blocks: Vec<Block> = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    let mut style = Span::default();
    let mut list_stack: Vec<Option<u64>> = Vec::new();
    let mut heading_level: Option<u8> = None;
    let mut in_quote = false;
    let mut code: Option<(Option<String>, String)> = None;
    let mut pending_marker: Option<String> = None;

    let mut table_header: Vec<String> = Vec::new();
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut table_row: Vec<String> = Vec::new();
    let mut in_table_head = false;
    let mut cell = String::new();
    let mut in_cell = false;

    let flush = |spans: &mut Vec<Span>| -> Vec<Span> {
        std::mem::take(spans)
            .into_iter()
            .filter(|span| !span.text.is_empty())
            .collect()
    };

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Item => {
                    let depth = list_stack.len().max(1);
                    let marker = match list_stack.last_mut() {
                        Some(Some(index)) => {
                            let current = *index;
                            *index += 1;
                            format!("{current}.")
                        }
                        _ => bullet_for_depth(depth).to_string(),
                    };
                    pending_marker = Some(marker);
                }
                Tag::Paragraph => {}
                Tag::Heading { level, .. } => {
                    heading_level = Some(match level {
                        HeadingLevel::H1 => 1,
                        HeadingLevel::H2 => 2,
                        HeadingLevel::H3 => 3,
                        HeadingLevel::H4 => 4,
                        HeadingLevel::H5 => 5,
                        HeadingLevel::H6 => 6,
                    });
                }
                Tag::Strong => style.bold = true,
                Tag::Emphasis => style.italic = true,
                Tag::Strikethrough => style.strikethrough = true,
                Tag::CodeBlock(kind) => {
                    let language = match kind {
                        pulldown_cmark::CodeBlockKind::Fenced(lang) => {
                            let lang = lang.trim().to_string();
                            (!lang.is_empty()).then_some(lang)
                        }
                        pulldown_cmark::CodeBlockKind::Indented => None,
                    };
                    code = Some((language, String::new()));
                }
                Tag::List(start) => {
                    // A nested list opens *before* its parent item ends, so the parent's text is
                    // still sitting in `spans`. Emit it now or it is swallowed by the child item.
                    let collected = flush(&mut spans);
                    if !collected.is_empty()
                        && let Some(marker) = pending_marker.take()
                    {
                        blocks.push(Block::ListItem {
                            depth: list_stack.len().max(1),
                            marker,
                            spans: collected,
                        });
                    }
                    list_stack.push(start);
                }
                Tag::BlockQuote(_) => in_quote = true,
                Tag::TableHead => in_table_head = true,
                Tag::TableRow => table_row.clear(),
                Tag::TableCell => {
                    in_cell = true;
                    cell.clear();
                }
                Tag::Image { dest_url, .. } => {
                    blocks.push(Block::Image {
                        url: dest_url.to_string(),
                        alt: String::new(),
                    });
                }
                _ => {}
            },
            Event::End(end) => match end {
                TagEnd::Paragraph => {
                    let collected = flush(&mut spans);
                    if !collected.is_empty() {
                        if let Some(marker) = pending_marker.take() {
                            blocks.push(Block::ListItem {
                                depth: list_stack.len().max(1),
                                marker,
                                spans: collected,
                            });
                        } else if in_quote {
                            blocks.push(Block::Quote(collected));
                        } else {
                            blocks.push(Block::Paragraph(collected));
                        }
                    }
                }
                TagEnd::Item => {
                    let collected = flush(&mut spans);
                    if !collected.is_empty() {
                        blocks.push(Block::ListItem {
                            depth: list_stack.len().max(1),
                            marker: pending_marker.take().unwrap_or_else(|| "\u{2022}".to_string()),
                            spans: collected,
                        });
                    }
                    pending_marker = None;
                }
                TagEnd::Heading(_) => {
                    let collected = flush(&mut spans);
                    if !collected.is_empty() {
                        blocks.push(Block::Heading {
                            level: heading_level.unwrap_or(1),
                            spans: collected,
                        });
                    }
                    heading_level = None;
                }
                TagEnd::Strong => style.bold = false,
                TagEnd::Emphasis => style.italic = false,
                TagEnd::Strikethrough => style.strikethrough = false,
                TagEnd::CodeBlock => {
                    if let Some((language, body)) = code.take() {
                        if is_chart_language(&language)
                            && let Some(input) = parse_chart_block(&body)
                            && let Some(data) = chart_input_to_office_data(&input)
                        {
                            blocks.push(Block::Chart(Box::new(data)));
                            continue;
                        }
                        blocks.push(Block::Code {
                            language,
                            lines: body.lines().map(str::to_string).collect(),
                        });
                    }
                }
                TagEnd::List(_) => {
                    list_stack.pop();
                }
                TagEnd::BlockQuote(_) => in_quote = false,
                TagEnd::Table => {
                    if !table_header.is_empty() || !table_rows.is_empty() {
                        blocks.push(Block::Table {
                            header: std::mem::take(&mut table_header),
                            rows: std::mem::take(&mut table_rows),
                        });
                    }
                }
                TagEnd::TableHead => {
                    in_table_head = false;
                    table_header = std::mem::take(&mut table_row);
                }
                TagEnd::TableRow => {
                    if !in_table_head {
                        table_rows.push(std::mem::take(&mut table_row));
                    }
                }
                TagEnd::TableCell => {
                    in_cell = false;
                    table_row.push(cell.trim().to_string());
                    cell.clear();
                }
                _ => {}
            },
            Event::Text(text) => {
                if let Some((_, body)) = code.as_mut() {
                    body.push_str(&text);
                } else if in_cell {
                    cell.push_str(&text);
                } else {
                    spans.push(Span {
                        text: text.to_string(),
                        ..style.clone()
                    });
                }
            }
            Event::Code(text) => {
                if in_cell {
                    cell.push_str(&text);
                } else {
                    spans.push(Span {
                        text: text.to_string(),
                        code: true,
                        ..style.clone()
                    });
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, body)) = code.as_mut() {
                    body.push('\n');
                } else if in_cell {
                    cell.push(' ');
                } else {
                    spans.push(Span {
                        text: " ".to_string(),
                        ..style.clone()
                    });
                }
            }
            Event::Rule => blocks.push(Block::Rule),
            Event::TaskListMarker(done) => {
                pending_marker = Some(if done { "[x]".into() } else { "[ ]".into() });
            }
            _ => {}
        }
    }

    let trailing = flush(&mut spans);
    if !trailing.is_empty() {
        blocks.push(Block::Paragraph(trailing));
    }

    blocks
}

fn bullet_for_depth(depth: usize) -> &'static str {
    match depth {
        1 => "\u{2022}",
        2 => "\u{25E6}",
        _ => "\u{25AA}",
    }
}

// ---------------------------------------------------------------------------
// Drawing primitives
// ---------------------------------------------------------------------------

struct PageWriter<'a> {
    layout: &'a PdfLayout,
    pages: Vec<String>,
    current: String,
    y: f64,
    /// Image keys (`/Im{n}`) actually drawn, in insertion order.
    used_images: Vec<String>,
}

impl<'a> PageWriter<'a> {
    fn new(layout: &'a PdfLayout) -> Self {
        Self {
            layout,
            pages: Vec::new(),
            current: String::new(),
            y: layout.start_y(),
            used_images: Vec::new(),
        }
    }

    fn push(&mut self, ops: &str) {
        self.current.push_str(ops);
    }

    /// Break to a fresh page unless `needed` points still fit below the cursor.
    fn require(&mut self, needed: f64) {
        if self.y - needed < self.layout.bottom_margin {
            self.break_page();
        }
    }

    fn break_page(&mut self) {
        self.pages.push(std::mem::take(&mut self.current));
        self.y = self.layout.start_y();
    }

    fn finish(mut self) -> (Vec<String>, Vec<String>) {
        if !self.current.trim().is_empty() || self.pages.is_empty() {
            self.pages.push(std::mem::take(&mut self.current));
        }
        (self.pages, self.used_images)
    }

    fn text(&mut self, font: Font, size: f64, color: Rgb, x: f64, y: f64, text: &str) {
        if text.is_empty() {
            return;
        }
        self.push(&format!(
            "BT {} {size:.2} Tf {} {x:.2} {y:.2} Td ({}) Tj ET\n",
            font.resource(),
            fill(color),
            pdf_string(text)
        ));
    }

    fn text_right(&mut self, font: Font, size: f64, color: Rgb, right: f64, y: f64, text: &str) {
        let x = right - font.width(text, size);
        self.text(font, size, color, x, y, text);
    }

    fn text_center(&mut self, font: Font, size: f64, color: Rgb, center: f64, y: f64, text: &str) {
        let x = center - font.width(text, size) / 2.0;
        self.text(font, size, color, x, y, text);
    }

    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, color: Rgb) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.push(&format!(
            "q {} {x:.2} {y:.2} {w:.2} {h:.2} re f Q\n",
            fill(color)
        ));
    }

    fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, color: Rgb, width: f64) {
        self.push(&format!(
            "q {} {width} w {x1:.2} {y1:.2} m {x2:.2} {y2:.2} l S Q\n",
            stroke(color)
        ));
    }

    /// Rounded rectangle, optionally filled and/or stroked.
    fn rounded_rect(
        &mut self,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        radius: f64,
        fill_color: Option<Rgb>,
        stroke_color: Option<Rgb>,
    ) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
        let path = rounded_rect_path(x, y, w, h, r);
        let mut ops = String::from("q ");
        if let Some(color) = fill_color {
            ops.push_str(&fill(color));
            ops.push(' ');
        }
        if let Some(color) = stroke_color {
            ops.push_str(&stroke(color));
            ops.push_str(" 0.7 w ");
        }
        ops.push_str(&path);
        ops.push_str(match (fill_color.is_some(), stroke_color.is_some()) {
            (true, true) => "B Q\n",
            (true, false) => "f Q\n",
            (false, true) => "S Q\n",
            (false, false) => "n Q\n",
        });
        self.push(&ops);
    }
}

/// Kappa for approximating a quarter circle with a cubic bezier.
const KAPPA: f64 = 0.5522847498;

fn rounded_rect_path(x: f64, y: f64, w: f64, h: f64, r: f64) -> String {
    if r <= 0.01 {
        return format!("{x:.2} {y:.2} {w:.2} {h:.2} re ");
    }
    let k = r * KAPPA;
    let (x1, y1) = (x + w, y + h);
    format!(
        "{:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c \
         {:.2} {:.2} l {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c \
         {:.2} {:.2} l {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c \
         {:.2} {:.2} l {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c h ",
        x + r,
        y,
        x1 - r,
        y,
        x1 - r + k,
        y,
        x1,
        y + r - k,
        x1,
        y + r,
        x1,
        y1 - r,
        x1,
        y1 - r + k,
        x1 - r + k,
        y1,
        x1 - r,
        y1,
        x + r,
        y1,
        x + r - k,
        y1,
        x,
        y1 - r + k,
        x,
        y1 - r,
        x,
        y + r,
        x,
        y + r - k,
        x + r - k,
        y,
        x + r,
        y,
    )
}

/// Circular arc approximated with cubic beziers, used for pie and donut slices.
fn arc_path(cx: f64, cy: f64, radius: f64, start: f64, end: f64, move_first: bool) -> String {
    let mut ops = String::new();
    let sweep = end - start;
    let segments = ((sweep.abs() / (std::f64::consts::FRAC_PI_2)).ceil() as usize).max(1);
    let step = sweep / segments as f64;
    let mut angle = start;

    if move_first {
        ops.push_str(&format!(
            "{:.2} {:.2} m ",
            cx + radius * angle.cos(),
            cy + radius * angle.sin()
        ));
    }

    for _ in 0..segments {
        let next = angle + step;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let (x0, y0) = (cx + radius * angle.cos(), cy + radius * angle.sin());
        let (x3, y3) = (cx + radius * next.cos(), cy + radius * next.sin());
        let c1 = (x0 - k * radius * angle.sin(), y0 + k * radius * angle.cos());
        let c2 = (x3 + k * radius * next.sin(), y3 - k * radius * next.cos());
        ops.push_str(&format!(
            "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ",
            c1.0, c1.1, c2.0, c2.1, x3, y3
        ));
        angle = next;
    }
    ops
}

// ---------------------------------------------------------------------------
// Text layout
// ---------------------------------------------------------------------------

/// One visual line: styled chunks plus the width each consumes.
type VisualLine = Vec<(Span, f64)>;

fn span_size(span: &Span, base: f64) -> f64 {
    if span.code { base * 0.9 } else { base }
}

/// Break styled spans into visual lines that fit `max_width`.
fn wrap_spans(spans: &[Span], max_width: f64, size: f64) -> Vec<VisualLine> {
    let mut lines: Vec<VisualLine> = Vec::new();
    let mut line: VisualLine = Vec::new();
    let mut line_width = 0.0;

    for span in spans {
        let font = Font::for_span(span);
        let font_size = span_size(span, size);
        // Keep the separating whitespace attached to the preceding word.
        for word in span.text.split_inclusive(' ') {
            if word.is_empty() {
                continue;
            }
            let word_width = font.width(word, font_size);
            if line_width + word_width > max_width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line_width = 0.0;
                let trimmed = word.trim_start();
                if trimmed.is_empty() {
                    continue;
                }
                let trimmed_width = font.width(trimmed, font_size);
                line.push((
                    Span {
                        text: trimmed.to_string(),
                        ..span.clone()
                    },
                    trimmed_width,
                ));
                line_width = trimmed_width;
                continue;
            }
            line.push((
                Span {
                    text: word.to_string(),
                    ..span.clone()
                },
                word_width,
            ));
            line_width += word_width;
        }
    }

    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

fn wrap_plain(text: &str, max_width: f64, font: Font, size: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if font.width(&candidate, size) > max_width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current = word.to_string();
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn truncate_to_width(text: &str, max_width: f64, font: Font, size: f64) -> String {
    if font.width(text, size) <= max_width {
        return text.to_string();
    }
    let ellipsis = font.width("...", size);
    let mut out = String::new();
    for ch in text.chars() {
        let mut candidate = out.clone();
        candidate.push(ch);
        if font.width(&candidate, size) + ellipsis > max_width {
            break;
        }
        out = candidate;
    }
    out.push_str("...");
    out
}

/// Draw one wrapped line, merging adjacent chunks that share styling.
///
/// `wrap_spans` splits on word boundaries; emitting a separate text op per word would bloat the
/// stream and break text extraction, since the separating spaces live in the positioning rather
/// than in any string.
fn draw_line(writer: &mut PageWriter, line: &VisualLine, left: f64, base: f64) {
    let mut x = left;
    let y = writer.y;
    let mut index = 0;

    while index < line.len() {
        let (span, _) = &line[index];
        let font = Font::for_span(span);
        let size = span_size(span, base);

        let mut text = String::new();
        let mut width = 0.0;
        let mut end = index;
        while end < line.len() {
            let (candidate, candidate_width) = &line[end];
            if Font::for_span(candidate) != font
                || candidate.code != span.code
                || candidate.strikethrough != span.strikethrough
            {
                break;
            }
            text.push_str(&candidate.text);
            width += candidate_width;
            end += 1;
        }

        if span.code {
            // A tinted chip behind inline code, the way it reads in the product.
            writer.rounded_rect(
                x - 1.5,
                y - size * 0.28,
                width + 3.0,
                size * 1.32,
                2.5,
                Some(SURFACE),
                None,
            );
            writer.text(font, size, ACCENT, x, y, text.trim_end());
        } else {
            writer.text(font, size, INK, x, y, &text);
        }

        if span.strikethrough {
            let mid = y + size * 0.3;
            writer.line(x, mid, x + width, mid, INK, 0.6);
        }
        x += width;
        index = end;
    }
}

// ---------------------------------------------------------------------------
// Block rendering
// ---------------------------------------------------------------------------

fn heading_size(level: u8, base: f64) -> f64 {
    match level {
        1 => base * 2.25,
        2 => base * 1.62,
        3 => base * 1.28,
        4 => base * 1.1,
        _ => base,
    }
}

/// Render blocks into per-page content streams.
///
/// `images` maps a markdown image URL to a decoded XObject; URLs absent from the map fall back
/// to a labelled placeholder card so a missing asset never aborts the document.
pub fn render_blocks(
    blocks: &[Block],
    layout: &PdfLayout,
    images: &HashMap<String, EmbeddedImage>,
) -> (Vec<String>, Vec<String>) {
    render_document(blocks, layout, images, &PdfMetadata::default())
}

/// Render blocks, optionally opening with the Flow-Like title block.
pub fn render_document(
    blocks: &[Block],
    layout: &PdfLayout,
    images: &HashMap<String, EmbeddedImage>,
    metadata: &PdfMetadata,
) -> (Vec<String>, Vec<String>) {
    let mut writer = PageWriter::new(layout);
    let base = layout.base_font_size;
    let line_height = layout.line_height();
    let left = layout.margin;
    let content_width = layout.content_width();

    let mut blocks = blocks;
    if metadata.cover && let Some(title) = metadata.title.as_deref() {
        draw_title_block(&mut writer, title, metadata.subject.as_deref(), layout);
        // A document usually opens with the same H1 the cover already sets. Printing both reads
        // as a mistake, so the duplicate is dropped rather than asking authors to strip it.
        if let Some(Block::Heading { level: 1, spans }) = blocks.first() {
            let heading: String = spans.iter().map(|s| s.text.as_str()).collect();
            if heading.trim().eq_ignore_ascii_case(title.trim()) {
                blocks = &blocks[1..];
            }
        }
    }

    for (index, block) in blocks.iter().enumerate() {
        match block {
            Block::Heading { level, spans } => {
                let size = heading_size(*level, base);
                let text: String = spans.iter().map(|s| s.text.as_str()).collect();
                let lines = wrap_plain(&text, content_width, Font::Bold, size);
                // A heading must not be stranded at the foot of a page. Reserve its own height
                // plus enough of whatever follows to prove the pair belongs together — otherwise
                // a chart or table that cannot fit leaves the heading above a page-tall hole.
                let follower = blocks
                    .get(index + 1)
                    .map(|next| keep_with_next_height(next, layout))
                    .unwrap_or(line_height);
                let lead_gap = if *level == 1 { size * 0.7 } else { size * 0.55 };
                writer.require(lead_gap + size * 1.35 * lines.len() as f64 + follower + 10.0);

                writer.y -= lead_gap;

                if *level == 1 {
                    // The brand ember, set as a short rule above the title.
                    let bar_y = writer.y + size * 0.92;
                    writer.rounded_rect(left, bar_y, 30.0, 3.0, 1.5, Some(ACCENT), None);
                    writer.y -= 8.0;
                }

                for line in &lines {
                    writer.require(size * 1.3);
                    let y = writer.y;
                    writer.text(Font::Bold, size, INK, left, y, line);
                    writer.y -= size * 1.3;
                }

                if *level == 2 {
                    let y = writer.y + size * 0.5;
                    writer.line(left, y, left + content_width, y, BORDER, 0.7);
                    writer.y -= 4.0;
                }
                writer.y -= base * 0.25;
            }

            Block::Paragraph(spans) => {
                for line in &wrap_spans(spans, content_width, base) {
                    writer.require(line_height);
                    draw_line(&mut writer, line, left, base);
                    writer.y -= line_height;
                }
                writer.y -= base * 0.55;
            }

            Block::ListItem {
                depth,
                marker,
                spans,
            } => {
                let indent = (*depth as f64 - 1.0) * 16.0;
                let gutter = 16.0;
                let text_left = left + indent + gutter;
                let lines = wrap_spans(spans, content_width - indent - gutter, base);

                for (index, line) in lines.iter().enumerate() {
                    writer.require(line_height);
                    if index == 0 {
                        draw_list_marker(&mut writer, marker, left + indent, base);
                    }
                    draw_line(&mut writer, line, text_left, base);
                    writer.y -= line_height;
                }
                writer.y -= base * 0.18;
            }

            Block::Quote(spans) => {
                let indent = 20.0;
                let lines = wrap_spans(spans, content_width - indent - 12.0, base);
                let mut top = writer.y + base * 0.95;

                for line in &lines {
                    if writer.y - line_height < layout.bottom_margin {
                        close_quote_band(&mut writer, left, top, content_width, base);
                        writer.break_page();
                        top = writer.y + base * 0.95;
                    }
                    draw_line(&mut writer, line, left + indent, base);
                    writer.y -= line_height;
                }
                close_quote_band(&mut writer, left, top, content_width, base);
                writer.y -= base * 0.6;
            }

            Block::Code { language, lines } => {
                draw_code_block(&mut writer, language.as_deref(), lines, layout);
            }

            Block::Rule => {
                writer.require(line_height * 2.4);
                writer.y -= line_height * 0.8;
                let y = writer.y;
                writer.rounded_rect(
                    left + content_width / 2.0 - 24.0,
                    y,
                    48.0,
                    2.0,
                    1.0,
                    Some(BORDER),
                    None,
                );
                writer.y -= line_height * 1.4;
            }

            Block::Image { url, alt } => {
                draw_image(&mut writer, url, alt, layout, images);
            }

            Block::Table { header, rows } => {
                draw_table(&mut writer, header, rows, layout);
            }

            Block::Chart(data) => {
                draw_chart(&mut writer, data, layout);
            }
        }
    }

    writer.finish()
}

/// How much vertical space a block needs before it is worth starting on this page.
///
/// Used only for heading widow control: a heading reserves its own height plus this, so a
/// heading and the thing it names stay together.
fn keep_with_next_height(block: &Block, layout: &PdfLayout) -> f64 {
    let base = layout.base_font_size;
    match block {
        // The chart card is atomic — it never splits, so it is all or nothing.
        Block::Chart(data) => chart_card_height(data, base) + base,
        // Header plus two body rows is enough to show the table has started.
        Block::Table { .. } => base * 0.88 * 2.1 * 3.0,
        Block::Image { .. } => 120.0,
        Block::Code { lines, .. } => {
            let shown = lines.len().min(3) as f64;
            base * 0.86 * 1.5 * shown + 20.0
        }
        _ => layout.line_height(),
    }
}

fn draw_title_block(
    writer: &mut PageWriter,
    title: &str,
    subject: Option<&str>,
    layout: &PdfLayout,
) {
    let left = layout.margin;
    let content_width = layout.content_width();
    let size = layout.base_font_size * 2.6;

    writer.rounded_rect(left, writer.y + size * 0.55, 42.0, 3.5, 1.75, Some(ACCENT), None);
    writer.y -= 14.0;

    for line in wrap_plain(title, content_width, Font::Bold, size) {
        let y = writer.y;
        writer.text(Font::Bold, size, INK, left, y, &line);
        writer.y -= size * 1.18;
    }

    if let Some(subject) = subject.filter(|s| !s.is_empty()) {
        writer.y -= 2.0;
        for line in wrap_plain(subject, content_width, Font::Regular, layout.base_font_size * 1.1) {
            let y = writer.y;
            writer.text(
                Font::Regular,
                layout.base_font_size * 1.1,
                MUTED,
                left,
                y,
                &line,
            );
            writer.y -= layout.base_font_size * 1.5;
        }
    }

    writer.y -= 6.0;
    let rule_y = writer.y;
    writer.line(left, rule_y, left + content_width, rule_y, BORDER, 0.7);
    writer.y -= layout.base_font_size * 2.2;
}

fn draw_list_marker(writer: &mut PageWriter, marker: &str, x: f64, base: f64) {
    let y = writer.y;
    match marker {
        "[x]" | "[ ]" => {
            let box_size = base * 0.85;
            let box_y = y - box_size * 0.18;
            let checked = marker == "[x]";
            writer.rounded_rect(
                x,
                box_y,
                box_size,
                box_size,
                2.0,
                Some(if checked { ACCENT } else { WHITE }),
                Some(if checked { ACCENT } else { BORDER }),
            );
            if checked {
                // A check drawn as two strokes, so it stays crisp at any size.
                writer.push(&format!(
                    "q {} 1.2 w 1 J 1 j {:.2} {:.2} m {:.2} {:.2} l {:.2} {:.2} l S Q\n",
                    stroke(WHITE),
                    x + box_size * 0.24,
                    box_y + box_size * 0.52,
                    x + box_size * 0.44,
                    box_y + box_size * 0.30,
                    x + box_size * 0.78,
                    box_y + box_size * 0.72,
                ));
            }
        }
        marker if marker.ends_with('.') => {
            writer.text(Font::Bold, base * 0.92, MUTED, x, y, marker);
        }
        marker => {
            writer.text(Font::Regular, base, ACCENT, x, y, marker);
        }
    }
}

fn close_quote_band(writer: &mut PageWriter, left: f64, top: f64, content_width: f64, base: f64) {
    let bottom = writer.y + base * 0.95;
    let height = top - bottom;
    if height <= 0.0 {
        return;
    }
    // Painted after the text, so the fill would hide it — draw only the bar and a hairline edge.
    writer.rounded_rect(left, bottom, 3.0, height, 1.5, Some(ACCENT), None);
    let _ = content_width;
}

fn draw_code_block(
    writer: &mut PageWriter,
    language: Option<&str>,
    lines: &[String],
    layout: &PdfLayout,
) {
    let left = layout.margin;
    let content_width = layout.content_width();
    let size = layout.base_font_size * 0.86;
    let code_line_height = size * 1.5;
    let padding = 10.0;

    let mut wrapped: Vec<String> = Vec::new();
    for line in lines {
        wrapped.extend(wrap_code_line(line, content_width - padding * 2.0, size));
    }
    if wrapped.is_empty() {
        wrapped.push(String::new());
    }

    let mut index = 0;
    while index < wrapped.len() {
        writer.require(code_line_height * 2.0 + padding * 2.0);
        let available = ((writer.y - layout.bottom_margin - padding * 2.0) / code_line_height)
            .floor()
            .max(1.0) as usize;
        let take = available.min(wrapped.len() - index);
        let block_height = take as f64 * code_line_height + padding * 2.0;
        let top = writer.y + size;
        let rect_y = top - block_height;

        writer.rounded_rect(
            left,
            rect_y,
            content_width,
            block_height,
            6.0,
            Some(SURFACE_SOFT),
            Some(BORDER),
        );

        if index == 0
            && let Some(language) = language.filter(|l| !l.is_empty())
        {
            writer.text_right(
                Font::Regular,
                size * 0.82,
                MUTED,
                left + content_width - padding,
                top - padding - size * 0.85,
                &language.to_uppercase(),
            );
        }

        writer.y = top - padding - size;
        for line in &wrapped[index..index + take] {
            let y = writer.y;
            writer.text(Font::Mono, size, INK, left + padding, y, line);
            writer.y -= code_line_height;
        }
        writer.y = rect_y - layout.base_font_size * 0.75;
        index += take;
    }
}

fn wrap_code_line(line: &str, max_width: f64, size: f64) -> Vec<String> {
    if Font::Mono.width(line, size) <= max_width {
        return vec![line.to_string()];
    }
    let per_char = Font::Mono.width("0", size).max(0.01);
    let chars_per_line = (max_width / per_char).floor().max(1.0) as usize;
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in line.chars() {
        current.push(ch);
        if current.chars().count() >= chars_per_line {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn draw_image(
    writer: &mut PageWriter,
    url: &str,
    alt: &str,
    layout: &PdfLayout,
    images: &HashMap<String, EmbeddedImage>,
) {
    let content_width = layout.content_width();
    let left = layout.margin;
    let base = layout.base_font_size;

    let Some(image) = images.get(url) else {
        writer.require(48.0);
        let top = writer.y + base;
        writer.rounded_rect(
            left,
            top - 40.0,
            content_width,
            40.0,
            6.0,
            Some(SURFACE_SOFT),
            Some(BORDER),
        );
        let label = if alt.is_empty() { url } else { alt };
        writer.text(
            Font::Italic,
            base * 0.88,
            MUTED,
            left + 12.0,
            top - 25.0,
            &truncate_to_width(
                &format!("Image unavailable — {label}"),
                content_width - 24.0,
                Font::Italic,
                base * 0.88,
            ),
        );
        writer.y = top - 40.0 - base;
        return;
    };

    let aspect = image.height as f64 / image.width.max(1) as f64;
    let mut draw_width = content_width.min(image.width as f64);
    let mut draw_height = draw_width * aspect;
    let available = layout.start_y() - layout.bottom_margin - base * 2.0;
    if draw_height > available {
        let scale = available / draw_height;
        draw_width *= scale;
        draw_height *= scale;
    }

    writer.require(draw_height + base * 2.0);

    let key = match writer.used_images.iter().position(|used| used == url) {
        Some(index) => format!("/Im{index}"),
        None => {
            writer.used_images.push(url.to_string());
            format!("/Im{}", writer.used_images.len() - 1)
        }
    };

    let x = left + (content_width - draw_width) / 2.0;
    let y = writer.y - draw_height;
    writer.push(&format!(
        "q {draw_width:.2} 0 0 {draw_height:.2} {x:.2} {y:.2} cm {key} Do Q\n"
    ));
    writer.rounded_rect(x, y, draw_width, draw_height, 4.0, None, Some(BORDER));
    writer.y -= draw_height + base * 0.9;

    if !alt.is_empty() {
        writer.require(base * 1.6);
        let caption_y = writer.y;
        writer.text_center(
            Font::Italic,
            base * 0.85,
            MUTED,
            left + content_width / 2.0,
            caption_y,
            &truncate_to_width(alt, content_width, Font::Italic, base * 0.85),
        );
        writer.y -= base * 1.6;
    }
}

fn is_numeric_cell(text: &str) -> bool {
    let cleaned: String = text
        .chars()
        .filter(|c| !matches!(c, ',' | ' ' | '%' | '$' | '\u{20AC}' | '\u{00A3}'))
        .collect();
    !cleaned.is_empty() && cleaned.parse::<f64>().is_ok()
}

fn draw_table(writer: &mut PageWriter, header: &[String], rows: &[Vec<String>], layout: &PdfLayout) {
    let columns = header.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return;
    }
    let left = layout.margin;
    let content_width = layout.content_width();
    let base = layout.base_font_size;
    let size = base * 0.88;
    let row_height = size * 2.1;
    let padding = 8.0;

    // Columns are sized by their widest cell, then normalised to the content width, so a narrow
    // "Qty" column does not get the same space as a prose column.
    let mut weights = vec![0.0_f64; columns];
    for index in 0..columns {
        let head_width = header
            .get(index)
            .map(|cell| Font::Bold.width(cell, size))
            .unwrap_or(0.0);
        let body_width = rows
            .iter()
            .filter_map(|row| row.get(index))
            .map(|cell| Font::Regular.width(cell, size))
            .fold(0.0_f64, f64::max);
        weights[index] = head_width.max(body_width).max(size * 2.0) + padding * 2.0;
    }
    let total: f64 = weights.iter().sum();
    let widths: Vec<f64> = weights
        .iter()
        .map(|w| w / total * content_width)
        .collect();
    let offsets: Vec<f64> = widths
        .iter()
        .scan(0.0, |acc, w| {
            let x = *acc;
            *acc += w;
            Some(x)
        })
        .collect();

    let numeric: Vec<bool> = (0..columns)
        .map(|index| {
            let cells: Vec<&String> = rows.iter().filter_map(|row| row.get(index)).collect();
            !cells.is_empty()
                && cells
                    .iter()
                    .all(|cell| cell.is_empty() || is_numeric_cell(cell))
        })
        .collect();

    let draw_header_row = |writer: &mut PageWriter| {
        if header.is_empty() {
            return;
        }
        writer.require(row_height * 2.0);
        let top = writer.y + size;
        writer.push(&format!(
            "q {} {}f Q\n",
            fill(ACCENT),
            rounded_rect_path(left, top - row_height, content_width, row_height, 5.0)
        ));
        // Square off the bottom corners so the body sits flush against the header.
        writer.rect(
            left,
            top - row_height,
            content_width,
            row_height / 2.0,
            ACCENT,
        );
        for index in 0..columns {
            let cell = header.get(index).map(String::as_str).unwrap_or("");
            let inner = widths[index] - padding * 2.0;
            let text = truncate_to_width(cell, inner, Font::Bold, size);
            let y = top - row_height + size * 0.62;
            if numeric[index] {
                writer.text_right(
                    Font::Bold,
                    size,
                    WHITE,
                    left + offsets[index] + widths[index] - padding,
                    y,
                    &text,
                );
            } else {
                writer.text(Font::Bold, size, WHITE, left + offsets[index] + padding, y, &text);
            }
        }
        writer.y = top - row_height - size;
    };

    draw_header_row(writer);

    for (row_index, row) in rows.iter().enumerate() {
        if writer.y - row_height < layout.bottom_margin {
            writer.break_page();
            draw_header_row(writer);
        }
        let top = writer.y + size;
        if row_index % 2 == 1 {
            writer.rect(left, top - row_height, content_width, row_height, SURFACE_SOFT);
        }
        for index in 0..columns {
            let cell = row.get(index).map(String::as_str).unwrap_or("");
            let inner = widths[index] - padding * 2.0;
            let text = truncate_to_width(cell, inner, Font::Regular, size);
            let y = top - row_height + size * 0.62;
            if numeric[index] {
                writer.text_right(
                    Font::Regular,
                    size,
                    INK,
                    left + offsets[index] + widths[index] - padding,
                    y,
                    &text,
                );
            } else {
                writer.text(
                    Font::Regular,
                    size,
                    INK,
                    left + offsets[index] + padding,
                    y,
                    &text,
                );
            }
        }
        let border_y = top - row_height;
        writer.line(left, border_y, left + content_width, border_y, BORDER, 0.5);
        writer.y = border_y - size;
    }
    writer.y -= base * 0.9;
}

// ---------------------------------------------------------------------------
// Charts
//
// Fed by the ```nivo / ```plotly fences the product already understands: the shared
// `document::chart` parser handles the `key: value` front matter (type, title, colors, stacked,
// layout) and the CSV body, then flattens to `OfficeChartData`.
// ---------------------------------------------------------------------------

/// Round a value up to a "nice" axis maximum and return (max, step).
fn nice_axis(max: f64, target_ticks: usize) -> (f64, f64) {
    if !max.is_finite() || max <= 0.0 {
        return (1.0, 0.25);
    }
    let raw_step = max / target_ticks.max(1) as f64;
    let magnitude = 10_f64.powf(raw_step.log10().floor());
    let normalized = raw_step / magnitude;
    // Rounding rather than ceiling: a raw step of 1.05 should settle on 1, not jump to 2 and
    // stretch the axis half again as tall as the data.
    let step = magnitude
        * if normalized <= 1.5 {
            1.0
        } else if normalized <= 2.25 {
            2.0
        } else if normalized <= 3.5 {
            2.5
        } else if normalized <= 7.5 {
            5.0
        } else {
            10.0
        };
    let axis_max = (max / step).ceil() * step;
    (axis_max, step)
}

fn format_tick(value: f64) -> String {
    let magnitude = value.abs();
    if magnitude >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if magnitude >= 1_000.0 {
        format!("{:.1}k", value / 1_000.0)
    } else if magnitude >= 10.0 || value.fract().abs() < 1e-9 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

fn series_color(index: usize, data: &OfficeChartData) -> Rgb {
    data.colors
        .get(index)
        .map(|hex| parse_hex(hex))
        .unwrap_or_else(|| CHART_RAMP[index % CHART_RAMP.len()])
}

/// Height of the whole chart card. Shared with the widow-control estimate so a heading can never
/// reserve less space than the chart it introduces actually takes.
fn chart_card_height(data: &OfficeChartData, base: f64) -> f64 {
    let legend_rows = if data.series.len() > 1 { 1 } else { 0 };
    let title_height = if data.title.is_some() { base * 2.0 } else { 0.0 };
    title_height + CHART_PLOT_HEIGHT + 34.0 + legend_rows as f64 * (base * 1.8) + 20.0
}

const CHART_PLOT_HEIGHT: f64 = 168.0;

fn draw_chart(writer: &mut PageWriter, data: &OfficeChartData, layout: &PdfLayout) {
    let left = layout.margin;
    let base = layout.base_font_size;
    let card_width = layout.content_width();
    let plot_height = CHART_PLOT_HEIGHT;

    let title_height = if data.title.is_some() { base * 2.0 } else { 0.0 };
    let card_height = chart_card_height(data, base);

    writer.require(card_height + base);

    let card_top = writer.y + base;
    let card_bottom = card_top - card_height;
    writer.rounded_rect(
        left,
        card_bottom,
        card_width,
        card_height,
        8.0,
        Some(WHITE),
        Some(BORDER),
    );

    let mut cursor = card_top - 16.0;
    if let Some(title) = &data.title {
        writer.text(
            Font::Bold,
            base * 1.05,
            INK,
            left + 16.0,
            cursor - base,
            &truncate_to_width(title, card_width - 32.0, Font::Bold, base * 1.05),
        );
        cursor -= title_height;
    }

    let axis_label_width = 34.0;
    let plot_left = left + 16.0 + axis_label_width;
    let plot_right = left + card_width - 16.0;
    let plot_top = cursor - 6.0;
    let plot_bottom = plot_top - plot_height;
    let plot_width = plot_right - plot_left;

    match data.chart_type {
        ChartType::Pie => draw_pie(
            writer,
            data,
            left + 16.0,
            plot_bottom,
            card_width - 32.0,
            plot_height,
            base,
        ),
        ChartType::Radar => draw_radar(
            writer,
            data,
            (plot_left + plot_right) / 2.0,
            (plot_top + plot_bottom) / 2.0,
            plot_height / 2.0 - 12.0,
            base,
        ),
        ChartType::Funnel => draw_funnel(
            writer,
            data,
            plot_left,
            plot_bottom,
            plot_width,
            plot_height,
            base,
        ),
        _ => draw_cartesian(
            writer,
            data,
            plot_left,
            plot_bottom,
            plot_width,
            plot_height,
            base,
        ),
    }

    if data.series.len() > 1 {
        draw_legend(
            writer,
            data,
            left + 16.0,
            plot_bottom - 30.0,
            card_width - 32.0,
            base,
        );
    }

    writer.y = card_bottom - base * 1.2;
}

/// Bar, column, line, area and scatter share one axis frame.
fn draw_cartesian(
    writer: &mut PageWriter,
    data: &OfficeChartData,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    base: f64,
) {
    let horizontal = data.layout == ChartLayout::Horizontal;
    let stacked = data.stacked;
    let tick_size = base * 0.72;

    let max_value = if stacked {
        (0..data.categories.len().max(1))
            .map(|index| {
                data.series
                    .iter()
                    .map(|series| series.values.get(index).copied().unwrap_or(0.0).max(0.0))
                    .sum::<f64>()
            })
            .fold(0.0_f64, f64::max)
    } else {
        data.series
            .iter()
            .flat_map(|series| &series.values)
            .copied()
            .fold(0.0_f64, f64::max)
    };
    let (axis_max, step) = nice_axis(max_value, 4);

    // Gridlines and value labels
    let mut tick = 0.0;
    while tick <= axis_max + step / 2.0 {
        let ratio = tick / axis_max;
        if horizontal {
            let gx = x + ratio * width;
            writer.line(gx, y, gx, y + height, GRID, 0.5);
            writer.text_center(Font::Regular, tick_size, MUTED, gx, y - 12.0, &format_tick(tick));
        } else {
            let gy = y + ratio * height;
            writer.line(x, gy, x + width, gy, GRID, 0.5);
            writer.text_right(
                Font::Regular,
                tick_size,
                MUTED,
                x - 6.0,
                gy - tick_size * 0.35,
                &format_tick(tick),
            );
        }
        tick += step;
    }
    writer.line(x, y, x + width, y, BORDER, 0.8);

    let categories = data.categories.len().max(1);
    let series_count = data.series.len().max(1);

    match data.chart_type {
        ChartType::Line | ChartType::Area | ChartType::Scatter => {
            let step_x = if categories > 1 {
                width / (categories - 1) as f64
            } else {
                width
            };
            for (index, series) in data.series.iter().enumerate() {
                let color = series_color(index, data);
                let points: Vec<(f64, f64)> = series
                    .values
                    .iter()
                    .enumerate()
                    .map(|(i, value)| {
                        (
                            x + i as f64 * step_x,
                            y + (value / axis_max).clamp(0.0, 1.0) * height,
                        )
                    })
                    .collect();
                if points.is_empty() {
                    continue;
                }

                if data.chart_type == ChartType::Area {
                    let mut path = format!("{:.2} {:.2} m ", points[0].0, y);
                    for (px, py) in &points {
                        path.push_str(&format!("{px:.2} {py:.2} l "));
                    }
                    path.push_str(&format!("{:.2} {:.2} l h ", points[points.len() - 1].0, y));
                    // No alpha without an ExtGState, so the fill is a light tint of the series hue.
                    let tint = (
                        color.0 * 0.22 + 0.78,
                        color.1 * 0.22 + 0.78,
                        color.2 * 0.22 + 0.78,
                    );
                    writer.push(&format!("q {} {path}f Q\n", fill(tint)));
                }

                if data.chart_type != ChartType::Scatter {
                    let mut path = format!("{:.2} {:.2} m ", points[0].0, points[0].1);
                    for (px, py) in points.iter().skip(1) {
                        path.push_str(&format!("{px:.2} {py:.2} l "));
                    }
                    writer.push(&format!("q {} 1.8 w 1 J 1 j {path}S Q\n", stroke(color)));
                }

                for (px, py) in &points {
                    let radius = if data.chart_type == ChartType::Scatter {
                        3.2
                    } else {
                        2.4
                    };
                    writer.push(&format!(
                        "q {} {}f Q\n",
                        fill(color),
                        arc_path(*px, *py, radius, 0.0, std::f64::consts::TAU, true)
                    ));
                }
            }

            for (index, category) in data.categories.iter().enumerate() {
                let cx = x + index as f64 * step_x;
                writer.text_center(
                    Font::Regular,
                    tick_size,
                    MUTED,
                    cx,
                    y - 14.0,
                    &truncate_to_width(category, step_x.max(28.0), Font::Regular, tick_size),
                );
            }
        }

        _ => {
            // Bar / column
            let band = if horizontal { height } else { width } / categories as f64;
            let bar_slot = band * 0.72;
            let bar_width = if stacked {
                bar_slot
            } else {
                bar_slot / series_count as f64
            };
            let pad = (band - bar_slot) / 2.0;

            for category in 0..categories {
                let mut offset = 0.0_f64;
                for (index, series) in data.series.iter().enumerate() {
                    let value = series.values.get(category).copied().unwrap_or(0.0).max(0.0);
                    let extent = (value / axis_max).clamp(0.0, 1.0)
                        * if horizontal { width } else { height };
                    let color = series_color(index, data);

                    if horizontal {
                        let by = y + height
                            - (category as f64 * band + pad)
                            - bar_width * if stacked { 1.0 } else { index as f64 + 1.0 };
                        let bx = x + if stacked { offset } else { 0.0 };
                        writer.rounded_rect(bx, by, extent.max(0.6), bar_width * 0.9, 2.0, Some(color), None);
                    } else {
                        let bx = x
                            + category as f64 * band
                            + pad
                            + if stacked { 0.0 } else { index as f64 * bar_width };
                        let by = y + if stacked { offset } else { 0.0 };
                        writer.rounded_rect(bx, by, bar_width * 0.9, extent.max(0.6), 2.0, Some(color), None);
                    }
                    if stacked {
                        offset += extent;
                    }
                }
            }

            for (index, category) in data.categories.iter().enumerate() {
                if horizontal {
                    let cy = y + height - (index as f64 * band + band / 2.0);
                    writer.text_right(
                        Font::Regular,
                        tick_size,
                        MUTED,
                        x - 6.0,
                        cy - tick_size * 0.35,
                        &truncate_to_width(category, 40.0, Font::Regular, tick_size),
                    );
                } else {
                    let cx = x + index as f64 * band + band / 2.0;
                    writer.text_center(
                        Font::Regular,
                        tick_size,
                        MUTED,
                        cx,
                        y - 14.0,
                        &truncate_to_width(category, band, Font::Regular, tick_size),
                    );
                }
            }
        }
    }
}

fn draw_pie(
    writer: &mut PageWriter,
    data: &OfficeChartData,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    base: f64,
) {
    let Some(series) = data.series.first() else {
        return;
    };
    let total: f64 = series.values.iter().map(|v| v.max(0.0)).sum();
    if total <= 0.0 {
        return;
    }

    let radius = (height / 2.0 - 8.0).min(width / 3.2);
    let cx = x + radius + 12.0;
    let cy = y + height / 2.0;
    let inner = radius * 0.56;
    let mut angle = std::f64::consts::FRAC_PI_2;

    for (index, value) in series.values.iter().enumerate() {
        let fraction = value.max(0.0) / total;
        if fraction <= 0.0 {
            continue;
        }
        let end = angle - fraction * std::f64::consts::TAU;
        let color = series_color(index, data);

        // Donut segment: outer arc forward, inner arc back.
        let mut path = arc_path(cx, cy, radius, angle, end, true);
        path.push_str(&format!(
            "{:.2} {:.2} l ",
            cx + inner * end.cos(),
            cy + inner * end.sin()
        ));
        path.push_str(&arc_path(cx, cy, inner, end, angle, false));
        path.push_str("h ");
        writer.push(&format!("q {} {path}f Q\n", fill(color)));
        angle = end;
    }

    // Percentage legend to the right of the donut.
    let legend_x = cx + radius + 22.0;
    let mut legend_y = cy + (series.values.len() as f64 * base * 1.5) / 2.0 - base;
    for (index, value) in series.values.iter().enumerate() {
        let label = data
            .categories
            .get(index)
            .map(String::as_str)
            .unwrap_or("\u{2014}");
        let percent = value.max(0.0) / total * 100.0;
        writer.rounded_rect(
            legend_x,
            legend_y - base * 0.1,
            base * 0.7,
            base * 0.7,
            2.0,
            Some(series_color(index, data)),
            None,
        );
        let text = format!("{label}  {percent:.0}%");
        writer.text(
            Font::Regular,
            base * 0.82,
            INK,
            legend_x + base * 1.2,
            legend_y,
            &truncate_to_width(&text, x + width - legend_x - base * 1.5, Font::Regular, base * 0.82),
        );
        legend_y -= base * 1.5;
    }
}

fn draw_radar(
    writer: &mut PageWriter,
    data: &OfficeChartData,
    cx: f64,
    cy: f64,
    radius: f64,
    base: f64,
) {
    let axes = data.categories.len().max(3);
    let max_value = data
        .series
        .iter()
        .flat_map(|series| &series.values)
        .copied()
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let angle_at = |index: usize| {
        std::f64::consts::FRAC_PI_2 - (index as f64 / axes as f64) * std::f64::consts::TAU
    };

    // Web rings
    for ring in 1..=4 {
        let r = radius * ring as f64 / 4.0;
        let mut path = String::new();
        for index in 0..axes {
            let a = angle_at(index);
            let (px, py) = (cx + r * a.cos(), cy + r * a.sin());
            path.push_str(&format!(
                "{px:.2} {py:.2} {} ",
                if index == 0 { "m" } else { "l" }
            ));
        }
        path.push_str("h ");
        writer.push(&format!("q {} 0.5 w {path}S Q\n", stroke(GRID)));
    }
    for index in 0..axes {
        let a = angle_at(index);
        writer.line(cx, cy, cx + radius * a.cos(), cy + radius * a.sin(), GRID, 0.5);
    }

    for (series_index, series) in data.series.iter().enumerate() {
        let color = series_color(series_index, data);
        let mut path = String::new();
        for index in 0..axes {
            let value = series.values.get(index).copied().unwrap_or(0.0).max(0.0);
            let r = (value / max_value).clamp(0.0, 1.0) * radius;
            let a = angle_at(index);
            let (px, py) = (cx + r * a.cos(), cy + r * a.sin());
            path.push_str(&format!(
                "{px:.2} {py:.2} {} ",
                if index == 0 { "m" } else { "l" }
            ));
        }
        path.push_str("h ");
        writer.push(&format!("q {} 1.6 w 1 j {path}S Q\n", stroke(color)));
    }

    for (index, category) in data.categories.iter().enumerate().take(axes) {
        let a = angle_at(index);
        let label_r = radius + 12.0;
        writer.text_center(
            Font::Regular,
            base * 0.72,
            MUTED,
            cx + label_r * a.cos(),
            cy + label_r * a.sin() - base * 0.25,
            &truncate_to_width(category, 56.0, Font::Regular, base * 0.72),
        );
    }
}

fn draw_funnel(
    writer: &mut PageWriter,
    data: &OfficeChartData,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    base: f64,
) {
    let Some(series) = data.series.first() else {
        return;
    };
    let stages = series.values.len();
    if stages == 0 {
        return;
    }
    let max_value = series
        .values
        .iter()
        .copied()
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let stage_height = height / stages as f64;
    let label_width = 92.0;
    // Reserve room on the right for the value label, so the longest bar cannot push it off the
    // edge of the chart card.
    let value_gutter = series
        .values
        .iter()
        .map(|value| Font::Bold.width(&format_tick(*value), base * 0.78))
        .fold(0.0_f64, f64::max)
        + 12.0;
    let bar_area = (width - label_width - value_gutter).max(24.0);

    for (index, value) in series.values.iter().enumerate() {
        let ratio = (value.max(0.0) / max_value).clamp(0.0, 1.0);
        let bar_width = bar_area * ratio;
        let top = y + height - index as f64 * stage_height;
        let bar_height = stage_height * 0.68;
        let bar_y = top - stage_height + (stage_height - bar_height) / 2.0;
        let color = series_color(index, data);

        writer.text_right(
            Font::Regular,
            base * 0.8,
            MUTED,
            x + label_width - 10.0,
            bar_y + bar_height / 2.0 - base * 0.28,
            &truncate_to_width(
                data.categories
                    .get(index)
                    .map(String::as_str)
                    .unwrap_or("\u{2014}"),
                label_width - 14.0,
                Font::Regular,
                base * 0.8,
            ),
        );
        writer.rounded_rect(
            x + label_width,
            bar_y,
            bar_width.max(1.0),
            bar_height,
            3.0,
            Some(color),
            None,
        );
        writer.text(
            Font::Bold,
            base * 0.78,
            MUTED,
            x + label_width + bar_width.max(1.0) + 6.0,
            bar_y + bar_height / 2.0 - base * 0.28,
            &format_tick(*value),
        );
    }
}

fn draw_legend(
    writer: &mut PageWriter,
    data: &OfficeChartData,
    x: f64,
    y: f64,
    width: f64,
    base: f64,
) {
    let size = base * 0.78;
    let mut cursor = x;
    for (index, series) in data.series.iter().enumerate() {
        let label = truncate_to_width(&series.name, 120.0, Font::Regular, size);
        let entry_width = base * 1.3 + Font::Regular.width(&label, size) + 14.0;
        if cursor + entry_width > x + width {
            break;
        }
        writer.rounded_rect(
            cursor,
            y - size * 0.08,
            size * 0.8,
            size * 0.8,
            1.5,
            Some(series_color(index, data)),
            None,
        );
        writer.text(Font::Regular, size, MUTED, cursor + base * 1.2, y, &label);
        cursor += entry_width;
    }
}

fn parse_hex(hex: &str) -> Rgb {
    let hex = hex.trim_start_matches('#');
    let component = |range: std::ops::Range<usize>| {
        hex.get(range)
            .and_then(|slice| u8::from_str_radix(slice, 16).ok())
            .unwrap_or(128) as f64
            / 255.0
    };
    (component(0..2), component(2..4), component(4..6))
}

// ---------------------------------------------------------------------------
// Image decoding
// ---------------------------------------------------------------------------

/// Decode arbitrary image bytes into a PDF-embeddable XObject payload.
///
/// JPEG passes through untouched via `DCTDecode`; everything else is decoded to raw RGB8 and
/// deflated, with any alpha channel lifted into a soft mask.
pub fn decode_image(bytes: &[u8]) -> flow_like_types::Result<EmbeddedImage> {
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;

    let is_jpeg = bytes.starts_with(&[0xFF, 0xD8, 0xFF]);
    let decoded = image::load_from_memory(bytes)
        .map_err(|err| flow_like_types::anyhow!("Failed to decode image: {err}"))?;
    let width = image::GenericImageView::width(&decoded);
    let height = image::GenericImageView::height(&decoded);

    if is_jpeg {
        return Ok(EmbeddedImage {
            width,
            height,
            data: bytes.to_vec(),
            filter: "DCTDecode",
            soft_mask: None,
        });
    }

    let rgba = decoded.to_rgba8();
    let has_alpha = rgba.pixels().any(|pixel| pixel.0[3] != 255);

    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    let mut alpha = Vec::with_capacity((width * height) as usize);
    for pixel in rgba.pixels() {
        rgb.extend_from_slice(&pixel.0[..3]);
        alpha.push(pixel.0[3]);
    }

    let deflate = |data: &[u8]| -> flow_like_types::Result<Vec<u8>> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data)?;
        Ok(encoder.finish()?)
    };

    Ok(EmbeddedImage {
        width,
        height,
        data: deflate(&rgb)?,
        filter: "FlateDecode",
        soft_mask: has_alpha.then(|| deflate(&alpha)).transpose()?,
    })
}

// ---------------------------------------------------------------------------
// Document assembly
// ---------------------------------------------------------------------------

/// Assemble page content streams into a finished PDF, adding the running header and footer.
pub fn build_pdf(
    pages: Vec<String>,
    image_keys: &[String],
    images: &HashMap<String, EmbeddedImage>,
    layout: &PdfLayout,
    metadata: &PdfMetadata,
) -> flow_like_types::Result<Vec<u8>> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();

    let fonts = [
        ("F1", "Helvetica"),
        ("F2", "Helvetica-Bold"),
        ("F3", "Helvetica-Oblique"),
        ("F4", "Helvetica-BoldOblique"),
        ("F5", "Courier"),
    ];
    let mut font_dict = lopdf::Dictionary::new();
    for (key, base_font) in fonts {
        let id = doc.add_object(Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => base_font,
            "Encoding" => "WinAnsiEncoding",
        }));
        font_dict.set(key, Object::Reference(id));
    }

    let mut xobject_dict = lopdf::Dictionary::new();
    for (index, url) in image_keys.iter().enumerate() {
        let Some(image) = images.get(url) else {
            continue;
        };
        let mut image_dict = dictionary! {
            "Type" => Object::Name(b"XObject".to_vec()),
            "Subtype" => Object::Name(b"Image".to_vec()),
            "Width" => Object::Integer(image.width as i64),
            "Height" => Object::Integer(image.height as i64),
            "ColorSpace" => Object::Name(b"DeviceRGB".to_vec()),
            "BitsPerComponent" => Object::Integer(8),
            "Filter" => Object::Name(image.filter.as_bytes().to_vec()),
        };
        if let Some(mask) = &image.soft_mask {
            let mask_id = doc.add_object(Object::Stream(Stream::new(
                dictionary! {
                    "Type" => Object::Name(b"XObject".to_vec()),
                    "Subtype" => Object::Name(b"Image".to_vec()),
                    "Width" => Object::Integer(image.width as i64),
                    "Height" => Object::Integer(image.height as i64),
                    "ColorSpace" => Object::Name(b"DeviceGray".to_vec()),
                    "BitsPerComponent" => Object::Integer(8),
                    "Filter" => Object::Name(b"FlateDecode".to_vec()),
                },
                mask.clone(),
            )));
            image_dict.set("SMask", Object::Reference(mask_id));
        }
        let image_id = doc.add_object(Object::Stream(Stream::new(image_dict, image.data.clone())));
        xobject_dict.set(format!("Im{index}"), Object::Reference(image_id));
    }

    let mut resources = dictionary! { "Font" => Object::Dictionary(font_dict) };
    if !xobject_dict.is_empty() {
        resources.set("XObject", Object::Dictionary(xobject_dict));
    }
    let resources_id = doc.add_object(Object::Dictionary(resources));

    let total = pages.len();
    let mut page_ids = Vec::with_capacity(total);
    for (index, content) in pages.into_iter().enumerate() {
        let mut stream = String::with_capacity(content.len() + 512);
        stream.push_str(&page_furniture(layout, metadata, index, total));
        stream.push_str(&content);

        let content_id = doc.add_object(Stream::new(dictionary! {}, stream.into_bytes()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference(pages_id),
            "MediaBox" => vec![
                0.into(),
                0.into(),
                Object::Real(layout.page_width as f32),
                Object::Real(layout.page_height as f32),
            ],
            "Contents" => Object::Reference(content_id),
            "Resources" => Object::Reference(resources_id),
        });
        page_ids.push(Object::Reference(page_id));
    }

    let count = page_ids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids,
            "Count" => count,
        }),
    );

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => Object::Reference(pages_id),
    });
    doc.trailer.set("Root", Object::Reference(catalog_id));

    let mut info = lopdf::Dictionary::new();
    if let Some(title) = &metadata.title {
        info.set("Title", Object::string_literal(title.as_str()));
    }
    if let Some(author) = &metadata.author {
        info.set("Author", Object::string_literal(author.as_str()));
    }
    if let Some(subject) = &metadata.subject {
        info.set("Subject", Object::string_literal(subject.as_str()));
    }
    info.set("Producer", Object::string_literal("Flow Like"));
    let info_id = doc.add_object(Object::Dictionary(info));
    doc.trailer.set("Info", Object::Reference(info_id));

    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)?;
    Ok(bytes)
}

/// The running header and footer. Drawn before page content so text always sits on top.
fn page_furniture(
    layout: &PdfLayout,
    metadata: &PdfMetadata,
    index: usize,
    total: usize,
) -> String {
    let mut ops = String::new();
    let left = layout.margin;
    let right = layout.page_width - layout.margin;
    let cover_page = index == 0 && metadata.cover && metadata.title.is_some();

    // A hairline accent edge down the left of every page ties the set together.
    ops.push_str(&format!(
        "q {} {:.2} {:.2} 2.5 {:.2} re f Q\n",
        fill(ACCENT),
        0.0,
        layout.page_height - 96.0,
        96.0
    ));

    if !cover_page && let Some(title) = metadata.title.as_deref() {
        let header_y = layout.page_height - layout.top_margin + layout.base_font_size * 2.2;
        let label = truncate_to_width(
            title,
            layout.content_width() - 60.0,
            Font::Regular,
            layout.base_font_size * 0.78,
        );
        ops.push_str(&format!(
            "BT /F1 {:.2} Tf {} {:.2} {:.2} Td ({}) Tj ET\n",
            layout.base_font_size * 0.78,
            fill(MUTED),
            left,
            header_y,
            pdf_string(&label)
        ));
        ops.push_str(&format!(
            "q {} 0.6 w {:.2} {:.2} m {:.2} {:.2} l S Q\n",
            stroke(BORDER),
            left,
            header_y - 7.0,
            right,
            header_y - 7.0
        ));
    }

    if metadata.page_numbers {
        let footer_y = layout.bottom_margin / 2.0;
        let label = format!("{} / {}", index + 1, total);
        let size = layout.base_font_size * 0.78;
        ops.push_str(&format!(
            "q {} 0.6 w {:.2} {:.2} m {:.2} {:.2} l S Q\n",
            stroke(BORDER),
            left,
            footer_y + 14.0,
            right,
            footer_y + 14.0
        ));
        ops.push_str(&format!(
            "BT /F1 {size:.2} Tf {} {:.2} {:.2} Td ({}) Tj ET\n",
            fill(MUTED),
            right - Font::Regular.width(&label, size),
            footer_y,
            pdf_string(&label)
        ));
    }

    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ordered_and_nested_lists_with_real_markers() {
        let blocks = parse_markdown("1. first\n2. second\n   - nested\n");
        let markers: Vec<String> = blocks
            .iter()
            .filter_map(|block| match block {
                Block::ListItem { marker, .. } => Some(marker.clone()),
                _ => None,
            })
            .collect();
        // Nested bullets change glyph with depth, so the child is "◦" rather than "•".
        assert_eq!(markers, vec!["1.", "2.", "\u{25E6}"]);

        let depths: Vec<usize> = blocks
            .iter()
            .filter_map(|block| match block {
                Block::ListItem { depth, .. } => Some(*depth),
                _ => None,
            })
            .collect();
        assert_eq!(depths, vec![1, 1, 2]);
    }

    #[test]
    fn parses_tables_headings_and_code() {
        let blocks = parse_markdown(
            "# Title\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n```rust\nlet x = 1;\n```\n",
        );
        assert!(matches!(blocks[0], Block::Heading { level: 1, .. }));
        match &blocks[1] {
            Block::Table { header, rows } => {
                assert_eq!(header, &vec!["a".to_string(), "b".to_string()]);
                assert_eq!(rows.len(), 1);
            }
            other => panic!("expected table, got {other:?}"),
        }
        match &blocks[2] {
            Block::Code { language, lines } => {
                assert_eq!(language.as_deref(), Some("rust"));
                assert_eq!(lines, &vec!["let x = 1;".to_string()]);
            }
            other => panic!("expected code block, got {other:?}"),
        }
    }

    #[test]
    fn nivo_and_plotly_fences_become_charts() {
        for language in ["nivo", "plotly"] {
            let markdown = format!(
                "```{language}\ntype: bar\ntitle: Revenue\ncolors: [#FF4343, #4B5563]\n---\nQuarter,A,B\nQ1,120,80\nQ2,150,95\n```\n"
            );
            let blocks = parse_markdown(&markdown);
            match blocks.first() {
                Some(Block::Chart(data)) => {
                    assert_eq!(data.chart_type, ChartType::Bar);
                    assert_eq!(data.title.as_deref(), Some("Revenue"));
                    assert_eq!(data.categories, vec!["Q1".to_string(), "Q2".to_string()]);
                    assert_eq!(data.series.len(), 2);
                }
                other => panic!("expected a chart for {language}, got {other:?}"),
            }
        }
    }

    #[test]
    fn every_chart_type_renders_without_panicking() {
        let layout = PdfLayout::default();
        for kind in [
            "bar", "line", "area", "scatter", "pie", "radar", "funnel",
        ] {
            let markdown = format!(
                "```nivo\ntype: {kind}\ntitle: {kind} chart\n---\nStage,Alpha,Beta\nOne,10,4\nTwo,26,12\nThree,18,9\n```\n"
            );
            let blocks = parse_markdown(&markdown);
            assert!(
                matches!(blocks.first(), Some(Block::Chart(_))),
                "{kind} did not parse into a chart"
            );
            let (pages, _) = render_blocks(&blocks, &layout, &HashMap::new());
            assert!(!pages.is_empty());
            assert!(pages[0].contains(" re ") || pages[0].contains(" c "));
        }
    }

    #[test]
    fn stacked_and_horizontal_bars_are_honoured() {
        let markdown = "```nivo\ntype: bar\nstacked: true\nlayout: horizontal\n---\nStage,A,B\nOne,10,4\nTwo,26,12\n```\n";
        let blocks = parse_markdown(markdown);
        match blocks.first() {
            Some(Block::Chart(data)) => {
                assert!(data.stacked);
                assert_eq!(data.layout, ChartLayout::Horizontal);
            }
            other => panic!("expected chart, got {other:?}"),
        }
        let (pages, _) = render_blocks(&blocks, &PdfLayout::default(), &HashMap::new());
        assert!(!pages[0].is_empty());
    }

    #[test]
    fn long_documents_paginate_instead_of_truncating() {
        let markdown = (0..200)
            .map(|index| format!("Paragraph number {index} with enough text to occupy a line."))
            .collect::<Vec<_>>()
            .join("\n\n");
        let layout = PdfLayout::default();
        let blocks = parse_markdown(&markdown);
        let (pages, _) = render_blocks(&blocks, &layout, &HashMap::new());
        assert!(
            pages.len() > 3,
            "expected multiple pages, got {}",
            pages.len()
        );
        assert!(
            pages
                .last()
                .is_some_and(|page| page.contains("Paragraph number 199"))
        );
    }

    #[test]
    fn tables_repeat_their_header_on_every_page() {
        let mut markdown = String::from("| Region | Revenue |\n| --- | --- |\n");
        for index in 0..120 {
            markdown.push_str(&format!("| Region {index} | {} |\n", index * 37));
        }
        let layout = PdfLayout::default();
        let (pages, _) = render_blocks(&parse_markdown(&markdown), &layout, &HashMap::new());
        assert!(pages.len() > 1, "expected the table to span pages");
        for page in &pages {
            assert!(page.contains("(Region) Tj"), "header row missing on a page");
        }
    }

    #[test]
    fn text_wraps_within_the_content_width() {
        let layout = PdfLayout::default();
        let spans = vec![Span {
            text: "word ".repeat(80),
            ..Default::default()
        }];
        let lines = wrap_spans(&spans, layout.content_width(), layout.base_font_size);
        assert!(lines.len() > 1);
        for line in &lines {
            let width: f64 = line.iter().map(|(_, width)| width).sum();
            assert!(
                width <= layout.content_width() + 1.0,
                "line overflowed: {width}"
            );
        }
    }

    #[test]
    fn escapes_pdf_delimiters_and_transliterates_non_winansi() {
        assert_eq!(pdf_string("a(b)c\\d"), "a\\(b\\)c\\\\d");
        assert_eq!(pdf_string("smart \u{2019}quote\u{2019}"), "smart 'quote'");
        assert_eq!(pdf_string("emoji \u{1F600}"), "emoji ?");
        // Bullets must survive as the WinAnsi octal escape, not as "?".
        assert_eq!(pdf_string("\u{2022}"), "\\225");
    }

    #[test]
    fn nice_axis_produces_round_numbers() {
        assert_eq!(nice_axis(97.0, 4), (100.0, 25.0));
        assert_eq!(nice_axis(4.2, 4), (5.0, 1.0));
        assert_eq!(nice_axis(37.0, 4), (40.0, 10.0));
        assert_eq!(nice_axis(1000.0, 4), (1000.0, 250.0));
        assert_eq!(nice_axis(0.0, 4), (1.0, 0.25));

        // The axis must always cover the data without wildly overshooting it.
        for max in [1.0, 3.0, 8.5, 64.0, 512.0, 99_999.0] {
            let (axis_max, step) = nice_axis(max, 4);
            assert!(axis_max >= max, "axis {axis_max} below data {max}");
            assert!(axis_max <= max * 1.5, "axis {axis_max} overshoots {max}");
            assert!(step > 0.0);
        }
    }

    #[test]
    fn builds_a_loadable_pdf_with_furniture() {
        let layout = PdfLayout::default();
        let blocks = parse_markdown("# Hello\n\nSome **bold** text and a list:\n\n- one\n- two\n");
        let metadata = PdfMetadata {
            title: Some("Quarterly Report".into()),
            subject: Some("Prepared by Flow Like".into()),
            page_numbers: true,
            cover: true,
            ..Default::default()
        };
        let (pages, keys) = render_document(&blocks, &layout, &HashMap::new(), &metadata);
        let bytes = build_pdf(pages, &keys, &HashMap::new(), &layout, &metadata)
            .expect("build pdf");
        assert!(bytes.starts_with(b"%PDF-"));
        let reloaded = Document::load_mem(&bytes).expect("reload pdf");
        assert_eq!(reloaded.page_iter().count(), 1);
    }

    #[test]
    fn embeds_png_images_with_a_soft_mask() {
        let mut buffer = Vec::new();
        let mut png = image::RgbaImage::new(4, 4);
        for pixel in png.pixels_mut() {
            *pixel = image::Rgba([255, 0, 0, 128]);
        }
        image::DynamicImage::ImageRgba8(png)
            .write_to(
                &mut std::io::Cursor::new(&mut buffer),
                image::ImageFormat::Png,
            )
            .expect("encode png");

        let embedded = decode_image(&buffer).expect("decode");
        assert_eq!(embedded.width, 4);
        assert_eq!(embedded.filter, "FlateDecode");
        assert!(embedded.soft_mask.is_some());

        let layout = PdfLayout::default();
        let mut images = HashMap::new();
        images.insert("img://a".to_string(), embedded);
        let blocks = vec![Block::Image {
            url: "img://a".into(),
            alt: "Red".into(),
        }];
        let (pages, keys) = render_blocks(&blocks, &layout, &images);
        assert_eq!(keys, vec!["img://a".to_string()]);
        assert!(pages[0].contains("/Im0 Do"));

        let bytes =
            build_pdf(pages, &keys, &images, &layout, &PdfMetadata::default()).expect("build pdf");
        assert!(Document::load_mem(&bytes).is_ok());
    }

    /// Writes a showcase PDF for eyeballing the design. Not part of the normal run:
    /// `cargo test -p flow-like-catalog-media --features execute -- --ignored showcase`
    #[test]
    #[ignore = "writes a file for manual visual review"]
    fn showcase() {
        let markdown = r##"# Quarterly Business Review

Flow Like turns a **markdown** document into a typeset PDF — *selectable text*, real tables,
embedded charts and `inline code`, with no browser in the loop.

## Highlights

- Revenue grew **34%** quarter over quarter
- Three new enterprise accounts, one ~~churned~~ renewed
- Median document render time fell to `112ms`
  - p95 stayed under 400ms
  - p99 improved twofold
1. Ship the editor
2. Ship the converters
3. Ship the PDF

> The blockquote carries the accent bar, so a pulled-out remark reads as deliberate rather
> than as an indented accident.

### Task list

- [x] Storage-backed image uploads
- [x] Plate JSON to Markdown and HTML
- [ ] Typed element contracts

## Revenue by quarter

```nivo
type: bar
title: Revenue by quarter
---
Quarter,Enterprise,Mid-market,Self-serve
Q1,120,80,32
Q2,150,95,41
Q3,180,110,58
Q4,242,131,74
```

## Adoption trend

```plotly
type: area
title: Weekly active documents
---
Week,Documents
W1,120
W2,168
W3,201
W4,264
W5,318
W6,402
```

## Traffic mix

```nivo
type: pie
title: Where documents come from
---
Source,Share
Editor,52
API,26
Automation,14
Import,8
```

## Funnel

```nivo
type: funnel
title: Activation funnel
---
Stage,Users
Signed up,1000
Created doc,720
Shared doc,410
Upgraded,180
```

## Detail table

| Region | Accounts | Revenue | Growth |
|--------|----------|---------|--------|
| North America | 142 | 482000 | 34.2 |
| EMEA | 98 | 311500 | 21.8 |
| APAC | 61 | 198200 | 44.1 |
| LATAM | 27 | 64800 | 12.5 |

## Implementation

```rust
let blocks = parse_markdown(&source);
let (pages, keys) = render_document(&blocks, &layout, &images, &metadata);
let bytes = build_pdf(pages, &keys, &images, &layout, &metadata)?;
```

---

Generated by the `pdf_create_from_markdown` node.
"##;

        let layout = PdfLayout::default();
        let metadata = PdfMetadata {
            title: Some("Quarterly Business Review".into()),
            subject: Some("Flow Like · Document automation".into()),
            author: Some("Flow Like".into()),
            page_numbers: true,
            cover: true,
        };
        let blocks = parse_markdown(markdown);
        let (pages, keys) = render_document(&blocks, &layout, &HashMap::new(), &metadata);
        let bytes = build_pdf(pages, &keys, &HashMap::new(), &layout, &metadata).expect("build");

        let path = std::env::var("SHOWCASE_PDF")
            .unwrap_or_else(|_| "/tmp/flow-like-showcase.pdf".to_string());
        std::fs::write(&path, &bytes).expect("write");
        println!("showcase written to {path}");
    }

    #[test]
    fn missing_images_fall_back_to_a_placeholder() {
        let layout = PdfLayout::default();
        let blocks = vec![Block::Image {
            url: "https://example.com/missing.png".into(),
            alt: "Chart".into(),
        }];
        let (pages, keys) = render_blocks(&blocks, &layout, &HashMap::new());
        assert!(keys.is_empty());
        assert!(pages[0].contains("Image unavailable"));
    }
}

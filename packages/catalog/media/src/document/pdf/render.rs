//! Markdown → PDF layout engine.
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

use crate::document::chart::{OfficeChartData, chart_input_to_office_data, parse_chart_block};

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
    pub top_margin: f64,
    pub bottom_margin: f64,
    pub base_font_size: f64,
    pub accent: (f64, f64, f64),
}

impl Default for PdfLayout {
    fn default() -> Self {
        Self {
            page_width: 595.276, // A4
            page_height: 841.89,
            margin: 56.0,
            top_margin: 64.0,
            bottom_margin: 56.0,
            base_font_size: 11.0,
            accent: (1.0, 0.263, 0.263),
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
            c if (c as u32) < 128 => c as u8,
            c if (c as u32) <= 255 => c as u32 as u8,
            _ => b'?',
        };
        if byte < 32 || byte > 126 {
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

    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
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
    let mut in_table = false;
    let mut cell = String::new();
    let mut in_cell = false;

    let flush = |spans: &mut Vec<Span>| -> Vec<Span> {
        let taken = std::mem::take(spans);
        taken
            .into_iter()
            .filter(|span| !span.text.is_empty())
            .collect()
    };

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph | Tag::Item => {
                    if let Tag::Item = tag {
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
                }
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
                Tag::Table(_) => {
                    in_table = true;
                    table_header.clear();
                    table_rows.clear();
                }
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
                            marker: pending_marker.take().unwrap_or_else(|| "•".to_string()),
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
                    in_table = false;
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
        let _ = in_table;
    }

    let trailing = flush(&mut spans);
    if !trailing.is_empty() {
        blocks.push(Block::Paragraph(trailing));
    }

    blocks
}

fn bullet_for_depth(depth: usize) -> &'static str {
    match depth {
        1 => "•",
        2 => "◦",
        _ => "▪",
    }
}

// ---------------------------------------------------------------------------
// Layout
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

    fn text(&mut self, font: Font, size: f64, color: &str, x: f64, y: f64, text: &str) {
        if text.is_empty() {
            return;
        }
        self.push(&format!(
            "BT {} {size} Tf {color} {x:.2} {y:.2} Td ({}) Tj ET\n",
            font.resource(),
            pdf_string(text)
        ));
    }

    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, color: &str) {
        self.push(&format!("q {color} {x:.2} {y:.2} {w:.2} {h:.2} re f Q\n"));
    }

    fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, color: &str, width: f64) {
        self.push(&format!(
            "q {color} {width} w {x1:.2} {y1:.2} m {x2:.2} {y2:.2} l S Q\n"
        ));
    }
}

const TEXT_COLOR: &str = "0.102 0.102 0.102 rg";
const MUTED_COLOR: &str = "0.416 0.416 0.416 rg";
const CODE_COLOR: &str = "0.839 0.192 0.518 rg";
const CODE_BG: &str = "0.973 0.976 0.98 rg";

fn heading_size(level: u8, base: f64) -> f64 {
    match level {
        1 => base * 2.0,
        2 => base * 1.62,
        3 => base * 1.36,
        4 => base * 1.18,
        5 => base * 1.06,
        _ => base,
    }
}

/// One visual line: styled chunks plus the width already consumed.
type VisualLine = Vec<(Span, f64)>;

/// Break styled spans into visual lines that fit `max_width`.
fn wrap_spans(spans: &[Span], max_width: f64, size: f64) -> Vec<VisualLine> {
    let mut lines: Vec<VisualLine> = Vec::new();
    let mut line: VisualLine = Vec::new();
    let mut line_width = 0.0;

    for span in spans {
        let font = Font::for_span(span);
        let span_size = if span.code { size * 0.92 } else { size };
        // Keep the separating whitespace attached to the following word.
        for (index, word) in span.text.split_inclusive(' ').enumerate() {
            if word.is_empty() {
                continue;
            }
            let word_width = font.width(word, span_size);
            if line_width + word_width > max_width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                line_width = 0.0;
                // A wrapped line never starts with the space that separated it.
                let trimmed = word.trim_start();
                if trimmed.is_empty() {
                    continue;
                }
                let trimmed_width = font.width(trimmed, span_size);
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
            let _ = index;
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

/// Render blocks into per-page content streams.
///
/// `images` maps a markdown image URL to a decoded XObject; URLs absent from the map fall back
/// to a labelled placeholder box so a missing asset never aborts the document.
pub fn render_blocks(
    blocks: &[Block],
    layout: &PdfLayout,
    images: &HashMap<String, EmbeddedImage>,
) -> (Vec<String>, Vec<String>) {
    let mut writer = PageWriter::new(layout);
    let base = layout.base_font_size;
    let line_height = base * 1.45;
    let left = layout.margin;
    let content_width = layout.content_width();

    for block in blocks {
        match block {
            Block::Heading { level, spans } => {
                let size = heading_size(*level, base);
                writer.require(size * 2.2);
                writer.y -= size * 0.5;
                let text: String = spans.iter().map(|s| s.text.as_str()).collect();
                for line in wrap_plain(&text, content_width, Font::Bold, size) {
                    writer.require(size * 1.3);
                    let y = writer.y;
                    writer.text(Font::Bold, size, "0.067 0.067 0.067 rg", left, y, &line);
                    writer.y -= size * 1.3;
                }
                if *level <= 2 {
                    let y = writer.y + size * 0.45;
                    writer.rect(left, y, content_width, 0.6, "0.8 0.8 0.8 rg");
                    writer.y -= 6.0;
                }
            }

            Block::Paragraph(spans) => {
                let lines = wrap_spans(spans, content_width, base);
                for line in &lines {
                    writer.require(line_height);
                    draw_line(&mut writer, line, left, base);
                    writer.y -= line_height;
                }
                writer.y -= base * 0.45;
            }

            Block::ListItem {
                depth,
                marker,
                spans,
            } => {
                let indent = (*depth as f64 - 1.0) * 18.0;
                let marker_width = 18.0;
                let text_left = left + indent + marker_width;
                let lines = wrap_spans(spans, content_width - indent - marker_width, base);
                for (index, line) in lines.iter().enumerate() {
                    writer.require(line_height);
                    if index == 0 {
                        let y = writer.y;
                        writer.text(Font::Regular, base, MUTED_COLOR, left + indent, y, marker);
                    }
                    draw_line(&mut writer, line, text_left, base);
                    writer.y -= line_height;
                }
            }

            Block::Quote(spans) => {
                let indent = 18.0;
                let start_y = writer.y;
                let lines = wrap_spans(spans, content_width - indent, base);
                let mut bar_top = start_y;
                for line in &lines {
                    if writer.y - line_height < layout.bottom_margin {
                        // Close the bar on this page before breaking.
                        let bar_height = bar_top - writer.y;
                        if bar_height > 0.0 {
                            let y = writer.y + base * 0.6;
                            writer.rect(left + 2.0, y, 3.0, bar_height, "0.6 0.6 0.6 rg");
                        }
                        writer.break_page();
                        bar_top = writer.y;
                    }
                    draw_line(&mut writer, line, left + indent, base);
                    writer.y -= line_height;
                }
                let bar_height = bar_top - writer.y;
                if bar_height > 0.0 {
                    let y = writer.y + base * 0.6;
                    writer.rect(left + 2.0, y, 3.0, bar_height, "0.6 0.6 0.6 rg");
                }
                writer.y -= base * 0.45;
            }

            Block::Code { language, lines } => {
                let code_size = base * 0.9;
                let code_line_height = code_size * 1.35;
                let max_chars_width = content_width - 16.0;
                let mut remaining: Vec<String> = Vec::new();
                for line in lines {
                    let wrapped = wrap_code_line(line, max_chars_width, code_size);
                    remaining.extend(wrapped);
                }
                if let Some(language) = language {
                    writer.require(code_line_height * 2.0);
                    let y = writer.y;
                    writer.text(Font::Mono, code_size * 0.85, MUTED_COLOR, left, y, language);
                    writer.y -= code_line_height * 0.9;
                }
                let mut index = 0;
                while index < remaining.len() {
                    writer.require(code_line_height * 2.0);
                    let available =
                        ((writer.y - layout.bottom_margin) / code_line_height).floor() as usize;
                    let take = available.max(1).min(remaining.len() - index);
                    let block_height = take as f64 * code_line_height + 8.0;
                    let rect_y = writer.y - block_height + code_line_height - 4.0;
                    writer.rect(left, rect_y, content_width, block_height, CODE_BG);
                    for line in &remaining[index..index + take] {
                        let y = writer.y;
                        writer.text(Font::Mono, code_size, CODE_COLOR, left + 8.0, y, line);
                        writer.y -= code_line_height;
                    }
                    index += take;
                    writer.y -= 8.0;
                }
                writer.y -= base * 0.3;
            }

            Block::Rule => {
                writer.require(line_height);
                let y = writer.y + base * 0.4;
                writer.line(
                    left,
                    y,
                    left + content_width,
                    y,
                    "0.8 0.8 0.8 RG",
                    0.6,
                );
                writer.y -= line_height;
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
        let size = if span.code { base * 0.92 } else { base };
        let color = if span.code { CODE_COLOR } else { TEXT_COLOR };

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

        writer.text(font, size, color, x, y, &text);
        if span.strikethrough {
            let mid = y + size * 0.3;
            writer.line(x, mid, x + width, mid, "0.102 0.102 0.102 RG", 0.6);
        }
        x += width;
        index = end;
    }
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

    let Some(image) = images.get(url) else {
        writer.require(44.0);
        let y = writer.y;
        writer.rect(left, y - 26.0, content_width, 32.0, "0.941 0.941 0.941 rg");
        let label = if alt.is_empty() { url } else { alt };
        writer.text(
            Font::Regular,
            layout.base_font_size * 0.9,
            MUTED_COLOR,
            left + 10.0,
            y - 16.0,
            &format!("Image: {label}"),
        );
        writer.y -= 44.0;
        return;
    };

    let aspect = image.height as f64 / image.width.max(1) as f64;
    let draw_width = content_width.min(image.width as f64);
    let draw_height = draw_width * aspect;
    // An image taller than the text area is scaled down rather than clipped away.
    let available = layout.start_y() - layout.bottom_margin;
    let (draw_width, draw_height) = if draw_height > available {
        let scale = available / draw_height;
        (draw_width * scale, draw_height * scale)
    } else {
        (draw_width, draw_height)
    };

    writer.require(draw_height + 12.0);

    let key = match writer.used_images.iter().position(|used| used == url) {
        Some(index) => format!("/Im{index}"),
        None => {
            writer.used_images.push(url.to_string());
            format!("/Im{}", writer.used_images.len() - 1)
        }
    };

    let y = writer.y - draw_height;
    writer.push(&format!(
        "q {draw_width:.2} 0 0 {draw_height:.2} {left:.2} {y:.2} cm {key} Do Q\n"
    ));
    writer.y -= draw_height + 8.0;

    if !alt.is_empty() {
        writer.require(layout.base_font_size * 1.4);
        let caption_y = writer.y;
        writer.text(
            Font::Italic,
            layout.base_font_size * 0.85,
            MUTED_COLOR,
            left,
            caption_y,
            alt,
        );
        writer.y -= layout.base_font_size * 1.4;
    }
}

fn draw_table(writer: &mut PageWriter, header: &[String], rows: &[Vec<String>], layout: &PdfLayout) {
    let columns = header.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return;
    }
    let left = layout.margin;
    let content_width = layout.content_width();
    let column_width = content_width / columns as f64;
    let cell_size = layout.base_font_size * 0.9;
    let row_height = cell_size * 1.9;
    let padding = 5.0;
    let accent = format!(
        "{} {} {} rg",
        layout.accent.0, layout.accent.1, layout.accent.2
    );

    let mut draw_header = |writer: &mut PageWriter| {
        if header.is_empty() {
            return;
        }
        writer.require(row_height * 2.0);
        let y = writer.y;
        writer.rect(left, y - row_height + cell_size, content_width, row_height, &accent);
        for (index, cell) in header.iter().enumerate() {
            let x = left + index as f64 * column_width + padding;
            let text = truncate_to_width(cell, column_width - padding * 2.0, Font::Bold, cell_size);
            writer.text(Font::Bold, cell_size, "1 1 1 rg", x, y, &text);
        }
        writer.y -= row_height;
    };

    draw_header(writer);

    for row in rows {
        if writer.y - row_height < layout.bottom_margin {
            writer.break_page();
            draw_header(writer);
        }
        let y = writer.y;
        for index in 0..columns {
            let cell = row.get(index).map(String::as_str).unwrap_or("");
            let x = left + index as f64 * column_width + padding;
            let text =
                truncate_to_width(cell, column_width - padding * 2.0, Font::Regular, cell_size);
            writer.text(Font::Regular, cell_size, TEXT_COLOR, x, y, &text);
        }
        let border_y = y - row_height + cell_size;
        writer.line(
            left,
            border_y,
            left + content_width,
            border_y,
            "0.85 0.85 0.85 RG",
            0.5,
        );
        writer.y -= row_height;
    }
    writer.y -= layout.base_font_size * 0.5;
}

fn truncate_to_width(text: &str, max_width: f64, font: Font, size: f64) -> String {
    if font.width(text, size) <= max_width {
        return text.to_string();
    }
    let ellipsis_width = font.width("…", size);
    let mut out = String::new();
    for ch in text.chars() {
        let mut candidate = out.clone();
        candidate.push(ch);
        if font.width(&candidate, size) + ellipsis_width > max_width {
            break;
        }
        out = candidate;
    }
    out.push_str("...");
    out
}

fn draw_chart(writer: &mut PageWriter, data: &OfficeChartData, layout: &PdfLayout) {
    use crate::document::chart::ChartType;

    let left = layout.margin;
    let chart_width = layout.content_width();
    let chart_height = 150.0;

    writer.require(chart_height + 56.0);

    if let Some(title) = &data.title {
        let y = writer.y;
        writer.text(
            Font::Bold,
            layout.base_font_size * 1.05,
            "0.067 0.067 0.067 rg",
            left,
            y,
            title,
        );
        writer.y -= 20.0;
    }

    let top = writer.y;
    let bottom = top - chart_height;
    writer.rect(left, bottom, chart_width, chart_height, CODE_BG);

    let colors: Vec<(f64, f64, f64)> = data.colors.iter().map(|hex| parse_hex(hex)).collect();
    let max_value = data
        .series
        .iter()
        .flat_map(|series| &series.values)
        .copied()
        .fold(0.0_f64, f64::max)
        .max(1.0);

    match data.chart_type {
        ChartType::Bar => {
            let category_count = data.categories.len().max(1);
            let series_count = data.series.len().max(1);
            let group_width = chart_width / category_count as f64;
            let bar_width = (group_width * 0.7) / series_count as f64;
            let group_padding = group_width * 0.15;

            for category in 0..category_count {
                for (series_index, series) in data.series.iter().enumerate() {
                    let value = series.values.get(category).copied().unwrap_or(0.0);
                    let bar_height = (value / max_value) * (chart_height - 24.0);
                    let x = left
                        + category as f64 * group_width
                        + group_padding
                        + series_index as f64 * bar_width;
                    let (r, g, b) = colors
                        .get(series_index)
                        .copied()
                        .unwrap_or((0.8, 0.2, 0.2));
                    writer.rect(
                        x,
                        bottom,
                        bar_width * 0.9,
                        bar_height,
                        &format!("{r} {g} {b} rg"),
                    );
                }
            }
            for (index, category) in data.categories.iter().enumerate() {
                let x = left + index as f64 * group_width + group_padding;
                writer.text(
                    Font::Regular,
                    layout.base_font_size * 0.72,
                    MUTED_COLOR,
                    x,
                    bottom - 14.0,
                    category,
                );
            }
        }
        _ => {
            let point_count = data
                .series
                .iter()
                .map(|series| series.values.len())
                .max()
                .unwrap_or(0);
            if point_count > 1 {
                let step = chart_width / (point_count - 1) as f64;
                for (series_index, series) in data.series.iter().enumerate() {
                    let (r, g, b) = colors
                        .get(series_index)
                        .copied()
                        .unwrap_or((0.8, 0.2, 0.2));
                    let mut path = String::new();
                    for (index, value) in series.values.iter().enumerate() {
                        let x = left + index as f64 * step;
                        let y = bottom + (value / max_value) * (chart_height - 24.0);
                        if index == 0 {
                            path.push_str(&format!("{x:.2} {y:.2} m "));
                        } else {
                            path.push_str(&format!("{x:.2} {y:.2} l "));
                        }
                    }
                    writer.push(&format!("q {r} {g} {b} RG 1.5 w {path}S Q\n"));
                }
            }
        }
    }

    writer.y = bottom - 26.0;
}

fn parse_hex(hex: &str) -> (f64, f64, f64) {
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

#[derive(Debug, Clone, Default)]
pub struct PdfMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub page_numbers: bool,
}

/// Assemble page content streams into a finished PDF.
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
    for (index, mut content) in pages.into_iter().enumerate() {
        if metadata.page_numbers {
            let label = format!("{} / {}", index + 1, total);
            let width = Font::Regular.width(&label, 9.0);
            content.push_str(&format!(
                "BT /F1 9 Tf {MUTED_COLOR} {:.2} {:.2} Td ({}) Tj ET\n",
                (layout.page_width - width) / 2.0,
                layout.bottom_margin / 2.0,
                pdf_string(&label)
            ));
        }
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
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
        assert_eq!(markers, vec!["1.", "2.", "◦"]);

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
        let blocks =
            parse_markdown("# Title\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\n```rust\nlet x = 1;\n```\n");
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
        assert!(pages.last().is_some_and(|page| page.contains("Paragraph number 199")));
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
    }

    #[test]
    fn builds_a_loadable_pdf() {
        let layout = PdfLayout::default();
        let blocks = parse_markdown("# Hello\n\nSome **bold** text and a list:\n\n- one\n- two\n");
        let (pages, keys) = render_blocks(&blocks, &layout, &HashMap::new());
        let bytes = build_pdf(
            pages,
            &keys,
            &HashMap::new(),
            &layout,
            &PdfMetadata {
                title: Some("Test".into()),
                page_numbers: true,
                ..Default::default()
            },
        )
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
            .write_to(&mut std::io::Cursor::new(&mut buffer), image::ImageFormat::Png)
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

        let bytes = build_pdf(pages, &keys, &images, &layout, &PdfMetadata::default())
            .expect("build pdf");
        assert!(Document::load_mem(&bytes).is_ok());
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
        assert!(pages[0].contains("Image: Chart"));
    }
}

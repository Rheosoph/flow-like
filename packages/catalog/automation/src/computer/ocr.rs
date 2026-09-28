#![cfg_attr(not(feature = "execute"), allow(dead_code))]

use crate::types::handles::AutomationSession;
use crate::types::screen_frame::ScreenFrame;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A rectangle in image pixels, origin top-left.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PixelBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PixelBox {
    pub fn union(&self, other: &PixelBox) -> PixelBox {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        PixelBox {
            x,
            y,
            width: right - x,
            height: bottom - y,
        }
    }

    pub fn intersects(&self, other: &PixelBox) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }
}

/// A rectangle in desktop input coordinates, the space the mouse nodes use.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl InputRect {
    pub fn area(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    pub fn center(&self) -> InputPoint {
        InputPoint {
            x: (self.x as i64 + self.width as i64 / 2) as i32,
            y: (self.y as i64 + self.height as i64 / 2) as i32,
        }
    }

    pub fn intersection(&self, other: &InputRect) -> Option<InputRect> {
        let x0 = (self.x as i64).max(other.x as i64);
        let y0 = (self.y as i64).max(other.y as i64);
        let x1 = (self.x as i64 + self.width as i64).min(other.x as i64 + other.width as i64);
        let y1 = (self.y as i64 + self.height as i64).min(other.y as i64 + other.height as i64);
        (x1 > x0 && y1 > y0).then(|| InputRect {
            x: x0 as i32,
            y: y0 as i32,
            width: (x1 - x0) as u32,
            height: (y1 - y0) as u32,
        })
    }

    pub fn iou(&self, other: &InputRect) -> f64 {
        let Some(overlap) = self.intersection(other) else {
            return 0.0;
        };
        let union = self.area() + other.area() - overlap.area();
        if union == 0 {
            0.0
        } else {
            overlap.area() as f64 / union as f64
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputPoint {
    pub x: i32,
    pub y: i32,
}

/// One recognized word. `bbox` is filled when the image's screen frame is known.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct OcrWord {
    pub text: String,
    /// 0–1; absent when the engine does not report confidence (Windows).
    #[serde(default)]
    pub confidence: Option<f64>,
    pub bbox_px: PixelBox,
    #[serde(default)]
    pub bbox: Option<InputRect>,
}

/// One recognized line. `bbox`/`center` are desktop input coordinates, filled when the
/// image's screen frame is known; `bbox_px` is always in pixels of the recognized image.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct OcrLine {
    pub text: String,
    #[serde(default)]
    pub confidence: Option<f64>,
    pub bbox_px: PixelBox,
    #[serde(default)]
    pub bbox: Option<InputRect>,
    #[serde(default)]
    pub center: Option<InputPoint>,
    #[serde(default)]
    pub words: Vec<OcrWord>,
}

#[derive(Clone, Debug)]
pub struct OcrOptions {
    /// BCP-47 tags (`en-US`, `de`) or Tesseract codes (`eng`); empty lets the engine decide.
    pub languages: Vec<String>,
    pub language_correction: bool,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            languages: Vec::new(),
            language_correction: true,
        }
    }
}

/// A text match on screen. `bbox`/`center` are desktop input coordinates.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct TextMatch {
    /// The matched words as recognized.
    pub text: String,
    pub line_text: String,
    /// 1 for exact, contains and regex matches; the similarity for fuzzy matches.
    pub score: f64,
    #[serde(default)]
    pub confidence: Option<f64>,
    pub bbox_px: PixelBox,
    #[serde(default)]
    pub bbox: Option<InputRect>,
    #[serde(default)]
    pub center: Option<InputPoint>,
}

/// Language table: ISO 639-1 code, Tesseract code.
const LANGUAGES: [(&str, &str); 32] = [
    ("en", "eng"),
    ("de", "deu"),
    ("fr", "fra"),
    ("es", "spa"),
    ("it", "ita"),
    ("pt", "por"),
    ("nl", "nld"),
    ("sv", "swe"),
    ("da", "dan"),
    ("no", "nor"),
    ("nb", "nor"),
    ("fi", "fin"),
    ("pl", "pol"),
    ("cs", "ces"),
    ("sk", "slk"),
    ("hu", "hun"),
    ("ro", "ron"),
    ("tr", "tur"),
    ("el", "ell"),
    ("ru", "rus"),
    ("uk", "ukr"),
    ("bg", "bul"),
    ("ar", "ara"),
    ("he", "heb"),
    ("hi", "hin"),
    ("th", "tha"),
    ("vi", "vie"),
    ("id", "ind"),
    ("ms", "msa"),
    ("ja", "jpn"),
    ("ko", "kor"),
    ("zh", "chi_sim"),
];

/// The primary ISO 639-1 subtag of a BCP-47 tag or Tesseract code (`de-DE` → `de`, `deu` → `de`).
pub(crate) fn primary_language(language: &str) -> String {
    let lower = language.trim().to_lowercase().replace('_', "-");
    if lower.starts_with("chi-") {
        return "zh".into();
    }
    let primary = lower.split('-').next().unwrap_or_default().to_string();
    LANGUAGES
        .iter()
        .find(|(_, tesseract)| *tesseract == primary)
        .map(|(iso, _)| iso.to_string())
        .unwrap_or(primary)
}

/// Tesseract `-l` code for a BCP-47 tag or Tesseract code.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn tesseract_language(language: &str) -> String {
    let lower = language.trim().to_lowercase().replace('-', "_");
    let mut parts = lower.split('_');
    let primary = parts.next().unwrap_or_default();
    if primary == "zh" {
        let traditional = parts.any(|part| ["hant", "tw", "hk", "mo"].contains(&part));
        return if traditional { "chi_tra" } else { "chi_sim" }.into();
    }
    if primary.len() == 3 && primary.chars().all(|c| c.is_ascii_alphabetic()) {
        return lower;
    }
    LANGUAGES
        .iter()
        .find(|(iso, _)| *iso == primary)
        .map(|(_, tesseract)| tesseract.to_string())
        .unwrap_or(lower)
}

/// Picks the engine language matching a requested tag: exact tag first, then the same
/// primary language (`de` → `de-DE`, `eng` → `en-US`).
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) fn match_language(requested: &str, available: &[String]) -> Option<String> {
    let wanted = requested.trim().to_lowercase().replace('_', "-");
    if let Some(exact) = available.iter().find(|tag| tag.to_lowercase() == wanted) {
        return Some(exact.clone());
    }
    let primary = primary_language(requested);
    if primary == "zh" {
        let traditional = wanted.contains("hant") || wanted.contains("tw") || wanted.contains("hk");
        let script = if traditional { "hant" } else { "hans" };
        if let Some(tag) = available
            .iter()
            .find(|tag| tag.to_lowercase().starts_with("zh") && tag.to_lowercase().contains(script))
        {
            return Some(tag.clone());
        }
    }
    available
        .iter()
        .find(|tag| primary_language(tag) == primary)
        .cloned()
}

pub(crate) fn parse_languages(text: &str) -> Vec<String> {
    text.split([',', ';', '+', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Converts a Vision-style box (normalized, origin bottom-left) into pixels, origin top-left.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn normalized_bottom_left_to_pixels(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    image_width: u32,
    image_height: u32,
) -> Option<PixelBox> {
    if ![x, y, width, height].iter().all(|v| v.is_finite()) || image_width == 0 || image_height == 0
    {
        return None;
    }
    let (w, h) = (image_width as f64, image_height as f64);
    let left = (x * w).floor().clamp(0.0, w);
    let right = ((x + width) * w).ceil().clamp(0.0, w);
    let top = ((1.0 - y - height) * h).floor().clamp(0.0, h);
    let bottom = ((1.0 - y) * h).ceil().clamp(0.0, h);
    (right > left && bottom > top).then(|| PixelBox {
        x: left as u32,
        y: top as u32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    })
}

/// Whitespace-separated words of `text` as (word, UTF-16 offset, UTF-16 length, char offset,
/// char length). UTF-16 offsets address NSString ranges.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn word_spans(text: &str) -> Vec<(&str, usize, usize, usize, usize)> {
    let mut spans = Vec::new();
    let (mut utf16, mut chars) = (0usize, 0usize);
    let mut start: Option<(usize, usize, usize)> = None;
    for (byte, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some((b, u, ch)) = start.take() {
                spans.push((&text[b..byte], u, utf16 - u, ch, chars - ch));
            }
        } else if start.is_none() {
            start = Some((byte, utf16, chars));
        }
        utf16 += c.len_utf16();
        chars += 1;
    }
    if let Some((b, u, ch)) = start {
        spans.push((&text[b..], u, utf16 - u, ch, chars - ch));
    }
    spans
}

/// Slice of a line box proportional to a character range, for engines without word boxes.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn proportional_word_box(
    line: &PixelBox,
    char_start: usize,
    char_len: usize,
    total_chars: usize,
) -> PixelBox {
    if total_chars == 0 {
        return *line;
    }
    let unit = line.width as f64 / total_chars as f64;
    let x0 = line.x as f64 + char_start as f64 * unit;
    let x1 = line.x as f64 + (char_start + char_len) as f64 * unit;
    PixelBox {
        x: x0.floor() as u32,
        y: line.y,
        width: ((x1.ceil() - x0.floor()) as u32).max(1),
        height: line.height,
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn finite_confidence(value: f64) -> Option<f64> {
    value.is_finite().then(|| value.clamp(0.0, 1.0))
}

/// Maps a pixel box through a frame into an input rectangle and its center.
pub(crate) fn pixel_box_to_input(
    frame: &ScreenFrame,
    bbox: &PixelBox,
) -> flow_like_types::Result<(InputRect, InputPoint)> {
    let (x, y) = frame.pixel_to_input(bbox.x as f64, bbox.y as f64)?;
    let max_x = frame.pixel_width as f64 - 0.5;
    let max_y = frame.pixel_height as f64 - 0.5;
    let (cx, cy) = frame.pixel_to_input(
        (bbox.x as f64 + bbox.width as f64 / 2.0).min(max_x),
        (bbox.y as f64 + bbox.height as f64 / 2.0).min(max_y),
    )?;
    Ok((
        InputRect {
            x,
            y,
            width: ((bbox.width as f64 / frame.scale_x()).round() as u32).max(1),
            height: ((bbox.height as f64 / frame.scale_y()).round() as u32).max(1),
        },
        InputPoint { x: cx, y: cy },
    ))
}

pub(crate) fn locate_lines(lines: &mut [OcrLine], frame: &ScreenFrame) {
    for line in lines {
        if let Ok((bbox, center)) = pixel_box_to_input(frame, &line.bbox_px) {
            line.bbox = Some(bbox);
            line.center = Some(center);
        }
        for word in &mut line.words {
            word.bbox = pixel_box_to_input(frame, &word.bbox_px)
                .ok()
                .map(|(bbox, _)| bbox);
        }
    }
}

/// Sort key for top-to-bottom, left-to-right reading order with a row tolerance of `band`.
pub(crate) fn reading_order_key(top: i64, height: i64, left: i64, band: i64) -> (i64, i64) {
    ((top + height / 2).div_euclid(band.max(1)), left)
}

pub(crate) fn sort_lines(lines: &mut [OcrLine]) {
    lines.sort_by_key(|line| {
        let b = line.bbox_px;
        reading_order_key(b.y as i64, b.height as i64, b.x as i64, 12)
    });
}

fn normalize_display(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{2018}' | '\u{2019}' | '\u{201B}' | '`' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201E}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            c if c.is_whitespace() => ' ',
            c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_key(text: &str, case_sensitive: bool) -> String {
    let display = normalize_display(text);
    let trimmed = display.trim_end_matches(['…', ':', '.', ',', ';']);
    let trimmed = if trimmed.is_empty() {
        &display
    } else {
        trimmed
    };
    if case_sensitive {
        trimmed.to_string()
    } else {
        trimmed.to_lowercase()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchMode {
    Exact,
    Contains,
    Regex,
    Fuzzy,
}

impl MatchMode {
    pub fn parse(value: &str) -> flow_like_types::Result<Self> {
        Ok(match value.trim().to_lowercase().as_str() {
            "exact" => Self::Exact,
            "contains" => Self::Contains,
            "regex" => Self::Regex,
            "fuzzy" => Self::Fuzzy,
            other => {
                return Err(flow_like_types::anyhow!(
                    "Unknown text match mode '{}'; use exact, contains, regex or fuzzy",
                    other
                ));
            }
        })
    }
}

pub struct TextQuery {
    mode: MatchMode,
    key: String,
    words: usize,
    case_sensitive: bool,
    threshold: f64,
    regex: Option<flow_like_types::regex::Regex>,
}

impl TextQuery {
    pub fn new(
        text: &str,
        mode: MatchMode,
        case_sensitive: bool,
        threshold: f64,
    ) -> flow_like_types::Result<Self> {
        if text.trim().is_empty() {
            return Err(flow_like_types::anyhow!("Text to find is empty"));
        }
        if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
            return Err(flow_like_types::anyhow!(
                "Fuzzy threshold must be between 0 and 1, got {}",
                threshold
            ));
        }
        let regex = if mode == MatchMode::Regex {
            Some(
                flow_like_types::regex::RegexBuilder::new(text)
                    .case_insensitive(!case_sensitive)
                    .size_limit(1 << 20)
                    .build()
                    .map_err(|e| {
                        flow_like_types::anyhow!("Invalid text pattern '{}': {}", text, e)
                    })?,
            )
        } else {
            None
        };
        let key = normalize_key(text, case_sensitive);
        Ok(Self {
            mode,
            words: key.split(' ').count(),
            key,
            case_sensitive,
            threshold,
            regex,
        })
    }
}

struct Token {
    display: String,
    key: String,
    bbox: PixelBox,
}

fn tokens(line: &OcrLine, case_sensitive: bool) -> Vec<Token> {
    let make = |text: &str, bbox: PixelBox| Token {
        display: normalize_display(text),
        key: normalize_key(text, case_sensitive),
        bbox,
    };
    if line.words.is_empty() {
        return vec![make(&line.text, line.bbox_px)];
    }
    line.words
        .iter()
        .filter(|w| !w.text.trim().is_empty())
        .map(|w| make(&w.text, w.bbox_px))
        .collect()
}

fn joined(parts: &[&str]) -> (String, Vec<(usize, usize)>) {
    let mut text = String::new();
    let mut offsets = Vec::with_capacity(parts.len());
    for part in parts {
        if !text.is_empty() {
            text.push(' ');
        }
        offsets.push((text.len(), text.len() + part.len()));
        text.push_str(part);
    }
    (text, offsets)
}

fn span_for_range(offsets: &[(usize, usize)], start: usize, end: usize) -> Option<(usize, usize)> {
    let first = offsets.iter().position(|(_, e)| *e > start)?;
    let last = offsets.iter().rposition(|(s, _)| *s < end)?;
    (last >= first).then_some((first, last + 1))
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != cb);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

pub(crate) fn similarity(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 1.0;
    }
    1.0 - levenshtein(&a, &b) as f64 / longest as f64
}

/// Every match of `query` in reading order. Matches cover whole words; the box is the union
/// of the matched words' boxes, so clicking a match hits the words, not the whole line.
pub fn find_matches(lines: &[OcrLine], query: &TextQuery) -> Vec<TextMatch> {
    const MAX_SPAN: usize = 64;
    let mut matches = Vec::new();
    for line in lines {
        let tokens = tokens(line, query.case_sensitive);
        if tokens.is_empty() {
            continue;
        }
        let push = |matches: &mut Vec<TextMatch>, start: usize, end: usize, score: f64| {
            let span = &tokens[start..end];
            matches.push(TextMatch {
                text: span
                    .iter()
                    .map(|t| t.display.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                line_text: line.text.clone(),
                score,
                confidence: line.confidence,
                bbox_px: span[1..]
                    .iter()
                    .fold(span[0].bbox, |acc, t| acc.union(&t.bbox)),
                bbox: None,
                center: None,
            });
        };
        let join_keys = |start: usize, end: usize| {
            tokens[start..end]
                .iter()
                .map(|t| t.key.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        };
        match query.mode {
            MatchMode::Exact => {
                let mut start = 0;
                while start < tokens.len() {
                    let mut found = None;
                    for end in start + 1..=tokens.len().min(start + MAX_SPAN) {
                        let key = join_keys(start, end);
                        if key == query.key {
                            found = Some(end);
                            break;
                        }
                        if key.len() > query.key.len() {
                            break;
                        }
                    }
                    match found {
                        Some(end) => {
                            push(&mut matches, start, end, 1.0);
                            start = end;
                        }
                        None => start += 1,
                    }
                }
            }
            MatchMode::Contains => {
                let keys: Vec<&str> = tokens.iter().map(|t| t.key.as_str()).collect();
                let (text, offsets) = joined(&keys);
                for (at, _) in text.match_indices(query.key.as_str()) {
                    if let Some((start, end)) = span_for_range(&offsets, at, at + query.key.len()) {
                        push(&mut matches, start, end, 1.0);
                    }
                }
            }
            MatchMode::Regex => {
                let Some(regex) = &query.regex else {
                    continue;
                };
                let displays: Vec<&str> = tokens.iter().map(|t| t.display.as_str()).collect();
                let (text, offsets) = joined(&displays);
                for found in regex.find_iter(&text).filter(|m| !m.is_empty()) {
                    if let Some((start, end)) = span_for_range(&offsets, found.start(), found.end())
                    {
                        push(&mut matches, start, end, 1.0);
                    }
                }
            }
            MatchMode::Fuzzy => {
                let mut best: Option<(usize, usize, f64)> = None;
                let shortest = query.words.saturating_sub(1).max(1);
                for start in 0..tokens.len() {
                    for len in shortest..=query.words + 1 {
                        let end = start + len;
                        if end > tokens.len() {
                            break;
                        }
                        let score = similarity(&join_keys(start, end), &query.key);
                        if best.is_none_or(|(_, _, b)| score > b) {
                            best = Some((start, end, score));
                        }
                    }
                }
                if let Some((start, end, score)) = best.filter(|b| b.2 >= query.threshold) {
                    push(&mut matches, start, end, score);
                }
            }
        }
    }
    matches.sort_by_key(|m| {
        let b = m.bbox_px;
        reading_order_key(b.y as i64, b.height as i64, b.x as i64, 12)
    });
    matches
}

#[cfg(feature = "execute")]
pub(crate) async fn ocr_image(
    image: &image::DynamicImage,
    languages: &[String],
) -> flow_like_types::Result<Vec<OcrLine>> {
    ocr_image_with(
        image,
        OcrOptions {
            languages: languages.to_vec(),
            ..Default::default()
        },
    )
    .await
}

#[cfg(feature = "execute")]
pub(crate) async fn ocr_image_with(
    image: &image::DynamicImage,
    options: OcrOptions,
) -> flow_like_types::Result<Vec<OcrLine>> {
    ocr_rgba(image.to_rgba8(), options).await
}

/// Recognizes text in reading order; boxes are in pixels of `rgba`.
#[cfg(feature = "execute")]
pub(crate) async fn ocr_rgba(
    rgba: image::RgbaImage,
    options: OcrOptions,
) -> flow_like_types::Result<Vec<OcrLine>> {
    if rgba.width() < 4 || rgba.height() < 4 {
        return Err(flow_like_types::anyhow!(
            "Image of {}x{} pixels is too small for text recognition",
            rgba.width(),
            rgba.height()
        ));
    }
    let mut lines = super::native::recognize_text(rgba, options).await?;
    lines.retain(|line| !line.text.trim().is_empty());
    sort_lines(&mut lines);
    Ok(lines)
}

/// Where on the desktop to look: a window, an input-space region, or a whole display.
#[cfg(feature = "execute")]
pub(crate) struct CaptureScope {
    window_title: String,
    display_index: i64,
    region: Option<InputRect>,
}

pub(crate) fn add_scope_pins(node: &mut Node) {
    node.add_input_pin(
        "window_title",
        "Window Title",
        "Look only inside the window with this title; empty uses the region or display",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "display_index",
        "Display Index",
        "Display to capture when no window or region is given",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(0)));
    for (name, friendly, description) in [
        (
            "region_x",
            "Region X",
            "Left edge of the region in desktop input coordinates",
        ),
        (
            "region_y",
            "Region Y",
            "Top edge of the region in desktop input coordinates",
        ),
        (
            "region_width",
            "Region Width",
            "Region width; 0 captures the whole display",
        ),
        (
            "region_height",
            "Region Height",
            "Region height; 0 captures the whole display",
        ),
    ] {
        node.add_input_pin(name, friendly, description, VariableType::Integer)
            .set_default_value(Some(json!(0)));
    }
}

#[cfg(feature = "execute")]
pub(crate) async fn read_scope(
    context: &mut ExecutionContext,
) -> flow_like_types::Result<CaptureScope> {
    let width: i64 = context.evaluate_pin("region_width").await?;
    let height: i64 = context.evaluate_pin("region_height").await?;
    let region = if width > 0 && height > 0 {
        Some(InputRect {
            x: i32::try_from(context.evaluate_pin::<i64>("region_x").await?)?,
            y: i32::try_from(context.evaluate_pin::<i64>("region_y").await?)?,
            width: u32::try_from(width)?,
            height: u32::try_from(height)?,
        })
    } else {
        None
    };
    Ok(CaptureScope {
        window_title: context.evaluate_pin("window_title").await?,
        display_index: context.evaluate_pin("display_index").await?,
        region,
    })
}

#[cfg(feature = "execute")]
pub(crate) async fn capture_scope(
    scope: &CaptureScope,
) -> flow_like_types::Result<(image::RgbaImage, ScreenFrame)> {
    if !scope.window_title.trim().is_empty() {
        let window = super::accessibility::resolve_window(scope.window_title.trim())
            .await?
            .ok_or_else(|| flow_like_types::anyhow!("Window '{}' not found", scope.window_title))?;
        return super::zoom::capture_input_rect_async(super::zoom::window_rect(&window)).await;
    }
    if let Some(region) = scope.region {
        return super::zoom::capture_input_rect_async(region).await;
    }
    let index = u32::try_from(scope.display_index).map_err(|_| {
        flow_like_types::anyhow!("Display index {} is invalid", scope.display_index)
    })?;
    super::zoom::capture_display_async(index).await
}

fn add_language_pin(node: &mut Node) {
    node.add_input_pin(
        "languages",
        "Languages",
        "Comma-separated languages such as en-US, de; empty lets the OS engine choose",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
}

fn add_match_pins(node: &mut Node) {
    node.add_input_pin(
        "text",
        "Text",
        "Text or pattern to find",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
    node.add_input_pin(
        "match_mode",
        "Match Mode",
        "exact: whole words equal the text; contains: substring; regex: pattern; fuzzy: similar words",
        VariableType::String,
    )
    .set_default_value(Some(json!("exact")))
    .set_options(
        PinOptions::new()
            .set_valid_values(
                ["exact", "contains", "regex", "fuzzy"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            )
            .build(),
    );
    node.add_input_pin(
        "case_sensitive",
        "Case Sensitive",
        "Compare letter case; whitespace and typographic quotes are always normalized",
        VariableType::Boolean,
    )
    .set_default_value(Some(json!(false)));
    node.add_input_pin(
        "fuzzy_threshold",
        "Fuzzy Threshold",
        "Minimum similarity (0–1) for fuzzy matches",
        VariableType::Float,
    )
    .set_default_value(Some(json!(0.8)))
    .set_options(
        PinOptions::new()
            .set_range((0.0, 1.0))
            .set_step(0.05)
            .build(),
    );
    node.add_input_pin(
        "occurrence",
        "Occurrence",
        "Which match to use in reading order (1 = first)",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(1)))
    .set_options(
        PinOptions::new()
            .set_range((1.0, 1000.0))
            .set_step(1.0)
            .build(),
    );
    add_scope_pins(node);
    add_language_pin(node);
}

fn add_session_pins(node: &mut Node) {
    node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);
    node.add_input_pin(
        "session",
        "Session",
        "Computer session handle",
        VariableType::Struct,
    )
    .set_schema::<AutomationSession>();
}

fn add_session_out(node: &mut Node) {
    node.add_output_pin(
        "session_out",
        "Session",
        "Computer session handle (pass-through)",
        VariableType::Struct,
    )
    .set_schema::<AutomationSession>();
}

fn add_frame_out(node: &mut Node, description: &str) {
    node.add_output_pin("frame", "Frame", description, VariableType::Struct)
        .set_schema::<ScreenFrame>();
}

fn vision_scores() -> flow_like::flow::node::NodeScores {
    flow_like::flow::node::NodeScores::new()
        .set_privacy(4)
        .set_security(5)
        .set_performance(6)
        .set_governance(6)
        .set_reliability(7)
        .set_cost(10)
        .build()
}

/// Captures the scope, recognizes text and returns the matches in desktop coordinates.
#[cfg(feature = "execute")]
async fn locate_text(
    context: &mut ExecutionContext,
) -> flow_like_types::Result<(Vec<TextMatch>, ScreenFrame)> {
    let text: String = context.evaluate_pin("text").await?;
    let mode = MatchMode::parse(&context.evaluate_pin::<String>("match_mode").await?)?;
    let case_sensitive: bool = context.evaluate_pin("case_sensitive").await?;
    let threshold: f64 = context.evaluate_pin("fuzzy_threshold").await?;
    let query = TextQuery::new(&text, mode, case_sensitive, threshold)?;
    let languages = parse_languages(&context.evaluate_pin::<String>("languages").await?);
    let scope = read_scope(context).await?;
    let (image, frame) = capture_scope(&scope).await?;
    let lines = ocr_image(&image::DynamicImage::ImageRgba8(image), &languages).await?;
    let mut matches = find_matches(&lines, &query);
    for found in &mut matches {
        if let Ok((bbox, center)) = pixel_box_to_input(&frame, &found.bbox_px) {
            found.bbox = Some(bbox);
            found.center = Some(center);
        }
    }
    matches.retain(|m| m.center.is_some());
    Ok((matches, frame))
}

#[cfg(feature = "execute")]
async fn pick_occurrence(
    context: &mut ExecutionContext,
    matches: &[TextMatch],
) -> flow_like_types::Result<Option<TextMatch>> {
    let occurrence: i64 = context.evaluate_pin("occurrence").await?;
    if occurrence < 1 {
        return Err(flow_like_types::anyhow!(
            "Occurrence must be 1 or greater, got {}",
            occurrence
        ));
    }
    Ok(matches.get(occurrence as usize - 1).cloned())
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerOcrNode;

impl ComputerOcrNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ComputerOcrNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_ocr",
            "Read Text (OCR)",
            "Recognizes text in an image or on the screen with the operating system's OCR engine (Apple Vision, Windows OCR, Tesseract on Linux). Lines carry pixel boxes and, when the screen frame is known, desktop coordinates for the mouse nodes",
            "Automation/Computer/Vision",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "ocr");
        node.add_icon("/flow/icons/vision.svg");
        node.set_scores(vision_scores());
        node.set_only_offline(true);

        add_session_pins(&mut node);
        node.add_input_pin(
            "image",
            "Image",
            "Image to read; leave unconnected to capture the window, region or display below",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>()
        .set_default_value(Some(json!(null)))
        .set_options(PinOptions::new().set_optional(true).build());
        node.add_input_pin(
            "image_frame",
            "Frame",
            "Screen frame of the connected image; maps recognized boxes to desktop coordinates",
            VariableType::Struct,
        )
        .set_schema::<ScreenFrame>()
        .set_default_value(Some(json!(null)))
        .set_options(PinOptions::new().set_optional(true).build());
        add_scope_pins(&mut node);
        add_language_pin(&mut node);
        node.add_input_pin(
            "language_correction",
            "Language Correction",
            "Let the engine correct words with its language model (Apple Vision); turn off for codes and identifiers",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "min_confidence",
            "Min Confidence",
            "Drop lines below this confidence (0–1); engines without confidence keep all lines",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)))
        .set_options(
            PinOptions::new()
                .set_range((0.0, 1.0))
                .set_step(0.05)
                .build(),
        );

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        add_session_out(&mut node);
        node.add_output_pin(
            "text",
            "Text",
            "All recognized lines in reading order, separated by newlines",
            VariableType::String,
        );
        node.add_output_pin(
            "lines",
            "Lines",
            "Recognized lines with confidence, pixel boxes, desktop boxes and word boxes",
            VariableType::Struct,
        )
        .set_schema::<OcrLine>()
        .set_value_type(ValueType::Array);
        add_frame_out(
            &mut node,
            "Screen frame of the recognized image; empty when an image without frame was read",
        );
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let image: Option<NodeImage> = context.evaluate_pin("image").await?;
        let frame: Option<ScreenFrame> = context.evaluate_pin("image_frame").await?;
        let options = OcrOptions {
            languages: parse_languages(&context.evaluate_pin::<String>("languages").await?),
            language_correction: context.evaluate_pin("language_correction").await?,
        };
        let min_confidence: f64 = context.evaluate_pin("min_confidence").await?;

        let (picture, frame) = match image {
            Some(image) => {
                let picture = image.get_image(context).await?.lock().await.clone();
                if let Some(frame) = &frame {
                    if (frame.pixel_width, frame.pixel_height)
                        != (picture.width(), picture.height())
                    {
                        return Err(flow_like_types::anyhow!(
                            "The frame describes a {}x{} image but the image is {}x{}; connect the frame produced with this image",
                            frame.pixel_width,
                            frame.pixel_height,
                            picture.width(),
                            picture.height()
                        ));
                    }
                }
                (picture, frame)
            }
            None => {
                let scope = read_scope(context).await?;
                let (capture, frame) = capture_scope(&scope).await?;
                (image::DynamicImage::ImageRgba8(capture), Some(frame))
            }
        };

        let mut lines = ocr_image_with(&picture, options).await?;
        lines.retain(|line| line.confidence.is_none_or(|c| c >= min_confidence));
        if let Some(frame) = &frame {
            locate_lines(&mut lines, frame);
        }
        let text = lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("text", json!(text)).await?;
        context.set_pin_value("lines", json!(lines)).await?;
        context.set_pin_value("frame", json!(frame)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerFindTextNode;

impl ComputerFindTextNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ComputerFindTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_find_text",
            "Find Text on Screen",
            "Finds text on a display, region or window with OCR and returns its position in desktop coordinates",
            "Automation/Computer/Vision",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "findText");
        node.add_icon("/flow/icons/vision.svg");
        node.set_scores(vision_scores());
        node.set_only_offline(true);

        add_session_pins(&mut node);
        add_match_pins(&mut node);

        node.add_output_pin(
            "exec_out",
            "▶",
            "The text was found",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_not_found",
            "Not Found",
            "The text is not on screen",
            VariableType::Execution,
        );
        add_session_out(&mut node);
        node.add_output_pin(
            "found",
            "Found",
            "Whether the requested occurrence exists",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "x",
            "X",
            "Center X of the match in desktop input coordinates",
            VariableType::Integer,
        );
        node.add_output_pin(
            "y",
            "Y",
            "Center Y of the match in desktop input coordinates",
            VariableType::Integer,
        );
        node.add_output_pin(
            "bbox",
            "Bounds",
            "Bounds of the match in desktop input coordinates",
            VariableType::Struct,
        )
        .set_schema::<InputRect>();
        node.add_output_pin("match", "Match", "The selected match", VariableType::Struct)
            .set_schema::<TextMatch>();
        node.add_output_pin(
            "matches",
            "All Matches",
            "Every match in reading order",
            VariableType::Struct,
        )
        .set_schema::<TextMatch>()
        .set_value_type(ValueType::Array);
        add_frame_out(&mut node, "Screen frame of the searched capture");
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_not_found").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;

        let (matches, frame) = locate_text(context).await?;
        let selected = pick_occurrence(context, &matches).await?;
        let center = selected.as_ref().and_then(|m| m.center);

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("found", json!(center.is_some()))
            .await?;
        context
            .set_pin_value("x", json!(center.map_or(0, |c| c.x)))
            .await?;
        context
            .set_pin_value("y", json!(center.map_or(0, |c| c.y)))
            .await?;
        context
            .set_pin_value("bbox", json!(selected.as_ref().and_then(|m| m.bbox)))
            .await?;
        context.set_pin_value("match", json!(selected)).await?;
        context.set_pin_value("matches", json!(matches)).await?;
        context.set_pin_value("frame", json!(frame)).await?;
        if center.is_some() {
            context.activate_exec_pin("exec_out").await?;
        } else {
            context.activate_exec_pin("exec_not_found").await?;
        }
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerClickTextNode;

impl ComputerClickTextNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ComputerClickTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_click_text",
            "Click Text",
            "Finds text on screen with OCR and clicks the center of the matched words",
            "Automation/Computer/Vision",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "clickText");
        node.add_icon("/flow/icons/vision.svg");
        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(3)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        add_session_pins(&mut node);
        add_match_pins(&mut node);
        node.add_input_pin("button", "Button", "Mouse button", VariableType::String)
            .set_default_value(Some(json!("left")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(
                        ["left", "right", "middle"]
                            .iter()
                            .map(|s| s.to_string())
                            .collect(),
                    )
                    .build(),
            );
        node.add_input_pin(
            "double",
            "Double Click",
            "Click twice",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "▶",
            "The text was clicked",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_not_found",
            "Not Found",
            "The text is not on screen; nothing was clicked",
            VariableType::Execution,
        );
        add_session_out(&mut node);
        node.add_output_pin(
            "found",
            "Found",
            "Whether the text was found and clicked",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "x",
            "X",
            "Clicked X in desktop input coordinates",
            VariableType::Integer,
        );
        node.add_output_pin(
            "y",
            "Y",
            "Clicked Y in desktop input coordinates",
            VariableType::Integer,
        );
        node.add_output_pin("match", "Match", "The clicked match", VariableType::Struct)
            .set_schema::<TextMatch>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_not_found").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let button: String = context.evaluate_pin("button").await?;
        let double: bool = context.evaluate_pin("double").await?;

        let (matches, _) = locate_text(context).await?;
        let selected = pick_occurrence(context, &matches).await?;
        let center = selected.as_ref().and_then(|m| m.center);
        if let Some(point) = center {
            super::capture_state::click_point(context, &session, point, &button, double).await?;
            session.apply_delay(context).await?;
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("found", json!(center.is_some()))
            .await?;
        context
            .set_pin_value("x", json!(center.map_or(0, |c| c.x)))
            .await?;
        context
            .set_pin_value("y", json!(center.map_or(0, |c| c.y)))
            .await?;
        context.set_pin_value("match", json!(selected)).await?;
        if center.is_some() {
            context.activate_exec_pin("exec_out").await?;
        } else {
            context.activate_exec_pin("exec_not_found").await?;
        }
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, x: u32) -> OcrWord {
        OcrWord {
            text: text.into(),
            confidence: Some(0.9),
            bbox_px: PixelBox {
                x,
                y: 10,
                width: 10 * text.chars().count() as u32,
                height: 20,
            },
            bbox: None,
        }
    }

    fn line(words: &[(&str, u32)], y: u32) -> OcrLine {
        let mut words: Vec<OcrWord> = words.iter().map(|(t, x)| word(t, *x)).collect();
        for w in &mut words {
            w.bbox_px.y = y;
        }
        let bbox_px = words[1..]
            .iter()
            .fold(words[0].bbox_px, |acc, w| acc.union(&w.bbox_px));
        OcrLine {
            text: words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            confidence: Some(0.9),
            bbox_px,
            bbox: None,
            center: None,
            words,
        }
    }

    fn query(text: &str, mode: MatchMode) -> TextQuery {
        TextQuery::new(text, mode, false, 0.8).unwrap()
    }

    #[test]
    fn vision_boxes_flip_to_top_left_pixels() {
        let bbox = normalized_bottom_left_to_pixels(0.25, 0.5, 0.5, 0.25, 800, 400).unwrap();
        assert_eq!(
            bbox,
            PixelBox {
                x: 200,
                y: 100,
                width: 400,
                height: 100
            }
        );
        let edge = normalized_bottom_left_to_pixels(0.9, -0.1, 0.2, 0.2, 100, 100).unwrap();
        assert_eq!((edge.x, edge.y, edge.width, edge.height), (90, 90, 10, 10));
        assert!(normalized_bottom_left_to_pixels(f64::NAN, 0.0, 1.0, 1.0, 10, 10).is_none());
        assert!(normalized_bottom_left_to_pixels(0.0, 0.0, 0.0, 1.0, 10, 10).is_none());
    }

    #[test]
    fn word_spans_use_utf16_offsets() {
        let spans = word_spans("Grüße  🙂 da");
        assert_eq!(
            spans,
            vec![
                ("Grüße", 0, 5, 0, 5),
                ("🙂", 7, 2, 7, 1),
                ("da", 10, 2, 9, 2)
            ]
        );
        let line = PixelBox {
            x: 100,
            y: 5,
            width: 110,
            height: 20,
        };
        assert_eq!(proportional_word_box(&line, 9, 2, 11).x, 190);
    }

    #[test]
    fn exact_matches_whole_words_and_boxes_only_the_span() {
        let lines = [
            line(&[("Save", 0), ("As…", 60)], 0),
            line(&[("Unsaved", 0)], 40),
        ];
        let found = find_matches(&lines, &query("save", MatchMode::Exact));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "Save");
        assert_eq!(found[0].bbox_px.width, 40);
        let as_found = find_matches(&lines, &query("save as", MatchMode::Exact));
        assert_eq!(as_found[0].bbox_px.width, 90);
        assert_eq!(
            find_matches(&lines, &query("save", MatchMode::Contains)).len(),
            2
        );
    }

    #[test]
    fn regex_and_fuzzy_modes() {
        let lines = [line(&[("Total:", 0), ("42,00", 70), ("EUR", 130)], 0)];
        let regex = find_matches(&lines, &query(r"\d+,\d\d", MatchMode::Regex));
        assert_eq!(regex[0].text, "42,00");
        assert_eq!(regex[0].bbox_px.x, 70);
        let fuzzy = find_matches(&lines, &query("Totall", MatchMode::Fuzzy));
        assert_eq!(fuzzy[0].text, "Total:");
        assert!(fuzzy[0].score >= 0.8 && fuzzy[0].score < 1.0);
        assert!(find_matches(&lines, &query("invoice", MatchMode::Fuzzy)).is_empty());
        assert!(TextQuery::new("(", MatchMode::Regex, false, 0.8).is_err());
    }

    #[test]
    fn matches_follow_reading_order() {
        let lines = [
            line(&[("OK", 300)], 200),
            line(&[("OK", 10)], 203),
            line(&[("OK", 50)], 20),
        ];
        let found = find_matches(&lines, &query("ok", MatchMode::Exact));
        let xs: Vec<_> = found.iter().map(|m| m.bbox_px.x).collect();
        assert_eq!(xs, [50, 10, 300]);
    }

    #[test]
    fn pixel_boxes_map_through_retina_frame() {
        let frame = ScreenFrame::new(Some(1), (-1440, -200, 1440, 900), (2880, 1800)).unwrap();
        let (bbox, center) = pixel_box_to_input(
            &frame,
            &PixelBox {
                x: 1000,
                y: 600,
                width: 200,
                height: 40,
            },
        )
        .unwrap();
        assert_eq!(
            bbox,
            InputRect {
                x: -940,
                y: 100,
                width: 100,
                height: 20
            }
        );
        assert_eq!(center, InputPoint { x: -890, y: 110 });
    }

    #[test]
    fn languages_map_between_engines() {
        assert_eq!(tesseract_language("de-DE"), "deu");
        assert_eq!(tesseract_language("en"), "eng");
        assert_eq!(tesseract_language("eng"), "eng");
        assert_eq!(tesseract_language("zh-Hant"), "chi_tra");
        assert_eq!(tesseract_language("zh-CN"), "chi_sim");
        assert_eq!(tesseract_language("chi_tra"), "chi_tra");
        let available: Vec<String> = ["en-US", "de-DE", "zh-Hans", "zh-Hant"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(match_language("deu", &available).as_deref(), Some("de-DE"));
        assert_eq!(match_language("EN", &available).as_deref(), Some("en-US"));
        assert_eq!(
            match_language("zh-TW", &available).as_deref(),
            Some("zh-Hant")
        );
        assert_eq!(match_language("fr", &available), None);
        assert_eq!(parse_languages("en-US, de ;fr"), ["en-US", "de", "fr"]);
    }
}

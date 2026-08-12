//! Heuristic table detection on a typed sheet grid.
//!
//! Pipeline (per detected region): region detection via connected components
//! with gap tolerance + projection-profile splitting, row-role classification
//! (title / footnote / aggregate / repeated header), ensemble header detection
//! with multi-row flattening, and typed row emission.
//!
//! Parameters follow published measurements (TableSense, DeExcelerator,
//! SpreadsheetLLM, DuckDB's sniffer): gap tolerance 1, min density 0.1,
//! type-contrast header voting with a 0.3 acceptance threshold.

use flow_like_types::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(feature = "execute")]
use super::grid::{CellValue, SheetGrid, truncate_chars};
#[cfg(feature = "execute")]
use once_cell::sync::Lazy;
#[cfg(feature = "execute")]
use regex::Regex;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ExtractConfig {
    /// Blank rows tolerated inside one table (more consecutive blanks split tables)
    pub row_gap_tolerance: usize,
    /// Blank columns tolerated inside one table
    pub col_gap_tolerance: usize,
    /// Minimum data rows for a region to count as a table
    pub min_table_rows: usize,
    /// Minimum non-empty cells for a region to count as a table
    pub min_table_cells: usize,
    /// Minimum non-empty density (0.0–1.0) of a region's bounding box
    pub min_density: f32,
    /// Maximum header rows to detect and flatten
    pub max_header_rows: usize,
    /// Joiner when flattening multi-row headers
    pub header_joiner: String,
    /// Drop subtotal/total/aggregate rows from the data
    pub drop_aggregate_rows: bool,
    /// Use Excel defined tables (ListObjects) as authoritative when present
    pub use_defined_tables: bool,
    /// Try several gap-tolerance segmentations and keep the best-scoring one
    pub adaptive_segmentation: bool,
}

impl Default for ExtractConfig {
    fn default() -> Self {
        Self {
            row_gap_tolerance: 1,
            col_gap_tolerance: 1,
            min_table_rows: 2,
            min_table_cells: 4,
            min_density: 0.1,
            max_header_rows: 3,
            header_joiner: " / ".to_string(),
            drop_aggregate_rows: true,
            use_defined_tables: true,
            adaptive_segmentation: true,
        }
    }
}

/// Grid-relative rectangle, inclusive bounds.
#[cfg(feature = "execute")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub r0: usize,
    pub c0: usize,
    pub r1: usize,
    pub c1: usize,
}

#[cfg(feature = "execute")]
impl Rect {
    #[inline]
    pub fn height(&self) -> usize {
        self.r1 - self.r0 + 1
    }
    #[inline]
    pub fn width(&self) -> usize {
        self.c1 - self.c0 + 1
    }
    fn overlaps(&self, o: &Rect) -> bool {
        self.r0 <= o.r1 && o.r0 <= self.r1 && self.c0 <= o.c1 && o.c0 <= self.c1
    }
}

/// An extracted table with typed rows, ready for `CSVTable`.
#[cfg(feature = "execute")]
#[derive(Clone, Debug)]
pub struct DetectedTable {
    /// Title text peeled from rows above the header, if any
    pub title: Option<String>,
    /// A1 range of the table region (absolute sheet coordinates)
    pub range_a1: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<flow_like_types::Value>>,
    /// Footnote lines peeled from below the data
    pub notes: Vec<String>,
    /// False when headers were generated (`column_1`, …)
    pub header_detected: bool,
    /// Aggregate/repeated-header rows dropped from the data
    pub dropped_rows: usize,
    /// 0.0–1.0 heuristic quality estimate
    pub confidence: f32,
}

// ============================ Row statistics ============================

#[cfg(feature = "execute")]
#[derive(Clone, Copy, Debug, Default)]
struct RowStats {
    filled: usize,
    strings: usize,
    scalars: usize, // numbers, dates, bools
    wide_merge: bool,
}

#[cfg(feature = "execute")]
fn row_stats(grid: &SheetGrid, rect: &Rect, r: usize) -> RowStats {
    let mut s = RowStats::default();
    for c in rect.c0..=rect.c1 {
        match grid.cell(r, c) {
            CellValue::Empty => {}
            CellValue::Text(t) => {
                if t.trim().is_empty() {
                    continue;
                }
                s.filled += 1;
                s.strings += 1;
            }
            CellValue::Error(_) => s.filled += 1,
            _ => {
                s.filled += 1;
                s.scalars += 1;
            }
        }
    }
    if let Some(m) = (rect.c0..=rect.c1).find_map(|c| grid.merge_at(r, c)) {
        let span = m.c1.min(rect.c1).saturating_sub(m.c0.max(rect.c0)) + 1;
        s.wide_merge = span * 2 > rect.width();
    }
    s
}

#[cfg(feature = "execute")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    Str,
    Num,
    Date,
    Bool,
}

#[cfg(feature = "execute")]
pub(crate) fn cell_kind(v: &CellValue) -> Option<Kind> {
    match v {
        CellValue::Empty | CellValue::Error(_) => None,
        CellValue::Text(t) => {
            if t.trim().is_empty() {
                None
            } else {
                Some(Kind::Str)
            }
        }
        CellValue::Int(_) | CellValue::Float(_) | CellValue::Duration(_) => Some(Kind::Num),
        CellValue::DateTime(_) => Some(Kind::Date),
        CellValue::Bool(_) => Some(Kind::Bool),
    }
}

/// Dominant kind of a column's body cells plus its dominance ratio (0–1).
#[cfg(feature = "execute")]
pub(crate) fn column_dominant(
    grid: &SheetGrid,
    c: usize,
    rows: impl Iterator<Item = usize>,
) -> Option<(Kind, f32)> {
    let mut counts = [0usize; 4];
    let mut total = 0usize;
    for r in rows {
        if let Some(k) = cell_kind(grid.cell(r, c)) {
            counts[k as usize] += 1;
            total += 1;
        }
    }
    if total == 0 {
        return None;
    }
    let (idx, max) = counts
        .iter()
        .enumerate()
        .max_by_key(|entry| *entry.1)
        .map(|(i, v)| (i, *v))?;
    let kind = match idx {
        0 => Kind::Str,
        1 => Kind::Num,
        2 => Kind::Date,
        _ => Kind::Bool,
    };
    Some((kind, max as f32 / total as f32))
}

// ============================ Region detection ============================

#[cfg(feature = "execute")]
fn build_mask(grid: &SheetGrid) -> Vec<bool> {
    let mut mask = vec![false; grid.height * grid.width];
    for r in 0..grid.height {
        for c in 0..grid.width {
            if grid.is_filled(r, c) {
                mask[r * grid.width + c] = true;
            }
        }
    }
    // Paint merged regions so centered titles/headers form one component
    for m in &grid.merges {
        if !grid.cell(m.r0, m.c0).is_empty() {
            for r in m.r0..=m.r1.min(grid.height.saturating_sub(1)) {
                for c in m.c0..=m.c1.min(grid.width.saturating_sub(1)) {
                    mask[r * grid.width + c] = true;
                }
            }
        }
    }
    mask
}

/// 8-connected components on the filled mask, returned as bounding boxes.
#[cfg(feature = "execute")]
fn component_boxes(mask: &[bool], height: usize, width: usize) -> Vec<Rect> {
    let mut visited = vec![false; mask.len()];
    let mut boxes = Vec::new();
    let mut stack = Vec::new();

    for start in 0..mask.len() {
        if !mask[start] || visited[start] {
            continue;
        }
        visited[start] = true;
        stack.push(start);
        let (mut r0, mut c0) = (start / width, start % width);
        let (mut r1, mut c1) = (r0, c0);

        while let Some(idx) = stack.pop() {
            let (r, c) = (idx / width, idx % width);
            r0 = r0.min(r);
            c0 = c0.min(c);
            r1 = r1.max(r);
            c1 = c1.max(c);
            let rlo = r.saturating_sub(1);
            let rhi = (r + 1).min(height - 1);
            let clo = c.saturating_sub(1);
            let chi = (c + 1).min(width - 1);
            for nr in rlo..=rhi {
                for nc in clo..=chi {
                    let nidx = nr * width + nc;
                    if mask[nidx] && !visited[nidx] {
                        visited[nidx] = true;
                        stack.push(nidx);
                    }
                }
            }
        }
        boxes.push(Rect { r0, c0, r1, c1 });
    }
    boxes
}

#[cfg(feature = "execute")]
fn projection_overlap(a0: usize, a1: usize, b0: usize, b1: usize) -> f32 {
    let inter = (a1.min(b1) + 1).saturating_sub(a0.max(b0));
    let shorter = (a1 - a0 + 1).min(b1 - b0 + 1);
    if shorter == 0 {
        0.0
    } else {
        inter as f32 / shorter as f32
    }
}

/// True when the first row of `rect` looks like a header for the rows below it.
/// Used as a veto so two stacked tables separated by a small gap stay apart.
#[cfg(feature = "execute")]
fn first_row_headerish(grid: &SheetGrid, rect: &Rect) -> bool {
    if rect.height() < 3 {
        return false;
    }
    header_vote(grid, rect, rect.r0, rect.r0 + 1) > 0.3
}

/// Small, sparse boxes (titles, captions) that decoration peeling will handle
/// once merged into the adjacent table's region.
#[cfg(feature = "execute")]
fn is_decoration_box(grid: &SheetGrid, rect: &Rect, union_width: usize) -> bool {
    rect.height() <= 2
        && (rect.r0..=rect.r1).all(|r| {
            let filled = (rect.c0..=rect.c1)
                .filter(|&c| grid.is_filled(r, c))
                .count();
            filled <= (union_width / 4).max(2)
        })
}

#[cfg(feature = "execute")]
fn should_merge_boxes(grid: &SheetGrid, a: &Rect, b: &Rect, cfg: &ExtractConfig) -> bool {
    if a.overlaps(b) {
        return true;
    }
    // Vertical stacking: gap of blank rows between the boxes
    if a.c0 <= b.c1 && b.c0 <= a.c1 {
        let (top, bot) = if a.r1 < b.r0 { (a, b) } else { (b, a) };
        if bot.r0 > top.r1 {
            let gap = bot.r0 - top.r1 - 1;
            let union_width = a.c1.max(b.c1) - a.c0.min(b.c0) + 1;
            if gap <= cfg.row_gap_tolerance
                && projection_overlap(a.c0, a.c1, b.c0, b.c1) >= 0.5
                && (is_decoration_box(grid, top, union_width) || !first_row_headerish(grid, bot))
            {
                return true;
            }
        }
    }
    // Side-by-side: gap of blank columns between the boxes
    if a.r0 <= b.r1 && b.r0 <= a.r1 {
        let (left, right) = if a.c1 < b.c0 { (a, b) } else { (b, a) };
        if right.c0 > left.c1 {
            let gap = right.c0 - left.c1 - 1;
            if gap <= cfg.col_gap_tolerance && projection_overlap(a.r0, a.r1, b.r0, b.r1) >= 0.5 {
                return true;
            }
        }
    }
    false
}

#[cfg(feature = "execute")]
fn merge_boxes(grid: &SheetGrid, mut boxes: Vec<Rect>, cfg: &ExtractConfig) -> Vec<Rect> {
    let mut merged = true;
    let mut rounds = 0usize;
    while merged && rounds < 64 {
        merged = false;
        rounds += 1;
        'outer: for i in 0..boxes.len() {
            for j in (i + 1)..boxes.len() {
                if should_merge_boxes(grid, &boxes[i], &boxes[j], cfg) {
                    let b = boxes.swap_remove(j);
                    let a = &mut boxes[i];
                    a.r0 = a.r0.min(b.r0);
                    a.c0 = a.c0.min(b.c0);
                    a.r1 = a.r1.max(b.r1);
                    a.c1 = a.c1.max(b.c1);
                    merged = true;
                    break 'outer;
                }
            }
        }
    }
    boxes
}

/// Recursive projection-profile splitting: cut a box along blank row/column
/// runs longer than the gap tolerance (handles regions glued together by a
/// bridging footnote or stray cell).
#[cfg(feature = "execute")]
fn split_by_profiles(grid: &SheetGrid, rect: Rect, cfg: &ExtractConfig, out: &mut Vec<Rect>) {
    let col_profile: Vec<usize> = (rect.c0..=rect.c1)
        .map(|c| {
            (rect.r0..=rect.r1)
                .filter(|&r| grid.is_filled(r, c))
                .count()
        })
        .collect();
    if let Some((cut_start, cut_len)) = longest_zero_run(&col_profile)
        && cut_len > cfg.col_gap_tolerance
        && cut_start > 0
        && cut_start + cut_len < rect.width()
    {
        let left = Rect {
            c1: rect.c0 + cut_start - 1,
            ..rect
        };
        let right = Rect {
            c0: rect.c0 + cut_start + cut_len,
            ..rect
        };
        split_by_profiles(grid, left, cfg, out);
        split_by_profiles(grid, right, cfg, out);
        return;
    }

    let row_profile: Vec<usize> = (rect.r0..=rect.r1)
        .map(|r| {
            (rect.c0..=rect.c1)
                .filter(|&c| grid.is_filled(r, c))
                .count()
        })
        .collect();
    if let Some((cut_start, cut_len)) = longest_zero_run(&row_profile)
        && cut_len > cfg.row_gap_tolerance
        && cut_start > 0
        && cut_start + cut_len < rect.height()
    {
        let top = Rect {
            r1: rect.r0 + cut_start - 1,
            ..rect
        };
        let bottom = Rect {
            r0: rect.r0 + cut_start + cut_len,
            ..rect
        };
        split_by_profiles(grid, top, cfg, out);
        split_by_profiles(grid, bottom, cfg, out);
        return;
    }

    out.push(rect);
}

#[cfg(feature = "execute")]
fn longest_zero_run(profile: &[usize]) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    let mut run_start = None;
    for (i, &v) in profile.iter().enumerate() {
        if v == 0 {
            run_start.get_or_insert(i);
        } else if let Some(s) = run_start.take() {
            let len = i - s;
            if best.is_none_or(|(_, bl)| len > bl) {
                best = Some((s, len));
            }
        }
    }
    if let Some(s) = run_start {
        let len = profile.len() - s;
        if best.is_none_or(|(_, bl)| len > bl) {
            best = Some((s, len));
        }
    }
    best
}

/// Shrinks a rect to its non-empty bounding box. Returns None if empty.
#[cfg(feature = "execute")]
pub(crate) fn tighten(grid: &SheetGrid, rect: &Rect) -> Option<Rect> {
    let mut r0 = None;
    let mut r1 = 0;
    let mut c0 = usize::MAX;
    let mut c1 = 0;
    for r in rect.r0..=rect.r1.min(grid.height.saturating_sub(1)) {
        for c in rect.c0..=rect.c1.min(grid.width.saturating_sub(1)) {
            if grid.is_filled(r, c) {
                r0.get_or_insert(r);
                r1 = r1.max(r);
                c0 = c0.min(c);
                c1 = c1.max(c);
            }
        }
    }
    r0.map(|r0| Rect { r0, c0, r1, c1 })
}

/// Detects candidate table regions in a sheet, sorted top-left first.
#[cfg(feature = "execute")]
pub fn detect_table_regions(grid: &SheetGrid, cfg: &ExtractConfig) -> Vec<Rect> {
    if grid.height == 0 || grid.width == 0 {
        return Vec::new();
    }
    let mask = build_mask(grid);
    let boxes = component_boxes(&mask, grid.height, grid.width);
    let boxes = merge_boxes(grid, boxes, cfg);

    let mut split = Vec::with_capacity(boxes.len());
    for b in boxes {
        split_by_profiles(grid, b, cfg, &mut split);
    }

    let mut out: Vec<Rect> = split
        .into_iter()
        .filter_map(|b| tighten(grid, &b))
        .filter(|b| {
            let filled = (b.r0..=b.r1)
                .map(|r| (b.c0..=b.c1).filter(|&c| grid.is_filled(r, c)).count())
                .sum::<usize>();
            let density = filled as f32 / (b.height() * b.width()) as f32;
            b.height() >= cfg.min_table_rows
                && filled >= cfg.min_table_cells
                && density >= cfg.min_density
        })
        .collect();
    out.sort_by_key(|b| (b.r0, b.c0));
    out
}

// ============================ Header detection ============================

/// Type-contrast column vote (csv.Sniffer / DuckDB-style, generalized).
/// Scores `header_row` as a header for the body starting at `body_start`.
/// Range: -1.0 .. 1.0; accept above ~0.3.
#[cfg(feature = "execute")]
fn header_vote(grid: &SheetGrid, rect: &Rect, header_row: usize, body_start: usize) -> f32 {
    if body_start > rect.r1 {
        return -1.0;
    }
    let sample_end = (body_start + 20).min(rect.r1);
    let mut votes = 0i32;
    let mut considered = 0usize;

    for c in rect.c0..=rect.c1 {
        let head = grid.cell_merged(header_row, c);
        let head_kind = cell_kind(head);
        let dominant = column_dominant(grid, c, body_start..=sample_end);
        match (head_kind, dominant) {
            (None, _) => {}
            (Some(Kind::Str), Some((Kind::Num | Kind::Date | Kind::Bool, _))) => {
                considered += 1;
                votes += 2;
            }
            (Some(Kind::Str), Some((Kind::Str, _))) => {
                considered += 1;
                let head_text = head.display();
                let repeated = (body_start..=sample_end)
                    .any(|r| grid.cell(r, c).display().eq_ignore_ascii_case(&head_text));
                votes += if repeated { 0 } else { 1 };
            }
            (Some(Kind::Str), None) => {
                considered += 1;
                votes += 1;
            }
            (Some(Kind::Num), Some((Kind::Num, _))) => {
                // Year-style headers (2019 | 2020 | 2021) over numeric bodies
                considered += 1;
                if let CellValue::Int(y) = head {
                    votes += if (1900..=2100).contains(y) { 1 } else { -2 };
                } else {
                    votes -= 2;
                }
            }
            (Some(_), _) => {
                considered += 1;
                votes -= 2;
            }
        }
    }
    if considered == 0 {
        return -1.0;
    }
    votes as f32 / (2 * considered) as f32
}

/// True when the row's filled cells are predominantly strings or empty.
#[cfg(feature = "execute")]
fn row_stringish(grid: &SheetGrid, rect: &Rect, r: usize) -> bool {
    let s = row_stats(grid, rect, r);
    s.filled == 0 || s.strings * 5 >= s.filled * 4
}

/// Detects header depth (0 = no header) at the top of the region.
#[cfg(feature = "execute")]
fn detect_header_depth(grid: &SheetGrid, rect: &Rect, cfg: &ExtractConfig) -> (usize, f32) {
    let max_h = cfg
        .max_header_rows
        .min(rect.height().saturating_sub(1))
        .min(rect.height().saturating_sub(cfg.min_table_rows.min(1)));
    let mut best = (0usize, 0.3f32);
    for h in 1..=max_h {
        let band_bottom = rect.r0 + h - 1;
        let body_start = rect.r0 + h;
        // Upper band rows must look like header material
        if h > 1 && !(rect.r0..band_bottom).all(|r| row_stringish(grid, rect, r)) {
            break;
        }
        // A unit row ("(EUR)", "kg") under the header belongs to unit folding,
        // not to the header band — its text-over-numbers vote would win here.
        if h > 1 && detect_unit_row(grid, rect, band_bottom) {
            break;
        }
        let score = header_vote(grid, rect, band_bottom, body_start);
        // Prefer deeper bands only on a real improvement
        if score > best.1 + if best.0 == 0 { 0.0 } else { 0.05 } {
            best = (h, score);
        }
    }
    best
}

// ============================ Row roles ============================

#[cfg(feature = "execute")]
static AGGREGATE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^\s*((sub)?total|sum(me)?\b|gesamt(ergebnis)?|insgesamt|zwischensumme|grand\s+total|average|mittelwert|Ø)",
    )
    .unwrap()
});

#[cfg(feature = "execute")]
static NOTE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(\*|note[:\s]|notes[:\s]|source[:\s]|quelle[:\s]|hinweis|stand[:\s]|see\s|vgl\.|\d+\))")
        .unwrap()
});

#[cfg(feature = "execute")]
fn first_filled_text(grid: &SheetGrid, rect: &Rect, r: usize) -> Option<String> {
    (rect.c0..=rect.c1).find_map(|c| {
        let v = grid.cell_merged(r, c);
        (!v.is_empty()).then(|| v.display())
    })
}

/// Peels title rows from the top of the region. Returns (title, new_top).
#[cfg(feature = "execute")]
fn peel_titles(grid: &SheetGrid, rect: &Rect, cfg: &ExtractConfig) -> (Option<String>, usize) {
    let mut top = rect.r0;
    let mut parts: Vec<String> = Vec::new();
    let max_peel = 3usize;
    while top < rect.r1 && parts.len() < max_peel && rect.r1 - top + 1 > cfg.min_table_rows {
        let s = row_stats(grid, rect, top);
        if s.filled == 0 {
            top += 1;
            continue;
        }
        let sparse = s.filled == 1 || s.filled * 4 <= rect.width();
        let texty = s.strings >= s.scalars;
        let title_like =
            texty && ((sparse && rect.width() >= 3) || (s.wide_merge && s.filled <= 2));
        if !title_like {
            break;
        }
        // A structurally header-like row is not a title (vote is only
        // meaningful when the row actually spans the columns)
        if s.filled * 2 >= rect.width() && header_vote(grid, rect, top, top + 1) > 0.45 {
            break;
        }
        if let Some(t) = first_filled_text(grid, rect, top) {
            parts.push(t);
        }
        top += 1;
    }
    let title = if parts.is_empty() {
        None
    } else {
        Some(parts.join(" — "))
    };
    (title, top)
}

/// Peels footnote rows from the bottom. Returns (notes, new_bottom).
#[cfg(feature = "execute")]
fn peel_footnotes(grid: &SheetGrid, rect: &Rect, top: usize) -> (Vec<String>, usize) {
    let mut bottom = rect.r1;
    let mut notes: Vec<String> = Vec::new();
    while bottom > top && notes.len() < 3 {
        let s = row_stats(grid, rect, bottom);
        if s.filled == 0 {
            bottom -= 1;
            continue;
        }
        let sparse = s.filled <= (rect.width() / 4).max(1);
        if !(sparse && s.strings >= 1) {
            break;
        }
        let Some(text) = first_filled_text(grid, rect, bottom) else {
            break;
        };
        if !NOTE_RE.is_match(&text) {
            break;
        }
        notes.push(text);
        bottom -= 1;
    }
    notes.reverse();
    (notes, bottom)
}

// ============================ Header flattening ============================

#[cfg(feature = "execute")]
fn flatten_headers(
    grid: &SheetGrid,
    rect: &Rect,
    header_top: usize,
    depth: usize,
    cfg: &ExtractConfig,
) -> Vec<String> {
    let width = rect.width();
    if depth == 0 {
        return (1..=width).map(|i| format!("column_{i}")).collect();
    }

    // levels[l][c]: header text with merge anchors resolved
    let mut levels: Vec<Vec<String>> = Vec::with_capacity(depth);
    for l in 0..depth {
        let r = header_top + l;
        let mut level: Vec<String> = (rect.c0..=rect.c1)
            .map(|c| clean_header_text(&grid.cell_merged(r, c).display(), cfg))
            .collect();
        // Forward-fill upper levels (group bands span columns even without merges)
        if l + 1 < depth {
            for i in 1..level.len() {
                if level[i].is_empty() {
                    level[i] = level[i - 1].clone();
                }
            }
        }
        levels.push(level);
    }

    let mut headers: Vec<String> = Vec::with_capacity(width);
    for c in 0..width {
        let mut parts: Vec<&str> = Vec::with_capacity(depth);
        for level in &levels {
            let cell = level[c].as_str();
            if cell.is_empty() || parts.last().is_some_and(|&p| p == cell) {
                continue;
            }
            parts.push(cell);
        }
        headers.push(parts.join(&cfg.header_joiner));
    }

    // Name empties and dedup duplicates
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for (i, h) in headers.iter_mut().enumerate() {
        if h.is_empty() {
            *h = format!("column_{}", i + 1);
        }
        let n = seen.entry(h.to_ascii_lowercase()).or_insert(0);
        *n += 1;
        if *n > 1 {
            *h = format!("{} ({})", h, *n);
        }
    }
    headers
}

#[cfg(feature = "execute")]
fn clean_header_text(s: &str, cfg: &ExtractConfig) -> String {
    let joined = s
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(&cfg.header_joiner);
    joined.trim_end_matches(['*', ' ']).trim().to_string()
}

/// Detects a unit row directly under the header band ("(in thousands)", "%", "kg").
#[cfg(feature = "execute")]
fn detect_unit_row(grid: &SheetGrid, rect: &Rect, r: usize) -> bool {
    if r > rect.r1 {
        return false;
    }
    static UNIT_TOKENS: &[&str] = &[
        "%", "‰", "€", "$", "¥", "£", "kg", "g", "t", "km", "cm", "mm", "l", "ml", "pcs", "stk",
        "h", "min", "sec", "s", "eur", "usd", "gbp", "chf",
    ];
    let mut filled = 0usize;
    let mut unit_like = 0usize;
    for c in rect.c0..=rect.c1 {
        let v = grid.cell(r, c);
        if v.is_empty() {
            continue;
        }
        filled += 1;
        let t = v.display();
        let t = t.trim();
        let bracketed = t.starts_with('(') && t.ends_with(')');
        let symbolic = UNIT_TOKENS.iter().any(|u| t.eq_ignore_ascii_case(u));
        if bracketed || symbolic || t.ends_with('%') {
            unit_like += 1;
        }
    }
    filled > 0 && unit_like * 2 >= filled && unit_like >= 1
}

#[cfg(feature = "execute")]
fn merge_units_into_headers(grid: &SheetGrid, rect: &Rect, r: usize, headers: &mut [String]) {
    for (i, c) in (rect.c0..=rect.c1).enumerate() {
        let v = grid.cell(r, c);
        if v.is_empty() || i >= headers.len() {
            continue;
        }
        let unit = v.display();
        let unit = unit.trim().trim_matches(['(', ')']).trim().to_string();
        if unit.is_empty() {
            continue;
        }
        if headers[i].is_empty() || headers[i].starts_with("column_") {
            headers[i] = unit;
        } else {
            headers[i] = format!("{} [{}]", headers[i], unit);
        }
    }
}

// ============================ Table building ============================

/// Overrides for LLM-guided extraction: force a header depth and skip
/// specific absolute (1-based, sheet-coordinate) rows.
#[cfg(feature = "execute")]
#[derive(Clone, Debug, Default)]
pub struct BuildOverrides {
    pub header_rows: Option<usize>,
    pub skip_rows: std::collections::HashSet<usize>,
    pub column_names: Option<Vec<String>>,
}

/// Builds a table from a region: peels decoration, detects headers, emits
/// typed rows.
#[cfg(feature = "execute")]
pub fn build_table_from_rect(
    grid: &SheetGrid,
    rect: &Rect,
    cfg: &ExtractConfig,
    overrides: &BuildOverrides,
) -> Option<DetectedTable> {
    let (title, top) = if overrides.header_rows.is_some() {
        (None, rect.r0)
    } else {
        peel_titles(grid, rect, cfg)
    };
    let (notes, bottom) = peel_footnotes(grid, rect, top);
    if bottom < top {
        return None;
    }
    let body_rect = Rect {
        r0: top,
        r1: bottom,
        ..*rect
    };
    let body_rect = tighten(grid, &body_rect)?;

    let (depth, header_score) = match overrides.header_rows {
        Some(h) => (h.min(body_rect.height().saturating_sub(1)), 1.0),
        None => detect_header_depth(grid, &body_rect, cfg),
    };

    let mut headers = flatten_headers(grid, &body_rect, body_rect.r0, depth, cfg);
    if let Some(names) = &overrides.column_names
        && !names.is_empty()
    {
        for (i, n) in names.iter().enumerate() {
            if i < headers.len() && !n.trim().is_empty() {
                headers[i] = n.trim().to_string();
            }
        }
    }

    let mut data_start = body_rect.r0 + depth;
    if depth > 0
        && data_start <= body_rect.r1
        && overrides.header_rows.is_none()
        && detect_unit_row(grid, &body_rect, data_start)
    {
        merge_units_into_headers(grid, &body_rect, data_start, &mut headers);
        data_start += 1;
    }

    let normalized_headers: Vec<String> = headers.iter().map(|h| h.to_ascii_lowercase()).collect();
    let mut rows: Vec<Vec<flow_like_types::Value>> = Vec::new();
    let mut dropped = 0usize;

    for r in data_start..=body_rect.r1 {
        let abs_row_1b = grid.start_row + r + 1;
        if overrides.skip_rows.contains(&abs_row_1b) {
            dropped += 1;
            continue;
        }
        let stats = row_stats(grid, &body_rect, r);
        if stats.filled == 0 {
            continue;
        }
        // Repeated header rows (paginated exports)
        if depth > 0 {
            let matches_header = (body_rect.c0..=body_rect.c1).enumerate().all(|(i, c)| {
                let v = grid.cell(r, c);
                v.is_empty()
                    || normalized_headers
                        .get(i)
                        .is_some_and(|h| v.display().to_ascii_lowercase() == *h)
            });
            if matches_header && stats.filled * 2 >= body_rect.width() {
                dropped += 1;
                continue;
            }
        }
        if cfg.drop_aggregate_rows
            && let Some(first) = first_filled_text(grid, &body_rect, r)
            && AGGREGATE_RE.is_match(&first)
        {
            dropped += 1;
            continue;
        }
        let row: Vec<flow_like_types::Value> = (body_rect.c0..=body_rect.c1)
            .map(|c| grid.cell_merged(r, c).to_json())
            .collect();
        rows.push(row);
    }

    if rows.is_empty() && depth == 0 {
        return None;
    }

    // Confidence: header quality + column type coherence + density
    let type_coherence = {
        let cols_total = body_rect.width().max(1);
        let coherent = (body_rect.c0..=body_rect.c1)
            .filter(|&c| {
                column_dominant(grid, c, data_start..=body_rect.r1)
                    .is_none_or(|(_, ratio)| ratio >= 0.95)
            })
            .count();
        coherent as f32 / cols_total as f32
    };
    let density = {
        let filled = (body_rect.r0..=body_rect.r1)
            .map(|r| {
                (body_rect.c0..=body_rect.c1)
                    .filter(|&c| grid.is_filled(r, c))
                    .count()
            })
            .sum::<usize>();
        filled as f32 / (body_rect.height() * body_rect.width()) as f32
    };
    let confidence =
        (0.4 * header_score.clamp(0.0, 1.0) + 0.4 * type_coherence + 0.2 * density).clamp(0.0, 1.0);

    Some(DetectedTable {
        title: title.map(|t| truncate_chars(&t, 200)),
        range_a1: grid.a1_range(body_rect.r0, body_rect.c0, body_rect.r1, body_rect.c1),
        headers,
        rows,
        notes,
        header_detected: depth > 0,
        dropped_rows: dropped,
        confidence,
    })
}

/// Detects and builds all tables in a sheet grid.
#[cfg(feature = "execute")]
pub fn extract_tables_from_grid(grid: &SheetGrid, cfg: &ExtractConfig) -> Vec<DetectedTable> {
    if cfg.adaptive_segmentation && grid.height * grid.width <= ADAPTIVE_MAX_CELLS {
        return extract_tables_adaptive(grid, cfg);
    }
    extract_with_rects(grid, cfg)
        .into_iter()
        .map(|(_, t)| t)
        .collect()
}

#[cfg(feature = "execute")]
const ADAPTIVE_MAX_CELLS: usize = 500_000;

#[cfg(feature = "execute")]
fn extract_with_rects(grid: &SheetGrid, cfg: &ExtractConfig) -> Vec<(Rect, DetectedTable)> {
    detect_table_regions(grid, cfg)
        .into_iter()
        .filter_map(|rect| {
            build_table_from_rect(grid, &rect, cfg, &BuildOverrides::default()).map(|t| (rect, t))
        })
        .collect()
}

/// Segmentation fitness: coverage-weighted table confidence minus a small
/// over-segmentation penalty (the transferable core of genetic-search table
/// recognition — score candidate segmentations, keep the best).
#[cfg(feature = "execute")]
fn segmentation_fitness(grid: &SheetGrid, tables: &[(Rect, DetectedTable)]) -> f32 {
    let total_filled = grid.count_filled().max(1);
    let covered_confidence: f32 = tables
        .iter()
        .map(|(rect, t)| {
            let filled = (rect.r0..=rect.r1)
                .map(|r| {
                    (rect.c0..=rect.c1)
                        .filter(|&c| grid.is_filled(r, c))
                        .count()
                })
                .sum::<usize>();
            t.confidence * filled as f32
        })
        .sum();
    covered_confidence / total_filled as f32 - 0.02 * tables.len() as f32
}

/// Tries the configured gap tolerances plus tighter/looser variants and keeps
/// the segmentation with the best fitness.
#[cfg(feature = "execute")]
fn extract_tables_adaptive(grid: &SheetGrid, cfg: &ExtractConfig) -> Vec<DetectedTable> {
    let mut variants = vec![
        (cfg.row_gap_tolerance, cfg.col_gap_tolerance),
        (0, 0),
        (2, 1),
    ];
    variants.dedup();

    let mut best: Option<(f32, Vec<(Rect, DetectedTable)>)> = None;
    for (row_gap, col_gap) in variants {
        let variant_cfg = ExtractConfig {
            row_gap_tolerance: row_gap,
            col_gap_tolerance: col_gap,
            ..cfg.clone()
        };
        let tables = extract_with_rects(grid, &variant_cfg);
        if tables.is_empty() {
            continue;
        }
        let fitness = segmentation_fitness(grid, &tables);
        if best.as_ref().is_none_or(|(bf, _)| fitness > *bf) {
            best = Some((fitness, tables));
        }
    }
    best.map(|(_, tables)| tables.into_iter().map(|(_, t)| t).collect())
        .unwrap_or_default()
}

/// Treats the sheet's whole used range as a single table (register-sheet mode).
#[cfg(feature = "execute")]
pub fn whole_sheet_table(grid: &SheetGrid, cfg: &ExtractConfig) -> Option<DetectedTable> {
    if grid.height == 0 || grid.width == 0 {
        return None;
    }
    let full = Rect {
        r0: 0,
        c0: 0,
        r1: grid.height - 1,
        c1: grid.width - 1,
    };
    let rect = tighten(grid, &full)?;
    build_table_from_rect(grid, &rect, cfg, &BuildOverrides::default())
}

// ============================ Workbook orchestration ============================

/// How a workbook is turned into tables.
#[cfg(feature = "execute")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetTableMode {
    /// One table per sheet from its used range (headers still detected)
    WholeSheet,
    /// Detect all tables per sheet (defined tables first, then heuristics)
    DetectTables,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
pub struct WorkbookTables {
    pub tables: Vec<crate::data::excel::CSVTable>,
    pub warnings: Vec<String>,
}

/// Extracts tables from a workbook into `CSVTable`s with normalized, unique
/// names: a sheet's first table gets the normalized sheet name, further tables
/// get `_2`, `_3`, …. Defined Excel tables (ListObjects) keep their own name
/// and shadow heuristic detection on the ranges they cover.
#[cfg(feature = "execute")]
pub fn extract_workbook_tables(
    bytes: Vec<u8>,
    sheet_filter: Option<&str>,
    cfg: &ExtractConfig,
    mode: SheetTableMode,
    name_prefix: &str,
    source: Option<crate::data::path::FlowPath>,
) -> flow_like_types::Result<WorkbookTables> {
    use super::grid::{Workbook, normalize_table_name, unique_table_name};

    let mut wb = Workbook::open(bytes)?;
    let sheet_names: Vec<String> = match sheet_filter {
        Some(f) if !f.trim().is_empty() => {
            let wanted = f.trim();
            let names = wb.sheet_names();
            let found = names.into_iter().find(|n| n == wanted).ok_or_else(|| {
                flow_like_types::anyhow!("Sheet '{wanted}' not found in workbook")
            })?;
            vec![found]
        }
        _ => wb.sheet_names(),
    };

    let mut used_names = std::collections::HashSet::new();
    let mut tables = Vec::new();
    let mut warnings = Vec::new();

    for sheet in &sheet_names {
        let mut covered: Vec<super::grid::AbsoluteBounds> = Vec::new();

        if mode == SheetTableMode::DetectTables && cfg.use_defined_tables {
            for dt in wb.defined_tables(sheet) {
                let base = normalize_table_name(&format!("{name_prefix}{}", dt.name));
                let name = unique_table_name(&used_names, &base);
                used_names.insert(name.clone());
                if let Some(b) = dt.bounds {
                    covered.push(b);
                }
                let mut headers = dt.columns.clone();
                let width = dt.rows.first().map(|r| r.len()).unwrap_or(headers.len());
                if headers.is_empty() {
                    headers = (1..=width).map(|i| format!("column_{i}")).collect();
                }
                let rows: Vec<Vec<flow_like_types::Value>> = dt
                    .rows
                    .iter()
                    .skip_while(|row| {
                        // Table ranges include the header row — skip it if present
                        !dt.columns.is_empty()
                            && row.len() == dt.columns.len()
                            && row
                                .iter()
                                .zip(&dt.columns)
                                .all(|(v, h)| v.display().eq_ignore_ascii_case(h))
                    })
                    .map(|row| row.iter().map(|v| v.to_json()).collect())
                    .collect();
                let mut csv = crate::data::excel::CSVTable::new(headers, rows, source.clone());
                csv.name = Some(name);
                tables.push(csv);
            }
        }

        let grid = match wb.read_grid(sheet) {
            Ok(g) => g,
            Err(e) => {
                warnings.push(format!("Sheet '{sheet}': {e}"));
                continue;
            }
        };

        let detected: Vec<DetectedTable> = match mode {
            SheetTableMode::WholeSheet => whole_sheet_table(&grid, cfg).into_iter().collect(),
            SheetTableMode::DetectTables if covered.is_empty() => {
                extract_tables_from_grid(&grid, cfg)
            }
            SheetTableMode::DetectTables => {
                // Defined tables cover parts of the sheet — detect only outside them
                let regions: Vec<Rect> = detect_table_regions(&grid, cfg)
                    .into_iter()
                    .filter(|r| {
                        !covered.iter().any(|b| {
                            let abs_r0 = grid.start_row + r.r0;
                            let abs_r1 = grid.start_row + r.r1;
                            let abs_c0 = grid.start_col + r.c0;
                            let abs_c1 = grid.start_col + r.c1;
                            abs_r0 <= b.r1 && b.r0 <= abs_r1 && abs_c0 <= b.c1 && b.c0 <= abs_c1
                        })
                    })
                    .collect();
                regions
                    .iter()
                    .filter_map(|rect| {
                        build_table_from_rect(&grid, rect, cfg, &BuildOverrides::default())
                    })
                    .collect()
            }
        };

        if detected.is_empty() && covered.is_empty() {
            warnings.push(format!("Sheet '{sheet}': no tables found"));
        }

        let sheet_base = normalize_table_name(&format!("{name_prefix}{sheet}"));
        for table in detected {
            let name = unique_table_name(&used_names, &sheet_base);
            used_names.insert(name.clone());
            tables.push(detected_to_csv(table, name, source.clone()));
        }
    }

    Ok(WorkbookTables { tables, warnings })
}

#[cfg(feature = "execute")]
pub fn detected_to_csv(
    table: DetectedTable,
    name: String,
    source: Option<crate::data::path::FlowPath>,
) -> crate::data::excel::CSVTable {
    let mut csv = crate::data::excel::CSVTable::new(table.headers, table.rows, source);
    csv.name = Some(name);
    csv.title = table.title;
    csv.range = Some(table.range_a1);
    csv
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use crate::data::excel::grid::MergeSpan;

    /// "" → Empty, integers → Int, decimals → Float, rest → Text.
    fn grid_from(rows: &[&[&str]]) -> SheetGrid {
        let cells: Vec<Vec<CellValue>> = rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|s| {
                        if s.is_empty() {
                            CellValue::Empty
                        } else if let Ok(i) = s.parse::<i64>() {
                            CellValue::Int(i)
                        } else if let Ok(f) = s.parse::<f64>() {
                            CellValue::Float(f)
                        } else {
                            CellValue::Text(s.to_string())
                        }
                    })
                    .collect()
            })
            .collect();
        SheetGrid::from_rows(cells, Vec::new())
    }

    #[test]
    fn simple_table_with_header() {
        let grid = grid_from(&[
            &["Name", "Age", "City"],
            &["Alice", "30", "Berlin"],
            &["Bob", "25", "Hamburg"],
            &["Carol", "41", "Munich"],
        ]);
        let tables = extract_tables_from_grid(&grid, &ExtractConfig::default());
        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert!(t.header_detected);
        assert_eq!(t.headers, vec!["Name", "Age", "City"]);
        assert_eq!(t.rows.len(), 3);
        assert_eq!(t.range_a1, "A1:C4");
    }

    #[test]
    fn stacked_tables_split_and_internal_blank_kept() {
        let grid = grid_from(&[
            &["Product", "Price"],
            &["Apple", "3"],
            &["", ""],
            &["Pear", "4"],
            &["", ""],
            &["", ""],
            &["", ""],
            &["Country", "Population"],
            &["Germany", "83000000"],
            &["France", "68000000"],
        ]);
        let tables = extract_tables_from_grid(&grid, &ExtractConfig::default());
        assert_eq!(tables.len(), 2, "expected 2 tables, got {:?}", tables.len());
        assert_eq!(tables[0].headers, vec!["Product", "Price"]);
        assert_eq!(tables[0].rows.len(), 2, "internal blank row must not split");
        assert_eq!(tables[1].headers, vec!["Country", "Population"]);
    }

    #[test]
    fn side_by_side_tables_split() {
        let grid = grid_from(&[
            &["Name", "Age", "", "", "Item", "Price"],
            &["Alice", "30", "", "", "Nail", "1"],
            &["Bob", "25", "", "", "Screw", "2"],
            &["Carol", "41", "", "", "Bolt", "3"],
        ]);
        let tables = extract_tables_from_grid(&grid, &ExtractConfig::default());
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].headers, vec!["Name", "Age"]);
        assert_eq!(tables[1].headers, vec!["Item", "Price"]);
    }

    #[test]
    fn title_peeled_and_kept() {
        let grid = grid_from(&[
            &["Sales Report 2024", "", "", ""],
            &["", "", "", ""],
            &["Region", "Product", "Units", "Revenue"],
            &["North", "Widget", "12", "120.5"],
            &["South", "Widget", "8", "80.5"],
            &["North", "Gadget", "3", "300.5"],
        ]);
        let tables = extract_tables_from_grid(&grid, &ExtractConfig::default());
        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert_eq!(t.title.as_deref(), Some("Sales Report 2024"));
        assert_eq!(t.headers, vec!["Region", "Product", "Units", "Revenue"]);
        assert_eq!(t.rows.len(), 3);
    }

    #[test]
    fn aggregate_rows_dropped_and_kept_when_disabled() {
        let rows: &[&[&str]] = &[
            &["Item", "Amount"],
            &["A", "1"],
            &["B", "2"],
            &["Total", "3"],
        ];
        let grid = grid_from(rows);
        let tables = extract_tables_from_grid(&grid, &ExtractConfig::default());
        assert_eq!(tables[0].rows.len(), 2);
        assert_eq!(tables[0].dropped_rows, 1);

        let cfg = ExtractConfig {
            drop_aggregate_rows: false,
            ..Default::default()
        };
        let tables = extract_tables_from_grid(&grid, &cfg);
        assert_eq!(tables[0].rows.len(), 3);
    }

    #[test]
    fn repeated_header_rows_dropped() {
        let grid = grid_from(&[
            &["Name", "Age"],
            &["Alice", "30"],
            &["Name", "Age"],
            &["Bob", "25"],
        ]);
        let tables = extract_tables_from_grid(&grid, &ExtractConfig::default());
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].rows.len(), 2);
    }

    #[test]
    fn multi_row_header_flattened_with_merge() {
        let cells = vec![
            vec![
                CellValue::Text("Region".into()),
                CellValue::Text("2023".into()),
                CellValue::Empty,
                CellValue::Text("2024".into()),
                CellValue::Empty,
            ],
            vec![
                CellValue::Empty,
                CellValue::Text("Q1".into()),
                CellValue::Text("Q2".into()),
                CellValue::Text("Q1".into()),
                CellValue::Text("Q2".into()),
            ],
            vec![
                CellValue::Text("North".into()),
                CellValue::Int(1),
                CellValue::Int(2),
                CellValue::Int(3),
                CellValue::Int(4),
            ],
            vec![
                CellValue::Text("South".into()),
                CellValue::Int(5),
                CellValue::Int(6),
                CellValue::Int(7),
                CellValue::Int(8),
            ],
        ];
        let merges = vec![
            MergeSpan {
                r0: 0,
                c0: 1,
                r1: 0,
                c1: 2,
            },
            MergeSpan {
                r0: 0,
                c0: 3,
                r1: 0,
                c1: 4,
            },
        ];
        let grid = SheetGrid::from_rows(cells, merges);
        let tables = extract_tables_from_grid(&grid, &ExtractConfig::default());
        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert!(t.header_detected);
        assert_eq!(
            t.headers,
            vec!["Region", "2023 / Q1", "2023 / Q2", "2024 / Q1", "2024 / Q2"]
        );
        assert_eq!(t.rows.len(), 2);
    }

    #[test]
    fn headerless_numeric_block_gets_generated_names() {
        let grid = grid_from(&[
            &["1", "2", "3"],
            &["4", "5", "6"],
            &["7", "8", "9"],
            &["10", "11", "12"],
        ]);
        let tables = extract_tables_from_grid(&grid, &ExtractConfig::default());
        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert!(!t.header_detected);
        assert_eq!(t.headers, vec!["column_1", "column_2", "column_3"]);
        assert_eq!(t.rows.len(), 4);
    }

    #[test]
    fn footnotes_peeled() {
        let grid = grid_from(&[
            &["Name", "Score"],
            &["Alice", "10"],
            &["Bob", "20"],
            &["* preliminary results", ""],
        ]);
        let tables = extract_tables_from_grid(&grid, &ExtractConfig::default());
        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.notes, vec!["* preliminary results"]);
    }

    #[test]
    fn typed_values_survive() {
        let grid = grid_from(&[
            &["Name", "Count", "Ratio"],
            &["A", "10", "0.5"],
            &["B", "20", "1.5"],
        ]);
        let tables = extract_tables_from_grid(&grid, &ExtractConfig::default());
        let t = &tables[0];
        assert_eq!(t.rows[0][1], flow_like_types::json::json!(10));
        assert_eq!(t.rows[1][2], flow_like_types::json::json!(1.5));
    }

    #[test]
    fn build_overrides_force_header_and_skip_rows() {
        let grid = grid_from(&[
            &["Name", "Age"],
            &["Alice", "30"],
            &["Bob", "25"],
            &["Carol", "41"],
        ]);
        let rect = Rect {
            r0: 0,
            c0: 0,
            r1: 3,
            c1: 1,
        };
        let overrides = BuildOverrides {
            header_rows: Some(1),
            skip_rows: [3usize].into_iter().collect(), // absolute 1-based → "Bob"
            column_names: Some(vec!["person".into(), "years".into()]),
        };
        let t = build_table_from_rect(&grid, &rect, &ExtractConfig::default(), &overrides).unwrap();
        assert_eq!(t.headers, vec!["person", "years"]);
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[0][0], flow_like_types::json::json!("Alice"));
        assert_eq!(t.rows[1][0], flow_like_types::json::json!("Carol"));
    }

    #[test]
    fn whole_sheet_mode_single_table() {
        let grid = grid_from(&[
            &["", "", ""],
            &["", "Name", "Age"],
            &["", "Alice", "30"],
            &["", "Bob", "25"],
        ]);
        let t = whole_sheet_table(&grid, &ExtractConfig::default()).unwrap();
        assert_eq!(t.headers, vec!["Name", "Age"]);
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.range_a1, "B2:C4");
    }

    #[test]
    fn workbook_end_to_end_with_umya() {
        let mut book = umya_spreadsheet::new_file();
        {
            let sheet = book.get_sheet_mut(&0).unwrap();
            sheet.set_name("Sales Data (2024)");
            let header = ["Region", "Product", "Units"];
            for (c, h) in header.iter().enumerate() {
                sheet
                    .get_cell_mut(((c + 1) as u32, 1u32))
                    .set_value_string(*h);
            }
            let data = [
                ("North", "Widget", 12.0),
                ("South", "Widget", 8.0),
                ("North", "Gadget", 3.0),
            ];
            for (r, (region, product, units)) in data.iter().enumerate() {
                let row = (r + 2) as u32;
                sheet.get_cell_mut((1u32, row)).set_value_string(*region);
                sheet.get_cell_mut((2u32, row)).set_value_string(*product);
                sheet.get_cell_mut((3u32, row)).set_value_number(*units);
            }
        }
        {
            let sheet2 = book.new_sheet("Übersicht").unwrap();
            sheet2.get_cell_mut((1u32, 1u32)).set_value_string("Key");
            sheet2.get_cell_mut((2u32, 1u32)).set_value_string("Value");
            sheet2.get_cell_mut((1u32, 2u32)).set_value_string("Done");
            sheet2.get_cell_mut((2u32, 2u32)).set_value_number(23.0);
            sheet2.get_cell_mut((1u32, 3u32)).set_value_string("Open");
            sheet2.get_cell_mut((2u32, 3u32)).set_value_number(5.0);
        }
        let mut buf: Vec<u8> = Vec::new();
        umya_spreadsheet::writer::xlsx::write_writer(&book, &mut buf).unwrap();

        let result = extract_workbook_tables(
            buf,
            None,
            &ExtractConfig::default(),
            SheetTableMode::WholeSheet,
            "",
            None,
        )
        .unwrap();
        let names: Vec<_> = result
            .tables
            .iter()
            .filter_map(|t| t.name.clone())
            .collect();
        assert_eq!(names, vec!["sales_data_2024", "uebersicht"]);
        assert_eq!(
            result.tables[0].headers(),
            vec!["Region", "Product", "Units"]
        );
        assert_eq!(result.tables[0].row_count(), 3);
        assert_eq!(result.tables[1].row_count(), 2);
    }

    #[test]
    fn workbook_detect_mode_multiple_tables_per_sheet() {
        let mut book = umya_spreadsheet::new_file();
        let sheet = book.get_sheet_mut(&0).unwrap();
        sheet.set_name("Mixed");
        // Table 1 at A1
        sheet.get_cell_mut((1u32, 1u32)).set_value_string("Name");
        sheet.get_cell_mut((2u32, 1u32)).set_value_string("Age");
        sheet.get_cell_mut((1u32, 2u32)).set_value_string("Alice");
        sheet.get_cell_mut((2u32, 2u32)).set_value_number(30.0);
        sheet.get_cell_mut((1u32, 3u32)).set_value_string("Bob");
        sheet.get_cell_mut((2u32, 3u32)).set_value_number(25.0);
        // Table 2 at A7 (3 blank rows between)
        sheet.get_cell_mut((1u32, 7u32)).set_value_string("City");
        sheet.get_cell_mut((2u32, 7u32)).set_value_string("Pop");
        sheet.get_cell_mut((1u32, 8u32)).set_value_string("Berlin");
        sheet.get_cell_mut((2u32, 8u32)).set_value_number(3.7);
        sheet.get_cell_mut((1u32, 9u32)).set_value_string("Paris");
        sheet.get_cell_mut((2u32, 9u32)).set_value_number(2.1);
        let mut buf: Vec<u8> = Vec::new();
        umya_spreadsheet::writer::xlsx::write_writer(&book, &mut buf).unwrap();

        let result = extract_workbook_tables(
            buf,
            None,
            &ExtractConfig::default(),
            SheetTableMode::DetectTables,
            "",
            None,
        )
        .unwrap();
        let names: Vec<_> = result
            .tables
            .iter()
            .filter_map(|t| t.name.clone())
            .collect();
        assert_eq!(names, vec!["mixed", "mixed_2"]);
        assert_eq!(result.tables[0].headers(), vec!["Name", "Age"]);
        assert_eq!(result.tables[1].headers(), vec!["City", "Pop"]);
    }
}

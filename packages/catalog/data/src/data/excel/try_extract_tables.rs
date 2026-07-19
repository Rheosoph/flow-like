use crate::data::excel::CSVTable;
use crate::data::path::FlowPath;
#[cfg(feature = "execute")]
use calamine::{Data, Range, Reader, open_workbook_auto_from_rs};
#[cfg(feature = "execute")]
use chrono::{Days, NaiveDate, NaiveDateTime, NaiveTime};
use flow_like::flow::node::NodeLogic;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::Node,
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(not(feature = "execute"))]
use flow_like_types::Result;
#[cfg(feature = "execute")]
use flow_like_types::{Context, Error, Result};
use flow_like_types::{JsonSchema, async_trait, json::json, tokio};
#[cfg(feature = "execute")]
use once_cell::sync::Lazy;
#[cfg(feature = "execute")]
use rayon::prelude::*;
#[cfg(feature = "execute")]
use regex::Regex;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::cmp::{max, min};
#[cfg(feature = "execute")]
use std::collections::VecDeque;
#[cfg(feature = "execute")]
use std::io::Cursor;
#[cfg(feature = "execute")]
use strsim::jaro_winkler;

#[cfg(feature = "execute")]
static TOTALS_RE: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r"(?i)^\s*(total|summe|subtotal|gesamt)\b").unwrap());

#[cfg(feature = "execute")]
const MAX_SUPPORTED_ROWS: usize = 10_000_000;
#[cfg(feature = "execute")]
const MAX_SUPPORTED_COLS: usize = 50_000;
#[cfg(feature = "execute")]
const MIN_VALID_GRID_SIZE: usize = 1;

#[cfg(feature = "execute")]
#[inline]
fn safe_grid_get(grid: &[Vec<String>], r: usize, c: usize) -> &str {
    grid.get(r)
        .and_then(|row| row.get(c))
        .map(|s| s.as_str())
        .unwrap_or("")
}

#[cfg(feature = "execute")]
fn validate_grid_dimensions(height: usize, width: usize) -> Result<()> {
    if height == 0 || width == 0 {
        return Err(Error::msg("Grid has zero dimensions"));
    }
    if height > MAX_SUPPORTED_ROWS {
        return Err(Error::msg(format!(
            "Grid height {} exceeds maximum supported {}",
            height, MAX_SUPPORTED_ROWS
        )));
    }
    if width > MAX_SUPPORTED_COLS {
        return Err(Error::msg(format!(
            "Grid width {} exceeds maximum supported {}",
            width, MAX_SUPPORTED_COLS
        )));
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn build_occupancy(grid: &Vec<Vec<String>>) -> Vec<Vec<bool>> {
    grid.iter()
        .map(|row| row.iter().map(|s| !s.trim().is_empty()).collect())
        .collect()
}

#[cfg(feature = "execute")]
#[inline]
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if !ch.is_whitespace() {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out
}

// ============================ Config ============================

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum OutputMode {
    /// Keep behavior: build Vec<Table> in memory
    InMemory,
    /// Stream tables to CSV files (temp folder) instead of RAM
    CsvFiles,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExtractConfig {
    /// CSV delimiter (`,` or `;` typical)
    pub delimiter: u8,
    /// Use CRLF line endings (Excel-friendly) vs LF
    pub crlf: bool,
    /// Write UTF-8 BOM (Excel-friendly on Windows)
    pub bom: bool,
    /// Protect against CSV injection by prefixing a single quote to dangerous prefixes (=+-@)
    pub csv_injection_hardening: bool,
    /// Consider a row/col "empty" if non-empty density below this (0.0..1.0)
    pub empty_density_threshold: f32,
    /// Break between tables if we see >= this many consecutive empty rows
    pub gap_break_rows: usize,
    /// Break between tables if we see >= this many consecutive empty cols
    pub gap_break_cols: usize,
    /// Allow up to this many blank rows *inside* a table
    pub allow_internal_blank_rows: usize,
    /// Allow up to this many blank cols *inside* a table
    pub allow_internal_blank_cols: usize,
    /// Minimum non-empty cell count for a rectangle to be considered a table
    pub min_table_cells: usize,
    /// Max header rows to consider when flattening
    pub max_header_rows: usize,
    /// Max left header columns to detect
    pub max_left_header_cols: usize,
    /// Joiner to flatten multi-row headers
    pub header_joiner: String,
    /// Drop totals rows matching regex (case-insensitive)
    pub drop_totals: bool,
    /// Allow merging rectangles across large blank/merged "spacer" bands
    pub stitch_across_spacers: bool,
    /// Sample size for type inference per column when stitching
    pub stitch_type_sample_rows: usize,
    /// Minimum ratio of compatible column types to allow stitching (0.0..1.0)
    pub stitch_min_type_match_ratio: f32,
    /// Minimum required column-overlap (intersection / union) to consider adjacency
    pub stitch_min_col_overlap: f32,
    /// After extraction, merge non-adjacent tables if headers are very similar
    pub group_similar_headers: bool,
    /// Similarity threshold (0.0..1.0) to merge tables by header
    pub header_merge_threshold: f64,
    /// How many rows to sample below header when validating schema
    pub schema_sample_rows: usize,
    /// Minimum fraction (0.0..1.0) of columns that must keep a consistent kind in body
    pub min_body_consistency: f32,
    /// Enable merged cell processing (requires second parse with umya); disable to save memory/time on huge sheets
    pub enable_merges: bool,
    /// Skip building merge map if total cells exceed this number (acts as safety valve)
    pub max_merge_map_cells: usize,
    /// Take (move) strings out of grid when building tables to avoid cloning large data
    pub take_cells_on_extract: bool,
    /// If height*width exceeds this, switch to "huge mode" (auto-disable heavy features).
    pub huge_cells_threshold: usize,
    /// In huge mode, cap how many rows we materialize for the in-memory path (0 = unlimited).
    pub huge_cap_rows: usize,
    /// In huge mode, use a smaller sample for schema checks.
    pub huge_schema_sample_rows: usize,
    /// Choose output strategy for huge sheets (default stays InMemory).
    pub output_mode: OutputMode,
}

impl Default for ExtractConfig {
    fn default() -> Self {
        Self {
            delimiter: b',',
            crlf: false,
            bom: false,
            csv_injection_hardening: true,
            empty_density_threshold: 0.05,
            gap_break_rows: 2,
            gap_break_cols: 2,
            allow_internal_blank_rows: 1,
            allow_internal_blank_cols: 1,
            min_table_cells: 8,
            max_header_rows: 3,
            max_left_header_cols: 3,
            header_joiner: " / ".to_string(),
            drop_totals: true,
            stitch_across_spacers: true,
            stitch_type_sample_rows: 16,
            stitch_min_type_match_ratio: 0.80,
            stitch_min_col_overlap: 0.70,
            group_similar_headers: true,
            header_merge_threshold: 0.97,
            schema_sample_rows: 25,
            min_body_consistency: 0.6,
            enable_merges: true,
            max_merge_map_cells: 2_000_000, // ~2M cells safeguard (~16MB of pointers)
            take_cells_on_extract: true,
            huge_cells_threshold: 20_000_000, // ~20M grid cells triggers huge-mode
            huge_cap_rows: 100_000,           // keep memory sane if staying in-memory
            huge_schema_sample_rows: 6,
            output_mode: OutputMode::InMemory,
        }
    }
}

// ============================ Public API ============================

/// Extract raw tables (headers + rows of strings) from a sheet.
#[cfg(feature = "execute")]
pub fn extract_tables(
    data: Vec<u8>,
    sheet_name: &str,
    cfg_in: &ExtractConfig,
) -> Result<Vec<Table>> {
    if data.is_empty() {
        return Err(Error::msg("Input data is empty"));
    }
    if sheet_name.trim().is_empty() {
        return Err(Error::msg("Sheet name cannot be empty"));
    }

    let cursor = Cursor::new(data);
    let mut wb = open_workbook_auto_from_rs(cursor.clone())
        .with_context(|| "Opening workbook failed - file may be corrupted or unsupported format")?;

    let range: Range<Data> = wb
        .worksheet_range(sheet_name)
        .with_context(|| format!("Sheet '{}' not found or unreadable", sheet_name))?;

    let height = range.get_size().0;
    let width = range.get_size().1;

    if height < MIN_VALID_GRID_SIZE || width < MIN_VALID_GRID_SIZE {
        return Ok(Vec::new());
    }

    validate_grid_dimensions(height, width)?;

    let cell_count = height.saturating_mul(width);
    let huge_mode = cell_count >= cfg_in.huge_cells_threshold;

    let mut cfg = cfg_in.clone();
    if huge_mode {
        cfg.enable_merges = false;
        cfg.stitch_across_spacers = false;
        cfg.group_similar_headers = false;
        cfg.allow_internal_blank_rows = 0;
        cfg.allow_internal_blank_cols = 0;
        cfg.gap_break_rows = usize::MAX;
        cfg.gap_break_cols = usize::MAX;
        cfg.schema_sample_rows = cfg.huge_schema_sample_rows;
    }

    let (grid_raw, height, width) = read_sheet_grid_capped(range, cfg.huge_cap_rows, huge_mode)?;

    if height == 0 || width == 0 {
        return Ok(Vec::new());
    }

    let cell_count = height.saturating_mul(width);
    let merges = if cfg.enable_merges && cell_count <= cfg.max_merge_map_cells {
        read_merged_cells(cursor, sheet_name).unwrap_or_else(|e| {
            eprintln!(
                "WARN: Failed to read merged cells, continuing without: {}",
                e
            );
            Vec::new()
        })
    } else {
        Vec::new()
    };

    let merge_map = if !merges.is_empty() {
        build_merge_map(height, width, &merges)
    } else {
        Vec::new()
    };

    let mut grid = apply_merges(grid_raw, &merges);

    let rects_coarse = segment_rectangles(&grid, height, width, &cfg);
    let mut rects: Vec<Rect> = Vec::with_capacity(rects_coarse.len() * 2);
    for r in rects_coarse {
        if let Some(validated) = validate_rect(&r, height, width) {
            let parts = split_rect_by_connectivity(&grid, &validated, &cfg);
            rects.extend(parts);
        }
    }

    let mut built: Vec<TableWithRect> = Vec::with_capacity(rects.len());
    for rect in rects {
        let nonempty = count_nonempty_in_rect(&grid, &rect);
        if nonempty < cfg.min_table_cells {
            continue;
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            build_table_from_rect(&mut grid, &rect, &cfg, &merge_map)
        })) {
            Ok(table) if !table.headers.is_empty() || !table.rows.is_empty() => {
                built.push(TableWithRect { rect, table });
            }
            Ok(_) => {}
            Err(_) => {
                eprintln!("WARN: Failed to build table from rect {:?}, skipping", rect);
            }
        }
    }

    let stitched = stitch_tables(&grid, built, &cfg);
    let grouped = if cfg.group_similar_headers {
        group_tables_by_header_similarity(stitched, &cfg)
    } else {
        stitched
    };

    let tables: Vec<Table> = grouped
        .into_iter()
        .map(|t| t.table)
        .filter(|t| !t.headers.is_empty() || !t.rows.is_empty())
        .collect();

    Ok(tables)
}

#[cfg(feature = "execute")]
fn validate_rect(rect: &Rect, height: usize, width: usize) -> Option<Rect> {
    if rect.r1 < rect.r0 || rect.c1 < rect.c0 {
        return None;
    }
    Some(Rect {
        r0: rect.r0.min(height.saturating_sub(1)),
        c0: rect.c0.min(width.saturating_sub(1)),
        r1: rect.r1.min(height.saturating_sub(1)),
        c1: rect.c1.min(width.saturating_sub(1)),
    })
}

// ============================ Types ============================

#[cfg(feature = "execute")]
#[derive(Clone, Debug)]
struct Rect {
    r0: usize,
    c0: usize,
    r1: usize, // inclusive
    c1: usize, // inclusive
}

#[cfg(feature = "execute")]
#[derive(Clone, Debug)]
pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

// ============================ IO helpers ============================

#[cfg(feature = "execute")]
fn read_sheet_grid_capped(
    range: calamine::Range<Data>,
    cap_rows: usize,
    huge_mode: bool,
) -> Result<(Vec<Vec<String>>, usize, usize)> {
    let mut height = range.get_size().0;
    let width = range.get_size().1;

    if height == 0 || width == 0 {
        return Ok((Vec::new(), 0, 0));
    }

    if huge_mode && cap_rows > 0 {
        height = height.min(cap_rows);
    }

    height = height.min(MAX_SUPPORTED_ROWS);
    let width = width.min(MAX_SUPPORTED_COLS);

    let mut grid = vec![vec![String::new(); width]; height];
    let is_1904 = false;

    for (r, row) in range.rows().take(height).enumerate() {
        let row_len = row.len().min(width);
        for (c, cell) in row.iter().take(row_len).enumerate() {
            grid[r][c] = if huge_mode {
                data_to_string(cell)
            } else {
                data_to_string_iso(cell, is_1904)
            };
        }
    }
    Ok((grid, height, width))
}

#[cfg(feature = "execute")]
fn excel_serial_to_iso(serial: f64, is_1904: bool) -> String {
    let days = serial.floor() as i64;
    let secs = ((serial - serial.floor()) * 86_400.0).round() as i64;

    // Excel epoch handling (with the 1900 leap-year bug)
    let base = if is_1904 {
        NaiveDate::from_ymd_opt(1904, 1, 1).unwrap()
    } else {
        // Serial 0 corresponds to 1899-12-30 in practice
        NaiveDate::from_ymd_opt(1899, 12, 30).unwrap()
    };

    let mut date = base + Days::new(days as u64);
    if !is_1904 && days >= 60 {
        // Skip the non-existent 1900-02-29
        date = date + Days::new(1);
    }

    let secs_norm = ((secs % 86_400) + 86_400) % 86_400;
    let time = NaiveTime::from_num_seconds_from_midnight_opt(secs_norm as u32, 0).unwrap();
    NaiveDateTime::new(date, time)
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string()
}

#[cfg(feature = "execute")]
fn data_to_string_iso(v: &Data, is_1904: bool) -> String {
    match v {
        Data::DateTime(serial) => excel_serial_to_iso(serial.as_f64(), is_1904),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        _ => data_to_string(v),
    }
}

#[cfg(feature = "execute")]
fn data_to_string(v: &Data) -> String {
    match v {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => {
            if f.fract() == 0.0 && *f >= -9_007_199_254_740_992.0 && *f <= 9_007_199_254_740_992.0 {
                format!("{:.0}", f)
            } else {
                f.to_string()
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        Data::DateTime(serial) => serial.to_string(),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#ERROR:{:?}", e),
    }
}

// ============================ Merges ============================

#[cfg(feature = "execute")]
#[derive(Clone, Copy, Debug)]
struct Merge {
    r0: usize,
    c0: usize,
    r1: usize,
    c1: usize,
}

#[cfg(feature = "execute")]
type MergeMap = Vec<Vec<Option<Merge>>>;

#[cfg(feature = "execute")]
fn build_merge_map(height: usize, width: usize, merges: &[Merge]) -> MergeMap {
    let mut map = vec![vec![None; width]; height];
    if height == 0 || width == 0 {
        return map;
    }

    for m in merges {
        // Clamp merge bounds to grid to avoid OOB
        let r0 = m.r0.min(height - 1);
        let c0 = m.c0.min(width - 1);
        let r1 = m.r1.min(height - 1);
        let c1 = m.c1.min(width - 1);
        if r0 >= height || c0 >= width {
            continue;
        }
        for r in r0..=r1 {
            for c in c0..=c1 {
                map[r][c] = Some(*m);
            }
        }
    }
    map
}

#[cfg(feature = "execute")]
fn is_horiz_merged_non_anchor(mm: &MergeMap, r: usize, c: usize) -> bool {
    if mm.is_empty() {
        return false;
    }
    if r >= mm.len() {
        return false;
    }
    if c >= mm[r].len() {
        return false;
    }
    if let Some(m) = mm[r][c] {
        (m.c1 > m.c0) && c != m.c0 && r >= m.r0 && r <= m.r1
    } else {
        false
    }
}

#[cfg(feature = "execute")]
fn read_merged_cells<P: std::io::Read + std::io::Seek + Clone>(
    data: P,
    sheet: &str,
) -> Result<Vec<Merge>> {
    let book = umya_spreadsheet::reader::xlsx::read_reader(data, true)
        .with_context(|| "umya read failed")?;

    let ws = book
        .get_sheet_by_name(sheet)
        .ok_or_else(|| Error::msg(format!("Sheet not found (umya): {sheet}")))?;

    let merged = ws.get_merge_cells(); // &[umya_spreadsheet::Range]
    let mut out = Vec::new();

    for r in merged {
        // 1) Prefer Display -> A1 like "A1:B2" if available
        let a1 = format!("{:?}", r);
        if let Some((r0, c0, r1, c1)) = parse_a1_range(&a1) {
            out.push(Merge { r0, c0, r1, c1 });
            continue;
        }

        // 2) Fallback: parse the Debug struct you showed
        let dbg = format!("{:?}", r);
        if let Some((r0, c0, r1, c1)) = parse_umya_debug_range(&dbg) {
            out.push(Merge { r0, c0, r1, c1 });
            continue;
        }

        // If both parsing strategies fail, log and skip.
        eprintln!("WARN: couldn't parse merge range: {}", dbg);
    }

    Ok(out)
}

/// Parse umya-spreadsheet Range debug output like:
/// `Range { start_col: Some(ColumnReference { num: 23, ... }),
///          start_row: Some(RowReference { num: 185, ... }),
///          end_col:   Some(ColumnReference { num: 24, ... }),
///          end_row:   Some(RowReference { num: 185, ... }) }`
/// Returns zero-based (r0,c0,r1,c1).
#[cfg(feature = "execute")]
fn parse_umya_debug_range(s: &str) -> Option<(usize, usize, usize, usize)> {
    // Grab the first four `num: <int>` in order: start_col, start_row, end_col, end_row.
    let re = Regex::new(r"num:\s*(\d+)").ok()?;
    let nums: Vec<usize> = re
        .captures_iter(s)
        .filter_map(|cap| cap.get(1)?.as_str().parse::<usize>().ok())
        .collect();

    if nums.len() < 4 {
        return None;
    }

    let (start_col_1b, start_row_1b, end_col_1b, end_row_1b) = (nums[0], nums[1], nums[2], nums[3]);

    // Convert 1-based (umya) -> 0-based (your grid)
    let c0 = start_col_1b.saturating_sub(1);
    let r0 = start_row_1b.saturating_sub(1);
    let c1 = end_col_1b.saturating_sub(1);
    let r1 = end_row_1b.saturating_sub(1);

    Some((
        std::cmp::min(r0, r1),
        std::cmp::min(c0, c1),
        std::cmp::max(r0, r1),
        std::cmp::max(c0, c1),
    ))
}

#[cfg(feature = "execute")]
fn col_letters_to_idx(s: &str) -> Option<usize> {
    let mut val: usize = 0;
    for ch in s.chars() {
        if !ch.is_ascii_alphabetic() {
            return None;
        }
        val = val * 26 + ((ch.to_ascii_uppercase() as u8 - b'A') as usize + 1);
    }
    Some(val - 1)
}

#[cfg(feature = "execute")]
fn parse_a1_cell(a1: &str) -> Option<(usize, usize)> {
    // e.g., "BC23" -> (22, 54) zero-based
    let mut letters = String::new();
    let mut digits = String::new();
    for ch in a1.chars() {
        if ch.is_ascii_alphabetic() {
            if !digits.is_empty() {
                return None;
            }
            letters.push(ch);
        } else if ch.is_ascii_digit() {
            digits.push(ch);
        } else {
            return None;
        }
    }
    let c = col_letters_to_idx(&letters)?;
    let r: usize = digits.parse().ok()?;
    Some((r - 1, c))
}

#[cfg(feature = "execute")]
fn parse_a1_range(r: &str) -> Option<(usize, usize, usize, usize)> {
    let parts: Vec<&str> = r.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let (r0, c0) = parse_a1_cell(parts[0])?;
    let (r1, c1) = parse_a1_cell(parts[1])?;
    Some((min(r0, r1), min(c0, c1), max(r0, r1), max(c0, c1)))
}

#[cfg(feature = "execute")]
fn apply_merges(mut grid: Vec<Vec<String>>, merges: &[Merge]) -> Vec<Vec<String>> {
    let height = grid.len();
    if height == 0 {
        return grid;
    }
    let width = grid[0].len();
    if width == 0 {
        return grid;
    }

    for m in merges {
        // Clamp to grid
        let r0 = m.r0.min(height - 1);
        let c0 = m.c0.min(width - 1);
        let r1 = m.r1.min(height - 1);
        let c1 = m.c1.min(width - 1);
        if r0 >= height || c0 >= width {
            continue;
        }

        let base = grid
            .get(r0)
            .and_then(|row| row.get(c0))
            .cloned()
            .unwrap_or_default();
        if base.is_empty() {
            continue;
        } // nothing to propagate
        for r in r0..=r1 {
            for c in c0..=c1 {
                grid[r][c] = base.clone();
            }
        }
    }
    grid
}

// ============================ Segmentation ============================

#[cfg(feature = "execute")]
fn segment_rectangles(
    grid: &Vec<Vec<String>>,
    height: usize,
    width: usize,
    cfg: &ExtractConfig,
) -> Vec<Rect> {
    if height == 0 || width == 0 {
        return vec![];
    }

    let occ = build_occupancy(grid);

    let row_nonempty: Vec<usize> = occ
        .par_iter()
        .map(|row| row.iter().filter(|&&b| b).count())
        .collect();

    let col_nonempty: Vec<usize> = (0..width)
        .into_par_iter()
        .map(|c| (0..height).filter(|&r| occ[r][c]).count())
        .collect();

    let row_cuts = find_cuts(
        &row_nonempty,
        width,
        cfg.empty_density_threshold,
        cfg.gap_break_rows,
    );
    let col_cuts = find_cuts(
        &col_nonempty,
        height,
        cfg.empty_density_threshold,
        cfg.gap_break_cols,
    );

    // Build rectangles as cartesian of row segments × col segments
    let mut rects = Vec::new();
    for (r0, r1) in row_cuts {
        for (c0, c1) in &col_cuts {
            let rect = Rect {
                r0,
                c0: *c0,
                r1,
                c1: *c1,
            };
            // skip rectangles that are effectively empty
            if count_nonempty_in_rect(grid, &rect) > 0 {
                rects.push(rect);
            }
        }
    }
    rects
}

#[cfg(feature = "execute")]
fn find_cuts(
    counts: &Vec<usize>,
    denom: usize,
    thresh: f32,
    gap_break: usize,
) -> Vec<(usize, usize)> {
    // segments are [start,end] inclusive
    let mut out = Vec::new();
    let mut i = 0;
    while i < counts.len() {
        // skip empties to next non-empty
        while i < counts.len() && density(counts[i], denom) <= thresh {
            i += 1;
        }
        if i >= counts.len() {
            break;
        }
        let start = i;
        // grow until we get >=gap_break consecutive empties
        let mut consec_empty = 0;
        let mut end = i;
        while i < counts.len() {
            if density(counts[i], denom) <= thresh {
                consec_empty += 1;
                if consec_empty >= gap_break {
                    break;
                }
            } else {
                consec_empty = 0;
                end = i;
            }
            i += 1;
        }
        out.push((start, end));
    }
    if out.is_empty() {
        // single full segment if everything sparse
        out.push((0, counts.len().saturating_sub(1)));
    }
    out
}

#[cfg(feature = "execute")]
#[inline]
fn density(nz: usize, total: usize) -> f32 {
    if total == 0 {
        0.0
    } else {
        (nz as f32) / (total as f32)
    }
}

#[cfg(feature = "execute")]
fn count_nonempty_in_rect(grid: &[Vec<String>], rect: &Rect) -> usize {
    let height = grid.len();
    if height == 0 {
        return 0;
    }
    let width = grid.first().map(|r| r.len()).unwrap_or(0);
    if width == 0 {
        return 0;
    }

    let r0 = rect.r0.min(height.saturating_sub(1));
    let r1 = rect.r1.min(height.saturating_sub(1));
    let c0 = rect.c0.min(width.saturating_sub(1));
    let c1 = rect.c1.min(width.saturating_sub(1));

    let mut n = 0;
    for r in r0..=r1 {
        if let Some(row) = grid.get(r) {
            for c in c0..=c1 {
                if let Some(cell) = row.get(c)
                    && !cell.trim().is_empty()
                {
                    n += 1;
                }
            }
        }
    }
    n
}

// ============================ Table Build ============================

#[cfg(feature = "execute")]
fn build_table_from_rect(
    grid: &mut Vec<Vec<String>>,
    rect: &Rect,
    cfg: &ExtractConfig,
    merge_map: &MergeMap,
) -> Table {
    // detect header band (1..=max_header_rows), skipping merged banner rows
    let header_rows = detect_header_rows(grid, rect, cfg, merge_map);
    // detect left header (0..=max_left_header_cols)
    let left_cols = detect_left_header_cols(grid, rect, &header_rows, cfg);

    // flatten headers
    let mut headers = flatten_headers(grid, rect, &header_rows, left_cols, cfg, merge_map);

    // data rows start after header band
    let mut data_r0 = rect.r0 + header_rows.len();
    let mut rows: Vec<Vec<String>> = Vec::new();

    // regex to drop totals
    let totals_re = &*TOTALS_RE;

    // Unit row: if directly under header looks like unit tokens, merge into header
    if data_r0 <= rect.r1
        && let Some(unit_row) = detect_unit_row(grid, rect, data_r0, &headers)
    {
        merge_unit_into_headers(&mut headers, &unit_row);
        data_r0 += 1; // skip the unit row in data
    }

    let mut max_width = headers.len();

    let mut consec_blank_rows = 0usize;
    let blank_allowed = cfg.allow_internal_blank_rows;

    for r in data_r0..=rect.r1 {
        let mut row = Vec::new();
        for c in rect.c0..=rect.c1 {
            let raw = if cfg.take_cells_on_extract {
                std::mem::take(&mut grid[r][c])
            } else {
                grid[r][c].clone()
            };
            let mut v = raw;
            if is_horiz_merged_non_anchor(merge_map, r, c) {
                v.clear();
            }
            row.push(v);
        }
        // pad/clip to left_cols + data columns
        row = project_row_for_headers(row, rect, left_cols, headers.len());

        let nonempty = row.iter().any(|s| !s.trim().is_empty());
        if !nonempty {
            consec_blank_rows += 1;
            if consec_blank_rows <= blank_allowed {
                continue;
            } else {
                break;
            } // assume table ended
        } else {
            consec_blank_rows = 0;
        }

        // drop repeated header rows inside body
        if row_eq_headers(&row, &headers) {
            continue;
        }

        // optionally drop totals rows
        if cfg.drop_totals
            && let Some(first) = row.first()
            && totals_re.is_match(first)
        {
            continue;
        }

        max_width = max(max_width, row.len());
        rows.push(row);
    }

    // normalize row widths
    for r in &mut rows {
        if r.len() < max_width {
            r.resize(max_width, String::new());
        }
    }

    let mut table = Table { headers, rows };

    // Optional totals column drop
    if cfg.drop_totals {
        drop_totals_column(&mut table);
    }

    // Drop truly identical columns (header modulo " (n)" and identical cell values)
    dedup_identical_columns(&mut table);

    table
}

// ============================ Units & Totals helpers ============================

#[cfg(feature = "execute")]
fn detect_unit_row(
    grid: &Vec<Vec<String>>,
    rect: &Rect,
    r: usize,
    headers: &Vec<String>,
) -> Option<Vec<String>> {
    if r < rect.r0 || r > rect.r1 {
        return None;
    }
    let mut vals: Vec<String> = Vec::new();
    let mut tokens = 0usize;
    let mut nonempty = 0usize;
    for (i, c) in (rect.c0..=rect.c1).enumerate() {
        if i >= headers.len() {
            break;
        }
        let s = grid[r][c].trim();
        if s.is_empty() {
            vals.push(String::new());
            continue;
        }
        nonempty += 1;
        let is_short = s.chars().count() <= 5;
        let has_space = s.contains(char::is_whitespace);
        let symbolic_units = [
            "%", "‰", "€", "$", "¥", "£", "kg", "g", "t", "h", "m", "s", "km", "cm", "mm", "pcs",
            "stk", "l", "ml",
        ];
        let is_symbolic = symbolic_units.iter().any(|&u| s.eq_ignore_ascii_case(u));
        if (is_short && !has_space) || is_symbolic {
            tokens += 1;
        }
        vals.push(s.to_string());
    }
    if nonempty > 0 && tokens * 2 >= nonempty {
        Some(vals)
    } else {
        None
    }
}

#[cfg(feature = "execute")]
fn merge_unit_into_headers(headers: &mut Vec<String>, unit_row: &Vec<String>) {
    for (h, u) in headers.iter_mut().zip(unit_row) {
        if u.trim().is_empty() {
            continue;
        }
        if h.trim().is_empty() {
            *h = u.clone();
        } else {
            *h = format!("{} [{}]", h, u);
        }
    }
}

#[cfg(feature = "execute")]
fn drop_totals_column(t: &mut Table) {
    if t.headers.is_empty() {
        return;
    }
    let re = &*TOTALS_RE;
    let last = t.headers.len() - 1;
    if re.is_match(t.headers[last].as_str()) {
        t.headers.pop();
        for row in &mut t.rows {
            if row.len() > last {
                row.pop();
            }
        }
    }
}

#[cfg(feature = "execute")]
fn is_banner_row(grid: &Vec<Vec<String>>, rect: &Rect, r: usize, merge_map: &MergeMap) -> bool {
    if r < rect.r0 || r > rect.r1 {
        return false;
    }
    let width = rect.c1.saturating_sub(rect.c0) + 1;
    if width < 3 {
        return false;
    }
    let min_span = std::cmp::max(2, width / 2); // span at least half the rect
    // If there's no merge map (merges disabled or skipped), skip merged banner logic safely
    if !merge_map.is_empty() {
        // Defensive bounds check: merge_map may be smaller than grid if constructed with clamping.
        if r < merge_map.len() {
            // Check for a merged anchor at this row that spans wide columns and has text
            for c in rect.c0..=rect.c1 {
                if c >= merge_map[r].len() {
                    break;
                }
                if let Some(m) = merge_map[r][c]
                    && m.r0 == r
                    && m.c0 == c
                {
                    let span = m.c1.saturating_sub(m.c0) + 1;
                    if span >= min_span {
                        let s = grid[r][c].trim();
                        if !s.is_empty() {
                            return true;
                        }
                    }
                }
            }
        }
    }

    // Fallback: entire row has exactly one non-empty cell and others empty
    let mut nonempty_count = 0usize;
    for c in rect.c0..=rect.c1 {
        if !grid[r][c].trim().is_empty() {
            nonempty_count += 1;
        }
    }
    nonempty_count == 1
}

#[cfg(feature = "execute")]
fn row_type_stats(row: &Vec<String>, c0: usize, c1: usize) -> (f32, f32, usize) {
    let mut alpha = 0usize;
    let mut numeric = 0usize;
    let mut nonempty = 0usize;
    for c in c0..=c1 {
        let s = row[c].trim();
        if s.is_empty() {
            continue;
        }
        nonempty += 1;
        let has_digit = s.chars().any(|ch| ch.is_ascii_digit());
        let has_alpha = s.chars().any(|ch| ch.is_ascii_alphabetic());
        if has_alpha {
            alpha += 1;
        }
        if has_digit && !has_alpha {
            numeric += 1;
        }
    }
    let w = max(1, (c1 + 1).saturating_sub(c0));
    (alpha as f32 / w as f32, numeric as f32 / w as f32, nonempty)
}

#[cfg(feature = "execute")]
fn detect_left_header_cols(
    grid: &Vec<Vec<String>>,
    rect: &Rect,
    header_rows: &Vec<usize>,
    cfg: &ExtractConfig,
) -> usize {
    let mut count = 0usize;
    'cols: for c in rect.c0..=rect.c1 {
        if count >= cfg.max_left_header_cols {
            break;
        }
        let mut textish = 0usize;
        let mut nonempty = 0usize;
        let mut repeats = 0usize;
        let mut prev: Option<String> = None;
        for r in (rect.r0 + header_rows.len())..=rect.r1 {
            let s = grid[r][c].trim();
            if s.is_empty() {
                continue;
            }
            nonempty += 1;
            if s.chars().any(|ch| ch.is_ascii_alphabetic()) {
                textish += 1;
            }
            if let Some(p) = &prev
                && p == s
            {
                repeats += 1;
            }
            prev = Some(s.to_string());
        }
        // Heuristic: mostly text labels and some repeats (categories)
        if nonempty > 0 && textish * 2 >= nonempty && repeats >= nonempty / 4 {
            count += 1;
        } else {
            break 'cols;
        }
    }
    count
}

#[cfg(feature = "execute")]
fn flatten_headers(
    grid: &mut Vec<Vec<String>>,
    rect: &Rect,
    header_rows: &Vec<usize>,
    left_cols: usize,
    cfg: &ExtractConfig,
    merge_map: &MergeMap,
) -> Vec<String> {
    let mut headers = Vec::new();
    for c in rect.c0..=rect.c1 {
        let mut parts = Vec::new();
        for off in header_rows {
            let r = rect.r0 + *off;
            let mut s = if cfg.take_cells_on_extract {
                std::mem::take(&mut grid[r][c])
            } else {
                grid[r][c].clone()
            };

            // Suppress horizontally-merged non-anchor duplicates across header rows
            if is_horiz_merged_non_anchor(merge_map, r, c) {
                s.clear();
            }

            if s.contains('\n') {
                let cleaned = s.replace('\r', "");
                let mut buf = String::new();
                for (i, part) in cleaned
                    .split('\n')
                    .map(str::trim)
                    .filter(|x| !x.is_empty())
                    .enumerate()
                {
                    if i > 0 {
                        buf.push_str(&cfg.header_joiner);
                    }
                    buf.push_str(part);
                }
                s = buf;
            }
            if !s.trim().is_empty() {
                parts.push(s.trim().to_string());
            }
        }
        let name = if parts.is_empty() {
            String::new()
        } else {
            parts.join(&cfg.header_joiner)
        };
        headers.push(name);
    }

    // If *all* headers are empty, keep full rect width (do NOT truncate).
    if headers.iter().any(|h| !h.trim().is_empty()) {
        // Otherwise trim only trailing *all-empty* columns.
        let mut last_nonempty = headers.len().saturating_sub(1);
        while last_nonempty > 0 && headers[last_nonempty].trim().is_empty() {
            last_nonempty -= 1;
        }
        headers.truncate(last_nonempty + 1);
    }

    // Ensure left header columns get reasonable names if empty
    for i in 0..left_cols.min(headers.len()) {
        if headers[i].trim().is_empty() {
            headers[i] = format!("RowHeader{}", i + 1);
        }
    }

    // Dedup duplicate header names
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for h in &mut headers {
        let base = h.trim();
        if base.is_empty() {
            *h = "Column".to_string();
            continue;
        }
        let n = seen.entry(base.to_string()).or_insert(0);
        if *n > 0 {
            *h = format!("{} ({})", base, *n + 1);
        } else {
            *h = base.to_string();
        }
        *n += 1;
    }
    headers
}

#[cfg(feature = "execute")]
fn project_row_for_headers(
    row: Vec<String>,
    _rect: &Rect,
    _left_cols: usize,
    header_len: usize,
) -> Vec<String> {
    // Row already represents rect.c0..=rect.c1 in order, so slice from 0
    let end = min(header_len, row.len());
    let mut out = row[0..end].to_vec();
    // Ensure left_cols exist (already in range). Pad if necessary.
    if out.len() < header_len {
        out.resize(header_len, String::new());
    }
    out
}

#[cfg(feature = "execute")]
fn row_eq_headers(row: &Vec<String>, headers: &Vec<String>) -> bool {
    if row.len() != headers.len() {
        return false;
    }
    for (a, b) in row.iter().zip(headers) {
        if normalize(a) != normalize(b) {
            return false;
        }
    }
    true
}

#[cfg(feature = "execute")]
fn detect_header_rows(
    grid: &Vec<Vec<String>>,
    rect: &Rect,
    cfg: &ExtractConfig,
    merge_map: &MergeMap,
) -> Vec<usize> {
    // First run legacy/banner logic to determine a starting row
    let mut banner_skip = 0usize;
    for _ in 0..2 {
        let r = rect.r0 + banner_skip;
        if r <= rect.r1 && is_banner_row(grid, rect, r, merge_map) {
            banner_skip += 1;
        } else {
            break;
        }
    }

    // Evaluate candidate header depths h=1..=max_header_rows using a schema-aware score.
    let max_h = cfg
        .max_header_rows
        .min(rect.r1.saturating_sub(rect.r0 + banner_skip) + 1);
    if max_h == 0 {
        return vec![];
    }

    let mut best_h = 1usize;
    let mut best_score = -1.0f32;

    for h in 1..=max_h {
        let start = rect.r0 + banner_skip;
        let data_start = start + h;
        if data_start > rect.r1 {
            break;
        }

        let score = score_schema_fit(grid, rect, data_start, cfg);
        // Light header-ish check on header band itself to avoid picking h=0-like garbage
        let mut headerish = 0.0f32;
        for rr in start..data_start {
            let (alpha, numeric, ne) = row_type_stats(&grid[rr], rect.c0, rect.c1);
            if ne == 0 {
                continue;
            }
            headerish += (alpha - numeric).max(0.0);
        }
        let h_norm = headerish / (h as f32).max(1.0);
        let total = score + 0.15 * h_norm; // prefer header-ish text rows slightly

        if total > best_score {
            best_score = total;
            best_h = h;
        }
    }

    // If schema confidence is too low, fall back to legacy heuristic selection
    if best_score < 0.05 {
        // legacy: keep previous method but starting at banner_skip
        let mut out = Vec::new();
        let max = cfg
            .max_header_rows
            .min(rect.r1.saturating_sub(rect.r0 + banner_skip) + 1);
        for i in 0..max {
            let r = rect.r0 + banner_skip + i;
            let (alpha, numeric, nonempty) = row_type_stats(&grid[r], rect.c0, rect.c1);
            if nonempty == 0 {
                break;
            }
            let looks_header = alpha >= 0.60 && numeric <= 0.30;
            if !looks_header {
                break;
            }
            out.push(r - rect.r0);
        }
        if out.is_empty() {
            out.push(banner_skip);
        }
        return out;
    }

    (0..best_h).map(|i| banner_skip + i).collect()
}

#[cfg(feature = "execute")]
fn score_schema_fit(
    grid: &Vec<Vec<String>>,
    rect: &Rect,
    data_r0: usize,
    cfg: &ExtractConfig,
) -> f32 {
    if data_r0 > rect.r1 {
        return 0.0;
    }
    let c0 = rect.c0;
    let c1 = rect.c1;
    let sample_end = (data_r0 + cfg.schema_sample_rows - 1).min(rect.r1);
    let mut col_kind_counts: Vec<[usize; 5]> = vec![[0; 5]; c1 - c0 + 1]; // Bool, DateTime, Numeric, Text, Empty

    let mut rows_seen = 0usize;
    for r in data_r0..=sample_end {
        rows_seen += 1;
        for (i, c) in (c0..=c1).enumerate() {
            let k = detect_kind_idx(grid[r][c].as_str());
            col_kind_counts[i][k] += 1;
        }
    }
    if rows_seen == 0 {
        return 0.0;
    }

    // Compute consistency per column as 1 - entropy proxy (majority / non-empty)
    let mut per_col = Vec::with_capacity(col_kind_counts.len());
    for counts in &col_kind_counts {
        let nonempty = counts[0] + counts[1] + counts[2] + counts[3]; // exclude Empty
        if nonempty == 0 {
            per_col.push(0.0);
            continue;
        }
        let majority = counts[0].max(counts[1]).max(counts[2]).max(counts[3]);
        per_col.push(majority as f32 / nonempty as f32);
    }

    // Aggregate: fraction of columns meeting consistency threshold and their mean consistency
    let mut ok_cols = 0usize;
    let mut sum_ok = 0.0f32;
    let mut considered = 0usize;
    for &v in &per_col {
        if v <= 0.0 {
            continue;
        }
        considered += 1;
        if v >= cfg.min_body_consistency {
            ok_cols += 1;
            sum_ok += v;
        }
    }
    if considered == 0 {
        return 0.0;
    }
    let frac_ok = ok_cols as f32 / considered as f32;
    let avg_ok = if ok_cols == 0 {
        0.0
    } else {
        sum_ok / ok_cols as f32
    };

    0.7 * frac_ok + 0.3 * avg_ok
}

#[cfg(feature = "execute")]
#[inline]
fn detect_kind_idx(s: &str) -> usize {
    match detect_kind(s) {
        Kind::Bool => 0,
        Kind::DateTime => 1,
        Kind::Numeric => 2,
        Kind::Text => 3,
        Kind::Empty | Kind::Mixed => 4,
    }
}
// ============================ Connectivity split ============================

#[cfg(feature = "execute")]
fn is_nonempty(s: &str) -> bool {
    !s.trim().is_empty()
}

#[cfg(feature = "execute")]
fn split_rect_by_connectivity(
    grid: &Vec<Vec<String>>,
    rect: &Rect,
    cfg: &ExtractConfig,
) -> Vec<Rect> {
    let mut visited: Vec<Vec<bool>> =
        vec![vec![false; rect.c1 - rect.c0 + 1]; rect.r1 - rect.r0 + 1];
    let mut parts: Vec<Rect> = Vec::new();

    for r in rect.r0..=rect.r1 {
        for c in rect.c0..=rect.c1 {
            if !is_nonempty(&grid[r][c]) {
                continue;
            }
            let vr = r - rect.r0;
            let vc = c - rect.c0;
            if visited[vr][vc] {
                continue;
            }

            let mut q: VecDeque<(usize, usize)> = VecDeque::new();
            q.push_back((r, c));
            visited[vr][vc] = true;

            let mut comp_r0 = r;
            let mut comp_c0 = c;
            let mut comp_r1 = r;
            let mut comp_c1 = c;

            while let Some((cr, cc)) = q.pop_front() {
                comp_r0 = std::cmp::min(comp_r0, cr);
                comp_c0 = std::cmp::min(comp_c0, cc);
                comp_r1 = std::cmp::max(comp_r1, cr);
                comp_c1 = std::cmp::max(comp_c1, cc);

                // Explore immediate neighbors (4-neighborhood)
                let neigh = [
                    (cr.wrapping_sub(1), cc, cr > rect.r0),
                    (cr + 1, cc, cr < rect.r1),
                    (cr, cc.wrapping_sub(1), cc > rect.c0),
                    (cr, cc + 1, cc < rect.c1),
                ];
                for &(nr, nc, ok) in &neigh {
                    if !ok {
                        continue;
                    }
                    if is_nonempty(&grid[nr][nc]) {
                        let vr = nr - rect.r0;
                        let vc = nc - rect.c0;
                        if !visited[vr][vc] {
                            visited[vr][vc] = true;
                            q.push_back((nr, nc));
                        }
                    }
                }

                // Horizontal bridging right
                let mut gaps = 0usize;
                let mut x = cc + 1;
                while x <= rect.c1 && gaps < cfg.allow_internal_blank_cols {
                    if is_nonempty(&grid[cr][x]) {
                        break;
                    }
                    gaps += 1;
                    x += 1;
                }
                if x <= rect.c1
                    && gaps > 0
                    && gaps <= cfg.allow_internal_blank_cols
                    && is_nonempty(&grid[cr][x])
                {
                    let vr = cr - rect.r0;
                    let vc = x - rect.c0;
                    if !visited[vr][vc] {
                        visited[vr][vc] = true;
                        q.push_back((cr, x));
                    }
                }

                // Horizontal bridging left
                gaps = 0;
                let mut x2 = cc;
                while x2 > rect.c0 {
                    let next = x2 - 1;
                    if is_nonempty(&grid[cr][next]) {
                        break;
                    }
                    gaps += 1;
                    x2 = next;
                    if gaps >= cfg.allow_internal_blank_cols {
                        break;
                    }
                }
                if x2 > rect.c0.saturating_sub(1) && gaps > 0 {
                    let target = x2 - 1;
                    if target >= rect.c0 && is_nonempty(&grid[cr][target]) {
                        let vr = cr - rect.r0;
                        let vc = target - rect.c0;
                        if !visited[vr][vc] {
                            visited[vr][vc] = true;
                            q.push_back((cr, target));
                        }
                    }
                }

                // Vertical bridging down
                gaps = 0;
                let mut y = cr + 1;
                while y <= rect.r1 && gaps < cfg.allow_internal_blank_rows {
                    if is_nonempty(&grid[y][cc]) {
                        break;
                    }
                    gaps += 1;
                    y += 1;
                }
                if y <= rect.r1
                    && gaps > 0
                    && gaps <= cfg.allow_internal_blank_rows
                    && is_nonempty(&grid[y][cc])
                {
                    let vr = y - rect.r0;
                    let vc = cc - rect.c0;
                    if !visited[vr][vc] {
                        visited[vr][vc] = true;
                        q.push_back((y, cc));
                    }
                }

                // Vertical bridging up
                gaps = 0;
                let mut y2 = cr;
                while y2 > rect.r0 {
                    let next = y2 - 1;
                    if is_nonempty(&grid[next][cc]) {
                        break;
                    }
                    gaps += 1;
                    y2 = next;
                    if gaps >= cfg.allow_internal_blank_rows {
                        break;
                    }
                }
                if y2 > rect.r0.saturating_sub(1) && gaps > 0 {
                    let target = y2 - 1;
                    if target >= rect.r0 && is_nonempty(&grid[target][cc]) {
                        let vr = target - rect.r0;
                        let vc = cc - rect.c0;
                        if !visited[vr][vc] {
                            visited[vr][vc] = true;
                            q.push_back((target, cc));
                        }
                    }
                }
            }

            parts.push(Rect {
                r0: comp_r0,
                c0: comp_c0,
                r1: comp_r1,
                c1: comp_c1,
            });
        }
    }

    // Small optimization: if no split happened, return the input rect.
    if parts.len() <= 1 {
        return vec![Rect {
            r0: rect.r0,
            c0: rect.c0,
            r1: rect.r1,
            c1: rect.c1,
        }];
    }
    parts
}

// ============================ CSV render ============================

#[cfg(feature = "execute")]
#[derive(Clone, Debug)]
struct TableWithRect {
    rect: Rect,
    table: Table,
}

#[cfg(feature = "execute")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Empty,
    Bool,
    DateTime,
    Numeric,
    Text,
    Mixed,
}

#[cfg(feature = "execute")]
fn detect_kind(s: &str) -> Kind {
    let t = s.trim();
    if t.is_empty() {
        return Kind::Empty;
    }
    let tl = t.to_ascii_lowercase();
    if tl == "true" || tl == "false" {
        return Kind::Bool;
    }
    // very light ISO checks: YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS
    if (t.len() >= 10 && t.chars().nth(4) == Some('-') && t.chars().nth(7) == Some('-'))
        || (t.len() >= 19 && t.chars().nth(10) == Some('T'))
    {
        return Kind::DateTime;
    }
    if t.parse::<i64>().is_ok() || t.parse::<f64>().is_ok() {
        return Kind::Numeric;
    }
    Kind::Text
}

#[cfg(feature = "execute")]
fn infer_col_kinds(table: &Table, sample: usize) -> Vec<Kind> {
    let w = table.headers.len();
    let mut kinds = vec![Kind::Empty; w];
    for c in 0..w {
        let mut seen = std::collections::HashSet::<Kind>::new();
        for r in 0..table.rows.len().min(sample) {
            let k = detect_kind(table.rows[r].get(c).map(|s| s.as_str()).unwrap_or(""));
            if k != Kind::Empty {
                seen.insert(match k {
                    Kind::Numeric => Kind::Numeric,
                    _ => k,
                });
            }
            if seen.len() > 1 {
                // early mixed
                kinds[c] = Kind::Mixed;
                break;
            }
        }
        if kinds[c] != Kind::Mixed {
            kinds[c] = seen.iter().next().copied().unwrap_or(Kind::Empty);
        }
    }
    kinds
}

#[cfg(feature = "execute")]
fn kinds_match_ratio(a: &[Kind], b: &[Kind]) -> f32 {
    let w = a.len().min(b.len());
    if w == 0 {
        return 1.0;
    }
    let mut considered = 0usize;
    let mut ok = 0usize;
    for i in 0..w {
        let (ka, kb) = (a[i], b[i]);
        // Empty is neutral, ignore unless both are empty
        if ka == Kind::Empty && kb == Kind::Empty {
            continue;
        }
        considered += 1;
        let compat = ka == kb
            || (ka == Kind::Numeric && kb == Kind::Numeric)
            || ka == Kind::Empty
            || kb == Kind::Empty;
        if compat {
            ok += 1;
        }
    }
    if considered == 0 {
        1.0
    } else {
        ok as f32 / considered as f32
    }
}

#[cfg(feature = "execute")]
fn col_overlap_ratio(a: &Rect, b: &Rect) -> f32 {
    let inter_c0 = std::cmp::max(a.c0, b.c0);
    let inter_c1 = std::cmp::min(a.c1, b.c1);
    let inter = if inter_c1 >= inter_c0 {
        inter_c1 - inter_c0 + 1
    } else {
        0
    };
    let uni_c0 = std::cmp::min(a.c0, b.c0);
    let uni_c1 = std::cmp::max(a.c1, b.c1);
    let uni = uni_c1 - uni_c0 + 1;
    if uni == 0 {
        0.0
    } else {
        inter as f32 / uni as f32
    }
}

#[cfg(feature = "execute")]
fn gap_has_headerish(grid: &Vec<Vec<String>>, a: &Rect, b: &Rect) -> bool {
    if b.r0 <= a.r1 + 1 {
        return false;
    }
    let c0 = std::cmp::max(a.c0, b.c0);
    let c1 = std::cmp::min(a.c1, b.c1);
    if c1 < c0 {
        return false;
    }
    for r in (a.r1 + 1)..b.r0 {
        let (alpha, numeric, nonempty) = row_type_stats(&grid[r], c0, c1);
        let looks_header = (alpha >= 0.5 && numeric <= 0.5) || (nonempty > 0 && alpha >= 0.3);
        if looks_header {
            return true;
        }
    }
    false
}

#[cfg(feature = "execute")]
fn merge_tables(mut a: Table, mut b: Table) -> Table {
    let target_len = a.headers.len().max(b.headers.len());

    if a.headers.len() < target_len {
        a.headers.resize(target_len, String::new());
    }
    if b.headers.len() < target_len {
        b.headers.resize(target_len, String::new());
    }

    for r in &mut a.rows {
        if r.len() < target_len {
            r.resize(target_len, String::new());
        }
    }
    for r in &mut b.rows {
        if r.len() < target_len {
            r.resize(target_len, String::new());
        }
    }

    // Prefer non-empty header set; otherwise keep `a`'s.
    let a_has_names = a.headers.iter().any(|h| !h.trim().is_empty());
    let b_has_names = b.headers.iter().any(|h| !h.trim().is_empty());
    let headers = if a_has_names || !b_has_names {
        a.headers.clone()
    } else {
        b.headers.clone()
    };

    let mut rows = a.rows;
    rows.extend(b.rows);
    Table { headers, rows }
}

#[cfg(feature = "execute")]
fn can_stitch(
    grid: &Vec<Vec<String>>,
    prev: &TableWithRect,
    next: &TableWithRect,
    cfg: &ExtractConfig,
) -> bool {
    if col_overlap_ratio(&prev.rect, &next.rect) < cfg.stitch_min_col_overlap {
        return false;
    }
    if gap_has_headerish(grid, &prev.rect, &next.rect) {
        return false; // a real header showed up between them
    }

    // Either headers are similar, or types mostly match.
    let headers_ok = headers_similar(&prev.table.headers, &next.table.headers);

    let prev_k = infer_col_kinds(&prev.table, cfg.stitch_type_sample_rows);
    let next_k = infer_col_kinds(&next.table, cfg.stitch_type_sample_rows);
    let type_ratio = kinds_match_ratio(&prev_k, &next_k);

    headers_ok || type_ratio >= cfg.stitch_min_type_match_ratio
}

#[cfg(feature = "execute")]
fn stitch_tables(
    grid: &Vec<Vec<String>>,
    mut items: Vec<TableWithRect>,
    cfg: &ExtractConfig,
) -> Vec<TableWithRect> {
    if items.is_empty() {
        return items;
    }
    items.sort_by_key(|t| (t.rect.r0, t.rect.c0));

    let mut out: Vec<TableWithRect> = Vec::new();
    let mut cur = items.remove(0);

    for nxt in items.into_iter() {
        if cfg.stitch_across_spacers && can_stitch(grid, &cur, &nxt, cfg) {
            // merge
            cur.table = merge_tables(cur.table, nxt.table);
            cur.rect = Rect {
                r0: std::cmp::min(cur.rect.r0, nxt.rect.r0),
                c0: std::cmp::min(cur.rect.c0, nxt.rect.c0),
                r1: std::cmp::max(cur.rect.r1, nxt.rect.r1),
                c1: std::cmp::max(cur.rect.c1, nxt.rect.c1),
            };
        } else {
            out.push(cur);
            cur = nxt;
        }
    }
    out.push(cur);
    out
}

// ============================ (Optional) Schema stitching ============================
/// Example: if two rectangles are separated by 2 blank rows but headers are "the same",
/// you could merge them. Hook this in `segment_rectangles` if desired.
#[cfg(feature = "execute")]
fn headers_similar(a: &[String], b: &[String]) -> bool {
    let w = min(a.len(), b.len());
    if w == 0 {
        return false;
    }
    let mut score = 0.0;
    for i in 0..w {
        let sa = a[i].to_ascii_lowercase();
        let sb = b[i].to_ascii_lowercase();
        score += jaro_winkler(&sa, &sb);
    }
    (score / (w as f64)) >= 0.92
}

/// Compute average Jaro-Winkler similarity of aligned headers (by index) between 0.0 and 1.0.
#[cfg(feature = "execute")]
fn headers_similarity(a: &[String], b: &[String]) -> f64 {
    let w = min(a.len(), b.len());
    if w == 0 {
        return 0.0;
    }
    let mut score = 0.0;
    for i in 0..w {
        let sa = a[i].to_ascii_lowercase();
        let sb = b[i].to_ascii_lowercase();
        score += jaro_winkler(&sa, &sb);
    }
    score / (w as f64)
}

/// Merge tables with very similar headers (high threshold), preserving row order.
#[cfg(feature = "execute")]
fn group_tables_by_header_similarity(
    mut items: Vec<TableWithRect>,
    cfg: &ExtractConfig,
) -> Vec<TableWithRect> {
    if items.len() <= 1 {
        return items;
    }
    // Sort by top-most position to keep stable concatenation order
    items.sort_by_key(|t| (t.rect.r0, t.rect.c0));

    let mut used = vec![false; items.len()];
    let mut out: Vec<TableWithRect> = Vec::new();

    for i in 0..items.len() {
        if used[i] {
            continue;
        }
        used[i] = true;
        let mut acc = items[i].clone();
        // Collect very similar tables and merge into acc
        for j in (i + 1)..items.len() {
            if used[j] {
                continue;
            }
            // Quick skip if both headers are empty
            let a_has = acc.table.headers.iter().any(|h| !h.trim().is_empty());
            let b_has = items[j].table.headers.iter().any(|h| !h.trim().is_empty());
            if !a_has && !b_has {
                continue;
            }

            // Widths must be close to avoid accidental merges
            let wa = acc.table.headers.len();
            let wb = items[j].table.headers.len();
            let w_min = wa.min(wb) as f64;
            let w_max = wa.max(wb) as f64;
            if w_min == 0.0 {
                continue;
            }
            let width_ratio = w_min / w_max; // 1.0 == same width
            if width_ratio < 0.9 {
                continue;
            }

            let sim = headers_similarity(&acc.table.headers, &items[j].table.headers);
            if sim >= cfg.header_merge_threshold {
                // Merge and enlarge rect
                acc.table = merge_tables(acc.table, items[j].table.clone());
                acc.rect = Rect {
                    r0: min(acc.rect.r0, items[j].rect.r0),
                    c0: min(acc.rect.c0, items[j].rect.c0),
                    r1: max(acc.rect.r1, items[j].rect.r1),
                    c1: max(acc.rect.c1, items[j].rect.c1),
                };
                used[j] = true;
            }
        }
        out.push(acc);
    }
    out
}

// ============================ De-dup helpers ============================

/// Drop columns that are identical in data and whose headers only differ by an automatic " (n)" suffix.
#[cfg(feature = "execute")]
fn dedup_identical_columns(t: &mut Table) {
    let w = t.headers.len();
    if w <= 1 {
        return;
    }

    use rayon::prelude::*;

    let sigs: Vec<(usize, String)> = (0..w)
        .into_par_iter()
        .map(|c| {
            let mut sig = strip_header_dup_suffix(&t.headers[c]).to_ascii_lowercase();
            sig.push('\0');
            for r in 0..t.rows.len() {
                if let Some(val) = t.rows[r].get(c) {
                    sig.push_str(val);
                }
                sig.push('\x1f');
            }
            (c, sig)
        })
        .collect();

    let mut keep = vec![true; w];
    let mut seen = std::collections::HashMap::<String, usize>::new();

    for (c, sig) in sigs {
        if let std::collections::hash_map::Entry::Vacant(e) = seen.entry(sig) {
            e.insert(c);
        } else {
            keep[c] = false;
        }
    }

    if keep.iter().all(|&k| k) {
        return;
    }

    let mut new_headers = Vec::with_capacity(keep.iter().filter(|k| **k).count());
    for (c, h) in t.headers.iter().enumerate() {
        if keep[c] {
            new_headers.push(h.clone());
        }
    }
    let mut new_rows = Vec::with_capacity(t.rows.len());
    for row in &t.rows {
        let mut nr = Vec::with_capacity(new_headers.len());
        for (c, v) in row.iter().enumerate() {
            if keep[c] {
                nr.push(v.clone());
            }
        }
        new_rows.push(nr);
    }
    t.headers = new_headers;
    t.rows = new_rows;
}

#[cfg(feature = "execute")]
fn strip_header_dup_suffix(h: &str) -> String {
    let s = h.trim();
    if let Some(open) = s.rfind(" (")
        && s.ends_with(')')
    {
        let num = &s[(open + 2)..(s.len() - 1)];
        if !num.is_empty() && num.chars().all(|ch| ch.is_ascii_digit()) {
            return s[..open].to_string();
        }
    }
    s.to_string()
}

#[crate::register_node]
#[derive(Default)]
pub struct ExtractExcelTablesNode {}

impl ExtractExcelTablesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ExtractExcelTablesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_excel_extract_tables",
            "Extract Tables (Excel)",
            "Extracts tables from an Excel worksheet",
            "Data/Excel",
        );
        node.add_icon("/flow/icons/file-spreadsheet.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin("file", "File", "Excel file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "sheet",
            "Sheet",
            "Worksheet name (optional - if empty, extracts from all sheets)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "extract_config",
            "Extract Config",
            "Extract Config",
            VariableType::Struct,
        )
        .set_schema::<ExtractConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!(ExtractConfig::default())));

        node.add_output_pin("exec_out", "Output", "Next", VariableType::Execution);
        node.add_output_pin(
            "tables",
            "Tables",
            "Extracted Vec<Table>",
            VariableType::Struct,
        )
        .set_schema::<CSVTable>()
        .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let flow_path: FlowPath = context.evaluate_pin("file").await?;
        let sheet_input: String = context.evaluate_pin("sheet").await?;
        let extract_config: ExtractConfig = context.evaluate_pin("extract_config").await?;

        let file_buffer = flow_path.get(context, false).await?;
        let file_buffer_clone = file_buffer.clone();

        // Determine which sheets to process
        let sheets_to_process: Vec<String> = if sheet_input.trim().is_empty() {
            tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
                let cursor = Cursor::new(&file_buffer_clone);
                let wb = open_workbook_auto_from_rs(cursor)?;
                Ok(wb.sheet_names().to_vec())
            })
            .await??
        } else {
            vec![sheet_input]
        };

        let cfg_clone = extract_config.clone();
        let flow_path_clone = flow_path.clone();

        let csv_tables: Vec<CSVTable> =
            tokio::task::spawn_blocking(move || -> Result<Vec<CSVTable>> {
                let mut out = Vec::new();
                for sheet_name in sheets_to_process.iter() {
                    let tables = match extract_tables(file_buffer.clone(), sheet_name, &cfg_clone) {
                        Ok(t) => t,
                        Err(e) => {
                            eprintln!("WARN: Failed to extract from sheet '{}': {}", sheet_name, e);
                            continue;
                        }
                    };
                    for (table_idx, t) in tables.into_iter().enumerate() {
                        let headers = t.headers.clone();
                        let mut rows_json: Vec<Vec<flow_like_types::Value>> =
                            Vec::with_capacity(t.rows.len());
                        for r in t.rows {
                            rows_json.push(r.into_iter().map(|s| json!(s)).collect());
                        }
                        let mut csv_table =
                            CSVTable::new(headers, rows_json, Some(flow_path_clone.clone()));
                        csv_table.name = Some(format!("{}_{}", sheet_name, table_idx + 1));
                        out.push(csv_table);
                    }
                }
                Ok(out)
            })
            .await??;

        context.set_pin_value("tables", json!(csv_tables)).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Data processing requires the 'execute' feature"
        ))
    }
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;

    #[test]
    fn merges_out_of_bounds_are_clamped_in_map() {
        let height = 3usize;
        let width = 3usize;
        let merges = vec![Merge {
            r0: 1,
            c0: 1,
            r1: 10,
            c1: 10,
        }];
        let mm = build_merge_map(height, width, &merges);
        assert_eq!(mm.len(), height);
        assert_eq!(mm[0].len(), width);
        // Check that bottom-right cell is marked as merged
        assert!(mm[2][2].is_some());
        // And a cell outside the intended area remains None
        assert!(mm[0][0].is_none());
    }

    #[test]
    fn apply_merges_clamps_and_propagates_value() {
        let grid = vec![
            vec!["a".to_string(), "".to_string(), "".to_string()],
            vec!["".to_string(), "b".to_string(), "".to_string()],
            vec!["".to_string(), "".to_string(), "".to_string()],
        ];
        let merges = vec![Merge {
            r0: 1,
            c0: 1,
            r1: 10,
            c1: 10,
        }];
        let out = apply_merges(grid.clone(), &merges);
        // Value at (1,1) should be propagated to bottom-right within bounds
        assert_eq!(out[2][2], "b");
        // Other cells should remain unchanged outside the merged block
        assert_eq!(out[0][0], "a");
        assert_eq!(out[0][1], "");
    }

    #[test]
    fn large_file_optional() {
        let path = std::path::Path::new("../../tests/data/Crimes_-_2001_to_Present_20250906.xlsx");
        if !path.exists() {
            return;
        } // skip silently
        let mut cfg = ExtractConfig::default();
        cfg.enable_merges = false;
        cfg.max_merge_map_cells = 0;
        cfg.take_cells_on_extract = true;
        let bytes: Vec<u8> = std::fs::read(path).expect("Failed to read test file");
        let res = extract_tables(bytes, &"Data", &cfg);
        assert!(res.is_ok(), "large extraction failed: {:?}", res.err());
        let tables = res.unwrap();
        assert!(!tables.is_empty(), "no tables extracted");
        let first_table = &tables[0];
        assert!(
            !first_table.headers.is_empty(),
            "first table has no headers"
        );
        assert!(!first_table.rows.is_empty(), "first table has no rows");
    }

    #[test]
    fn banner_row_does_not_panic_without_merge_map() {
        // 3x5 grid with a centered text row that should NOT panic when merges disabled
        let grid = vec![
            vec!["".into(), "".into(), "Title".into(), "".into(), "".into()],
            vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
            vec!["1".into(), "2".into(), "3".into(), "4".into(), "5".into()],
        ];
        let rect = Rect {
            r0: 0,
            c0: 0,
            r1: 2,
            c1: 4,
        };
        let merge_map: MergeMap = Vec::new(); // simulate disabled merges
        // Should run without panic and fallback to single-nonempty detection => banner row true
        assert!(is_banner_row(&grid, &rect, 0, &merge_map));
    }
}

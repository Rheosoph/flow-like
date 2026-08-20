//! SpreadsheetLLM-inspired compact sheet encodings for LLM consumption.
//!
//! Three composable ideas from SheetCompressor (arXiv 2407.09025):
//! structural-anchor row selection (boundary rows verbatim, homogeneous runs
//! elided), inverted-index translation (value → ranges, empties vanish), and
//! type aggregation (homogeneous numeric runs become type tokens). Optional
//! style annotations render bold cells as `**value**` and colored fills as
//! `[green]value` so styling never needs a raw dump.

use super::grid::{CellValue, SheetGrid, col_to_letters, truncate_chars};
use super::styles::SheetStyles;
use super::table_detect::{Kind, Rect, cell_kind, column_dominant};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct EncodeOptions {
    /// Maximum rows rendered verbatim (anchor rows are prioritized)
    pub max_rows: usize,
    /// Maximum non-empty cells rendered per row
    pub max_cols: usize,
    /// Cell display truncation (chars)
    pub max_cell_chars: usize,
    pub include_merges: bool,
    pub include_column_profiles: bool,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            max_rows: 80,
            max_cols: 30,
            max_cell_chars: 40,
            include_merges: true,
            include_column_profiles: true,
        }
    }
}

pub(crate) fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Str => "text",
        Kind::Num => "number",
        Kind::Date => "date",
        Kind::Bool => "bool",
    }
}

fn row_signature(grid: &SheetGrid, r: usize, max_cols: usize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for c in 0..grid.width.min(max_cols) {
        cell_kind(grid.cell(r, c)).hash(&mut hasher);
    }
    hasher.finish()
}

fn annotate_style(
    display: String,
    styles: Option<&SheetStyles>,
    r_abs: usize,
    c_abs: usize,
) -> String {
    let Some(style) = styles.and_then(|s| s.get_abs(r_abs, c_abs)) else {
        return display;
    };
    let mut out = display;
    if style.bold {
        out = format!("**{out}**");
    }
    if let Some(name) = style.fill_color_name() {
        out = format!("[{name}]{out}");
    }
    out
}

/// Renders one grid row as `Row N: A5="x" | B5=12 | …` with absolute addresses.
pub fn render_row(
    grid: &SheetGrid,
    styles: Option<&SheetStyles>,
    r: usize,
    c0: usize,
    c1: usize,
    opts: &EncodeOptions,
) -> String {
    let abs_row = grid.start_row + r + 1;
    let mut parts: Vec<String> = Vec::new();
    let mut shown = 0usize;
    for c in c0..=c1.min(grid.width.saturating_sub(1)) {
        let v = grid.cell(r, c);
        if v.is_empty() {
            continue;
        }
        if shown >= opts.max_cols {
            parts.push(format!("(+{} more)", c1 + 1 - c));
            break;
        }
        let addr = format!("{}{}", col_to_letters(grid.start_col + c), abs_row);
        let display = truncate_chars(&v.display(), opts.max_cell_chars);
        let rendered = match v {
            CellValue::Text(_) => format!("{addr}=\"{display}\""),
            _ => format!("{addr}={display}"),
        };
        parts.push(annotate_style(
            rendered,
            styles,
            grid.start_row + r,
            grid.start_col + c,
        ));
        shown += 1;
    }
    if parts.is_empty() {
        format!("Row {abs_row}: (empty)")
    } else {
        format!("Row {abs_row}: {}", parts.join(" | "))
    }
}

/// Renders a rectangular range verbatim (for on-demand inspection tools),
/// capped at `max_rows` rows.
pub fn render_range(
    grid: &SheetGrid,
    styles: Option<&SheetStyles>,
    rect: &Rect,
    max_rows: usize,
    opts: &EncodeOptions,
) -> String {
    let mut out = String::new();
    let end = rect.r1.min(grid.height.saturating_sub(1));
    for (rendered, r) in (rect.r0..=end).enumerate() {
        if rendered >= max_rows {
            out.push_str(&format!(
                "[{} more rows in range omitted — inspect a smaller range for details]\n",
                end - r + 1
            ));
            break;
        }
        out.push_str(&render_row(grid, styles, r, rect.c0, rect.c1, opts));
        out.push('\n');
    }
    out
}

/// Compact, address-annotated encoding of a whole sheet: header facts, merged
/// regions, heuristic candidates, column profiles, style summary and the
/// contents of structurally interesting rows.
pub fn encode_sheet_compact(
    grid: &SheetGrid,
    candidates: &[Rect],
    styles: Option<&SheetStyles>,
    opts: &EncodeOptions,
    sheet_name: &str,
) -> String {
    const MAX_MERGES_LISTED: usize = 30;
    let mut out = String::with_capacity(8192);
    let used = grid.a1_range(
        0,
        0,
        grid.height.saturating_sub(1),
        grid.width.saturating_sub(1),
    );
    out.push_str(&format!(
        "Sheet '{}': used range {} ({} rows x {} cols), {} non-empty cells.\n",
        sheet_name,
        used,
        grid.height,
        grid.width,
        grid.count_filled()
    ));

    if opts.include_merges && !grid.merges.is_empty() {
        let listed: Vec<String> = grid
            .merges
            .iter()
            .take(MAX_MERGES_LISTED)
            .map(|m| grid.a1_range(m.r0, m.c0, m.r1, m.c1))
            .collect();
        let suffix = if grid.merges.len() > MAX_MERGES_LISTED {
            format!(" (+{} more)", grid.merges.len() - MAX_MERGES_LISTED)
        } else {
            String::new()
        };
        out.push_str(&format!(
            "Merged regions: {}{}\n",
            listed.join(", "),
            suffix
        ));
    }

    if let Some(s) = styles
        && !s.is_empty()
    {
        out.push_str(&format!("Styling: {}\n", s.summarize(8)));
    }

    if !candidates.is_empty() {
        out.push_str("Heuristic table candidates (verify, refine, split or merge these):\n");
        for (i, r) in candidates.iter().enumerate() {
            out.push_str(&format!(
                "  {}. {} ({} rows x {} cols)\n",
                i + 1,
                grid.a1_range(r.r0, r.c0, r.r1, r.c1),
                r.height(),
                r.width()
            ));
        }
    }

    if opts.include_column_profiles && grid.height > 1 {
        out.push_str("Column profiles (dominant type over all rows): ");
        let profiled: Vec<String> = (0..grid.width.min(opts.max_cols))
            .filter_map(|c| {
                column_dominant(grid, c, 0..grid.height).map(|(k, ratio)| {
                    format!(
                        "{}: {} {:.0}%",
                        col_to_letters(grid.start_col + c),
                        kind_label(k),
                        ratio * 100.0
                    )
                })
            })
            .collect();
        out.push_str(&profiled.join(" | "));
        out.push('\n');
    }

    // Structural anchors: sheet edges, candidate boundaries, type-signature changes
    let mut must_keep: BTreeSet<usize> = BTreeSet::new();
    for r in 0..grid.height.min(8) {
        must_keep.insert(r);
    }
    for r in grid.height.saturating_sub(2)..grid.height {
        must_keep.insert(r);
    }
    for rect in candidates {
        for r in rect.r0.saturating_sub(2)..=(rect.r0 + 5).min(grid.height - 1) {
            must_keep.insert(r);
        }
        for r in rect.r1.saturating_sub(1)..=(rect.r1 + 2).min(grid.height - 1) {
            must_keep.insert(r);
        }
    }
    let mut anchors: BTreeSet<usize> = BTreeSet::new();
    if grid.height > 1 {
        let mut prev_sig = row_signature(grid, 0, opts.max_cols);
        for r in 1..grid.height {
            let sig = row_signature(grid, r, opts.max_cols);
            if sig != prev_sig {
                anchors.insert(r.saturating_sub(1));
                anchors.insert(r);
            }
            prev_sig = sig;
        }
    }
    let budget = opts.max_rows.max(must_keep.len());
    for r in anchors {
        if must_keep.len() >= budget {
            break;
        }
        must_keep.insert(r);
    }

    out.push_str(&format!(
        "\nCell contents ({} of {} rows shown; empty cells omitted; row numbers are absolute; **bold** and [color] mark styled cells):\n",
        must_keep.len(),
        grid.height
    ));
    let mut prev: Option<usize> = None;
    for &r in &must_keep {
        if let Some(p) = prev
            && r > p + 1
        {
            out.push_str(&format!(
                "[rows {}-{} omitted — similar structure]\n",
                grid.start_row + p + 2,
                grid.start_row + r
            ));
        }
        out.push_str(&render_row(
            grid,
            styles,
            r,
            0,
            grid.width.saturating_sub(1),
            opts,
        ));
        out.push('\n');
        prev = Some(r);
    }
    out
}

/// Inverted-index translation: distinct text values → the ranges holding them,
/// plus homogeneous type runs per column. Lossless for text, heavily
/// compressed for numeric regions. Useful for "where does X appear" tasks;
/// `contains` filters values by case-insensitive substring.
pub fn encode_inverted_index(
    grid: &SheetGrid,
    max_entries: usize,
    contains: Option<&str>,
) -> String {
    use std::collections::BTreeMap;

    let needle = contains
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let mut index: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for r in 0..grid.height {
        for c in 0..grid.width {
            let v = grid.cell(r, c);
            if v.is_empty() {
                continue;
            }
            if matches!(v, CellValue::Text(_) | CellValue::Bool(_)) {
                let display = v.display();
                if let Some(n) = &needle
                    && !display.to_lowercase().contains(n.as_str())
                {
                    continue;
                }
                let key = truncate_chars(&display, 60);
                index.entry(key).or_default().push((r, c));
            }
        }
    }
    if index.is_empty() {
        return match contains {
            Some(q) => format!("No text values containing '{q}' found."),
            None => "No text values on this sheet.".to_string(),
        };
    }

    let mut out = String::with_capacity(4096);
    out.push_str("Value index (value: locations):\n");
    let mut entries: Vec<(String, Vec<(usize, usize)>)> = index.into_iter().collect();
    entries.sort_by_key(|(_, locs)| std::cmp::Reverse(locs.len()));
    for (value, locs) in entries.into_iter().take(max_entries) {
        if locs.len() > 12 {
            let (mut r0, mut c0, mut r1, mut c1) = (usize::MAX, usize::MAX, 0, 0);
            for &(r, c) in &locs {
                r0 = r0.min(r);
                c0 = c0.min(c);
                r1 = r1.max(r);
                c1 = c1.max(c);
            }
            out.push_str(&format!(
                "\"{}\": {} cells within {}\n",
                value,
                locs.len(),
                grid.a1_range(r0, c0, r1, c1)
            ));
        } else {
            let addrs: Vec<String> = locs.iter().map(|&(r, c)| grid.a1(r, c)).collect();
            out.push_str(&format!("\"{}\": {}\n", value, addrs.join(",")));
        }
    }

    if needle.is_some() {
        return out;
    }
    out.push_str("Homogeneous numeric/date runs per column:\n");
    for c in 0..grid.width {
        let mut run_start: Option<(usize, Kind)> = None;
        let mut runs: Vec<String> = Vec::new();
        for r in 0..=grid.height {
            let kind = if r < grid.height {
                cell_kind(grid.cell(r, c)).filter(|k| matches!(k, Kind::Num | Kind::Date))
            } else {
                None
            };
            match (run_start, kind) {
                (None, Some(k)) => run_start = Some((r, k)),
                (Some((_, k0)), Some(k)) if k == k0 => {}
                (Some((start, k0)), _) => {
                    if r - start >= 3 {
                        runs.push(format!(
                            "{}: {}",
                            grid.a1_range(start, c, r - 1, c),
                            kind_label(k0)
                        ));
                    }
                    run_start = kind.map(|k| (r, k));
                }
                (None, None) => {}
            }
        }
        if !runs.is_empty() {
            out.push_str(&runs.join(" | "));
            out.push('\n');
        }
    }
    out
}

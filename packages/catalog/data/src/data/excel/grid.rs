//! Typed sheet grid shared by the Excel table-extraction nodes.
//!
//! Reads a worksheet exactly once via calamine, preserving cell types
//! (the previous implementation stringified every cell and re-parsed later).
//! Merged regions come straight from calamine for xlsx files and are stored
//! grid-relative, correcting for the used-range offset.

use flow_like_types::{Result, anyhow};

use calamine::{Data, Range, Reader, Sheets, open_workbook_auto_from_rs};
use chrono::NaiveDateTime;
use std::io::Cursor;

pub const MAX_SUPPORTED_ROWS: usize = 10_000_000;
pub const MAX_SUPPORTED_COLS: usize = 50_000;

#[derive(Clone, Debug, PartialEq)]
pub enum CellValue {
    Empty,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    DateTime(NaiveDateTime),
    /// Duration in seconds
    Duration(f64),
    Error(String),
}

impl CellValue {
    #[inline]
    pub fn is_empty(&self) -> bool {
        match self {
            CellValue::Empty => true,
            CellValue::Text(s) => s.trim().is_empty(),
            _ => false,
        }
    }

    /// Human-readable rendering (headers, LLM previews, CSV fallback).
    pub fn display(&self) -> String {
        match self {
            CellValue::Empty => String::new(),
            CellValue::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            CellValue::Int(i) => i.to_string(),
            CellValue::Float(f) => format_float(*f),
            CellValue::Text(s) => s.clone(),
            CellValue::DateTime(dt) => format_datetime(dt),
            CellValue::Duration(secs) => format!("{}s", secs),
            CellValue::Error(e) => format!("#ERR:{}", e),
        }
    }

    /// Typed JSON value for `CSVTable` so Arrow schema inference keeps real types.
    pub fn to_json(&self) -> flow_like_types::Value {
        use flow_like_types::json::json;
        match self {
            CellValue::Empty => flow_like_types::Value::Null,
            CellValue::Bool(b) => json!(b),
            CellValue::Int(i) => json!(i),
            CellValue::Float(f) => json!(f),
            CellValue::Text(s) => json!(s),
            CellValue::DateTime(dt) => json!(format_datetime(dt)),
            CellValue::Duration(secs) => json!(secs),
            CellValue::Error(_) => flow_like_types::Value::Null,
        }
    }
}

fn format_float(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_992.0 {
        format!("{:.0}", f)
    } else {
        f.to_string()
    }
}

fn format_datetime(dt: &NaiveDateTime) -> String {
    use chrono::Timelike;
    if dt.hour() == 0 && dt.minute() == 0 && dt.second() == 0 {
        dt.format("%Y-%m-%d").to_string()
    } else {
        dt.format("%Y-%m-%dT%H:%M:%S").to_string()
    }
}

fn cell_from_data(d: &Data) -> CellValue {
    match d {
        Data::Empty => CellValue::Empty,
        Data::String(s) => {
            if s.trim().is_empty() {
                CellValue::Empty
            } else {
                CellValue::Text(s.clone())
            }
        }
        Data::Int(i) => CellValue::Int(*i),
        Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() <= i64::MAX as f64 {
                CellValue::Int(*f as i64)
            } else {
                CellValue::Float(*f)
            }
        }
        Data::Bool(b) => CellValue::Bool(*b),
        Data::DateTime(dt) => {
            if dt.is_duration() {
                CellValue::Duration(dt.as_f64() * 86_400.0)
            } else if let Some(ndt) = dt.as_datetime() {
                CellValue::DateTime(ndt)
            } else {
                CellValue::Float(dt.as_f64())
            }
        }
        Data::DateTimeIso(s) => NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
            .ok()
            .or_else(|| {
                chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                    .ok()
                    .and_then(|d| d.and_hms_opt(0, 0, 0))
            })
            .map(CellValue::DateTime)
            .unwrap_or_else(|| CellValue::Text(s.clone())),
        Data::DurationIso(s) => CellValue::Text(s.clone()),
        Data::Error(e) => CellValue::Error(format!("{:?}", e)),
    }
}

/// Merged cell region, grid-relative, inclusive bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeSpan {
    pub r0: usize,
    pub c0: usize,
    pub r1: usize,
    pub c1: usize,
}

impl MergeSpan {
    #[inline]
    pub fn contains(&self, r: usize, c: usize) -> bool {
        r >= self.r0 && r <= self.r1 && c >= self.c0 && c <= self.c1
    }

    #[inline]
    pub fn is_anchor(&self, r: usize, c: usize) -> bool {
        r == self.r0 && c == self.c0
    }
}

/// A worksheet's used range as a typed, row-major grid.
pub struct SheetGrid {
    pub height: usize,
    pub width: usize,
    /// Absolute sheet coordinates of the grid origin (used-range top-left).
    pub start_row: usize,
    pub start_col: usize,
    cells: Vec<CellValue>,
    pub merges: Vec<MergeSpan>,
    merge_lookup: std::collections::HashMap<(usize, usize), usize>,
}

impl SheetGrid {
    /// Builds a grid from rows (primarily for tests and synthetic sheets).
    pub fn from_rows(rows: Vec<Vec<CellValue>>, merges: Vec<MergeSpan>) -> Self {
        let height = rows.len();
        let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let mut cells = vec![CellValue::Empty; height * width];
        for (r, row) in rows.into_iter().enumerate() {
            for (c, v) in row.into_iter().enumerate() {
                cells[r * width + c] = v;
            }
        }
        let mut merge_lookup = std::collections::HashMap::new();
        for (i, m) in merges.iter().enumerate() {
            for r in m.r0..=m.r1 {
                for c in m.c0..=m.c1 {
                    merge_lookup.insert((r, c), i);
                }
            }
        }
        Self {
            height,
            width,
            start_row: 0,
            start_col: 0,
            cells,
            merges,
            merge_lookup,
        }
    }

    #[inline]
    pub fn cell(&self, r: usize, c: usize) -> &CellValue {
        static EMPTY: CellValue = CellValue::Empty;
        if r < self.height && c < self.width {
            &self.cells[r * self.width + c]
        } else {
            &EMPTY
        }
    }

    #[inline]
    pub fn is_filled(&self, r: usize, c: usize) -> bool {
        !self.cell(r, c).is_empty()
    }

    /// The merge span covering a cell, if any.
    #[inline]
    pub fn merge_at(&self, r: usize, c: usize) -> Option<&MergeSpan> {
        self.merge_lookup
            .get(&(r, c))
            .and_then(|&i| self.merges.get(i))
    }

    /// Value of a cell, resolving merged regions to their anchor value.
    pub fn cell_merged(&self, r: usize, c: usize) -> &CellValue {
        if self.cell(r, c).is_empty()
            && let Some(m) = self.merge_at(r, c)
        {
            return self.cell(m.r0, m.c0);
        }
        self.cell(r, c)
    }

    pub fn count_filled(&self) -> usize {
        self.cells.iter().filter(|c| !c.is_empty()).count()
    }

    /// A1 notation for a grid-relative cell (adds the used-range offset).
    pub fn a1(&self, r: usize, c: usize) -> String {
        format!(
            "{}{}",
            col_to_letters(self.start_col + c),
            self.start_row + r + 1
        )
    }

    /// A1 range notation for grid-relative inclusive bounds.
    pub fn a1_range(&self, r0: usize, c0: usize, r1: usize, c1: usize) -> String {
        format!("{}:{}", self.a1(r0, c0), self.a1(r1, c1))
    }
}

/// Parses an A1 cell like "BC23" into 0-based absolute (row, col).
pub fn parse_a1_cell(a1: &str) -> Option<(usize, usize)> {
    let s = a1.trim().trim_start_matches('$');
    let letters_end = s.chars().take_while(|c| c.is_ascii_alphabetic()).count();
    if letters_end == 0 {
        return None;
    }
    let (letters, digits) = s.split_at(letters_end);
    let digits = digits.trim_start_matches('$');
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut col: usize = 0;
    for ch in letters.chars() {
        col = col
            .checked_mul(26)?
            .checked_add((ch.to_ascii_uppercase() as u8 - b'A') as usize + 1)?;
    }
    let row: usize = digits.parse().ok()?;
    if row == 0 || col == 0 {
        return None;
    }
    Some((row - 1, col - 1))
}

/// Parses an A1 range like "A3:F42" (or a single cell "A3") into 0-based
/// absolute inclusive bounds (r0, c0, r1, c1).
pub fn parse_a1_range(range: &str) -> Option<(usize, usize, usize, usize)> {
    let s = range.trim();
    let (a, b) = match s.split_once(':') {
        Some((a, b)) => (a, b),
        None => (s, s),
    };
    let (r0, c0) = parse_a1_cell(a)?;
    let (r1, c1) = parse_a1_cell(b)?;
    Some((r0.min(r1), c0.min(c1), r0.max(r1), c0.max(c1)))
}

pub fn col_to_letters(mut c: usize) -> String {
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Absolute (sheet-coordinate) inclusive cell bounds.
#[derive(Clone, Copy, Debug)]
pub struct AbsoluteBounds {
    pub r0: usize,
    pub c0: usize,
    pub r1: usize,
    pub c1: usize,
}

/// An Excel defined table (ListObject) with typed data rows.
pub struct DefinedTable {
    pub name: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<CellValue>>,
    pub bounds: Option<AbsoluteBounds>,
}

/// Opened workbook wrapper so multiple sheets can be read without re-parsing.
pub struct Workbook {
    sheets: Sheets<Cursor<Vec<u8>>>,
    merges_loaded: bool,
    tables_loaded: bool,
}

impl Workbook {
    pub fn open(bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() {
            return Err(anyhow!("Excel file is empty"));
        }
        let sheets = open_workbook_auto_from_rs(Cursor::new(bytes))
            .map_err(|e| anyhow!("Failed to open workbook (corrupt or unsupported format): {e}"))?;
        Ok(Self {
            sheets,
            merges_loaded: false,
            tables_loaded: false,
        })
    }

    pub fn sheet_names(&self) -> Vec<String> {
        self.sheets.sheet_names().to_vec()
    }

    /// Reads one worksheet into a typed grid, including merges (xlsx only).
    pub fn read_grid(&mut self, sheet_name: &str) -> Result<SheetGrid> {
        let range: Range<Data> = self
            .sheets
            .worksheet_range(sheet_name)
            .map_err(|e| anyhow!("Sheet '{sheet_name}' not found or unreadable: {e}"))?;

        let (height, width) = range.get_size();
        let (start_row, start_col) = range
            .start()
            .map(|(r, c)| (r as usize, c as usize))
            .unwrap_or((0, 0));

        if height > MAX_SUPPORTED_ROWS || width > MAX_SUPPORTED_COLS {
            return Err(anyhow!(
                "Sheet '{sheet_name}' is {height}x{width}, exceeding the supported maximum of {MAX_SUPPORTED_ROWS}x{MAX_SUPPORTED_COLS}"
            ));
        }

        let mut cells = vec![CellValue::Empty; height * width];
        for (r, row) in range.rows().enumerate() {
            let base = r * width;
            for (c, cell) in row.iter().enumerate().take(width) {
                if !matches!(cell, Data::Empty) {
                    cells[base + c] = cell_from_data(cell);
                }
            }
        }

        let merges = self.read_merges(sheet_name, start_row, start_col, height, width);
        let mut merge_lookup = std::collections::HashMap::new();
        for (i, m) in merges.iter().enumerate() {
            for r in m.r0..=m.r1 {
                for c in m.c0..=m.c1 {
                    merge_lookup.insert((r, c), i);
                }
            }
        }

        Ok(SheetGrid {
            height,
            width,
            start_row,
            start_col,
            cells,
            merges,
            merge_lookup,
        })
    }

    /// Excel defined tables (ListObjects) on a sheet — authoritative when present.
    /// xlsx only; other formats return an empty list.
    pub fn defined_tables(&mut self, sheet_name: &str) -> Vec<DefinedTable> {
        let Sheets::Xlsx(xlsx) = &mut self.sheets else {
            return Vec::new();
        };
        if !self.tables_loaded {
            if let Err(e) = xlsx.load_tables() {
                tracing::warn!("Failed to load defined tables: {e}");
                return Vec::new();
            }
            self.tables_loaded = true;
        }
        let names: Vec<String> = xlsx
            .table_names_in_sheet(sheet_name)
            .into_iter()
            .cloned()
            .collect();
        let mut out = Vec::new();
        for name in names {
            let Ok(table) = xlsx.table_by_name(&name) else {
                continue;
            };
            let columns = table.columns().to_vec();
            let data = table.data();
            let bounds = data.start().zip(data.end()).map(|(s, e)| AbsoluteBounds {
                r0: s.0 as usize,
                c0: s.1 as usize,
                r1: e.0 as usize,
                c1: e.1 as usize,
            });
            let rows: Vec<Vec<CellValue>> = data
                .rows()
                .map(|row| row.iter().map(cell_from_data).collect())
                .collect();
            out.push(DefinedTable {
                name,
                columns,
                rows,
                bounds,
            });
        }
        out
    }

    fn read_merges(
        &mut self,
        sheet_name: &str,
        start_row: usize,
        start_col: usize,
        height: usize,
        width: usize,
    ) -> Vec<MergeSpan> {
        let Sheets::Xlsx(xlsx) = &mut self.sheets else {
            return Vec::new();
        };
        if !self.merges_loaded {
            if let Err(e) = xlsx.load_merged_regions() {
                tracing::warn!("Failed to load merged regions: {e}");
                return Vec::new();
            }
            self.merges_loaded = true;
        }
        if height == 0 || width == 0 {
            return Vec::new();
        }
        xlsx.merged_regions_by_sheet(sheet_name)
            .into_iter()
            .filter_map(|(_, _, dim)| {
                let r0 = (dim.start.0 as usize).checked_sub(start_row)?;
                let c0 = (dim.start.1 as usize).checked_sub(start_col)?;
                if r0 >= height || c0 >= width {
                    return None;
                }
                let r1 = ((dim.end.0 as usize).saturating_sub(start_row)).min(height - 1);
                let c1 = ((dim.end.1 as usize).saturating_sub(start_col)).min(width - 1);
                (r1 > r0 || c1 > c0).then_some(MergeSpan { r0, c0, r1, c1 })
            })
            .collect()
    }
}

/// Normalizes a sheet or table name into a safe, lowercase SQL identifier:
/// transliterates common accents, maps other characters to `_`, collapses
/// repeats, and guards against digit-leading or empty results.
pub fn normalize_table_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.trim().chars() {
        match ch {
            'a'..='z' | '0'..='9' => out.push(ch),
            'A'..='Z' => out.push(ch.to_ascii_lowercase()),
            'ä' | 'Ä' => out.push_str("ae"),
            'ö' | 'Ö' => out.push_str("oe"),
            'ü' | 'Ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            'á' | 'à' | 'â' | 'å' | 'Á' | 'À' | 'Â' | 'Å' => out.push('a'),
            'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => out.push('e'),
            'í' | 'ì' | 'î' | 'Í' | 'Ì' | 'Î' => out.push('i'),
            'ó' | 'ò' | 'ô' | 'Ó' | 'Ò' | 'Ô' => out.push('o'),
            'ú' | 'ù' | 'û' | 'Ú' | 'Ù' | 'Û' => out.push('u'),
            'ñ' | 'Ñ' => out.push('n'),
            'ç' | 'Ç' => out.push('c'),
            _ => {
                if !out.ends_with('_') {
                    out.push('_');
                }
            }
        }
    }
    let trimmed = out.trim_matches('_');
    let mut result = if trimmed.is_empty() {
        "table".to_string()
    } else {
        trimmed.to_string()
    };
    if result.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        result.insert_str(0, "t_");
    }
    result.truncate(64);
    let retrimmed = result.trim_end_matches('_');
    if retrimmed.len() != result.len() {
        result = retrimmed.to_string();
    }
    result
}

/// Returns `base` if unused, otherwise `base_2`, `base_3`, ….
pub fn unique_table_name(existing: &std::collections::HashSet<String>, base: &str) -> String {
    if !existing.contains(base) {
        return base.to_string();
    }
    for i in 2.. {
        let candidate = format!("{base}_{i}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

/// Truncates on a char boundary, appending `…` when shortened.
pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_names() {
        assert_eq!(normalize_table_name("Sales Data (2024)"), "sales_data_2024");
        assert_eq!(normalize_table_name("Umsätze Q1"), "umsaetze_q1");
        assert_eq!(normalize_table_name("2024 Report"), "t_2024_report");
        assert_eq!(normalize_table_name("  __ "), "table");
        assert_eq!(normalize_table_name("Ärzte/Notfälle"), "aerzte_notfaelle");
    }

    #[test]
    fn unique_names() {
        let mut set = std::collections::HashSet::new();
        set.insert("sales".to_string());
        assert_eq!(unique_table_name(&set, "sales"), "sales_2");
        set.insert("sales_2".to_string());
        assert_eq!(unique_table_name(&set, "sales"), "sales_3");
        assert_eq!(unique_table_name(&set, "other"), "other");
    }

    #[test]
    fn col_letters() {
        assert_eq!(col_to_letters(0), "A");
        assert_eq!(col_to_letters(25), "Z");
        assert_eq!(col_to_letters(26), "AA");
        assert_eq!(col_to_letters(27), "AB");
        assert_eq!(col_to_letters(701), "ZZ");
        assert_eq!(col_to_letters(702), "AAA");
    }

    #[test]
    fn a1_parsing() {
        assert_eq!(parse_a1_cell("A1"), Some((0, 0)));
        assert_eq!(parse_a1_cell("BC23"), Some((22, 54)));
        assert_eq!(parse_a1_cell("$B$2"), Some((1, 1)));
        assert_eq!(parse_a1_cell("123"), None);
        assert_eq!(parse_a1_cell("A0"), None);
        assert_eq!(parse_a1_range("A3:F42"), Some((2, 0, 41, 5)));
        assert_eq!(parse_a1_range("F42:A3"), Some((2, 0, 41, 5)));
        assert_eq!(parse_a1_range("B7"), Some((6, 1, 6, 1)));
        assert_eq!(parse_a1_range("garbage"), None);
    }

    #[test]
    fn truncate_multibyte_safe() {
        assert_eq!(truncate_chars("héllo wörld", 6), "héllo…");
        assert_eq!(truncate_chars("short", 10), "short");
        assert_eq!(truncate_chars("日本語テキスト", 4), "日本語…");
    }

    #[test]
    fn float_display() {
        assert_eq!(CellValue::Int(42).display(), "42");
        assert_eq!(CellValue::Float(1.5).display(), "1.5");
    }
}

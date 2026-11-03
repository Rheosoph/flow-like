//! Context-free workbook inspection: the questions the AI table extractor asks about a sheet
//! (look at a range, find styled cells, find a value) as plain functions, so a caller that
//! reasons for itself can ask them without a model loop or an `ExecutionContext`.
//! Cells hold the values Excel cached; formulas are not evaluated.

use super::grid::{SheetGrid, Workbook, col_to_letters, parse_a1_range};
use super::sheet_compressor::{
    EncodeOptions, encode_inverted_index, encode_sheet_compact, render_range,
};
use super::styles::{SheetStyles, color_matches};
use super::table_detect::{
    BuildOverrides, ExtractConfig, Rect, build_table_from_rect, detect_table_regions,
};
use flow_like_types::{Result, anyhow};
use serde::Serialize;
use std::sync::Arc;

pub const MAX_INSPECT_ROWS: usize = 60;
pub const MAX_STYLE_MATCHES_LISTED: usize = 100;
pub const MAX_VALUE_INDEX_ENTRIES: usize = 300;
/// Sheets with more cells than this get the value index instead of the row sample.
const COMPACT_VIEW_MAX_CELLS: usize = 200_000;

/// One worksheet read once: its grid, the heuristic table regions and, for xlsx, its styles.
pub struct LoadedSheet {
    pub name: String,
    pub grid: Arc<SheetGrid>,
    pub candidates: Vec<Rect>,
    pub styles: Option<Arc<SheetStyles>>,
}

#[derive(Clone, Debug, Default)]
pub struct StyleFilter {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub fill_color: Option<String>,
    pub font_color: Option<String>,
}

impl StyleFilter {
    fn is_empty(&self) -> bool {
        self.bold.is_none()
            && self.italic.is_none()
            && self.fill_color.is_none()
            && self.font_color.is_none()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DefinedTableInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub rows: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct SheetOverview {
    pub name: String,
    pub hidden: bool,
    /// `None` for an empty sheet.
    pub used_range: Option<String>,
    pub rows: usize,
    pub cols: usize,
    pub filled_cells: usize,
    pub merged_regions: usize,
    pub defined_tables: Vec<DefinedTableInfo>,
    /// Set when the sheet could not be read.
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkbookOverview {
    pub sheets: Vec<SheetOverview>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TableCandidate {
    /// A1 range of the region, absolute sheet coordinates.
    pub range: String,
    pub title: Option<String>,
    pub headers: Vec<String>,
    pub header_detected: bool,
    pub data_rows: usize,
    /// 0.0–1.0 heuristic quality estimate.
    pub confidence: f32,
}

/// Sheets, used ranges, hidden sheets, merged regions and Excel's defined tables.
/// Synchronous and CPU-bound; run it on a blocking thread.
pub fn workbook_overview(bytes: Vec<u8>) -> Result<WorkbookOverview> {
    let mut workbook = Workbook::open(bytes)?;
    let hidden = workbook.hidden_sheet_names();
    let mut sheets = Vec::new();
    for name in workbook.sheet_names() {
        let defined_tables = workbook
            .defined_tables(&name)
            .into_iter()
            .map(|table| DefinedTableInfo {
                rows: table.rows.len(),
                name: table.name,
                columns: table.columns,
            })
            .collect();
        let mut overview = SheetOverview {
            hidden: hidden.contains(&name),
            used_range: None,
            rows: 0,
            cols: 0,
            filled_cells: 0,
            merged_regions: 0,
            defined_tables,
            error: None,
            name: name.clone(),
        };
        match workbook.read_grid(&name) {
            Ok(grid) => {
                if grid.height > 0 && grid.width > 0 {
                    overview.used_range =
                        Some(grid.a1_range(0, 0, grid.height - 1, grid.width - 1));
                }
                overview.rows = grid.height;
                overview.cols = grid.width;
                overview.filled_cells = grid.count_filled();
                overview.merged_regions = grid.merges.len();
            }
            Err(error) => overview.error = Some(error.to_string()),
        }
        sheets.push(overview);
    }
    Ok(WorkbookOverview { sheets })
}

/// Reads one sheet. Styles are parsed only when `with_styles` is set (xlsx only; a workbook
/// whose styles cannot be read yields `None`).
pub fn load_sheet(
    bytes: &[u8],
    sheet: &str,
    config: &ExtractConfig,
    with_styles: bool,
) -> Result<LoadedSheet> {
    let mut workbook = Workbook::open(bytes.to_vec())?;
    if !workbook.sheet_names().iter().any(|name| name == sheet) {
        return Err(anyhow!("Sheet '{sheet}' not found in workbook"));
    }
    let grid = workbook.read_grid(sheet)?;
    let candidates = detect_table_regions(&grid, config);
    let styles = with_styles
        .then(|| SheetStyles::load(bytes, sheet).ok().map(Arc::new))
        .flatten();
    Ok(LoadedSheet {
        name: sheet.to_string(),
        grid: Arc::new(grid),
        candidates,
        styles,
    })
}

/// An absolute A1 range as grid-relative bounds, clamped to the used range.
pub fn a1_to_rect(grid: &SheetGrid, range: &str) -> std::result::Result<Rect, String> {
    let (ar0, ac0, ar1, ac1) =
        parse_a1_range(range).ok_or_else(|| format!("'{range}' is not a valid A1 range"))?;
    let r0 = ar0
        .saturating_sub(grid.start_row)
        .min(grid.height.saturating_sub(1));
    let c0 = ac0
        .saturating_sub(grid.start_col)
        .min(grid.width.saturating_sub(1));
    let r1 = ar1
        .saturating_sub(grid.start_row)
        .min(grid.height.saturating_sub(1));
    let c1 = ac1
        .saturating_sub(grid.start_col)
        .min(grid.width.saturating_sub(1));
    if r1 < r0 || c1 < c0 {
        return Err(format!("Range '{range}' lies outside the used range"));
    }
    Ok(Rect { r0, c0, r1, c1 })
}

/// The verbatim cells of an A1 range, at most `max_rows` (and [`MAX_INSPECT_ROWS`]) rows, with
/// `**bold**` and `[color]` marks when styles are given. Problems come back as text.
pub fn inspect_range(
    grid: &SheetGrid,
    styles: Option<&SheetStyles>,
    range: &str,
    max_rows: Option<usize>,
    options: &EncodeOptions,
) -> String {
    let rect = match a1_to_rect(grid, range) {
        Ok(rect) => rect,
        Err(error) => return error,
    };
    let cap = max_rows.unwrap_or(MAX_INSPECT_ROWS).min(MAX_INSPECT_ROWS);
    render_range(grid, styles, &rect, cap, options)
}

/// Cell addresses matching a style filter; with no filter, a summary of the sheet's styling.
pub fn find_styled_cells(styles: Option<&SheetStyles>, filter: &StyleFilter) -> String {
    let Some(styles) = styles else {
        return "No style information available (styles require xlsx format).".to_string();
    };
    if styles.is_empty() {
        return "The sheet has no styled cells.".to_string();
    }
    if filter.is_empty() {
        return format!("Styling on this sheet: {}", styles.summarize(12));
    }
    let matches = styles.find(|style| {
        filter.bold.is_none_or(|bold| style.bold == bold)
            && filter.italic.is_none_or(|italic| style.italic == italic)
            && filter
                .fill_color
                .as_deref()
                .is_none_or(|query| color_matches(query, style.fill_color_name()))
            && filter
                .font_color
                .as_deref()
                .is_none_or(|query| color_matches(query, style.font_color_name()))
    });
    if matches.is_empty() {
        return format!(
            "No cells match. Styling present on this sheet: {}",
            styles.summarize(12)
        );
    }
    let addresses: Vec<String> = matches
        .iter()
        .take(MAX_STYLE_MATCHES_LISTED)
        .map(|&(row, col, _)| format!("{}{}", col_to_letters(col), row + 1))
        .collect();
    let suffix = if matches.len() > MAX_STYLE_MATCHES_LISTED {
        ", …"
    } else {
        ""
    };
    format!(
        "{} matching cells: {}{}",
        matches.len(),
        addresses.join(", "),
        suffix
    )
}

/// Where text values appear (case-insensitive substring `contains`), with homogeneous
/// numeric runs when unfiltered.
pub fn value_index(grid: &SheetGrid, contains: Option<&str>) -> String {
    encode_inverted_index(grid, MAX_VALUE_INDEX_ENTRIES, contains)
}

/// A compact, address-annotated picture of the sheet: header facts, merges, styling, table
/// candidates, column profiles and the structurally interesting rows. Sheets too large for a
/// row sample get the value index instead.
pub fn sheet_view(sheet: &LoadedSheet, options: &EncodeOptions) -> String {
    let grid = &sheet.grid;
    if grid.height * grid.width > COMPACT_VIEW_MAX_CELLS {
        return format!(
            "Sheet '{}' is too large for a row sample ({} rows x {} cols); value index instead:\n{}",
            sheet.name,
            grid.height,
            grid.width,
            value_index(grid, None)
        );
    }
    encode_sheet_compact(
        grid,
        &sheet.candidates,
        sheet.styles.as_deref(),
        options,
        &sheet.name,
    )
}

/// The detected table regions with their headers and confidence.
pub fn table_candidates(sheet: &LoadedSheet, config: &ExtractConfig) -> Vec<TableCandidate> {
    sheet
        .candidates
        .iter()
        .filter_map(|rect| {
            build_table_from_rect(&sheet.grid, rect, config, &BuildOverrides::default())
        })
        .map(|table| TableCandidate {
            range: table.range_a1,
            title: table.title,
            data_rows: table.rows.len(),
            headers: table.headers,
            header_detected: table.header_detected,
            confidence: table.confidence,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use umya_spreadsheet::structs::SheetStateValues;
    use umya_spreadsheet::{Spreadsheet, Worksheet};

    fn build_xlsx(build: impl FnOnce(&mut Spreadsheet)) -> Vec<u8> {
        let mut book = umya_spreadsheet::new_file();
        build(&mut book);
        let mut bytes: Vec<u8> = Vec::new();
        umya_spreadsheet::writer::xlsx::write_writer(&book, &mut bytes).expect("write xlsx");
        bytes
    }

    fn text(sheet: &mut Worksheet, col: u32, row: u32, value: &str) {
        sheet.get_cell_mut((col, row)).set_value_string(value);
    }

    fn number(sheet: &mut Worksheet, col: u32, row: u32, value: f64) {
        sheet.get_cell_mut((col, row)).set_value_number(value);
    }

    /// Title in A1:C1 (merged), a header row at row 3 and four data rows from C4 on, so the
    /// used range does not start at A1; the Status cells of "Late" rows are red and bold.
    fn orders_workbook() -> Vec<u8> {
        build_xlsx(|book| {
            let sheet = book.get_sheet_mut(&0).unwrap();
            sheet.set_name("Orders");
            text(sheet, 3, 3, "Order");
            text(sheet, 4, 3, "Status");
            text(sheet, 5, 3, "Total");
            let rows = [
                ("A-1", "Shipped", 120.0),
                ("A-2", "Late", 80.0),
                ("A-3", "Shipped", 45.5),
                ("A-4", "Late", 300.0),
            ];
            for (index, (order, status, total)) in rows.iter().enumerate() {
                let row = 4 + index as u32;
                text(sheet, 3, row, order);
                text(sheet, 4, row, status);
                number(sheet, 5, row, *total);
                if *status == "Late" {
                    let style = sheet.get_style_mut((4, row));
                    style.get_font_mut().set_bold(true);
                    style.set_background_color("FFFF0000");
                }
            }
            sheet.add_merge_cells("C1:E1");
            text(sheet, 3, 1, "Q3 orders");
            let hidden = book.new_sheet("Archive").unwrap();
            text(hidden, 1, 1, "old");
            hidden.set_state(SheetStateValues::Hidden);
        })
    }

    fn orders() -> LoadedSheet {
        load_sheet(
            &orders_workbook(),
            "Orders",
            &ExtractConfig::default(),
            true,
        )
        .unwrap()
    }

    #[test]
    fn overview_lists_sheets_with_used_range_and_hidden_state() {
        let overview = workbook_overview(orders_workbook()).unwrap();
        let names: Vec<_> = overview.sheets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Orders", "Archive"]);

        let orders = &overview.sheets[0];
        assert!(!orders.hidden);
        assert_eq!(orders.used_range.as_deref(), Some("C1:E7"));
        assert_eq!((orders.rows, orders.cols), (7, 3));
        assert_eq!(orders.merged_regions, 1);
        assert_eq!(orders.error, None);

        let archive = &overview.sheets[1];
        assert!(archive.hidden);
        assert_eq!(archive.used_range.as_deref(), Some("A1:A1"));
    }

    #[test]
    fn inspect_range_uses_absolute_addresses_and_style_marks() {
        let sheet = orders();
        let options = EncodeOptions::default();

        let rendered = inspect_range(
            &sheet.grid,
            sheet.styles.as_deref(),
            "C3:E5",
            None,
            &options,
        );
        assert!(rendered.contains("Row 3: C3=\"Order\""), "{rendered}");
        assert!(rendered.contains("[red]**D5=\"Late\"**"), "{rendered}");

        let capped = inspect_range(&sheet.grid, None, "C3:E7", Some(2), &options);
        assert!(capped.contains("more rows in range omitted"), "{capped}");
        assert!(!capped.contains("Row 5"), "{capped}");
    }

    #[test]
    fn inspect_range_reports_bad_ranges_as_text() {
        let sheet = orders();
        let options = EncodeOptions::default();
        assert_eq!(
            inspect_range(&sheet.grid, None, "nonsense", None, &options),
            "'nonsense' is not a valid A1 range"
        );
        assert!(a1_to_rect(&sheet.grid, "C3:E5").is_ok());
        assert_eq!(
            a1_to_rect(&sheet.grid, "E5:C3").unwrap(),
            a1_to_rect(&sheet.grid, "C3:E5").unwrap()
        );
    }

    #[test]
    fn find_styled_cells_filters_by_style_and_summarizes_without_a_filter() {
        let sheet = orders();
        let styles = sheet.styles.as_deref();

        let red = find_styled_cells(
            styles,
            &StyleFilter {
                fill_color: Some("red".into()),
                ..StyleFilter::default()
            },
        );
        assert_eq!(red, "2 matching cells: D5, D7");

        let none = find_styled_cells(
            styles,
            &StyleFilter {
                fill_color: Some("green".into()),
                ..StyleFilter::default()
            },
        );
        assert!(none.starts_with("No cells match."), "{none}");

        let summary = find_styled_cells(styles, &StyleFilter::default());
        assert!(summary.starts_with("Styling on this sheet:"), "{summary}");
        assert_eq!(
            find_styled_cells(None, &StyleFilter::default()),
            "No style information available (styles require xlsx format)."
        );
    }

    #[test]
    fn value_index_finds_labels_case_insensitively() {
        let sheet = orders();
        let found = value_index(&sheet.grid, Some("LATE"));
        assert!(found.contains("\"Late\": D5,D7"), "{found}");
        assert_eq!(
            value_index(&sheet.grid, Some("missing")),
            "No text values containing 'missing' found."
        );
        assert!(value_index(&sheet.grid, None).contains("Homogeneous numeric/date runs"));
    }

    #[test]
    fn views_and_candidates_describe_the_table() {
        let sheet = orders();
        let view = sheet_view(&sheet, &EncodeOptions::default());
        assert!(view.contains("Sheet 'Orders': used range C1:E7"), "{view}");
        assert!(view.contains("Merged regions: C1:E1"), "{view}");

        let candidates = table_candidates(&sheet, &ExtractConfig::default());
        assert_eq!(candidates.len(), 1);
        let table = &candidates[0];
        assert_eq!(table.headers, vec!["Order", "Status", "Total"]);
        assert!(table.header_detected);
        assert_eq!(table.data_rows, 4);
        assert!(table.confidence > 0.3);
    }

    #[test]
    fn unknown_sheets_and_corrupt_files_are_errors() {
        let error = load_sheet(&orders_workbook(), "Nope", &ExtractConfig::default(), false)
            .err()
            .expect("unknown sheet")
            .to_string();
        assert_eq!(error, "Sheet 'Nope' not found in workbook");
        assert!(workbook_overview(b"not a workbook".to_vec()).is_err());
        assert!(workbook_overview(Vec::new()).is_err());
    }
}

//! Bulletproof integration suite for the Excel extraction subsystem.
//!
//! Every test builds a REAL xlsx workbook in memory (umya-spreadsheet) and
//! runs it through the full public pipeline: calamine parse → typed grid →
//! table detection → CSVTable → Arrow → DataFusion SQL. No mocks, no
//! synthetic grids — if these pass, the shipping path works.
//!
//! Run: cargo test -p flow-like-catalog-data --features execute --test excel_extraction
#![cfg(feature = "execute")]

use flow_like_catalog_data::data::excel::grid::{Workbook, normalize_table_name};
use flow_like_catalog_data::data::excel::sheet_compressor::{
    EncodeOptions, encode_inverted_index, encode_sheet_compact, render_range,
};
use flow_like_catalog_data::data::excel::styles::{
    SheetStyles, classify_color, load_workbook_styles,
};
use flow_like_catalog_data::data::excel::table_detect::{
    ExtractConfig, Rect, SheetTableMode, detect_table_regions, extract_tables_from_grid,
    extract_workbook_tables, whole_sheet_table,
};
use flow_like_storage::arrow::datatypes::DataType;
use flow_like_storage::datafusion::prelude::SessionContext;
use umya_spreadsheet::{Spreadsheet, Worksheet};

// ============================ Fixture helpers ============================

fn build_xlsx(build: impl FnOnce(&mut Spreadsheet)) -> Vec<u8> {
    let mut book = umya_spreadsheet::new_file();
    build(&mut book);
    let mut buf: Vec<u8> = Vec::new();
    umya_spreadsheet::writer::xlsx::write_writer(&book, &mut buf).expect("write xlsx");
    buf
}

fn s(sheet: &mut Worksheet, col: u32, row: u32, v: &str) {
    sheet.get_cell_mut((col, row)).set_value_string(v);
}

fn n(sheet: &mut Worksheet, col: u32, row: u32, v: f64) {
    sheet.get_cell_mut((col, row)).set_value_number(v);
}

fn b(sheet: &mut Worksheet, col: u32, row: u32, v: bool) {
    sheet.get_cell_mut((col, row)).set_value_bool(v);
}

/// Standard people table at (col, row) 1-based origin.
fn people_table(sheet: &mut Worksheet, col: u32, row: u32) {
    for (i, h) in ["Name", "Age", "City"].iter().enumerate() {
        s(sheet, col + i as u32, row, h);
    }
    let data = [
        ("Alice", 30.0, "Berlin"),
        ("Bob", 25.0, "Hamburg"),
        ("Carol", 41.0, "Munich"),
    ];
    for (i, (name, age, city)) in data.iter().enumerate() {
        let r = row + 1 + i as u32;
        s(sheet, col, r, name);
        n(sheet, col + 1, r, *age);
        s(sheet, col + 2, r, city);
    }
}

fn detect(buf: Vec<u8>) -> flow_like_catalog_data::data::excel::table_detect::WorkbookTables {
    extract_workbook_tables(
        buf,
        None,
        &ExtractConfig::default(),
        SheetTableMode::DetectTables,
        "",
        None,
    )
    .expect("extraction should succeed")
}

// ============================ A. Core detection ============================

#[test]
fn clean_table_with_typed_arrow_schema() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        people_table(sheet, 1, 1);
    });
    let result = detect(buf);
    assert_eq!(result.tables.len(), 1);
    let t = &result.tables[0];
    assert_eq!(t.headers(), vec!["Name", "Age", "City"]);
    assert_eq!(t.row_count(), 3);
    assert_eq!(t.range.as_deref(), Some("A1:C4"));

    let schema = t.arrow_schema();
    assert_eq!(schema.field(0).data_type(), &DataType::Utf8);
    assert_eq!(schema.field(1).data_type(), &DataType::Int64);
    assert_eq!(schema.field(2).data_type(), &DataType::Utf8);
}

#[test]
fn title_and_footnote_are_metadata_not_data() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 1, 1, "Quarterly Overview");
        sheet.add_merge_cells("A1:C1");
        people_table(sheet, 1, 3);
        s(sheet, 1, 8, "* preliminary numbers");
    });
    let result = detect(buf);
    assert_eq!(result.tables.len(), 1);
    let t = &result.tables[0];
    assert_eq!(t.title.as_deref(), Some("Quarterly Overview"));
    assert_eq!(t.headers(), vec!["Name", "Age", "City"]);
    assert_eq!(t.row_count(), 3);
}

#[test]
fn merged_multi_row_header_flattens() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 1, 1, "Region");
        s(sheet, 2, 1, "2023");
        s(sheet, 4, 1, "2024");
        sheet.add_merge_cells("B1:C1");
        sheet.add_merge_cells("D1:E1");
        for (i, q) in ["Q1", "Q2", "Q1", "Q2"].iter().enumerate() {
            s(sheet, 2 + i as u32, 2, q);
        }
        for (r_off, (region, vals)) in [
            ("North", [1.0, 2.0, 3.0, 4.0]),
            ("South", [5.0, 6.0, 7.0, 8.0]),
        ]
        .iter()
        .enumerate()
        {
            let r = 3 + r_off as u32;
            s(sheet, 1, r, region);
            for (i, v) in vals.iter().enumerate() {
                n(sheet, 2 + i as u32, r, *v);
            }
        }
    });
    let result = detect(buf);
    assert_eq!(result.tables.len(), 1);
    assert_eq!(
        result.tables[0].headers(),
        vec!["Region", "2023 / Q1", "2023 / Q2", "2024 / Q1", "2024 / Q2"]
    );
    assert_eq!(result.tables[0].row_count(), 2);
}

#[test]
fn stacked_tables_with_different_schemas_stay_separate() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        people_table(sheet, 1, 1);
        // 3 blank rows, then a different table
        s(sheet, 1, 8, "Country");
        s(sheet, 2, 8, "Population");
        s(sheet, 1, 9, "Germany");
        n(sheet, 2, 9, 83.0);
        s(sheet, 1, 10, "France");
        n(sheet, 2, 10, 68.0);
    });
    let result = detect(buf);
    assert_eq!(result.tables.len(), 2);
    assert_eq!(result.tables[0].headers(), vec!["Name", "Age", "City"]);
    assert_eq!(result.tables[1].headers(), vec!["Country", "Population"]);
}

#[test]
fn continuation_after_single_blank_row_is_one_table() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 1, 1, "Product");
        s(sheet, 2, 1, "Price");
        s(sheet, 1, 2, "Apple");
        n(sheet, 2, 2, 3.0);
        // one blank separator row inside the table
        s(sheet, 1, 4, "Pear");
        n(sheet, 2, 4, 4.0);
        s(sheet, 1, 5, "Plum");
        n(sheet, 2, 5, 5.0);
    });
    let result = detect(buf);
    assert_eq!(result.tables.len(), 1);
    assert_eq!(result.tables[0].row_count(), 3);
}

#[test]
fn side_by_side_tables_split() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        people_table(sheet, 1, 1);
        // two blank columns, second table at col 6
        s(sheet, 6, 1, "Item");
        s(sheet, 7, 1, "Price");
        s(sheet, 6, 2, "Nail");
        n(sheet, 7, 2, 0.1);
        s(sheet, 6, 3, "Screw");
        n(sheet, 7, 3, 0.2);
        s(sheet, 6, 4, "Bolt");
        n(sheet, 7, 4, 0.3);
    });
    let result = detect(buf);
    assert_eq!(result.tables.len(), 2);
    assert_eq!(result.tables[0].headers(), vec!["Name", "Age", "City"]);
    assert_eq!(result.tables[1].headers(), vec!["Item", "Price"]);
    assert_eq!(result.tables[1].range.as_deref(), Some("F1:G4"));
}

#[test]
fn aggregate_and_repeated_header_rows_are_dropped() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 1, 1, "Item");
        s(sheet, 2, 1, "Amount");
        s(sheet, 1, 2, "A");
        n(sheet, 2, 2, 1.0);
        s(sheet, 1, 3, "Item"); // repeated header (paginated export)
        s(sheet, 2, 3, "Amount");
        s(sheet, 1, 4, "B");
        n(sheet, 2, 4, 2.0);
        s(sheet, 1, 5, "Subtotal");
        n(sheet, 2, 5, 3.0);
        s(sheet, 1, 6, "C");
        n(sheet, 2, 6, 4.0);
        s(sheet, 1, 7, "Grand Total");
        n(sheet, 2, 7, 7.0);
    });
    let result = detect(buf);
    assert_eq!(result.tables.len(), 1);
    let t = &result.tables[0];
    assert_eq!(t.row_count(), 3, "A, B, C only: {:?}", t.rows_as_strings());
    let first_col: Vec<String> = t.rows_as_strings().iter().map(|r| r[0].clone()).collect();
    assert_eq!(first_col, vec!["A", "B", "C"]);
}

#[test]
fn numeric_year_headers_are_recognized() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 1, 1, "Metric");
        n(sheet, 2, 1, 2019.0);
        n(sheet, 3, 1, 2020.0);
        n(sheet, 4, 1, 2021.0);
        for (i, (m, v)) in [("Revenue", [10.5, 11.5, 12.5]), ("Cost", [5.5, 6.5, 7.5])]
            .iter()
            .enumerate()
        {
            let r = 2 + i as u32;
            s(sheet, 1, r, m);
            for (c, x) in v.iter().enumerate() {
                n(sheet, 2 + c as u32, r, *x);
            }
        }
    });
    let result = detect(buf);
    assert_eq!(result.tables.len(), 1);
    assert_eq!(
        result.tables[0].headers(),
        vec!["Metric", "2019", "2020", "2021"]
    );
    assert_eq!(result.tables[0].row_count(), 2);
}

#[test]
fn unit_row_folds_into_headers() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 1, 1, "Product");
        s(sheet, 2, 1, "Price");
        s(sheet, 3, 1, "Weight");
        s(sheet, 2, 2, "(EUR)");
        s(sheet, 3, 2, "kg");
        s(sheet, 1, 3, "Widget");
        n(sheet, 2, 3, 9.99);
        n(sheet, 3, 3, 1.5);
        s(sheet, 1, 4, "Gadget");
        n(sheet, 2, 4, 19.99);
        n(sheet, 3, 4, 2.5);
    });
    let result = detect(buf);
    assert_eq!(result.tables.len(), 1);
    assert_eq!(
        result.tables[0].headers(),
        vec!["Product", "Price [EUR]", "Weight [kg]"]
    );
    assert_eq!(result.tables[0].row_count(), 2);
}

#[test]
fn headerless_numeric_block_gets_generated_columns() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        for r in 1..=4u32 {
            for c in 1..=3u32 {
                n(sheet, c, r, (r * 10 + c) as f64);
            }
        }
    });
    let result = detect(buf);
    assert_eq!(result.tables.len(), 1);
    assert_eq!(
        result.tables[0].headers(),
        vec!["column_1", "column_2", "column_3"]
    );
    assert_eq!(result.tables[0].row_count(), 4);
}

#[test]
fn stray_page_marker_is_ignored() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        people_table(sheet, 1, 1);
        s(sheet, 8, 20, "Page 1 of 3");
    });
    let result = detect(buf);
    assert_eq!(result.tables.len(), 1, "stray cell must not become a table");
    assert_eq!(result.tables[0].headers(), vec!["Name", "Age", "City"]);
}

#[test]
fn sparse_form_produces_no_garbage_table() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 2, 3, "Filled by:");
        s(sheet, 7, 9, "Date:");
        s(sheet, 4, 15, "Signature");
    });
    let result = detect(buf);
    assert!(
        result.tables.is_empty(),
        "scattered form labels are not a table: {:?}",
        result
            .tables
            .iter()
            .map(|t| t.headers())
            .collect::<Vec<_>>()
    );
}

#[test]
fn used_range_offset_reported_in_absolute_a1() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        people_table(sheet, 5, 10); // E10
    });
    let result = detect(buf);
    assert_eq!(result.tables.len(), 1);
    assert_eq!(result.tables[0].range.as_deref(), Some("E10:G13"));
}

#[test]
fn leading_zero_ids_stay_strings() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 1, 1, "Zip");
        s(sheet, 2, 1, "City");
        s(sheet, 1, 2, "00420");
        s(sheet, 2, 2, "Prague");
        s(sheet, 1, 3, "01067");
        s(sheet, 2, 3, "Dresden");
        s(sheet, 1, 4, "80331");
        s(sheet, 2, 4, "Munich");
    });
    let result = detect(buf);
    let t = &result.tables[0];
    assert_eq!(t.arrow_schema().field(0).data_type(), &DataType::Utf8);
    let rows = t.rows_as_strings();
    assert_eq!(rows[0][0], "00420", "leading zeros must survive");
}

#[test]
fn date_formatted_numbers_become_dates() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 1, 1, "Day");
        s(sheet, 2, 1, "Sales");
        // Excel serials with a date number format: 45292 = 2024-01-01
        for (i, (serial, sales)) in [(45292.0, 10.5), (45293.0, 11.5), (45294.0, 12.5)]
            .iter()
            .enumerate()
        {
            let r = 2 + i as u32;
            n(sheet, 1, r, *serial);
            sheet
                .get_style_mut((1u32, r))
                .get_number_format_mut()
                .set_format_code("yyyy-mm-dd");
            n(sheet, 2, r, *sales);
        }
    });
    let result = detect(buf);
    let t = &result.tables[0];
    assert_eq!(
        t.arrow_schema().field(0).data_type(),
        &DataType::Date64,
        "date-formatted serials must become dates"
    );
    assert_eq!(t.rows_as_strings()[0][0], "2024-01-01T00:00:00");
}

#[test]
fn boolean_cells_become_booleans() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 1, 1, "Feature");
        s(sheet, 2, 1, "Enabled");
        s(sheet, 1, 2, "Alpha");
        b(sheet, 2, 2, true);
        s(sheet, 1, 3, "Beta");
        b(sheet, 2, 3, false);
        s(sheet, 1, 4, "Gamma");
        b(sheet, 2, 4, true);
    });
    let result = detect(buf);
    let t = &result.tables[0];
    assert_eq!(t.arrow_schema().field(1).data_type(), &DataType::Boolean);
}

// ============================ B. Workbook orchestration ============================

#[test]
fn sheet_names_normalized_with_umlauts_and_suffixes() {
    let buf = build_xlsx(|book| {
        {
            let sheet = book.get_sheet_mut(&0).unwrap();
            sheet.set_name("Umsätze (Q1) 2024");
            people_table(sheet, 1, 1);
        }
        {
            let sheet2 = book.new_sheet("Übersicht").unwrap();
            people_table(sheet2, 1, 1);
        }
    });
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
    assert_eq!(names, vec!["umsaetze_q1_2024", "uebersicht"]);
}

#[test]
fn multiple_tables_per_sheet_get_numeric_suffixes() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        sheet.set_name("Mixed");
        people_table(sheet, 1, 1);
        s(sheet, 1, 8, "City");
        s(sheet, 2, 8, "Pop");
        s(sheet, 1, 9, "Berlin");
        n(sheet, 2, 9, 3.7);
        s(sheet, 1, 10, "Paris");
        n(sheet, 2, 10, 2.1);
    });
    let result = detect(buf);
    let names: Vec<_> = result
        .tables
        .iter()
        .filter_map(|t| t.name.clone())
        .collect();
    assert_eq!(names, vec!["mixed", "mixed_2"]);
}

#[test]
fn colliding_sheet_names_get_unique_table_names() {
    let buf = build_xlsx(|book| {
        {
            let sheet = book.get_sheet_mut(&0).unwrap();
            sheet.set_name("Data 1");
            people_table(sheet, 1, 1);
        }
        {
            let sheet2 = book.new_sheet("Data-1").unwrap();
            people_table(sheet2, 1, 1);
        }
    });
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
    assert_eq!(names, vec!["data_1", "data_1_2"]);
}

#[test]
fn sheet_filter_selects_and_unknown_sheet_errors() {
    let make = || {
        build_xlsx(|book| {
            {
                let sheet = book.get_sheet_mut(&0).unwrap();
                sheet.set_name("Keep");
                people_table(sheet, 1, 1);
            }
            {
                let sheet2 = book.new_sheet("Ignore").unwrap();
                people_table(sheet2, 1, 1);
            }
        })
    };
    let result = extract_workbook_tables(
        make(),
        Some("Keep"),
        &ExtractConfig::default(),
        SheetTableMode::WholeSheet,
        "",
        None,
    )
    .unwrap();
    assert_eq!(result.tables.len(), 1);
    assert_eq!(result.tables[0].name.as_deref(), Some("keep"));

    let err = extract_workbook_tables(
        make(),
        Some("Nope"),
        &ExtractConfig::default(),
        SheetTableMode::WholeSheet,
        "",
        None,
    );
    assert!(err.is_err());
    assert!(err.unwrap_err().to_string().contains("Nope"));
}

#[test]
fn name_prefix_is_applied_and_normalized() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        sheet.set_name("Sales");
        people_table(sheet, 1, 1);
    });
    let result = extract_workbook_tables(
        buf,
        None,
        &ExtractConfig::default(),
        SheetTableMode::WholeSheet,
        "Q1 ",
        None,
    )
    .unwrap();
    assert_eq!(result.tables[0].name.as_deref(), Some("q1_sales"));
}

#[test]
fn empty_workbook_yields_no_tables_and_a_warning() {
    let buf = build_xlsx(|_book| {});
    let result = detect(buf);
    assert!(result.tables.is_empty());
    assert!(!result.warnings.is_empty());
}

#[test]
fn garbage_bytes_fail_gracefully() {
    let garbage = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11, 0x22, 0x33];
    assert!(Workbook::open(garbage.clone()).is_err());
    assert!(
        extract_workbook_tables(
            garbage,
            None,
            &ExtractConfig::default(),
            SheetTableMode::DetectTables,
            "",
            None,
        )
        .is_err()
    );
    assert!(Workbook::open(Vec::new()).is_err());
}

// ============================ C. DataFusion round-trip ============================

#[tokio::test]
async fn registered_tables_answer_sql() {
    let buf = build_xlsx(|book| {
        {
            let sheet = book.get_sheet_mut(&0).unwrap();
            sheet.set_name("Orders");
            s(sheet, 1, 1, "Customer");
            s(sheet, 2, 1, "Amount");
            for (i, (cust, amount)) in [("acme", 100.5), ("acme", 49.5), ("globex", 200.0)]
                .iter()
                .enumerate()
            {
                let r = 2 + i as u32;
                s(sheet, 1, r, cust);
                n(sheet, 2, r, *amount);
            }
        }
        {
            let sheet2 = book.new_sheet("Customers").unwrap();
            s(sheet2, 1, 1, "Id");
            s(sheet2, 2, 1, "Country");
            s(sheet2, 1, 2, "acme");
            s(sheet2, 2, 2, "DE");
            s(sheet2, 1, 3, "globex");
            s(sheet2, 2, 3, "FR");
        }
    });

    let result = extract_workbook_tables(
        buf,
        None,
        &ExtractConfig::default(),
        SheetTableMode::WholeSheet,
        "",
        None,
    )
    .unwrap();
    assert_eq!(result.tables.len(), 2);

    let ctx = SessionContext::new();
    for table in &result.tables {
        let name = table.name.clone().unwrap();
        table.register_with_datafusion(&ctx, &name).unwrap();
    }

    // Aggregation over one sheet-table
    let batches = ctx
        .sql("SELECT SUM(\"Amount\") AS total FROM orders")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let total = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<flow_like_storage::arrow::array::Float64Array>()
        .unwrap()
        .value(0);
    assert!((total - 350.0).abs() < 1e-9);

    // Join across the two sheet-tables
    let batches = ctx
        .sql(
            "SELECT c.\"Country\", SUM(o.\"Amount\") AS total \
             FROM orders o JOIN customers c ON o.\"Customer\" = c.\"Id\" \
             GROUP BY c.\"Country\" ORDER BY total DESC",
        )
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    assert_eq!(batches[0].num_rows(), 2);
}

// ============================ D. Styles ============================

#[test]
fn styles_load_classify_and_find() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        people_table(sheet, 1, 1);
        for col in ["A1", "B1", "C1"] {
            sheet.get_style_mut(col).get_font_mut().set_bold(true);
        }
        sheet.get_style_mut("A3").set_background_color("FF4CAF50"); // green
        sheet.get_style_mut("B3").set_background_color("FFFF0000"); // red
    });

    let all = load_workbook_styles(&buf).unwrap();
    let styles = all.get("Sheet1").expect("styles for Sheet1");
    assert!(!styles.is_empty());

    let a1 = styles.get_abs(0, 0).expect("A1 styled");
    assert!(a1.bold);
    let a3 = styles.get_abs(2, 0).expect("A3 styled");
    assert_eq!(a3.fill_color_name(), Some("green"));

    let bold_cells = styles.find(|s| s.bold);
    assert_eq!(bold_cells.len(), 3);
    let green_cells = styles.find(|s| s.fill_color_name() == Some("green"));
    assert_eq!(green_cells.len(), 1);
    assert_eq!((green_cells[0].0, green_cells[0].1), (2, 0));

    let summary = styles.summarize(8);
    assert!(summary.contains("bold"), "summary: {summary}");
    assert!(summary.contains("green"), "summary: {summary}");
}

#[test]
fn styles_fail_on_non_xlsx_bytes() {
    assert!(SheetStyles::load(b"Name,Age\nAlice,30\n", "Sheet1").is_err());
    assert!(load_workbook_styles(&[0u8; 32]).is_err());
}

#[test]
fn color_classification_palette() {
    assert_eq!(classify_color("4CAF50"), Some("green"));
    assert_eq!(classify_color("FF0000"), Some("red"));
    assert_eq!(classify_color("FFFFFF"), None);
}

// ============================ E. Sheet compressor ============================

fn read_only_grid(
    buf: Vec<u8>,
    sheet: &str,
) -> flow_like_catalog_data::data::excel::grid::SheetGrid {
    let mut wb = Workbook::open(buf).unwrap();
    wb.read_grid(sheet).unwrap()
}

#[test]
fn encoding_contains_structure_and_elides_homogeneous_runs() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 1, 1, "Id");
        s(sheet, 2, 1, "Value");
        for r in 2..=300u32 {
            n(sheet, 1, r, r as f64);
            n(sheet, 2, r, (r * 2) as f64);
        }
    });
    let grid = read_only_grid(buf, "Sheet1");
    let candidates = detect_table_regions(&grid, &ExtractConfig::default());
    assert_eq!(candidates.len(), 1);

    let encoding = encode_sheet_compact(
        &grid,
        &candidates,
        None,
        &EncodeOptions::default(),
        "Sheet1",
    );
    assert!(encoding.contains("used range A1:B300"), "{encoding}");
    assert!(encoding.contains("Heuristic table candidates"));
    assert!(encoding.contains("A1:B300"));
    assert!(encoding.contains("Row 1:"));
    assert!(encoding.contains("omitted"), "long runs must be elided");
    assert!(encoding.contains("Column profiles"));
    // The encoding must stay small even for 300 rows
    assert!(
        encoding.len() < 12_000,
        "encoding too large: {}",
        encoding.len()
    );
}

#[test]
fn render_range_respects_caps_and_addresses() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        people_table(sheet, 1, 1);
    });
    let grid = read_only_grid(buf, "Sheet1");
    let rect = Rect {
        r0: 0,
        c0: 0,
        r1: 3,
        c1: 2,
    };
    let out = render_range(&grid, None, &rect, 2, &EncodeOptions::default());
    assert!(out.contains("A1=\"Name\""));
    assert!(out.contains("omitted"), "row cap must elide: {out}");

    let full = render_range(&grid, None, &rect, 10, &EncodeOptions::default());
    assert!(full.contains("A4=\"Carol\""));
}

#[test]
fn inverted_index_finds_values_and_filters() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 1, 1, "Section A");
        people_table(sheet, 1, 3);
        s(sheet, 1, 9, "Section B");
    });
    let grid = read_only_grid(buf, "Sheet1");

    let all = encode_inverted_index(&grid, 300, None);
    assert!(all.contains("\"Alice\": A4"), "{all}");

    let filtered = encode_inverted_index(&grid, 300, Some("section"));
    assert!(filtered.contains("Section A"));
    assert!(filtered.contains("Section B"));
    assert!(!filtered.contains("Alice"));

    let missing = encode_inverted_index(&grid, 300, Some("zzz-not-there"));
    assert!(missing.contains("No text values containing"));
}

#[test]
fn style_annotations_render_bold_and_color() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        people_table(sheet, 1, 1);
        sheet.get_style_mut("A1").get_font_mut().set_bold(true);
        sheet.get_style_mut("A2").set_background_color("FF4CAF50");
    });
    let styles = SheetStyles::load(&buf, "Sheet1").unwrap();
    let grid = read_only_grid(buf, "Sheet1");
    let rect = Rect {
        r0: 0,
        c0: 0,
        r1: 1,
        c1: 2,
    };
    let out = render_range(&grid, Some(&styles), &rect, 10, &EncodeOptions::default());
    assert!(out.contains("**A1=\"Name\"**"), "bold annotation: {out}");
    assert!(
        out.contains("[green]A2=\"Alice\""),
        "fill annotation: {out}"
    );
}

// ============================ F. Scale & robustness ============================

#[test]
fn five_thousand_rows_extract_completely() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 1, 1, "Id");
        s(sheet, 2, 1, "Name");
        s(sheet, 3, 1, "Score");
        for r in 2..=5001u32 {
            n(sheet, 1, r, r as f64);
            s(sheet, 2, r, &format!("user_{r}"));
            n(sheet, 3, r, (r as f64) * 0.5);
        }
    });
    let start = std::time::Instant::now();
    let result = detect(buf);
    assert_eq!(result.tables.len(), 1);
    assert_eq!(result.tables[0].row_count(), 5000);
    assert!(
        start.elapsed().as_secs() < 30,
        "5k-row sheet took {:?}",
        start.elapsed()
    );
}

#[test]
fn wide_sheet_with_many_columns() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        for c in 1..=120u32 {
            s(sheet, c, 1, &format!("col_{c}"));
            for r in 2..=6u32 {
                n(sheet, c, r, (c * r) as f64);
            }
        }
    });
    let result = detect(buf);
    assert_eq!(result.tables.len(), 1);
    assert_eq!(result.tables[0].headers().len(), 120);
    assert_eq!(result.tables[0].row_count(), 5);
}

#[test]
fn whole_sheet_mode_still_peels_decoration() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        s(sheet, 1, 1, "Internal Report");
        people_table(sheet, 1, 3);
    });
    let grid = read_only_grid(buf, "Sheet1");
    let t = whole_sheet_table(&grid, &ExtractConfig::default()).unwrap();
    assert_eq!(t.title.as_deref(), Some("Internal Report"));
    assert_eq!(t.headers, vec!["Name", "Age", "City"]);
}

#[test]
fn extract_from_grid_matches_workbook_path() {
    let buf = build_xlsx(|book| {
        let sheet = book.get_sheet_mut(&0).unwrap();
        people_table(sheet, 1, 1);
    });
    let grid = read_only_grid(buf, "Sheet1");
    let tables = extract_tables_from_grid(&grid, &ExtractConfig::default());
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].headers, vec!["Name", "Age", "City"]);
    assert!(
        tables[0].confidence > 0.5,
        "confidence {}",
        tables[0].confidence
    );
}

#[test]
fn normalization_rules_hold() {
    assert_eq!(normalize_table_name("Sales Data (2024)"), "sales_data_2024");
    assert_eq!(normalize_table_name("2024"), "t_2024");
    assert_eq!(normalize_table_name("Ärzte/Notfälle"), "aerzte_notfaelle");
    assert_eq!(normalize_table_name("  "), "table");
    let long = "x".repeat(200);
    assert!(normalize_table_name(&long).len() <= 64);
}

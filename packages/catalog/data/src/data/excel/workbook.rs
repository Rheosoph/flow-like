//! Context-free xlsx builder: new workbooks from plain rows.

use umya_spreadsheet::Spreadsheet;

#[derive(Debug, Clone, PartialEq)]
pub enum XlsxCell {
    Empty,
    Text(String),
    Number(f64),
    Bool(bool),
    /// Without the leading `=`.
    Formula(String),
}

impl XlsxCell {
    /// A string starting with `=` becomes a formula.
    pub fn from_text(text: &str) -> Self {
        match text.strip_prefix('=') {
            Some(formula) if !formula.trim().is_empty() => Self::Formula(formula.trim().to_string()),
            _ => Self::Text(text.to_string()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct XlsxSheet {
    pub name: String,
    pub rows: Vec<Vec<XlsxCell>>,
    /// Bold, shaded first row.
    pub header: bool,
    /// Column widths in characters, from column A.
    pub widths: Vec<f64>,
    /// Number format code per column (e.g. `0.00`, `#,##0`, `0%`, `yyyy-mm-dd`), from column A;
    /// empty entries keep General.
    pub formats: Vec<String>,
}

const INVALID_SHEET_CHARS: [char; 7] = ['\\', '/', '?', '*', '[', ']', ':'];

pub fn clean_sheet_name(name: &str, position: usize) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !INVALID_SHEET_CHARS.contains(c))
        .take(31)
        .collect();
    let cleaned = cleaned.trim().trim_matches('\'').to_string();
    if cleaned.is_empty() {
        format!("Sheet{}", position + 1)
    } else {
        cleaned
    }
}

fn unique_names(sheets: &[XlsxSheet]) -> Vec<String> {
    let mut names: Vec<String> = Vec::with_capacity(sheets.len());
    for (position, sheet) in sheets.iter().enumerate() {
        let base = clean_sheet_name(&sheet.name, position);
        let mut candidate = base.clone();
        let mut suffix = 2;
        while names.iter().any(|n| n.eq_ignore_ascii_case(&candidate)) {
            let tail = format!(" ({suffix})");
            let keep = 31usize.saturating_sub(tail.chars().count());
            candidate = format!("{}{tail}", base.chars().take(keep).collect::<String>());
            suffix += 1;
        }
        names.push(candidate);
    }
    names
}

fn fill_sheet(book: &mut Spreadsheet, index: usize, sheet: &XlsxSheet) -> flow_like_types::Result<()> {
    let worksheet = book
        .get_sheet_mut(&index)
        .ok_or_else(|| flow_like_types::anyhow!("The workbook lost sheet {index}"))?;
    for (row_index, row) in sheet.rows.iter().enumerate() {
        let row_number = row_index as u32 + 1;
        for (column_index, cell) in row.iter().enumerate() {
            let column_number = column_index as u32 + 1;
            let target = worksheet.get_cell_mut((column_number, row_number));
            match cell {
                XlsxCell::Empty => continue,
                XlsxCell::Text(text) => {
                    target.set_value_string(text.clone());
                }
                XlsxCell::Number(number) => {
                    target.set_value_number(*number);
                }
                XlsxCell::Bool(flag) => {
                    target.set_value_bool(*flag);
                }
                XlsxCell::Formula(formula) => {
                    target.set_formula(formula.clone());
                }
            }
            let is_header = sheet.header && row_index == 0;
            if !is_header
                && let Some(format) = sheet.formats.get(column_index).filter(|f| !f.is_empty())
                && !matches!(cell, XlsxCell::Text(_))
            {
                target.get_style_mut().get_number_format_mut().set_format_code(format.clone());
            }
            if is_header {
                let style = target.get_style_mut();
                style.get_font_mut().set_bold(true);
                style.set_background_color_solid("FFF3F4F6");
            }
        }
    }
    for (column_index, width) in sheet.widths.iter().enumerate() {
        if *width > 0.0 {
            worksheet
                .get_column_dimension_by_number_mut(&(column_index as u32 + 1))
                .set_width(width.min(255.0));
        }
    }
    Ok(())
}

pub fn build_workbook(sheets: &[XlsxSheet]) -> flow_like_types::Result<Vec<u8>> {
    if sheets.is_empty() {
        return Err(flow_like_types::anyhow!("A workbook needs at least one sheet"));
    }
    let names = unique_names(sheets);
    let mut book = umya_spreadsheet::new_file_empty_worksheet();
    for name in &names {
        book.new_sheet(name.clone())
            .map_err(|error| flow_like_types::anyhow!("Cannot add sheet '{name}': {error}"))?;
    }
    for (index, sheet) in sheets.iter().enumerate() {
        fill_sheet(&mut book, index, sheet)?;
    }
    let mut bytes = Vec::new();
    umya_spreadsheet::writer::xlsx::write_writer(&book, &mut bytes)
        .map_err(|error| flow_like_types::anyhow!("Failed to serialize the workbook: {error}"))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<XlsxSheet> {
        vec![XlsxSheet {
            name: "Sales: Q1".into(),
            rows: vec![
                vec![XlsxCell::from_text("Item"), XlsxCell::from_text("Price")],
                vec![XlsxCell::from_text("A"), XlsxCell::Number(1234.5)],
                vec![XlsxCell::from_text("Total"), XlsxCell::from_text("=SUM(B2:B2)")],
            ],
            header: true,
            widths: vec![20.0, 12.0],
            formats: vec![String::new(), "#,##0.00".into()],
        }]
    }

    #[test]
    fn builds_a_workbook_with_formula_format_and_header() {
        let bytes = build_workbook(&sample()).unwrap();
        let book = umya_spreadsheet::reader::xlsx::read_reader(std::io::Cursor::new(&bytes), true).unwrap();
        let sheet = book.get_sheet_by_name("Sales Q1").expect("sheet name is cleaned");
        assert_eq!(sheet.get_cell((2, 3)).unwrap().get_formula(), "SUM(B2:B2)");
        assert_eq!(sheet.get_cell((2, 2)).unwrap().get_value(), "1234.5");
        let format = sheet
            .get_cell((2, 2))
            .unwrap()
            .get_style()
            .get_number_format()
            .unwrap()
            .get_format_code()
            .to_string();
        assert_eq!(format, "#,##0.00");
        let bold = sheet.get_cell((1, 1)).unwrap().get_style().get_font().map(|f| *f.get_bold());
        assert_eq!(bold, Some(true));
        assert!(sheet.get_column_dimension("A").is_some_and(|c| *c.get_width() >= 20.0));
    }

    #[test]
    fn sheet_names_are_unique_and_valid() {
        let sheets = vec![
            XlsxSheet { name: "Data".into(), ..Default::default() },
            XlsxSheet { name: "data".into(), ..Default::default() },
            XlsxSheet { name: "///".into(), ..Default::default() },
        ];
        assert_eq!(unique_names(&sheets), vec!["Data", "data (2)", "Sheet3"]);
    }

    #[test]
    fn rejects_an_empty_workbook() {
        assert!(build_workbook(&[]).is_err());
    }
}

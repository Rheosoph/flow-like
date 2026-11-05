//! Native DrawingML charts shared by decks and Word documents, with the chart data embedded as a
//! workbook so "Edit data" works in PowerPoint and Word.

use std::collections::HashMap;

use crate::document::styles::{defaults, hex_to_ooxml};

use super::{add_override, add_relationship, xml_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChartKind {
    #[default]
    Bar,
    Line,
    Pie,
}

#[derive(Debug, Clone, Default)]
pub struct ChartSpec {
    pub kind: ChartKind,
    pub categories: Vec<String>,
    pub series: Vec<(String, Vec<f64>)>,
    pub title: Option<String>,
    pub stacked: bool,
    pub horizontal: bool,
    /// Series colours as hex; the Flow Like office palette when empty.
    pub colors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkbookCell {
    Empty,
    Text(String),
    Number(f64),
}

/// Builds the workbook that is embedded behind a chart. The caller supplies it so this crate
/// does not link a spreadsheet writer; without one, charts carry literal data and no
/// "Edit data".
pub type WorkbookFn = fn(&[Vec<WorkbookCell>]) -> flow_like_types::Result<Vec<u8>>;

pub const SHEET_NAME: &str = "Sheet1";
const REL_CHART_PACKAGE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/package";
const XLSX_TYPE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const CHART_TYPE: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";

fn column_letter(index: usize) -> String {
    let mut n = index + 1;
    let mut letters = Vec::new();
    while n > 0 {
        n -= 1;
        letters.push((b'A' + (n % 26) as u8) as char);
        n /= 26;
    }
    letters.iter().rev().collect()
}

/// Row 1 is the series names, column A the categories.
pub fn workbook_rows(spec: &ChartSpec) -> Vec<Vec<WorkbookCell>> {
    let mut header = vec![WorkbookCell::Empty];
    header.extend(spec.series.iter().map(|(name, _)| WorkbookCell::Text(name.clone())));
    let mut rows = vec![header];
    for (index, category) in spec.categories.iter().enumerate() {
        let mut row = vec![WorkbookCell::Text(category.clone())];
        row.extend(spec.series.iter().map(|(_, values)| {
            values
                .get(index)
                .map_or(WorkbookCell::Empty, |v| WorkbookCell::Number(*v))
        }));
        rows.push(row);
    }
    rows
}

fn string_cache(values: impl Iterator<Item = String>) -> String {
    let points: Vec<String> = values
        .enumerate()
        .map(|(i, v)| format!("<c:pt idx=\"{i}\"><c:v>{}</c:v></c:pt>", xml_text(&v)))
        .collect();
    format!("<c:ptCount val=\"{}\"/>{}", points.len(), points.concat())
}

fn number_cache(values: &[f64]) -> String {
    let points: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!("<c:pt idx=\"{i}\"><c:v>{v}</c:v></c:pt>"))
        .collect();
    format!(
        "<c:formatCode>General</c:formatCode><c:ptCount val=\"{}\"/>{points}",
        values.len()
    )
}

fn series_xml(spec: &ChartSpec, index: usize, name: &str, values: &[f64], linked: bool) -> String {
    let palette = defaults::CHART_COLORS;
    let color = hex_to_ooxml(
        spec.colors
            .get(index)
            .map(String::as_str)
            .unwrap_or(palette[index % palette.len()]),
    );
    let count = spec.categories.len();
    let column = column_letter(index + 1);
    let (tx, cat, val) = if linked {
        (
            format!(
                "<c:tx><c:strRef><c:f>{SHEET_NAME}!${column}$1</c:f><c:strCache>{}</c:strCache></c:strRef></c:tx>",
                string_cache(std::iter::once(name.to_string()))
            ),
            format!(
                "<c:cat><c:strRef><c:f>{SHEET_NAME}!$A$2:$A${}</c:f><c:strCache>{}</c:strCache></c:strRef></c:cat>",
                count + 1,
                string_cache(spec.categories.iter().cloned())
            ),
            format!(
                "<c:val><c:numRef><c:f>{SHEET_NAME}!${column}$2:${column}${}</c:f><c:numCache>{}</c:numCache></c:numRef></c:val>",
                count + 1,
                number_cache(values)
            ),
        )
    } else {
        (
            format!("<c:tx><c:v>{}</c:v></c:tx>", xml_text(name)),
            format!(
                "<c:cat><c:strLit>{}</c:strLit></c:cat>",
                string_cache(spec.categories.iter().cloned())
            ),
            format!("<c:val><c:numLit>{}</c:numLit></c:val>", number_cache(values)),
        )
    };
    let style = match spec.kind {
        ChartKind::Line => format!(
            "<c:spPr><a:ln w=\"28575\"><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill></a:ln></c:spPr><c:marker><c:symbol val=\"circle\"/><c:size val=\"6\"/></c:marker>"
        ),
        ChartKind::Pie => {
            let slices: String = (0..count)
                .map(|i| {
                    let slice = hex_to_ooxml(
                        spec.colors
                            .get(i)
                            .map(String::as_str)
                            .unwrap_or(palette[i % palette.len()]),
                    );
                    format!(
                        "<c:dPt><c:idx val=\"{i}\"/><c:bubble3D val=\"0\"/><c:spPr><a:solidFill><a:srgbClr val=\"{slice}\"/></a:solidFill></c:spPr></c:dPt>"
                    )
                })
                .collect();
            format!("<c:spPr><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill></c:spPr>{slices}")
        }
        ChartKind::Bar => format!(
            "<c:spPr><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill></c:spPr><c:invertIfNegative val=\"0\"/>"
        ),
    };
    let smooth = if spec.kind == ChartKind::Line { "<c:smooth val=\"0\"/>" } else { "" };
    format!(
        "<c:ser><c:idx val=\"{index}\"/><c:order val=\"{index}\"/>{tx}{style}{cat}{val}{smooth}</c:ser>"
    )
}

/// The chart part. `linked` points the series at the embedded workbook's cells.
pub fn chart_xml(spec: &ChartSpec, linked: bool) -> String {
    let series: String = spec
        .series
        .iter()
        .enumerate()
        .map(|(i, (name, values))| series_xml(spec, i, name, values, linked))
        .collect();
    let axes = "<c:axId val=\"111111\"/><c:axId val=\"222222\"/>";
    let plot = match spec.kind {
        ChartKind::Bar => {
            let (direction, grouping, overlap) = (
                if spec.horizontal { "bar" } else { "col" },
                if spec.stacked { "stacked" } else { "clustered" },
                if spec.stacked { "<c:overlap val=\"100\"/>" } else { "" },
            );
            format!(
                "<c:barChart><c:barDir val=\"{direction}\"/><c:grouping val=\"{grouping}\"/><c:varyColors val=\"0\"/>{series}<c:gapWidth val=\"80\"/>{overlap}{axes}</c:barChart>"
            )
        }
        ChartKind::Line => format!(
            "<c:lineChart><c:grouping val=\"{}\"/><c:varyColors val=\"0\"/>{series}<c:marker val=\"1\"/>{axes}</c:lineChart>",
            if spec.stacked { "stacked" } else { "standard" }
        ),
        ChartKind::Pie => format!(
            "<c:pieChart><c:varyColors val=\"1\"/>{series}<c:firstSliceAng val=\"0\"/></c:pieChart>"
        ),
    };
    let (category_position, value_position) = if spec.horizontal && spec.kind == ChartKind::Bar {
        ("l", "b")
    } else {
        ("b", "l")
    };
    let axis_defs = if spec.kind == ChartKind::Pie {
        String::new()
    } else {
        format!(
            "<c:catAx><c:axId val=\"111111\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"{category_position}\"/><c:numFmt formatCode=\"General\" sourceLinked=\"0\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/><c:crossAx val=\"222222\"/><c:crosses val=\"autoZero\"/><c:auto val=\"1\"/><c:lblAlgn val=\"ctr\"/><c:lblOffset val=\"100\"/><c:noMultiLvlLbl val=\"0\"/></c:catAx><c:valAx><c:axId val=\"222222\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"{value_position}\"/><c:majorGridlines/><c:numFmt formatCode=\"General\" sourceLinked=\"0\"/><c:majorTickMark val=\"none\"/><c:minorTickMark val=\"none\"/><c:tickLblPos val=\"nextTo\"/><c:crossAx val=\"111111\"/><c:crosses val=\"autoZero\"/><c:crossBetween val=\"between\"/></c:valAx>"
        )
    };
    let legend = if spec.series.len() > 1 || spec.kind == ChartKind::Pie {
        "<c:legend><c:legendPos val=\"b\"/><c:overlay val=\"0\"/></c:legend>"
    } else {
        ""
    };
    let title = spec
        .title
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .map(|t| {
            format!(
                "<c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"en-US\" sz=\"1400\" b=\"1\"/><a:t>{}</a:t></a:r></a:p></c:rich></c:tx><c:overlay val=\"0\"/></c:title><c:autoTitleDeleted val=\"0\"/>",
                xml_text(t)
            )
        })
        .unwrap_or_else(|| "<c:autoTitleDeleted val=\"1\"/>".to_string());
    let external = if linked {
        "<c:externalData r:id=\"rId1\"><c:autoUpdate val=\"0\"/></c:externalData>"
    } else {
        ""
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><c:roundedCorners val=\"0\"/><c:chart>{title}<c:plotArea><c:layout/>{plot}{axis_defs}</c:plotArea>{legend}<c:plotVisOnly val=\"1\"/></c:chart>{external}</c:chartSpace>"
    )
}

/// Adds `<root>/charts/chartN.xml` (and, with a workbook builder, its embedded
/// `<root>/embeddings/Microsoft_Excel_SheetN.xlsx` plus rels and content types). `root` is `ppt`
/// or `word`. Returns the part's path relative to `root`.
pub fn add_chart_parts(
    files: &mut HashMap<String, Vec<u8>>,
    root: &str,
    number: u32,
    spec: &ChartSpec,
    workbook: Option<WorkbookFn>,
) -> flow_like_types::Result<String> {
    let embedded = workbook
        .map(|build| build(&workbook_rows(spec)))
        .transpose()?;
    let part = format!("charts/chart{number}.xml");
    files.insert(
        format!("{root}/{part}"),
        chart_xml(spec, embedded.is_some()).into_bytes(),
    );
    add_override(files, &format!("{root}/{part}"), CHART_TYPE);
    if let Some(bytes) = embedded {
        let embedding = format!("embeddings/Microsoft_Excel_Sheet{number}.xlsx");
        files.insert(format!("{root}/{embedding}"), bytes);
        super::ensure_default_content_type_as(files, "xlsx", XLSX_TYPE);
        add_relationship(
            files,
            &format!("{root}/charts/_rels/chart{number}.xml.rels"),
            "rId1",
            REL_CHART_PACKAGE,
            &format!("../{embedding}"),
        );
    }
    Ok(part)
}

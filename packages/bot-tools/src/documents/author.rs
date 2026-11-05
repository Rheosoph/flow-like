//! `document_create` and `document_fill`: new Word, Excel, PowerPoint and PDF files in the
//! bot's workspace. Both only ever write a NEW file under `bot/exports/`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use flow_like_types::Bytes;
use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
use pulldown_cmark::{Event, Parser, Tag};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::backend::{
    BuiltDocument, ChartKind, DeckChart, DeckSlide, DocumentBackend, FillRows, MarkdownDocument,
    SheetCell, SheetData, SlideLayoutKind, TemplateFill,
};
use super::{backend_of, blocking, stem_of};
use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::files::{FileRef, guess_mime, handle_for, resolve, workspace_of};
use crate::layout::{EXPORTS_DIR, sanitize_name};
use crate::outcome::{ToolImage, ToolOutput};

pub const MAX_MARKDOWN_BYTES: usize = 500_000;
pub const MAX_SLIDES: usize = 100;
pub const MAX_SHEETS: usize = 50;
pub const MAX_CELLS: usize = 100_000;
pub const MAX_IMAGES: usize = 20;
pub const MAX_TABLE_CELLS: usize = 2_000;
pub const MAX_CHART_POINTS: usize = 1_000;
pub const MAX_FILL_ENTRIES: usize = 200;
const MAX_UNIQUE_NAMES: u32 = 999;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DocumentFormat {
    Pdf,
    Docx,
    Pptx,
    Xlsx,
}

impl DocumentFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Pptx => "pptx",
            Self::Xlsx => "xlsx",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Docx => {
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            }
            Self::Pptx => {
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            }
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        }
    }

    fn unit(self) -> &'static str {
        match self {
            Self::Pdf => "pages",
            Self::Docx => "pages",
            Self::Pptx => "slides",
            Self::Xlsx => "sheets",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SlideTable {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChartSeries {
    pub name: String,
    pub values: Vec<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SlideChart {
    pub kind: ChartKind,
    pub categories: Vec<String>,
    pub series: Vec<ChartSeries>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SlideArgs {
    /// `title` (big title, bullets become the subtitle), `bullets` (default), `image`, `table` or
    /// `chart`. An image, table or chart slide shows that content under the title.
    #[serde(default)]
    pub layout: Option<SlideLayoutKind>,
    pub title: String,
    #[serde(default)]
    pub bullets: Vec<String>,
    /// Speaker notes.
    #[serde(default)]
    pub notes: Option<String>,
    /// A workspace image (path or handle) for layout `image`.
    #[serde(default)]
    pub image: Option<FileRef>,
    #[serde(default)]
    pub table: Option<SlideTable>,
    #[serde(default)]
    pub chart: Option<SlideChart>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SheetArgs {
    pub name: String,
    /// Rows of cells. A string starting with `=` is a formula (e.g. "=SUM(B2:B9)"); numbers and
    /// booleans keep their type; null leaves a cell empty.
    pub rows: Vec<Vec<Value>>,
    /// Bold, shaded first row.
    #[serde(default)]
    pub header: Option<bool>,
    /// Column widths in characters, from column A.
    #[serde(default)]
    pub widths: Vec<f64>,
    /// Number format per column, from column A: "0.00", "#,##0", "0%", "yyyy-mm-dd", ""
    /// for General.
    #[serde(default)]
    pub formats: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DocumentCreateArgs {
    pub format: DocumentFormat,
    /// File name without a folder; the extension is added. The file is saved as a NEW file in
    /// bot/exports/ (an existing name gets a number appended), never overwriting.
    pub name: String,
    /// pdf and docx: the content as markdown (headings, lists, tables, code, quotes, images
    /// `![alt](bot/files/pic.png)` from workspace files). PDF also draws ```nivo / ```plotly CSV
    /// chart blocks.
    #[serde(default)]
    pub markdown: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    /// pdf and docx: title block on its own first page.
    #[serde(default)]
    pub cover: Option<bool>,
    /// pdf and docx: a4 (default), letter, legal, a5 or a3.
    #[serde(default)]
    pub page_size: Option<String>,
    #[serde(default)]
    pub page_numbers: Option<bool>,
    /// pptx: at most 100 slides.
    #[serde(default)]
    pub slides: Option<Vec<SlideArgs>>,
    /// xlsx: at most 50 sheets and 100000 cells in all.
    #[serde(default)]
    pub sheets: Option<Vec<SheetArgs>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DocumentFillArgs {
    /// The template: a .docx or .pptx with {{placeholders}}, or a PDF with form fields. A file in
    /// the workspace, a handle, or another app's file (needs the storage capability).
    pub template: FileRef,
    /// Placeholder name (for {{customer}} give "customer") or PDF form field name, and the text.
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    /// Read the values as inline markdown (bold, italic, links); Word and PowerPoint.
    #[serde(default)]
    pub markdown: Option<bool>,
    /// Picture alt text or shape name, and the new image.
    #[serde(default)]
    pub images: BTreeMap<String, FileRef>,
    /// Word: a placeholder inside a template table row ("item" for {{item}}) and the rows as
    /// objects whose keys fill the row's other {{column}} placeholders. PowerPoint: a placeholder
    /// inside a table and the whole table as lists of cell texts, header row first.
    #[serde(default)]
    pub tables: BTreeMap<String, Vec<Value>>,
    /// Name of the new file; defaults to the template's name plus "-filled".
    #[serde(default)]
    pub name: Option<String>,
}

fn too_many(what: &str, count: usize, max: usize) -> ToolError {
    ToolError::invalid(format!(
        "{count} {what} are more than the limit of {max}; split the work over several files"
    ))
}

fn require<'a, T>(value: &'a Option<T>, field: &str, format: DocumentFormat) -> ToolResult<&'a T> {
    value.as_ref().ok_or_else(|| {
        ToolError::invalid(format!(
            "A {} needs '{field}'",
            format.extension()
        ))
    })
}

pub fn export_stem(name: &str, extension: &str) -> ToolResult<String> {
    let clean = sanitize_name(name);
    let suffix = format!(".{extension}");
    let stem = if clean.to_lowercase().ends_with(&suffix) {
        &clean[..clean.len() - suffix.len()]
    } else {
        clean.as_str()
    };
    let stem = stem.trim().trim_matches('.');
    if stem.is_empty() || name.trim().is_empty() {
        return Err(ToolError::invalid("The file needs a name"));
    }
    Ok(stem.to_string())
}

/// The first `bot/exports/<stem>[-N].<ext>` that does not exist yet.
pub async fn unique_export_path(
    env: &dyn ToolEnv,
    workspace: &str,
    stem: &str,
    extension: &str,
) -> ToolResult<String> {
    for number in 1..=MAX_UNIQUE_NAMES {
        let file = if number == 1 {
            format!("{stem}.{extension}")
        } else {
            format!("{stem}-{number}.{extension}")
        };
        let path = format!("{EXPORTS_DIR}/{file}");
        if true || env.stat_file(workspace, &path).await?.is_none() {
            return Ok(path);
        }
    }
    Err(ToolError::new(
        code::ALREADY_HERE,
        format!("bot/exports/ already holds {MAX_UNIQUE_NAMES} files named '{stem}'; pick another name"),
    ))
}

fn nonempty(text: &Option<String>) -> Option<String> {
    text.as_ref().filter(|t| !t.trim().is_empty()).cloned()
}

fn image_urls(markdown: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    Parser::new(markdown)
        .filter_map(|event| match event {
            Event::Start(Tag::Image { dest_url, .. }) => Some(dest_url.to_string()),
            _ => None,
        })
        .filter(|url| !url.is_empty() && seen.insert(url.clone()))
        .collect()
}

async fn markdown_images(
    env: &dyn ToolEnv,
    markdown: &str,
    warnings: &mut Vec<String>,
) -> ToolResult<BTreeMap<String, Bytes>> {
    let urls = image_urls(markdown);
    if urls.len() > MAX_IMAGES {
        return Err(too_many("images", urls.len(), MAX_IMAGES));
    }
    let max = env.limits().max_read_bytes;
    let mut images = BTreeMap::new();
    for url in urls {
        if let Some(rest) = url.strip_prefix("data:") {
            match rest.split_once(";base64,").map(|(_, data)| STANDARD.decode(data.trim())) {
                Some(Ok(bytes)) => {
                    images.insert(url, Bytes::from(bytes));
                }
                _ => warnings.push("A data: image is not valid base64 and was skipped".to_string()),
            }
            continue;
        }
        if url.starts_with("http://") || url.starts_with("https://") {
            warnings.push(format!(
                "Image {url} is a web address: download it into the workspace first (file tools), then reference the workspace path"
            ));
            continue;
        }
        let loaded = match FileRef::parse(&url) {
            Ok(reference) => resolve(env, &reference, max).await,
            Err(error) => Err(error),
        };
        match loaded {
            Ok(file) => {
                images.insert(url, file.bytes);
            }
            Err(error) => warnings.push(format!("Image '{url}' was skipped: {}", error.message)),
        }
    }
    Ok(images)
}

fn markdown_document(
    args: &DocumentCreateArgs,
    markdown: &str,
    images: BTreeMap<String, Bytes>,
) -> ToolResult<MarkdownDocument> {
    if markdown.trim().is_empty() {
        return Err(ToolError::invalid("The markdown is empty"));
    }
    if markdown.len() > MAX_MARKDOWN_BYTES {
        return Err(ToolError::new(
            code::TOO_LARGE,
            format!(
                "The markdown is {} bytes; at most {MAX_MARKDOWN_BYTES} fit one document",
                markdown.len()
            ),
        ));
    }
    Ok(MarkdownDocument {
        markdown: markdown.to_string(),
        title: nonempty(&args.title),
        subtitle: nonempty(&args.subtitle),
        author: nonempty(&args.author),
        cover: args.cover.unwrap_or(false),
        page_size: nonempty(&args.page_size),
        page_numbers: args.page_numbers.unwrap_or(false),
        images,
    })
}

fn cell_of(value: &Value) -> ToolResult<SheetCell> {
    Ok(match value {
        Value::Null => SheetCell::Empty,
        Value::Bool(flag) => SheetCell::Bool(*flag),
        Value::Number(number) => SheetCell::Number(number.as_f64().ok_or_else(|| {
            ToolError::invalid(format!("The number {number} does not fit a spreadsheet cell"))
        })?),
        Value::String(text) => match text.strip_prefix('=') {
            Some(formula) if !formula.trim().is_empty() => {
                SheetCell::Formula(formula.trim().to_string())
            }
            _ => SheetCell::Text(text.clone()),
        },
        other => {
            return Err(ToolError::invalid(format!(
                "A cell is text, a number, true/false or null, not {other}"
            )));
        }
    })
}

fn sheets_of(sheets: &[SheetArgs]) -> ToolResult<Vec<SheetData>> {
    if sheets.is_empty() {
        return Err(ToolError::invalid("An xlsx needs at least one sheet"));
    }
    if sheets.len() > MAX_SHEETS {
        return Err(too_many("sheets", sheets.len(), MAX_SHEETS));
    }
    let cells: usize = sheets
        .iter()
        .map(|sheet| sheet.rows.iter().map(Vec::len).sum::<usize>())
        .sum();
    if cells > MAX_CELLS {
        return Err(too_many("cells", cells, MAX_CELLS));
    }
    sheets
        .iter()
        .map(|sheet| {
            let rows = sheet
                .rows
                .iter()
                .map(|row| row.iter().map(cell_of).collect::<ToolResult<Vec<_>>>())
                .collect::<ToolResult<Vec<_>>>()?;
            Ok(SheetData {
                name: sheet.name.clone(),
                rows,
                header: sheet.header.unwrap_or(false),
                widths: sheet.widths.clone(),
                formats: sheet.formats.clone(),
            })
        })
        .collect()
}

async fn slide_of(
    env: &dyn ToolEnv,
    position: usize,
    slide: &SlideArgs,
) -> ToolResult<DeckSlide> {
    let number = position + 1;
    let layout = slide.layout.unwrap_or(match (&slide.image, &slide.table, &slide.chart) {
        (Some(_), _, _) => SlideLayoutKind::Image,
        (_, Some(_), _) => SlideLayoutKind::Table,
        (_, _, Some(_)) => SlideLayoutKind::Chart,
        _ => SlideLayoutKind::Bullets,
    });
    let missing = |what: &str| {
        ToolError::invalid(format!("Slide {number} has layout {layout:?} but no {what}"))
    };
    let mut image = None;
    if layout == SlideLayoutKind::Image || slide.image.is_some() {
        let reference = slide.image.as_ref().ok_or_else(|| missing("image"))?;
        let file = resolve(env, reference, env.limits().max_read_bytes).await?;
        image = Some(file.bytes);
    }
    let table = match (&slide.table, layout) {
        (Some(table), _) => {
            let cells = table.rows.iter().map(Vec::len).sum::<usize>() + table.headers.len();
            if cells > MAX_TABLE_CELLS {
                return Err(too_many(
                    &format!("cells in the table of slide {number}"),
                    cells,
                    MAX_TABLE_CELLS,
                ));
            }
            Some((table.headers.clone(), table.rows.clone()))
        }
        (None, SlideLayoutKind::Table) => return Err(missing("table")),
        _ => None,
    };
    let chart = match (&slide.chart, layout) {
        (Some(chart), _) => {
            if chart.categories.is_empty() || chart.series.is_empty() {
                return Err(ToolError::invalid(format!(
                    "The chart of slide {number} needs categories and at least one series"
                )));
            }
            let points = chart.categories.len() * chart.series.len();
            if points > MAX_CHART_POINTS {
                return Err(too_many(
                    &format!("chart points on slide {number}"),
                    points,
                    MAX_CHART_POINTS,
                ));
            }
            if let Some(series) = chart
                .series
                .iter()
                .find(|s| s.values.len() != chart.categories.len() || s.values.iter().any(|v| !v.is_finite()))
            {
                return Err(ToolError::invalid(format!(
                    "Series '{}' on slide {number} needs one finite value per category ({})",
                    series.name,
                    chart.categories.len()
                )));
            }
            Some(DeckChart {
                kind: chart.kind,
                categories: chart.categories.clone(),
                series: chart
                    .series
                    .iter()
                    .map(|s| (s.name.clone(), s.values.clone()))
                    .collect(),
            })
        }
        (None, SlideLayoutKind::Chart) => return Err(missing("chart")),
        _ => None,
    };
    Ok(DeckSlide {
        layout,
        title: slide.title.clone(),
        bullets: slide.bullets.clone(),
        notes: nonempty(&slide.notes),
        image,
        table,
        chart,
    })
}

async fn save_new(
    env: &dyn ToolEnv,
    workspace: &str,
    stem: &str,
    format_extension: &str,
    mime: &str,
    bytes: Bytes,
) -> ToolResult<flow_like_types::bot_message::WorkspaceHandle> {
    let max = env.limits().max_write_bytes;
    if bytes.len() as u64 > max {
        return Err(ToolError::new(
            code::TOO_LARGE,
            format!("The file is {} bytes; the limit for a written file is {max}", bytes.len()),
        ));
    }
    let path = unique_export_path(env, workspace, stem, format_extension).await?;
    env.write_workspace(&path, bytes.clone(), mime).await?;
    Ok(handle_for(workspace, &path, &bytes, mime))
}

async fn preview(
    backend: &Arc<dyn DocumentBackend>,
    env: &dyn ToolEnv,
    pdf: &Bytes,
    label: &str,
    output: &mut ToolOutput,
) {
    let (backend, bytes, max_pixels) = (backend.clone(), pdf.to_vec(), env.limits().pdf_max_pixels);
    match blocking(move || backend.render_pages(bytes, &[1], max_pixels)).await {
        Ok(pages) => {
            if let Some(page) = pages.into_iter().next() {
                output.images.push(ToolImage {
                    label: format!("{label} page 1"),
                    mime: "image/png".to_string(),
                    data: page.png,
                });
            }
        }
        Err(error) => output
            .warnings
            .push(format!("The page 1 preview could not be rendered: {}", error.message)),
    }
}

async fn build(
    env: &dyn ToolEnv,
    backend: &Arc<dyn DocumentBackend>,
    args: &DocumentCreateArgs,
    warnings: &mut Vec<String>,
) -> ToolResult<BuiltDocument> {
    let format = args.format;
    let reject = |field: &str, present: bool, allowed: &str| {
        if present {
            return Err(ToolError::invalid(format!(
                "'{field}' only applies to {allowed}, not to a {}",
                format.extension()
            )));
        }
        Ok(())
    };
    match format {
        DocumentFormat::Pdf | DocumentFormat::Docx => {
            reject("slides", args.slides.is_some(), "pptx")?;
            reject("sheets", args.sheets.is_some(), "xlsx")?;
            let markdown = require(&args.markdown, "markdown", format)?;
            let images = markdown_images(env, markdown, warnings).await?;
            let request = markdown_document(args, markdown, images)?;
            let backend = backend.clone();
            blocking(move || {
                if format == DocumentFormat::Pdf {
                    backend.create_pdf(request)
                } else {
                    backend.create_docx(request)
                }
            })
            .await
        }
        DocumentFormat::Pptx => {
            reject("markdown", args.markdown.is_some(), "pdf and docx")?;
            reject("sheets", args.sheets.is_some(), "xlsx")?;
            let slides = require(&args.slides, "slides", format)?;
            if slides.is_empty() {
                return Err(ToolError::invalid("A pptx needs at least one slide"));
            }
            if slides.len() > MAX_SLIDES {
                return Err(too_many("slides", slides.len(), MAX_SLIDES));
            }
            if slides.iter().filter(|s| s.image.is_some()).count() > MAX_IMAGES {
                return Err(too_many("slide images", slides.len(), MAX_IMAGES));
            }
            let mut deck = Vec::with_capacity(slides.len());
            for (position, slide) in slides.iter().enumerate() {
                deck.push(slide_of(env, position, slide).await?);
            }
            let (title, author, backend) = (nonempty(&args.title), nonempty(&args.author), backend.clone());
            blocking(move || backend.create_pptx(title, author, deck)).await
        }
        DocumentFormat::Xlsx => {
            reject("markdown", args.markdown.is_some(), "pdf and docx")?;
            reject("slides", args.slides.is_some(), "pptx")?;
            let sheets = sheets_of(require(&args.sheets, "sheets", format)?)?;
            let backend = backend.clone();
            blocking(move || backend.create_xlsx(sheets)).await
        }
    }
}

pub async fn document_create(
    env: &dyn ToolEnv,
    args: DocumentCreateArgs,
) -> ToolResult<ToolOutput> {
    let workspace = workspace_of(env, "keep created documents")?;
    let format = args.format;
    let stem = export_stem(&args.name, format.extension())?;
    let backend = backend_of(env)?;
    let mut warnings = Vec::new();
    let built = build(env, &backend, &args, &mut warnings).await?;
    warnings.extend(built.warnings);
    let handle = save_new(env, &workspace, &stem, format.extension(), format.mime(), built.bytes.clone()).await?;

    let mut result = json!({ "format": format.extension(), "handle": handle });
    if let Some(count) = built.count {
        result[format.unit()] = json!(count);
    }
    let mut output = ToolOutput::new(Value::Null).saved(handle.clone());
    if format == DocumentFormat::Pdf {
        preview(&backend, env, &built.bytes, &stem, &mut output).await;
    }
    output.warnings.extend(warnings);
    output.result = result;
    Ok(output)
}

fn text_of(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn fill_rows(key: &str, rows: &[Value]) -> ToolResult<FillRows> {
    if rows.iter().all(Value::is_object) {
        let records = rows
            .iter()
            .map(|row| {
                row.as_object()
                    .into_iter()
                    .flatten()
                    .map(|(column, value)| (column.clone(), text_of(value)))
                    .collect()
            })
            .collect();
        return Ok(FillRows::Records(records));
    }
    if rows.iter().all(Value::is_array) {
        let grid = rows
            .iter()
            .map(|row| row.as_array().into_iter().flatten().map(text_of).collect())
            .collect();
        return Ok(FillRows::Grid(grid));
    }
    Err(ToolError::invalid(format!(
        "Table '{key}': rows are either all objects (Word) or all lists of cell texts (PowerPoint)"
    )))
}

pub async fn document_fill(env: &dyn ToolEnv, args: DocumentFillArgs) -> ToolResult<ToolOutput> {
    let workspace = workspace_of(env, "keep filled documents")?;
    let entries = args.values.len() + args.images.len() + args.tables.len();
    if entries == 0 {
        return Err(ToolError::invalid(
            "Nothing to fill: give values, images or tables",
        ));
    }
    if entries > MAX_FILL_ENTRIES {
        return Err(too_many("entries", entries, MAX_FILL_ENTRIES));
    }
    if args.images.len() > MAX_IMAGES {
        return Err(too_many("images", args.images.len(), MAX_IMAGES));
    }
    let backend = backend_of(env)?;
    let template = resolve(env, &args.template, env.limits().max_ingest_bytes).await?;
    let mut fill = TemplateFill {
        values: args.values.into_iter().collect(),
        markdown: args.markdown.unwrap_or(false),
        ..TemplateFill::default()
    };
    for (name, reference) in &args.images {
        let image = resolve(env, reference, env.limits().max_read_bytes).await?;
        fill.images.push((name.clone(), image.bytes));
    }
    for (key, rows) in &args.tables {
        fill.tables.push((key.clone(), fill_rows(key, rows)?));
    }

    let (worker, bytes) = (backend.clone(), template.bytes.clone());
    let filled = blocking(move || worker.fill_template(bytes, fill)).await.map_err(|error| {
        ToolError::new(
            code::UNSUPPORTED_FILE,
            format!("Filling '{}' failed: {}", template.name, error.message),
        )
    })?;
    if filled.filled.is_empty() {
        return Err(ToolError::invalid(format!(
            "Nothing in '{}' matched {}; no file was written. Placeholders are written {{{{name}}}} in the document",
            template.name,
            filled.missing.join(", ")
        )));
    }

    let extension = filled.format.as_str();
    let stem = match args.name.as_deref().filter(|n| !n.trim().is_empty()) {
        Some(name) => export_stem(name, extension)?,
        None => format!("{}-filled", stem_of(&template.name)),
    };
    let mime = guess_mime(&format!("x.{extension}"));
    let handle = save_new(env, &workspace, &stem, extension, &mime, filled.bytes.clone()).await?;
    let mut output = ToolOutput::new(json!({
        "format": extension,
        "handle": handle,
        "filled": filled.filled,
        "missing": filled.missing,
    }))
    .saved(handle)
    .classified(template.classification);
    if !filled.missing.is_empty() {
        output.warnings.push(format!(
            "Nothing matched {}; check the names against the template",
            filled.missing.join(", ")
        ));
    }
    if extension == "pdf" {
        preview(&backend, env, &filled.bytes, &stem, &mut output).await;
    }
    Ok(output)
}

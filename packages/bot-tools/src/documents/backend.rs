use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use flow_like_model_provider::llm::ModelLogic;
use flow_like_types::{Bytes, Result};
use serde_json::Value;

use crate::convert::Converted;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelTokens {
    pub input: u64,
    pub output: u64,
}

pub struct SummaryRequest<'a> {
    pub bytes: Bytes,
    pub extension: Option<&'a str>,
    pub goal: &'a str,
    pub language: Option<&'a str>,
    pub words: usize,
    pub key_points: usize,
    pub source_path: &'a str,
}

#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub summary: String,
    pub key_points: Vec<String>,
    pub failed_pages: Vec<u32>,
    /// Summary, key points and the notes of each page as markdown.
    pub markdown: String,
    pub tokens: ModelTokens,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Extracted {
    pub value: Value,
    pub tokens: ModelTokens,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RenderedPng {
    pub page: usize,
    pub png: Bytes,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExcelOperation {
    /// Sheets, used ranges, hidden sheets, merges and defined tables.
    Overview,
    /// A compact picture of one sheet.
    View,
    /// The cells of an A1 range, at most 60 rows.
    Range,
    /// Cells with a given style (bold, fill colour, …).
    FindStyled,
    /// Where values occur on a large sheet.
    ValueIndex,
    /// The detected tables of a sheet with their headers.
    Tables,
}

#[derive(Debug, Clone, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct StyleQuery {
    #[serde(default)]
    pub bold: Option<bool>,
    #[serde(default)]
    pub italic: Option<bool>,
    #[serde(default)]
    pub fill_color: Option<String>,
    #[serde(default)]
    pub font_color: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExcelRequest {
    pub operation: ExcelOperation,
    pub sheet: Option<String>,
    pub range: Option<String>,
    pub contains: Option<String>,
    pub max_rows: Option<usize>,
    pub style: StyleQuery,
}

/// A built file and what the builder knows about it.
#[derive(Debug, Clone, Default)]
pub struct BuiltDocument {
    pub bytes: Bytes,
    /// Pages for a PDF, slides for a deck; `None` when only the viewer knows (Word).
    pub count: Option<usize>,
    pub warnings: Vec<String>,
}

/// A PDF or Word document typeset from markdown. `images` maps the URL written in the markdown to
/// the image bytes the tool already resolved.
#[derive(Debug, Clone, Default)]
pub struct MarkdownDocument {
    pub markdown: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub author: Option<String>,
    pub cover: bool,
    pub page_size: Option<String>,
    pub page_numbers: bool,
    pub images: BTreeMap<String, Bytes>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SlideLayoutKind {
    Title,
    Bullets,
    Image,
    Table,
    Chart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Bar,
    Line,
    Pie,
}

#[derive(Debug, Clone)]
pub struct DeckChart {
    pub kind: ChartKind,
    pub categories: Vec<String>,
    pub series: Vec<(String, Vec<f64>)>,
}

#[derive(Debug, Clone)]
pub struct DeckSlide {
    pub layout: SlideLayoutKind,
    pub title: String,
    pub bullets: Vec<String>,
    pub notes: Option<String>,
    pub image: Option<Bytes>,
    pub table: Option<(Vec<String>, Vec<Vec<String>>)>,
    pub chart: Option<DeckChart>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SheetCell {
    Empty,
    Text(String),
    Number(f64),
    Bool(bool),
    /// Without the leading `=`.
    Formula(String),
}

#[derive(Debug, Clone, Default)]
pub struct SheetData {
    pub name: String,
    pub rows: Vec<Vec<SheetCell>>,
    pub header: bool,
    pub widths: Vec<f64>,
    pub formats: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum FillRows {
    /// Word: one record per row, keyed by the `{{column}}` placeholders of the template row.
    Records(Vec<BTreeMap<String, String>>),
    /// PowerPoint: the whole table, header row first.
    Grid(Vec<Vec<String>>),
}

#[derive(Debug, Clone, Default)]
pub struct TemplateFill {
    pub values: Vec<(String, String)>,
    pub markdown: bool,
    pub images: Vec<(String, Bytes)>,
    pub tables: Vec<(String, FillRows)>,
}

#[derive(Debug, Clone)]
pub struct FilledDocument {
    pub bytes: Bytes,
    /// `docx`, `pptx` or `pdf`.
    pub format: String,
    pub filled: Vec<String>,
    pub missing: Vec<String>,
}

fn no_authoring() -> flow_like_types::Error {
    flow_like_types::anyhow!("This host cannot create or fill documents")
}

/// The document services behind the tools. The catalog crates implement it (feature
/// `documents`); tests and other hosts can supply their own. The PDF calls are synchronous and
/// run on a blocking thread.
#[async_trait]
pub trait DocumentBackend: Send + Sync {
    async fn summarize(
        &self,
        model: Arc<dyn ModelLogic>,
        request: SummaryRequest<'_>,
    ) -> Result<Summary>;

    async fn extract(
        &self,
        model: Arc<dyn ModelLogic>,
        text: &str,
        schema: &str,
        hint: &str,
    ) -> Result<Extracted>;

    async fn convert(&self, bytes: Bytes, extension: Option<&str>) -> Result<Converted>;

    /// Looks into a workbook; the answer is text for the model.
    async fn excel(&self, bytes: Bytes, request: ExcelRequest) -> Result<String>;

    fn create_pdf(&self, _request: MarkdownDocument) -> Result<BuiltDocument> {
        Err(no_authoring())
    }

    fn create_docx(&self, _request: MarkdownDocument) -> Result<BuiltDocument> {
        Err(no_authoring())
    }

    fn create_pptx(
        &self,
        _title: Option<String>,
        _author: Option<String>,
        _slides: Vec<DeckSlide>,
    ) -> Result<BuiltDocument> {
        Err(no_authoring())
    }

    fn create_xlsx(&self, _sheets: Vec<SheetData>) -> Result<BuiltDocument> {
        Err(no_authoring())
    }

    /// Fills a Word, PowerPoint or PDF form template into a new file.
    fn fill_template(&self, _template: Bytes, _fill: TemplateFill) -> Result<FilledDocument> {
        Err(no_authoring())
    }

    fn page_count(&self, pdf: Vec<u8>) -> Result<usize>;

    /// Renders the 1-based pages, each at most `max_pixels` large, as PNG.
    fn render_pages(
        &self,
        pdf: Vec<u8>,
        pages: &[usize],
        max_pixels: u64,
    ) -> Result<Vec<RenderedPng>>;
}

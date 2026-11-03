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

    fn page_count(&self, pdf: Vec<u8>) -> Result<usize>;

    /// Renders the 1-based pages, each at most `max_pixels` large, as PNG.
    fn render_pages(
        &self,
        pdf: Vec<u8>,
        pages: &[usize],
        max_pixels: u64,
    ) -> Result<Vec<RenderedPng>>;
}

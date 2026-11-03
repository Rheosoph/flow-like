use std::io::Cursor;
use std::sync::Arc;

use async_trait::async_trait;
use flow_like_catalog_data::data::excel::inspect::{
    StyleFilter, find_styled_cells, inspect_range, load_sheet, sheet_view, table_candidates,
    value_index, workbook_overview,
};
use flow_like_catalog_data::data::excel::sheet_compressor::EncodeOptions;
use flow_like_catalog_data::data::excel::table_detect::ExtractConfig;
use flow_like_catalog_llm::llm::llm_extractor::{
    ExtractOptions, extract_with_model, prepare_schema,
};
use flow_like_catalog_media_image::image::pdf::{
    PageBackground, RenderOptions, page_count, render_pages,
};
use flow_like_catalog_processing::markitdown::service::{
    self, ConvertOptions, HistoryThinking, LlmConfig, ReaderModel, SummarizeOptions,
};
use flow_like_model_provider::llm::ModelLogic;
use flow_like_types::image::ImageFormat;
use flow_like_types::{Bytes, Result, anyhow};

use super::backend::{
    DocumentBackend, ExcelOperation, ExcelRequest, Extracted, ModelTokens, RenderedPng, Summary,
    SummaryRequest,
};
use crate::convert::{ConvertInput, Converted, DocumentConverter};

const RENDER_SCALE: f32 = 1.5;

fn inspect_workbook(bytes: &[u8], request: &ExcelRequest) -> Result<String> {
    let overview = workbook_overview(bytes.to_vec())?;
    if request.operation == ExcelOperation::Overview {
        return Ok(serde_json::to_string_pretty(&overview)?);
    }
    let sheet = match &request.sheet {
        Some(sheet) => sheet.clone(),
        None => overview
            .sheets
            .iter()
            .find(|sheet| !sheet.hidden)
            .or(overview.sheets.first())
            .map(|sheet| sheet.name.clone())
            .ok_or_else(|| anyhow!("The workbook has no sheets"))?,
    };
    let config = ExtractConfig::default();
    let loaded = load_sheet(
        bytes,
        &sheet,
        &config,
        request.operation == ExcelOperation::FindStyled
            || request.operation == ExcelOperation::Range,
    )?;
    let options = EncodeOptions::default();
    Ok(match request.operation {
        ExcelOperation::Overview => String::new(),
        ExcelOperation::View => sheet_view(&loaded, &options),
        ExcelOperation::Range => {
            let range = request
                .range
                .as_deref()
                .ok_or_else(|| anyhow!("Reading a range needs 'range', e.g. \"A1:F40\""))?;
            inspect_range(
                &loaded.grid,
                loaded.styles.as_deref(),
                range,
                request.max_rows,
                &options,
            )
        }
        ExcelOperation::FindStyled => find_styled_cells(
            loaded.styles.as_deref(),
            &StyleFilter {
                bold: request.style.bold,
                italic: request.style.italic,
                fill_color: request.style.fill_color.clone(),
                font_color: request.style.font_color.clone(),
            },
        ),
        ExcelOperation::ValueIndex => value_index(&loaded.grid, request.contains.as_deref()),
        ExcelOperation::Tables => {
            serde_json::to_string_pretty(&table_candidates(&loaded, &config))?
        }
    })
}

pub struct CatalogBackend;

pub fn default_backend() -> Option<Arc<dyn DocumentBackend>> {
    Some(Arc::new(CatalogBackend))
}

fn extension_of(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .map(|(_, ext)| format!(".{}", ext.to_lowercase()))
}

pub fn to_converted(document: &service::Document) -> Converted {
    let mut text = String::new();
    let mut page_starts = Vec::new();
    for page in &document.pages {
        page_starts.push(text.len());
        for block in &page.content {
            text.push_str(&block.to_markdown());
            text.push('\n');
        }
        text.push('\n');
    }
    Converted { text, page_starts }
}

#[async_trait]
impl DocumentBackend for CatalogBackend {
    async fn summarize(
        &self,
        model: Arc<dyn ModelLogic>,
        request: SummaryRequest<'_>,
    ) -> Result<Summary> {
        let model_name = model.default_model().await.unwrap_or_default();
        let reader = service::build_llm_client(&ReaderModel {
            model,
            model_name,
            thinking: HistoryThinking::Off,
            config: LlmConfig::default(),
        })
        .await?;
        let options = SummarizeOptions {
            goal: Some(request.goal.to_string()),
            language: request.language.map(str::to_string),
            summary_words: request.words,
            max_key_points: request.key_points,
            source_path: Some(request.source_path.to_string()),
            ..SummarizeOptions::default()
        };
        let done = service::summarize(request.bytes, request.extension, &options, &reader).await?;
        let mut warnings = reader.warnings.clone();
        warnings.extend(reader.rejected_settings());
        Ok(Summary {
            summary: done.summary.summary.clone(),
            key_points: done.summary.key_points.clone(),
            failed_pages: done.summary.failed_pages.clone(),
            markdown: done.markdown,
            tokens: ModelTokens {
                input: done.summary.usage.input_tokens,
                output: done.summary.usage.output_tokens,
            },
            warnings,
        })
    }

    async fn extract(
        &self,
        model: Arc<dyn ModelLogic>,
        text: &str,
        schema: &str,
        hint: &str,
    ) -> Result<Extracted> {
        let prepared = prepare_schema(schema)?;
        let options = ExtractOptions {
            hint: hint.to_string(),
            ..ExtractOptions::default()
        };
        let mut warnings = Vec::new();
        let extraction =
            extract_with_model(model.as_ref(), text, &prepared, &options, &mut warnings).await?;
        Ok(Extracted {
            value: extraction.value,
            tokens: ModelTokens {
                input: u64::from(extraction.stats.usage.prompt_tokens),
                output: u64::from(extraction.stats.usage.completion_tokens),
            },
            warnings,
        })
    }

    async fn convert(&self, bytes: Bytes, extension: Option<&str>) -> Result<Converted> {
        let document = service::convert(bytes, extension, &ConvertOptions::default(), None).await?;
        Ok(to_converted(&document))
    }

    async fn excel(&self, bytes: Bytes, request: ExcelRequest) -> Result<String> {
        flow_like_types::tokio::task::spawn_blocking(move || inspect_workbook(&bytes, &request))
            .await
            .map_err(|error| anyhow!("The workbook reader stopped: {error}"))?
    }

    fn page_count(&self, pdf: Vec<u8>) -> Result<usize> {
        page_count(pdf)
    }

    fn render_pages(
        &self,
        pdf: Vec<u8>,
        pages: &[usize],
        max_pixels: u64,
    ) -> Result<Vec<RenderedPng>> {
        let options = RenderOptions {
            scale: RENDER_SCALE,
            background: PageBackground::White,
            max_pixels: Some(max_pixels),
        };
        render_pages(pdf, Some(pages), &options)?
            .into_iter()
            .map(|rendered| {
                let mut png = Cursor::new(Vec::new());
                rendered
                    .image
                    .write_to(&mut png, ImageFormat::Png)
                    .map_err(|error| anyhow!("Encoding page {} failed: {error}", rendered.page))?;
                Ok(RenderedPng {
                    page: rendered.page,
                    width: rendered.image.width(),
                    height: rendered.image.height(),
                    png: Bytes::from(png.into_inner()),
                })
            })
            .collect()
    }
}

/// Reads documents through markitdown's text layers, without a model.
pub struct MarkitdownConverter;

pub fn default_converter() -> Option<Arc<dyn DocumentConverter>> {
    Some(Arc::new(MarkitdownConverter))
}

#[async_trait]
impl DocumentConverter for MarkitdownConverter {
    async fn convert(&self, input: ConvertInput<'_>) -> Result<Converted> {
        CatalogBackend
            .convert(input.bytes.clone(), extension_of(input.name).as_deref())
            .await
    }
}

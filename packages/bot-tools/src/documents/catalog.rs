use std::io::Cursor;
use std::sync::Arc;

use async_trait::async_trait;
use flow_like_catalog_data::data::excel::workbook::{XlsxCell, XlsxSheet, build_workbook};
use flow_like_catalog_data::data::excel::inspect::{
    StyleFilter, find_styled_cells, inspect_range, load_sheet, sheet_view, table_candidates,
    value_index, workbook_overview,
};
use flow_like_catalog_data::data::excel::sheet_compressor::EncodeOptions;
use flow_like_catalog_data::data::excel::table_detect::ExtractConfig;
use flow_like_catalog_media_document::document::service::docx::{DocxOptions, docx_from_markdown};
use flow_like_catalog_media_document::document::service::fill::{
    FillInput, TableRows, TemplateKind, fill_template,
};
use flow_like_catalog_media_document::document::service::pdf::{PdfOptions, pdf_from_markdown};
use flow_like_catalog_media_document::document::service::chart::{SHEET_NAME, WorkbookCell};
use flow_like_catalog_media_document::document::service::pptx::{
    ChartKind as DeckChartKind, DeckOptions, Slide, SlideChart, SlideLayout, pptx_from_slides,
};
use flow_like_catalog_media_document::document::service::{Built, Images};
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
    BuiltDocument, ChartKind, DeckSlide, DocumentBackend, ExcelOperation, ExcelRequest, Extracted,
    FillRows, FilledDocument, MarkdownDocument, ModelReading, ModelTokens, RenderedPng, SheetCell,
    SheetData, SlideLayoutKind, Summary, SummaryRequest, TemplateFill,
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

fn built(built: Built) -> BuiltDocument {
    BuiltDocument {
        bytes: Bytes::from(built.bytes),
        count: built.count,
        warnings: built.warnings,
    }
}

fn images_of(request: &MarkdownDocument) -> Images {
    request
        .images
        .iter()
        .map(|(url, bytes)| (url.clone(), bytes.to_vec()))
        .collect()
}

fn slide_of(slide: DeckSlide) -> Slide {
    Slide {
        layout: match slide.layout {
            SlideLayoutKind::Title => SlideLayout::Title,
            SlideLayoutKind::Bullets => SlideLayout::Bullets,
            SlideLayoutKind::Image => SlideLayout::Image,
            SlideLayoutKind::Table => SlideLayout::Table,
            SlideLayoutKind::Chart => SlideLayout::Chart,
        },
        title: slide.title,
        bullets: slide.bullets,
        notes: slide.notes,
        image: slide.image.map(|bytes| bytes.to_vec()),
        table: slide.table,
        chart: slide.chart.map(|chart| SlideChart {
            kind: match chart.kind {
                ChartKind::Bar => DeckChartKind::Bar,
                ChartKind::Line => DeckChartKind::Line,
                ChartKind::Pie => DeckChartKind::Pie,
            },
            categories: chart.categories,
            series: chart.series,
            ..SlideChart::default()
        }),
    }
}

/// The workbook behind a chart's "Edit data", built with the same xlsx builder as `document_create`.
fn chart_workbook(rows: &[Vec<WorkbookCell>]) -> Result<Vec<u8>> {
    let sheet = XlsxSheet {
        name: SHEET_NAME.to_string(),
        rows: rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| match cell {
                        WorkbookCell::Empty => XlsxCell::Empty,
                        WorkbookCell::Text(text) => XlsxCell::Text(text.clone()),
                        WorkbookCell::Number(number) => XlsxCell::Number(*number),
                    })
                    .collect()
            })
            .collect(),
        header: true,
        ..XlsxSheet::default()
    };
    build_workbook(&[sheet])
}

fn sheet_of(sheet: SheetData) -> XlsxSheet {
    XlsxSheet {
        name: sheet.name,
        rows: sheet
            .rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|cell| match cell {
                        SheetCell::Empty => XlsxCell::Empty,
                        SheetCell::Text(text) => XlsxCell::Text(text),
                        SheetCell::Number(number) => XlsxCell::Number(number),
                        SheetCell::Bool(flag) => XlsxCell::Bool(flag),
                        SheetCell::Formula(formula) => XlsxCell::Formula(formula),
                    })
                    .collect()
            })
            .collect(),
        header: sheet.header,
        widths: sheet.widths,
        formats: sheet.formats,
    }
}

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

async fn reader_for(model: Arc<dyn ModelLogic>) -> Result<service::ReaderClient> {
    let model_name = model.default_model().await.unwrap_or_default();
    service::build_llm_client(&ReaderModel {
        model,
        model_name,
        thinking: HistoryThinking::Off,
        config: LlmConfig::default(),
    })
    .await
}

#[async_trait]
impl DocumentBackend for CatalogBackend {
    async fn summarize(
        &self,
        model: Arc<dyn ModelLogic>,
        request: SummaryRequest<'_>,
    ) -> Result<Summary> {
        let reader = reader_for(model).await?;
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

    async fn read_with_model(
        &self,
        model: Arc<dyn ModelLogic>,
        bytes: Bytes,
        extension: Option<&str>,
        source_path: &str,
    ) -> Result<ModelReading> {
        let reader = reader_for(model).await?;
        let options = ConvertOptions {
            extract_images: true,
            force_ocr: false,
            source_path: Some(source_path.to_string()),
        };
        let document = service::convert(bytes, extension, &options, Some(&reader)).await?;
        let mut warnings = reader.warnings.clone();
        warnings.extend(reader.rejected_settings());
        Ok(ModelReading {
            converted: to_converted(&document),
            warnings,
        })
    }

    async fn excel(&self, bytes: Bytes, request: ExcelRequest) -> Result<String> {
        flow_like_types::tokio::task::spawn_blocking(move || inspect_workbook(&bytes, &request))
            .await
            .map_err(|error| anyhow!("The workbook reader stopped: {error}"))?
    }

    fn create_pdf(&self, request: MarkdownDocument) -> Result<BuiltDocument> {
        let options = PdfOptions {
            title: request.title.clone(),
            subtitle: request.subtitle.clone(),
            author: request.author.clone(),
            cover: request.cover,
            page_size: request.page_size.clone(),
            page_numbers: request.page_numbers,
        };
        pdf_from_markdown(&request.markdown, &options, &images_of(&request)).map(built)
    }

    fn create_docx(&self, request: MarkdownDocument) -> Result<BuiltDocument> {
        let options = DocxOptions {
            title: request.title.clone(),
            subtitle: request.subtitle.clone(),
            author: request.author.clone(),
            cover: request.cover,
            page_size: request.page_size.clone(),
            page_numbers: request.page_numbers,
            workbook: Some(chart_workbook),
        };
        docx_from_markdown(&request.markdown, &options, &images_of(&request)).map(built)
    }

    fn create_pptx(
        &self,
        title: Option<String>,
        author: Option<String>,
        slides: Vec<DeckSlide>,
    ) -> Result<BuiltDocument> {
        let slides: Vec<Slide> = slides.into_iter().map(slide_of).collect();
        pptx_from_slides(
            &DeckOptions {
                title,
                author,
                workbook: Some(chart_workbook),
            },
            &slides,
        ).map(built)
    }

    fn create_xlsx(&self, sheets: Vec<SheetData>) -> Result<BuiltDocument> {
        let sheets: Vec<XlsxSheet> = sheets.into_iter().map(sheet_of).collect();
        Ok(BuiltDocument {
            count: Some(sheets.len()),
            bytes: Bytes::from(build_workbook(&sheets)?),
            warnings: Vec::new(),
        })
    }

    fn fill_template(&self, template: Bytes, fill: TemplateFill) -> Result<FilledDocument> {
        let kind = TemplateKind::detect(&template).ok_or_else(|| {
            anyhow!("The template is not a Word (docx), PowerPoint (pptx) or PDF file")
        })?;
        let input = FillInput {
            values: fill.values,
            markdown: fill.markdown,
            images: fill
                .images
                .into_iter()
                .map(|(name, bytes)| (name, bytes.to_vec()))
                .collect(),
            tables: fill
                .tables
                .into_iter()
                .map(|(key, rows)| {
                    let rows = match rows {
                        FillRows::Records(records) => TableRows::Records(records),
                        FillRows::Grid(grid) => TableRows::Grid(grid),
                    };
                    (key, rows)
                })
                .collect(),
        };
        let filled = fill_template(kind, &template, &input)?;
        Ok(FilledDocument {
            bytes: Bytes::from(filled.bytes),
            format: kind.label().to_string(),
            filled: filled.filled,
            missing: filled.missing,
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::{ChartKind, DeckChart, DeckSlide, SlideLayoutKind};

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|window| window == needle)
    }

    #[test]
    fn chart_embeddings_are_real_workbooks_built_with_the_xlsx_builder() {
        let slide = DeckSlide {
            layout: SlideLayoutKind::Chart,
            title: "Sales".into(),
            bullets: Vec::new(),
            notes: None,
            image: None,
            table: None,
            chart: Some(DeckChart {
                kind: ChartKind::Bar,
                categories: vec!["Q1".into(), "Q2".into()],
                series: vec![("Sales".into(), vec![1.0, 2.0])],
            }),
        };
        let deck = CatalogBackend
            .create_pptx(None, None, vec![slide])
            .expect("deck");
        let docx = CatalogBackend
            .create_docx(MarkdownDocument {
                markdown: "```nivo\ntype: bar\n---\nQ,V\nQ1,1\nQ2,2\n```".into(),
                ..MarkdownDocument::default()
            })
            .expect("docx");
        for (bytes, part) in [
            (&deck.bytes, "ppt/embeddings/Microsoft_Excel_Sheet1.xlsx"),
            (&docx.bytes, "word/embeddings/Microsoft_Excel_Sheet1.xlsx"),
        ] {
            assert!(contains(bytes, part.as_bytes()), "{part}");
        }
        let files = flow_like_catalog_media_document::document::openxml::read_zip(&deck.bytes)
            .expect("zip");
        let workbook = &files["ppt/embeddings/Microsoft_Excel_Sheet1.xlsx"];
        assert!(workbook.starts_with(b"PK"));
        assert!(contains(workbook, b"xl/workbook.xml"));
        let book = flow_like_catalog_data::data::excel::inspect::workbook_overview(workbook.clone())
            .expect("readable workbook");
        assert_eq!(book.sheets[0].name, "Sheet1");
    }
}

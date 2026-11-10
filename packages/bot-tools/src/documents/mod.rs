mod author;
mod backend;
#[cfg(feature = "documents")]
mod catalog;

use std::sync::Arc;

use flow_like_types::Bytes;
use flow_like_types::tokio::task::spawn_blocking;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::convert::DocumentConverter;
use crate::env::{ModelRole, ToolEnv};
use crate::error::{ToolError, ToolResult, code};
use crate::files::{FileRef, ResolvedFile, file_text, handle_for, page_end, resolve};
use crate::layout::{EXTRACTIONS_DIR, NOTES_DIR, RENDERS_DIR, normalize_path, sanitize_name};
use crate::outcome::{ToolImage, ToolOutput};
use crate::spec::{Approval, ToolSpec, schema_of};
use crate::tables::{SaveMode, save_rows, validate_table_name};

pub use author::{
    DocumentCreateArgs, DocumentFillArgs, document_create, document_fill, export_stem,
    unique_export_path,
};
pub use backend::{
    BuiltDocument, ChartKind, DeckChart, DeckSlide, DocumentBackend, ExcelOperation, ExcelRequest,
    Extracted, FillRows, FilledDocument, MarkdownDocument, ModelReading, ModelTokens, RenderedPng,
    SheetCell, SheetData, SlideLayoutKind, StyleQuery, Summary, SummaryRequest, TemplateFill,
};

pub const FILE_SUMMARIZE: &str = "file_summarize";
pub const FILE_EXTRACT: &str = "file_extract";
pub const FILE_EXTRACT_CONTENT: &str = "file_extract_content";
pub const PDF_RENDER: &str = "pdf_render";
pub const EXCEL_INSPECT: &str = "excel_inspect";
pub const DOCUMENT_CREATE: &str = "document_create";
pub const DOCUMENT_FILL: &str = "document_fill";

const CAPABILITY: &str = "documents";
const TOKENS_PER_PDF_PAGE: u64 = 1500;
const DEFAULT_WORDS: usize = 250;
const DEFAULT_KEY_POINTS: usize = 8;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileSummarizeArgs {
    pub file: FileRef,
    /// What the summary must answer or concentrate on, e.g. a question. Required: a summary
    /// with a goal is far more useful than a general one.
    pub goal: String,
    /// Language of the summary; the document's language when omitted.
    #[serde(default)]
    pub language: Option<String>,
    /// Target length in words (default 250).
    #[serde(default)]
    pub words: Option<usize>,
    /// Most key points to list (default 8).
    #[serde(default)]
    pub key_points: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileExtractArgs {
    /// The files to extract from; each gets one result.
    pub files: Vec<FileRef>,
    /// A JSON Schema, or an example JSON value, describing what to extract.
    pub schema: Value,
    #[serde(default)]
    pub hint: Option<String>,
    /// Also append the results as rows to this workspace table (each row gets a `_source` column).
    #[serde(default)]
    pub save_to_table: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileExtractContentArgs {
    pub file: FileRef,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExcelInspectArgs {
    pub file: FileRef,
    pub operation: ExcelOperation,
    /// The sheet; the first visible one when omitted.
    #[serde(default)]
    pub sheet: Option<String>,
    /// An A1 range, e.g. "A1:F40" (operation `range`).
    #[serde(default)]
    pub range: Option<String>,
    /// Only values containing this text (operation `value_index`).
    #[serde(default)]
    pub contains: Option<String>,
    #[serde(default)]
    pub max_rows: Option<usize>,
    /// What to look for (operation `find_styled`).
    #[serde(default)]
    pub style: Option<StyleQuery>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PdfRenderArgs {
    pub file: FileRef,
    /// 1-based pages; every page when omitted.
    #[serde(default)]
    pub pages: Option<Vec<usize>>,
}

pub fn specs() -> Vec<ToolSpec> {
    let spec = |name, description, input_schema| ToolSpec {
        name,
        description,
        capability: Some(CAPABILITY),
        module: "documents",
        approval: Approval::None,
        approval_args: &[],
        mutating: false,
        input_schema,
    };
    vec![
        spec(
            FILE_SUMMARIZE,
            "Summarize a document (PDF, Word, slides, …) towards a goal, with key points citing pages. Always pass the goal: the question the summary has to answer.",
            schema_of::<FileSummarizeArgs>(),
        ),
        spec(
            FILE_EXTRACT,
            "Extract structured data from one or more documents according to a JSON Schema or an example value. Scanned or photographed documents are read by the model first. Results are saved as JSON files in the workspace.",
            schema_of::<FileExtractArgs>(),
        ),
        spec(
            FILE_EXTRACT_CONTENT,
            "Extract a document's full content as markdown with the model reading it: scanned or photographed pages are transcribed and images and figures described. Works on PDFs, Office files and photos. Slower than file_read and spends model tokens, so use it when file_read returns little or no text. Long content comes back cut and is saved in full to the workspace for file_read.",
            schema_of::<FileExtractContentArgs>(),
        ),
        spec(
            EXCEL_INSPECT,
            "Look into an Excel workbook without loading it all: sheets and tables, a compact view of a sheet, a cell range, styled cells or where a value occurs. Use data_sources to load its tables for SQL.",
            schema_of::<ExcelInspectArgs>(),
        ),
        spec(
            PDF_RENDER,
            "Render PDF pages to images. The first few pages are shown to you; every render is saved in the workspace and returned as a handle.",
            schema_of::<PdfRenderArgs>(),
        ),
        ToolSpec {
            mutating: true,
            ..spec(
                DOCUMENT_CREATE,
                "Create a NEW Word (docx), PDF, PowerPoint (pptx) or Excel (xlsx) file in the workspace (bot/exports/), never overwriting. pdf/docx take markdown (headings, lists, tables, code, images from workspace paths); pptx takes slides (title, bullets, image, table, chart, notes); xlsx takes sheets of rows where a string starting with '=' is a formula. Returns the file handle; a PDF also shows you page 1.",
                schema_of::<DocumentCreateArgs>(),
            )
        },
        ToolSpec {
            mutating: true,
            ..spec(
                DOCUMENT_FILL,
                "Fill a template into a NEW file in bot/exports/: replace {{placeholders}} in a Word or PowerPoint file (text, images, table rows) or fill the fields of a PDF form. Say which names did not match.",
                schema_of::<DocumentFillArgs>(),
            )
        },
    ]
}

pub(crate) fn default_backend() -> Option<Arc<dyn DocumentBackend>> {
    #[cfg(feature = "documents")]
    return catalog::default_backend();
    #[cfg(not(feature = "documents"))]
    None
}

pub(crate) fn default_converter() -> Option<Arc<dyn DocumentConverter>> {
    #[cfg(feature = "documents")]
    return catalog::default_converter();
    #[cfg(not(feature = "documents"))]
    None
}

fn backend_of(env: &dyn ToolEnv) -> ToolResult<Arc<dyn DocumentBackend>> {
    env.documents().ok_or_else(|| {
        ToolError::new(
            code::UNSUPPORTED_FILE,
            "This host has no document services, so documents cannot be summarized, extracted or rendered here",
        )
    })
}

fn is_pdf(file: &ResolvedFile) -> bool {
    file.mime.starts_with("application/pdf") || file.name.to_lowercase().ends_with(".pdf")
}

fn extension_of(name: &str) -> Option<String> {
    name.rsplit_once('.')
        .map(|(_, extension)| format!(".{}", extension.to_lowercase()))
}

fn stem_of(name: &str) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    sanitize_name(stem)
}

/// `<hash of the source>-<stem>`: two files with the same name in different places never share an
/// output.
fn output_stem(file: &ResolvedFile) -> String {
    let hash = blake3::hash(file.source_key().as_bytes()).to_hex();
    format!("{}-{}", &hash[..8], stem_of(&file.name))
}

fn needs_approval(env: &dyn ToolEnv, pages: Option<usize>, tokens: u64) -> ToolResult<()> {
    if env.allow_expensive_documents() {
        return Ok(());
    }
    let limits = env.limits();
    let too_many_pages = pages.is_some_and(|pages| pages > limits.document_max_pages);
    if !too_many_pages && tokens <= limits.document_max_tokens {
        return Ok(());
    }
    Err(ToolError::new(
        code::NEEDS_APPROVAL,
        format!(
            "This call is estimated at {} pages and about {tokens} model tokens, above the limit of {} pages and {} tokens. The owner has to approve it (or grant it standing approval) before it runs",
            pages.map_or_else(|| "an unknown number of".to_string(), |p| p.to_string()),
            limits.document_max_pages,
            limits.document_max_tokens,
        ),
    ))
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> flow_like_types::Result<T> + Send + 'static,
) -> ToolResult<T> {
    spawn_blocking(work)
        .await
        .map_err(|error| {
            ToolError::new(
                code::FAILED,
                format!("The document worker stopped: {error}"),
            )
        })?
        .map_err(ToolError::from)
}

async fn pdf_pages(backend: &Arc<dyn DocumentBackend>, bytes: &Bytes) -> ToolResult<usize> {
    let (backend, pdf) = (backend.clone(), bytes.to_vec());
    blocking(move || backend.page_count(pdf)).await
}

fn tokens_to_usage(output: &mut ToolOutput, tokens: ModelTokens) {
    output.usage.model_in += tokens.input;
    output.usage.model_out += tokens.output;
}

/// Pages and input tokens of letting the model read `file`.
async fn model_read_estimate(
    backend: &Arc<dyn DocumentBackend>,
    file: &ResolvedFile,
) -> ToolResult<(Option<usize>, u64)> {
    if is_pdf(file) {
        let pages = pdf_pages(backend, &file.bytes).await?;
        Ok((Some(pages), pages as u64 * TOKENS_PER_PDF_PAGE))
    } else {
        Ok((None, file.bytes.len() as u64 / 4))
    }
}

/// Pages of letting the model read `file`; asks for approval above the limits.
async fn approve_model_read(
    env: &dyn ToolEnv,
    backend: &Arc<dyn DocumentBackend>,
    file: &ResolvedFile,
) -> ToolResult<Option<usize>> {
    let (pages, tokens) = model_read_estimate(backend, file).await?;
    needs_approval(env, pages, tokens)?;
    Ok(pages)
}

fn reading_usage(input: u64, reading: &ModelReading) -> ModelTokens {
    ModelTokens {
        input,
        output: reading.converted.text.len() as u64 / 4,
    }
}

async fn read_with_model(
    env: &dyn ToolEnv,
    backend: &Arc<dyn DocumentBackend>,
    file: &ResolvedFile,
) -> ToolResult<ModelReading> {
    let model = env.model(ModelRole::Vision).await?;
    backend
        .read_with_model(
            model,
            file.bytes.clone(),
            extension_of(&file.name).as_deref(),
            &file.path,
        )
        .await
        .map_err(|error| {
            ToolError::new(
                code::UNSUPPORTED_FILE,
                format!(
                    "The model could not read '{}' ({}): {error:#}",
                    file.name, file.mime
                ),
            )
        })
}

pub async fn file_extract_content(
    env: &dyn ToolEnv,
    args: FileExtractContentArgs,
) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    let file = resolve(env, &args.file, env.limits().max_ingest_bytes).await?;
    let (pages, estimate) = model_read_estimate(&backend, &file).await?;
    needs_approval(env, pages, estimate)?;
    let reading = read_with_model(env, &backend, &file).await?;
    let text = reading.converted.text.trim();
    if text.is_empty() {
        return Err(ToolError::new(
            code::UNSUPPORTED_FILE,
            format!("The model found no content in '{}'", file.name),
        ));
    }
    let budget = env.limits().inline_cap_bytes.saturating_sub(1024).max(256);
    let cut = page_end(text, 0, budget);
    let mut result = json!({
        "file": file.name,
        "pages": pages,
        "content": &text[..cut],
        "truncated": cut < text.len(),
        "characters": text.chars().count(),
    });
    let mut output = ToolOutput::new(Value::Null).classified(file.classification);
    if let Some(workspace) = env.scope().workspace.clone() {
        let path = normalize_path(&format!("{EXTRACTIONS_DIR}/{}.md", output_stem(&file)))?;
        let bytes = Bytes::from(text.to_string().into_bytes());
        env.write_workspace(&path, bytes.clone(), "text/markdown")
            .await?;
        let handle = handle_for(&workspace, &path, &bytes, "text/markdown");
        result["saved"] = json!(handle);
        output = output.saved(handle);
    }
    output.result = result;
    output.warnings.extend(reading.warnings.iter().cloned());
    tokens_to_usage(&mut output, reading_usage(estimate, &reading));
    Ok(crate::files::untrusted(output))
}

pub async fn file_summarize(env: &dyn ToolEnv, args: FileSummarizeArgs) -> ToolResult<ToolOutput> {
    let goal = args.goal.trim();
    if goal.is_empty() {
        return Err(ToolError::invalid(
            "A summary needs a goal: say what it has to answer or concentrate on",
        ));
    }
    let backend = backend_of(env)?;
    let file = resolve(env, &args.file, env.limits().max_ingest_bytes).await?;
    let pages = approve_model_read(env, &backend, &file).await?;
    let model = env.model(ModelRole::Vision).await?;
    let summary = backend
        .summarize(
            model,
            SummaryRequest {
                bytes: file.bytes.clone(),
                extension: extension_of(&file.name).as_deref(),
                goal,
                language: args.language.as_deref().filter(|l| !l.trim().is_empty()),
                words: args.words.unwrap_or(DEFAULT_WORDS).clamp(50, 2000),
                key_points: args.key_points.unwrap_or(DEFAULT_KEY_POINTS).min(30),
                source_path: &file.path,
            },
        )
        .await?;

    let mut result = json!({
        "file": file.name,
        "goal": goal,
        "summary": summary.summary,
        "key_points": summary.key_points,
        "failed_pages": summary.failed_pages,
        "pages": pages,
    });
    let mut output = ToolOutput::new(Value::Null).classified(file.classification);
    if let Some(workspace) = env.scope().workspace.clone() {
        let path = format!("{NOTES_DIR}/{}.summary.md", output_stem(&file));
        let bytes = Bytes::from(summary.markdown.clone().into_bytes());
        env.write_workspace(&path, bytes.clone(), "text/markdown")
            .await?;
        let handle = handle_for(&workspace, &path, &bytes, "text/markdown");
        result["notes"] = json!(handle);
        output = output.saved(handle);
    }
    output.result = result;
    output.warnings.extend(summary.warnings);
    if !summary.failed_pages.is_empty() {
        output.warnings.push(format!(
            "Pages {:?} could not be read, so the summary may miss them",
            summary.failed_pages
        ));
    }
    tokens_to_usage(&mut output, summary.tokens);
    Ok(crate::files::untrusted(output))
}

fn schema_text(schema: &Value) -> ToolResult<String> {
    let text = match schema {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    if text.trim().is_empty() {
        return Err(ToolError::invalid("The schema is empty"));
    }
    serde_json::from_str::<Value>(&text)
        .map_err(|error| ToolError::invalid(format!("The schema is not valid JSON: {error}")))?;
    Ok(text)
}

pub async fn file_extract(env: &dyn ToolEnv, args: FileExtractArgs) -> ToolResult<ToolOutput> {
    let table = args
        .save_to_table
        .as_deref()
        .map(validate_table_name)
        .transpose()?;
    if args.files.is_empty() {
        return Err(ToolError::invalid("Name at least one file to extract from"));
    }
    let limits = env.limits();
    if args.files.len() > limits.max_files_per_add {
        return Err(ToolError::invalid(format!(
            "Extract from at most {} files per call; split the list",
            limits.max_files_per_add
        )));
    }
    let schema = schema_text(&args.schema)?;
    let backend = backend_of(env)?;
    let hint = args.hint.unwrap_or_default();

    let schema_tokens = schema.len() as u64 / 4;
    let mut sources = Vec::new();
    let mut failed = Vec::new();
    let mut tokens = 0u64;
    let mut pages: Option<usize> = None;
    for reference in &args.files {
        match extraction_source(env, &backend, reference).await {
            Ok(source) => {
                tokens += schema_tokens;
                match &source {
                    Source::Text(_, text) => tokens += text.len() as u64 / 4,
                    Source::NeedsModel(_, read_pages, read_tokens) => {
                        tokens += read_tokens;
                        if let Some(read_pages) = read_pages {
                            *pages.get_or_insert(0) += read_pages;
                        }
                    }
                }
                sources.push(source);
            }
            Err(error) => failed.push(failure(reference, &error)),
        }
    }
    needs_approval(env, pages, tokens)?;
    let mut done = Vec::new();
    let mut output = ToolOutput::new(Value::Null);
    let workspace = env.scope().workspace.clone();
    let mut texts = Vec::new();
    for source in sources {
        match source {
            Source::Text(file, text) => texts.push((file, text)),
            Source::NeedsModel(file, _, estimate) => {
                if env.is_cancelled() {
                    return Err(ToolError::new(code::CANCELLED, "The call was cancelled"));
                }
                match read_for_extraction(env, &backend, &file).await {
                    Ok(reading) => {
                        tokens_to_usage(&mut output, reading_usage(estimate, &reading));
                        texts.push((file, reading.converted.text));
                    }
                    Err(error) => failed.push(failure_of(&file, &error)),
                }
            }
        }
    }
    for (file, text) in texts {
        if env.is_cancelled() {
            return Err(ToolError::new(code::CANCELLED, "The call was cancelled"));
        }
        let model = env.model(ModelRole::Default).await?;
        let extracted = match backend.extract(model, &text, &schema, &hint).await {
            Ok(extracted) => extracted,
            Err(error) => {
                failed.push(json!({ "file": file.name, "code": code::FAILED, "error": format!("{error:#}") }));
                continue;
            }
        };
        tokens_to_usage(&mut output, extracted.tokens);
        output.warnings.extend(
            extracted
                .warnings
                .iter()
                .map(|warning| format!("{}: {warning}", file.name)),
        );
        let mut entry = json!({ "file": file.name, "value": extracted.value });
        if let Some(workspace) = &workspace {
            let path = normalize_path(&format!("{EXTRACTIONS_DIR}/{}.json", output_stem(&file)))?;
            let bytes = Bytes::from(serde_json::to_vec_pretty(&extracted.value).map_err(|e| {
                ToolError::new(code::FAILED, format!("Encoding the extraction failed: {e}"))
            })?);
            env.write_workspace(&path, bytes.clone(), "application/json")
                .await?;
            let handle = handle_for(workspace, &path, &bytes, "application/json");
            entry["handle"] = json!(handle);
            output = output.saved(handle);
        }
        output.classification = output.classification.max(file.classification);
        done.push(entry);
    }
    if done.is_empty() {
        let first = failed.first().cloned().unwrap_or(Value::Null);
        return Err(ToolError::new(
            code::FAILED,
            format!(
                "Nothing was extracted: {}",
                first["error"].as_str().unwrap_or("unknown error")
            ),
        ));
    }
    let mut saved_table = Value::Null;
    if let Some(table) = table {
        let workspace = crate::files::workspace_of(env, "save extractions into a table")?;
        let tables = env.app_tables(&workspace).await?;
        if !tables.writable {
            return Err(ToolError::refused(
                "The workspace tables are read-only here, so the extraction cannot be saved as a table",
            ));
        }
        let rows = extraction_rows(&done);
        let count = save_rows(&tables.connection, &table, rows, SaveMode::Append).await?;
        saved_table = json!({ "table": format!("ws.{table}"), "rows": count });
    }
    output.result = json!({ "results": done, "failed": failed, "saved_table": saved_table });
    Ok(crate::files::untrusted(output))
}

fn extraction_rows(done: &[Value]) -> Vec<Value> {
    let mut rows = Vec::new();
    for entry in done {
        let items = match &entry["value"] {
            Value::Array(items) => items.clone(),
            other => vec![other.clone()],
        };
        for item in items {
            let mut row = match item {
                Value::Object(fields) => fields,
                other => serde_json::Map::from_iter([("value".to_string(), other)]),
            };
            row.insert("_source".to_string(), entry["file"].clone());
            rows.push(Value::Object(row));
        }
    }
    rows
}

pub async fn excel_inspect(env: &dyn ToolEnv, args: ExcelInspectArgs) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    let file = resolve(env, &args.file, env.limits().max_ingest_bytes).await?;
    let operation = args.operation;
    let text = backend
        .excel(
            file.bytes.clone(),
            ExcelRequest {
                operation,
                sheet: args.sheet.clone(),
                range: args.range,
                contains: args.contains,
                max_rows: args.max_rows,
                style: args.style.unwrap_or_default(),
            },
        )
        .await
        .map_err(|error| {
            ToolError::new(
                code::UNSUPPORTED_FILE,
                format!("Reading the workbook '{}' failed: {error:#}", file.name),
            )
        })?;
    let output = ToolOutput::new(json!({
        "file": file.name,
        "operation": format!("{operation:?}"),
        "sheet": args.sheet,
        "text": text,
    }))
    .classified(file.classification);
    Ok(crate::files::untrusted(output))
}

fn failure(reference: &FileRef, error: &ToolError) -> Value {
    json!({
        "file": reference.label(),
        "code": error.code,
        "error": error.message,
    })
}

enum Source {
    Text(ResolvedFile, String),
    NeedsModel(ResolvedFile, Option<usize>, u64),
}

fn failure_of(file: &ResolvedFile, error: &ToolError) -> Value {
    json!({ "file": file.name, "code": error.code, "error": error.message })
}

/// The file's text, or, when it has none, the estimate of letting the model read it. Nothing
/// is read by the model here: the caller approves the whole call first.
async fn extraction_source(
    env: &dyn ToolEnv,
    backend: &Arc<dyn DocumentBackend>,
    reference: &FileRef,
) -> ToolResult<Source> {
    let file = resolve(env, reference, env.limits().max_ingest_bytes).await?;
    let converted = match file_text(env, &file).await {
        Ok(Some(converted)) => Ok(converted),
        Ok(None) => backend
            .convert(file.bytes.clone(), extension_of(&file.name).as_deref())
            .await
            .map_err(|error| {
                ToolError::new(
                    code::UNSUPPORTED_FILE,
                    format!("Reading '{}' ({}) failed: {error:#}", file.name, file.mime),
                )
            }),
        Err(error) => Err(error),
    };
    match converted {
        Ok(converted) if !converted.text.trim().is_empty() => {
            return Ok(Source::Text(file, converted.text));
        }
        Err(error) if error.code != code::UNSUPPORTED_FILE => return Err(error),
        _ => {}
    }
    let (pages, tokens) = model_read_estimate(backend, &file).await?;
    Ok(Source::NeedsModel(file, pages, tokens))
}

async fn read_for_extraction(
    env: &dyn ToolEnv,
    backend: &Arc<dyn DocumentBackend>,
    file: &ResolvedFile,
) -> ToolResult<ModelReading> {
    let reading = read_with_model(env, backend, file).await?;
    if reading.converted.text.trim().is_empty() {
        return Err(ToolError::new(
            code::UNSUPPORTED_FILE,
            format!(
                "'{}' has no text to extract from, even read by the model",
                file.name
            ),
        ));
    }
    Ok(reading)
}

fn select_pages(requested: Option<Vec<usize>>, count: usize) -> ToolResult<Vec<usize>> {
    let pages = match requested {
        Some(pages) if !pages.is_empty() => pages,
        Some(_) => return Err(ToolError::invalid("The page list is empty")),
        None => (1..=count).collect(),
    };
    if let Some(bad) = pages.iter().find(|page| **page == 0 || **page > count) {
        return Err(ToolError::invalid(format!(
            "Page {bad} does not exist: the document has {count} pages (numbered from 1)"
        )));
    }
    let mut unique = Vec::with_capacity(pages.len());
    for page in pages {
        if !unique.contains(&page) {
            unique.push(page);
        }
    }
    Ok(unique)
}

pub async fn pdf_render(env: &dyn ToolEnv, args: PdfRenderArgs) -> ToolResult<ToolOutput> {
    let workspace = crate::files::workspace_of(env, "keep rendered pages")?;
    let backend = backend_of(env)?;
    let limits = env.limits().clone();
    let file = resolve(env, &args.file, limits.max_ingest_bytes).await?;
    if !is_pdf(&file) {
        return Err(ToolError::new(
            code::UNSUPPORTED_FILE,
            format!("'{}' ({}) is not a PDF", file.name, file.mime),
        ));
    }
    let count = pdf_pages(&backend, &file.bytes).await?;
    let pages = select_pages(args.pages, count)?;
    needs_approval(env, Some(pages.len()), 0)?;
    let (render_backend, pdf, wanted, max_pixels) = (
        backend.clone(),
        file.bytes.to_vec(),
        pages.clone(),
        limits.pdf_max_pixels,
    );
    let rendered = blocking(move || render_backend.render_pages(pdf, &wanted, max_pixels)).await?;

    let stem = output_stem(&file);
    let mut output = ToolOutput::new(Value::Null).classified(file.classification);
    let mut listed = Vec::new();
    for (position, page) in rendered.iter().enumerate() {
        let path = format!("{RENDERS_DIR}/{stem}/page-{:03}.png", page.page);
        env.write_workspace(&path, page.png.clone(), "image/png")
            .await?;
        let handle = handle_for(&workspace, &path, &page.png, "image/png");
        let shown = position < limits.pdf_shown_pages;
        if shown {
            output.images.push(ToolImage {
                label: format!("{} page {}", file.name, page.page),
                mime: "image/png".to_string(),
                data: page.png.clone(),
            });
        }
        listed.push(json!({
            "page": page.page,
            "width": page.width,
            "height": page.height,
            "shown": shown,
            "handle": handle,
        }));
        output = output.saved(handle);
    }
    if rendered.len() > limits.pdf_shown_pages {
        output.warnings.push(format!(
            "Only the first {} pages are shown to you; the others are saved and can be looked at by asking for them (pages: [..])",
            limits.pdf_shown_pages
        ));
    }
    output.result = json!({ "file": file.name, "page_count": count, "pages": listed });
    Ok(output)
}

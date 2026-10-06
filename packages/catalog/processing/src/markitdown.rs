use flow_like::{
    bit::Bit,
    flow::{
        board::Board,
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_catalog_core::{FlowPath, NodeImage};
#[cfg(feature = "execute")]
use flow_like_types::Bytes;
#[cfg(feature = "execute")]
use flow_like_types::image::ImageReader;
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use markitdown::{
    ConversionOptions, Document, ExtractedImage, LlmConfig, MarkItDown,
    create_llm_client_with_config,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::io::Cursor;

#[cfg(feature = "execute")]
use flow_like_model_provider::summarization::{
    ChunkingMethod, DensificationStrategy, SummarizationConfig, SummarizationStrategy, TextChunk,
};

/// Represents a single page extracted from a document
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentPage {
    pub page_number: u32,
    pub content: String,
    pub images: Vec<NodeImage>,
}

impl DocumentPage {
    pub fn new(page_number: u32, content: String, images: Vec<NodeImage>) -> Self {
        Self {
            page_number,
            content,
            images,
        }
    }
}

/// Combines an array of DocumentPages into a single markdown string
pub fn pages_to_markdown(pages: &[DocumentPage]) -> String {
    let mut md = String::new();

    for (i, page) in pages.iter().enumerate() {
        if pages.len() > 1 {
            md.push_str(&format!("\n---\n## Page {}\n\n", page.page_number));
        }
        md.push_str(&page.content);
        if i < pages.len() - 1 {
            md.push('\n');
        }
    }

    md
}

/// Prompt contract of a document-parsing vision model.
///
/// The prompt strings are each model's documented prompt reproduced byte for byte,
/// including upstream whitespace and typos — these models are trained on the exact
/// string and drift silently degrades or empties their output.
#[cfg_attr(not(feature = "execute"), allow(dead_code))]
struct PromptPreset {
    name: &'static str,
    /// Prompt for a rendered document page. Empty = keep the library default.
    page: &'static str,
    /// Prompt for a standalone or embedded image. Empty = keep the library default.
    image: &'static str,
    /// Prompt for several images in one request. Empty = keep the library default;
    /// the page readers leave it empty because they never batch.
    batch: &'static str,
    /// The model reads rendered pages only. Forces full-page VLM OCR and one image
    /// per request, because it has no useful answer for a text layer or an image batch.
    page_reader: bool,
}

const DEFAULT_PAGE_PROMPT: &str = "Convert this document page to Markdown. Output only the page content.\n\n\
     - Transcribe every word exactly as printed, in the original language. Never translate, summarise or invent text.\n\
     - Follow the human reading order, including across columns.\n\
     - Use ATX headings (#, ##, ###) matching the visual hierarchy.\n\
     - Tables as Markdown; use HTML only when cells are merged or nested.\n\
     - Equations as LaTeX, $...$ inline and $$...$$ for display blocks.\n\
     - Figures, photos and diagrams as *[Figure: what it shows]*.\n\
     - Charts: recover the underlying numbers as a Markdown table when they are legible, otherwise describe the trend.\n\
     - Checkboxes as ☐ unchecked and ☑ checked.\n\
     - Keep footnotes, citations and reference lists intact.\n\
     - Wrap running headers, footers and page numbers in <header>, <footer> and <page_number> tags.\n\
     - Mark unreadable passages [illegible] instead of guessing.\n\
     - No commentary about your process, no code fences around the output, and never repeat a block you have already written.";

const DEFAULT_IMAGE_PROMPT: &str = "Describe this image so a reader who cannot see it loses nothing.\n\n\
     - Open with what the image is and what it shows.\n\
     - Transcribe all visible text exactly, in the original language.\n\
     - Tables as Markdown, equations as LaTeX.\n\
     - Charts: give the axes and series, and the data points as a Markdown table when the values are legible.\n\
     - Diagrams and flowcharts: name every node and describe the connections and direction of flow.\n\
     - Screenshots: describe the interface, then transcribe labels, fields and values.\n\
     - Note whatever carries meaning: colour coding, callouts, annotations, stamps, signatures.\n\
     - Use the source path hint, when one is given, to resolve what the image alone leaves ambiguous.\n\
     - Say plainly what is unreadable instead of guessing. No commentary about your process, no code fences.";

const DEFAULT_BATCH_PROMPT: &str = "Describe each of the following images. Emit one `## Image N` section per image, in the order given, and nothing else.\n\
     Within each section: open with what the image is, transcribe all visible text exactly in the original language, reproduce tables as Markdown and equations as LaTeX, and give the axes, series and legible data points of any chart.\n\
     Use each image's stated context (alt text, page number, source path) to resolve ambiguity. Say plainly what is unreadable instead of guessing.\n\
     Never merge two images into one section or skip a section, even when an image is blank or duplicated.";

const PRESET_OLM_OCR: &str = "Attached is one page of a document that you must process. Just return the plain text representation of this document as if you were reading it naturally. Convert equations to LateX and tables to HTML.\nIf there are any figures or charts, label them with the following markdown syntax ![Alt text describing the contents of the figure](page_startx_starty_width_height.png)\nReturn your output as markdown, with a front matter section on top specifying values for the primary_language, is_rotation_valid, rotation_correction, is_table, and is_diagram parameters.";

const PRESET_NANONETS: &str = "Extract the text from the above document as if you were reading it naturally. Return the tables in html format. Return the equations in LaTeX representation. If there is an image in the document and image caption is not present, add a small description of the image inside the <img></img> tag; otherwise, add the image caption inside <img></img>. Watermarks should be wrapped in brackets. Ex: <watermark>OFFICIAL COPY</watermark>. Page numbers should be wrapped in brackets. Ex: <page_number>14</page_number> or <page_number>9/22</page_number>. Prefer using ☐ and ☑ for check boxes.";

const PROMPT_PRESETS: &[PromptPreset] = &[
    PromptPreset {
        name: "Default",
        page: DEFAULT_PAGE_PROMPT,
        image: DEFAULT_IMAGE_PROMPT,
        batch: DEFAULT_BATCH_PROMPT,
        page_reader: false,
    },
    PromptPreset {
        name: "Unlimited-OCR",
        page: "<image>document parsing.",
        image: "<image>document parsing.",
        batch: "",
        page_reader: true,
    },
    PromptPreset {
        name: "DeepSeek-OCR",
        page: "<image>\n<|grounding|>Convert the document to markdown. ",
        image: "<image>\n<|grounding|>OCR this image.",
        batch: "",
        page_reader: true,
    },
    PromptPreset {
        name: "olmOCR",
        page: PRESET_OLM_OCR,
        image: PRESET_OLM_OCR,
        batch: "",
        page_reader: true,
    },
    PromptPreset {
        name: "Nanonets-OCR",
        page: PRESET_NANONETS,
        image: PRESET_NANONETS,
        batch: "",
        page_reader: true,
    },
    PromptPreset {
        name: "dots.ocr",
        page: "Extract the text content from this image.",
        image: "Extract the text content from this image.",
        batch: "",
        page_reader: true,
    },
    PromptPreset {
        name: "Granite-Docling",
        page: "Convert this page to docling.",
        image: "Convert this page to docling.",
        batch: "",
        page_reader: true,
    },
    PromptPreset {
        name: "PaddleOCR-VL",
        page: "OCR:",
        image: "OCR:",
        batch: "",
        page_reader: true,
    },
];

fn preset_names() -> Vec<String> {
    PROMPT_PRESETS.iter().map(|p| p.name.to_string()).collect()
}

const PROMPT_PRESET_DESCRIPTION: &str = "Prompt contract of the selected model. Document-parsing models only answer to their own trained prompt:\n\
     • Default — a tuned general prompt for vision models (GPT-4o, Claude, Gemini, Qwen-VL)\n\
     • Unlimited-OCR — baidu/Unlimited-OCR, self-hosted via vLLM\n\
     • DeepSeek-OCR — deepseek-ai/DeepSeek-OCR and -OCR-2\n\
     • olmOCR — allenai/olmOCR-2, emits YAML front matter\n\
     • Nanonets-OCR — nanonets/Nanonets-OCR-s and -OCR2\n\
     • dots.ocr — plain text extraction; use Page Prompt for its JSON layout mode\n\
     • Granite-Docling — IBM Granite-Docling and SmolDocling, emits DocTags\n\
     • PaddleOCR-VL — PaddlePaddle/PaddleOCR-VL\n\n\
     Every preset except Default forces full-page OCR and one image per request. Recommended temperature is 0.0 for all of them except olmOCR (0.1).";

#[cfg(feature = "execute")]
fn resolve_preset(name: &str) -> flow_like_types::Result<&'static PromptPreset> {
    PROMPT_PRESETS
        .iter()
        .find(|preset| preset.name == name)
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "Unknown prompt preset `{}`, expected one of: {}",
                name,
                preset_names().join(", ")
            )
        })
}

/// Pin literal wins over the preset; an empty result means "keep the library default".
#[cfg(feature = "execute")]
fn resolve_prompt(explicit: &str, preset: &str) -> Option<String> {
    let chosen = if explicit.trim().is_empty() {
        preset
    } else {
        explicit
    };
    (!chosen.is_empty()).then(|| chosen.to_string())
}

/// markitdown sends the image in its own message behind a fixed instruction, so a prompt
/// that positions the image itself cannot be honoured yet.
#[cfg(feature = "execute")]
fn warn_on_unplaceable_image_tag(context: &mut ExecutionContext, prompts: [Option<&String>; 2]) {
    let carries_tag = prompts
        .iter()
        .flatten()
        .any(|prompt| prompt.contains("<image>"));

    if carries_tag {
        context.log_message(
            "This prompt carries an <image> placeholder, but the converter sends the image in its own message behind a fixed instruction. The model will likely return empty output until markitdown supports single-message prompts.",
            flow_like::flow::execution::LogLevel::Warn,
        );
    }
}

#[cfg(feature = "execute")]
/// Reads the shared prompt pins and folds them into an [`LlmConfig`].
async fn apply_prompt_pins(
    context: &mut ExecutionContext,
    mut config: LlmConfig,
) -> flow_like_types::Result<(LlmConfig, bool)> {
    let preset_name: String = context.evaluate_pin("prompt_preset").await?;
    let page_prompt: String = context.evaluate_pin("page_prompt").await?;
    let image_prompt: String = context.evaluate_pin("image_prompt").await?;
    let batch_prompt: String = context.evaluate_pin("batch_prompt").await?;
    let force_ocr: bool = context.evaluate_pin("force_ocr").await?;

    let preset = resolve_preset(&preset_name)?;

    let page = resolve_prompt(&page_prompt, preset.page);
    let image = resolve_prompt(&image_prompt, preset.image);
    let batch = resolve_prompt(&batch_prompt, preset.batch);

    warn_on_unplaceable_image_tag(context, [page.as_ref(), image.as_ref()]);

    if let Some(page) = page {
        config = config.with_page_prompt(page);
    }
    if let Some(image) = image {
        config = config.with_image_prompt(image);
    }
    if let Some(batch) = batch {
        config = config.with_batch_prompt(batch);
    }
    if preset.page_reader {
        config = config.with_images_per_message(1);
        context.log_message(
            &format!(
                "Preset `{}` reads one rendered page per request; forcing full-page OCR and Images Per Message = 1.",
                preset.name
            ),
            flow_like::flow::execution::LogLevel::Debug,
        );
    }

    Ok((config, force_ocr || preset.page_reader))
}

fn add_prompt_pins(node: &mut Node) {
    node.add_input_pin(
        "prompt_preset",
        "Prompt Preset",
        PROMPT_PRESET_DESCRIPTION,
        VariableType::String,
    )
    .set_options(PinOptions::new().set_valid_values(preset_names()).build())
    .set_default_value(Some(json!("Default")));

    node.add_input_pin(
        "page_prompt",
        "Page Prompt",
        "Prompt for converting a rendered document page to text. Switching the preset rewrites this unless you have typed your own. Leave empty to fall back to the preset.",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));

    node.add_input_pin(
        "image_prompt",
        "Image Prompt",
        "Prompt for describing a standalone or embedded image. Switching the preset rewrites this unless you have typed your own. Leave empty to fall back to the preset.",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));

    node.add_input_pin(
        "batch_prompt",
        "Batch Image Prompt",
        "Prompt used when Images Per Message is greater than 1. Only the Default preset fills this in — the OCR presets never batch. Leave empty to fall back to the preset.",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));

    node.add_input_pin(
        "force_ocr",
        "Force OCR",
        "Run every PDF page through the model instead of only pages whose extracted text looks poor. Presets other than Default turn this on regardless.",
        VariableType::Boolean,
    )
    .set_default_value(Some(json!(false)));
}

fn pin_literal(node: &Node, name: &str) -> String {
    node.get_pin_by_name(name)
        .and_then(|pin| pin.default_value.as_ref())
        .and_then(|bytes| flow_like_types::json::from_slice::<flow_like_types::Value>(bytes).ok())
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// A literal belongs to the presets only while it is still byte-identical to one of them.
/// The `Default` row contributes the empty string, so an untouched pin counts as preset-owned.
fn is_preset_authored(current: &str, field: fn(&PromptPreset) -> &'static str) -> bool {
    current.is_empty() || PROMPT_PRESETS.iter().any(|preset| field(preset) == current)
}

fn sync_prompt_pin(
    node: &mut Node,
    pin_name: &str,
    target: &str,
    field: fn(&PromptPreset) -> &'static str,
) {
    let current = pin_literal(node, pin_name);
    if current == target || !is_preset_authored(&current, field) {
        return;
    }

    if let Some(pin) = node.get_pin_mut_by_name(pin_name) {
        pin.set_default_value(Some(json!(target)));
    }
}

/// Materialises the selected preset into the prompt pins so the prompt is visible and
/// editable on the node.
///
/// This runs on every board parse, so it writes only when the literal actually differs,
/// and it never overwrites a prompt the user typed — switching back to `Default` clears a
/// preset's prompt but leaves a hand-written one untouched.
fn apply_preset_to_pins(node: &mut Node) {
    let selected = pin_literal(node, "prompt_preset");
    let Some(preset) = PROMPT_PRESETS.iter().find(|preset| preset.name == selected) else {
        return;
    };

    let (page, image, batch) = (preset.page, preset.image, preset.batch);
    sync_prompt_pin(node, "page_prompt", page, |preset| preset.page);
    sync_prompt_pin(node, "image_prompt", image, |preset| preset.image);
    sync_prompt_pin(node, "batch_prompt", batch, |preset| preset.batch);
}

#[cfg(feature = "execute")]
/// Safely calls markitdown convert_bytes, catching any panics from the underlying crate.
async fn safe_convert_bytes(
    _md: &MarkItDown,
    bytes: Bytes,
    options: Option<ConversionOptions>,
) -> flow_like_types::Result<Document> {
    use flow_like_types::tokio;

    let handle = tokio::task::spawn(async move {
        let md = MarkItDown::new();
        md.convert_bytes(bytes, options).await
    });

    match handle.await {
        Ok(Ok(doc)) => Ok(doc),
        Ok(Err(e)) => Err(flow_like_types::anyhow!(
            "Document conversion failed: {}",
            e
        )),
        Err(e) if e.is_panic() => {
            let panic_msg = if let Ok(reason) = e.try_into_panic() {
                if let Some(s) = reason.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = reason.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "Unknown panic".to_string()
                }
            } else {
                "Unknown panic".to_string()
            };
            Err(flow_like_types::anyhow!(
                "Document conversion panicked: {}",
                panic_msg
            ))
        }
        Err(e) => Err(flow_like_types::anyhow!(
            "Document conversion task failed: {}",
            e
        )),
    }
}

#[cfg(feature = "execute")]
/// Converts a markitdown Document to DocumentPages, caching images
async fn document_to_pages(
    document: &Document,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<Vec<DocumentPage>> {
    let mut pages = Vec::with_capacity(document.pages.len());

    for page in &document.pages {
        let content = page.to_markdown();
        let mut images = Vec::new();

        for extracted in page.images() {
            if let Some(node_image) = extracted_image_to_node_image(extracted, context).await? {
                images.push(node_image);
            }
        }

        pages.push(DocumentPage::new(page.page_number, content, images));
    }

    Ok(pages)
}

#[cfg(feature = "execute")]
/// Converts a markitdown ExtractedImage to a NodeImage by decoding and caching
async fn extracted_image_to_node_image(
    extracted: &ExtractedImage,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<Option<NodeImage>> {
    if extracted.data.is_empty() {
        return Ok(None);
    }

    let cursor = Cursor::new(extracted.data.as_ref());
    let mut reader = ImageReader::new(cursor);

    if let Some(format) = flow_like_types::image::ImageFormat::from_mime_type(&extracted.mime_type)
    {
        reader.set_format(format);
    } else {
        reader = reader
            .with_guessed_format()
            .map_err(|e| flow_like_types::anyhow!("Failed to guess image format: {}", e))?;
    }

    let dynamic_image = match reader.decode() {
        Ok(img) => img,
        Err(e) => {
            tracing::warn!(
                "Skipping image '{}' (mime: {}): {}",
                extracted.id,
                extracted.mime_type,
                e
            );
            return Ok(None);
        }
    };

    let node_image = NodeImage::new(context, dynamic_image).await;
    Ok(Some(node_image))
}

#[crate::register_node]
#[derive(Default)]
pub struct ExtractDocumentNode {}

impl ExtractDocumentNode {
    pub fn new() -> Self {
        ExtractDocumentNode {}
    }
}

#[async_trait]
impl NodeLogic for ExtractDocumentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_processing_extract_document",
            "Extract Document",
            "Extracts text and content from documents (PDF, DOCX, XLSX, images, etc.) and converts to markdown.",
            "AI/Processing",
        );
        node.set_flowscript_name("ai.processing", "extractDocument");
        node.add_icon("/flow/icons/file-text.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(8)
                .set_governance(10)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to start document extraction.",
            VariableType::Execution,
        );

        node.add_input_pin(
            "file",
            "File",
            "Document file to extract (PDF, DOCX, XLSX, images, etc.).",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "extract_images",
            "Extract Images",
            "Whether to extract and embed images from the document.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution output after extraction completes.",
            VariableType::Execution,
        );

        node.add_output_pin(
            "pages",
            "Pages",
            "Extracted document pages with content and images.",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<DocumentPage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let file: FlowPath = context.evaluate_pin("file").await?;
        let extract_images: bool = context.evaluate_pin("extract_images").await?;

        let file_path = file.object_path();
        let extension = file_path
            .extension()
            .map(|e| format!(".{}", e))
            .unwrap_or_default();

        let file_buffer = file.get(context, false).await?;
        let bytes = Bytes::from(file_buffer);

        let options = ConversionOptions::default()
            .with_extension(&extension)
            .with_image_context_path(file.path)
            .with_images(extract_images);

        let md = MarkItDown::new();
        let result = safe_convert_bytes(&md, bytes, Some(options)).await?;

        let pages = document_to_pages(&result, context).await?;

        context.set_pin_value("pages", json!(pages)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Document processing requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ExtractDocumentAiNode {}

impl ExtractDocumentAiNode {
    pub fn new() -> Self {
        ExtractDocumentAiNode {}
    }
}

#[async_trait]
impl NodeLogic for ExtractDocumentAiNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_processing_extract_document_ai",
            "AI Extract Document",
            "Extracts text and content from documents using AI for enhanced image descriptions and OCR.",
            "AI/Processing",
        );
        node.set_flowscript_name("ai.processing", "extractDocumentAi");
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_version(3);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(4)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to start AI-powered document extraction.",
            VariableType::Execution,
        );

        node.add_input_pin(
            "file",
            "File",
            "Document file to extract (PDF, DOCX, XLSX, images, etc.).",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "model",
            "Model",
            "Vision-capable AI model for image analysis and OCR.",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "extract_images",
            "Extract Images",
            "Whether to extract and embed images from the document.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "images_per_message",
            "Images Per Message",
            "Number of images to batch per LLM request (higher = faster but may hit token limits).",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "pages_per_batch",
            "Pages Per Batch",
            "Number of PDF pages to process in parallel (higher = faster but uses more memory).",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(4)));

        node.add_input_pin(
            "temperature",
            "Temperature",
            "LLM temperature (0.0 = deterministic, 1.0 = creative). Lower is better for extraction.",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.1)));

        node.add_input_pin(
            "max_tokens",
            "Max Tokens",
            "Maximum output tokens per LLM call. Leave at 0 for model default. Set lower for unreliable models.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        add_prompt_pins(&mut node);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution output after extraction completes.",
            VariableType::Execution,
        );

        node.add_output_pin(
            "pages",
            "Pages",
            "Extracted document pages with AI-generated descriptions and images.",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<DocumentPage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let file: FlowPath = context.evaluate_pin("file").await?;
        let model_bit = context.evaluate_pin::<Bit>("model").await?;
        let extract_images: bool = context.evaluate_pin("extract_images").await?;
        let images_per_message: i64 = context.evaluate_pin("images_per_message").await?;
        let pages_per_batch: i64 = context.evaluate_pin("pages_per_batch").await?;
        let temperature: f64 = context.evaluate_pin("temperature").await?;
        let max_tokens: i64 = context.evaluate_pin("max_tokens").await?;

        let file_path = file.object_path();
        let extension = file_path
            .extension()
            .map(|e| format!(".{}", e))
            .unwrap_or_default();

        let file_buffer = file.get(context, false).await?;
        let bytes = Bytes::from(file_buffer);

        let model_factory = context.app_state.model_factory.clone();
        let model = model_factory
            .build(
                &model_bit,
                context.app_state.clone(),
                context.token.clone(),
                context.model_usage_context(),
            )
            .await?;

        #[allow(deprecated)]
        let completion_handle = model.completion_model_handle(None).await?;

        let llm_config = LlmConfig::default()
            .with_images_per_message(images_per_message.max(1) as usize)
            .with_pages_per_batch(pages_per_batch.max(1) as usize)
            .with_temperature(temperature)
            .with_max_tokens(if max_tokens > 0 {
                Some(max_tokens as u64)
            } else {
                None
            });

        let (llm_config, force_ocr) = apply_prompt_pins(context, llm_config).await?;

        let llm_client = create_llm_client_with_config(completion_handle, llm_config);

        let md = MarkItDown::new();
        let file_path_clone = file.path.clone();
        let mut options = ConversionOptions::default()
            .with_extension(&extension)
            .with_image_context_path(file.path)
            .with_images(extract_images)
            .with_force_llm_ocr(force_ocr)
            .with_llm(llm_client);
        options.url = Some(file_path_clone);

        let result = safe_convert_bytes(&md, bytes, Some(options)).await?;

        let pages = document_to_pages(&result, context).await?;

        context.set_pin_value("pages", json!(pages)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Processing requires the 'execute' feature"
        ))
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        apply_preset_to_pins(node);
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ExtractDocumentsNode {}

impl ExtractDocumentsNode {
    pub fn new() -> Self {
        ExtractDocumentsNode {}
    }
}

#[async_trait]
impl NodeLogic for ExtractDocumentsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_processing_extract_documents",
            "Extract Documents",
            "Extracts text and content from multiple documents in parallel.",
            "AI/Processing",
        );
        node.set_flowscript_name("ai.processing", "extractDocuments");
        node.add_icon("/flow/icons/files.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_governance(10)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to start batch document extraction.",
            VariableType::Execution,
        );

        node.add_input_pin(
            "files",
            "Files",
            "Array of document files to extract.",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "extract_images",
            "Extract Images",
            "Whether to extract and embed images from documents.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution output after all extractions complete.",
            VariableType::Execution,
        );

        node.add_output_pin(
            "results",
            "Results",
            "Array of extracted document pages for each file.",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<DocumentPage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let files: Vec<FlowPath> = context.evaluate_pin("files").await?;
        let extract_images: bool = context.evaluate_pin("extract_images").await?;

        let md = MarkItDown::new();
        let mut all_results: Vec<Vec<DocumentPage>> = Vec::with_capacity(files.len());

        for file in files {
            let file_path = file.object_path();
            let extension = file_path
                .extension()
                .map(|e| format!(".{}", e))
                .unwrap_or_default();

            let file_buffer = file.get(context, false).await?;
            let bytes = Bytes::from(file_buffer);

            let options = ConversionOptions::default()
                .with_extension(&extension)
                .with_image_context_path(file.path)
                .with_images(extract_images);

            let result = safe_convert_bytes(&md, bytes, Some(options)).await?;

            let pages = document_to_pages(&result, context).await?;
            all_results.push(pages);
        }

        let flat_results: Vec<DocumentPage> = all_results.into_iter().flatten().collect();
        context
            .set_pin_value("results", json!(flat_results))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Processing requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ExtractDocumentsAiNode {}

impl ExtractDocumentsAiNode {
    pub fn new() -> Self {
        ExtractDocumentsAiNode {}
    }
}

#[async_trait]
impl NodeLogic for ExtractDocumentsAiNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_processing_extract_documents_ai",
            "AI Extract Documents",
            "Extracts text and content from multiple documents using AI in parallel.",
            "AI/Processing",
        );
        node.set_flowscript_name("ai.processing", "extractDocumentsAi");
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_version(3);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(3)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to start AI-powered batch extraction.",
            VariableType::Execution,
        );

        node.add_input_pin(
            "files",
            "Files",
            "Array of document files to extract.",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "model",
            "Model",
            "Vision-capable AI model for image analysis and OCR.",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "extract_images",
            "Extract Images",
            "Whether to extract and embed images from documents.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "images_per_message",
            "Images Per Message",
            "Number of images to batch per LLM request (higher = faster but may hit token limits).",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(2)));

        node.add_input_pin(
            "pages_per_batch",
            "Pages Per Batch",
            "Number of PDF pages to process in parallel (higher = faster but uses more memory).",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(2)));

        node.add_input_pin(
            "temperature",
            "Temperature",
            "LLM temperature (0.0 = deterministic, 1.0 = creative). Lower is better for extraction.",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.1)));

        node.add_input_pin(
            "max_tokens",
            "Max Tokens",
            "Maximum output tokens per LLM call. Leave at 0 for model default. Set lower for unreliable models.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(4096)));

        add_prompt_pins(&mut node);

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution output after all extractions complete.",
            VariableType::Execution,
        );

        node.add_output_pin(
            "results",
            "Results",
            "Array of extracted document pages with AI descriptions for each file.",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<DocumentPage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let files: Vec<FlowPath> = context.evaluate_pin("files").await?;
        let model_bit = context.evaluate_pin::<Bit>("model").await?;
        let extract_images: bool = context.evaluate_pin("extract_images").await?;
        let images_per_message: i64 = context.evaluate_pin("images_per_message").await?;
        let pages_per_batch: i64 = context.evaluate_pin("pages_per_batch").await?;
        let temperature: f64 = context.evaluate_pin("temperature").await?;
        let max_tokens: i64 = context.evaluate_pin("max_tokens").await?;

        let model_factory = context.app_state.model_factory.clone();
        let model = model_factory
            .build(
                &model_bit,
                context.app_state.clone(),
                context.token.clone(),
                context.model_usage_context(),
            )
            .await?;

        #[allow(deprecated)]
        let completion_handle = model.completion_model_handle(None).await?;

        let llm_config = LlmConfig::default()
            .with_images_per_message(images_per_message.max(1) as usize)
            .with_pages_per_batch(pages_per_batch.max(1) as usize)
            .with_temperature(temperature)
            .with_max_tokens(if max_tokens > 0 {
                Some(max_tokens as u64)
            } else {
                None
            });

        let (llm_config, force_ocr) = apply_prompt_pins(context, llm_config).await?;

        let llm_client = create_llm_client_with_config(completion_handle, llm_config);

        let md = MarkItDown::new();
        let mut all_results: Vec<Vec<DocumentPage>> = Vec::with_capacity(files.len());

        for file in files {
            let file_path = file.object_path();
            let extension = file_path
                .extension()
                .map(|e| format!(".{}", e))
                .unwrap_or_default();

            let file_buffer = file.get(context, false).await?;
            let bytes = Bytes::from(file_buffer);

            let options = ConversionOptions::default()
                .with_extension(&extension)
                .with_images(extract_images)
                .with_image_context_path(file.path.clone())
                .with_force_llm_ocr(force_ocr)
                .with_llm(llm_client.clone());

            let result = safe_convert_bytes(&md, bytes, Some(options)).await?;

            let pages = document_to_pages(&result, context).await?;
            all_results.push(pages);
        }

        let flat_results: Vec<DocumentPage> = all_results.into_iter().flatten().collect();
        context
            .set_pin_value("results", json!(flat_results))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Processing requires the 'execute' feature"
        ))
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        apply_preset_to_pins(node);
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct PagesToMarkdownNode {}

impl PagesToMarkdownNode {
    pub fn new() -> Self {
        PagesToMarkdownNode {}
    }
}

#[async_trait]
impl NodeLogic for PagesToMarkdownNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_processing_pages_to_markdown",
            "Pages to Markdown",
            "Combines an array of document pages into a single markdown string.",
            "AI/Processing",
        );
        node.set_flowscript_name("ai.processing", "pagesToMarkdown");
        node.add_icon("/flow/icons/file-text.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(10)
                .set_governance(10)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to combine pages.",
            VariableType::Execution,
        );

        node.add_input_pin(
            "pages",
            "Pages",
            "Array of document pages to combine.",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<DocumentPage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution output after combining pages.",
            VariableType::Execution,
        );

        node.add_output_pin(
            "markdown",
            "Markdown",
            "Combined markdown content from all pages.",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let pages: Vec<DocumentPage> = context.evaluate_pin("pages").await?;
        let markdown = pages_to_markdown(&pages);

        context.set_pin_value("markdown", json!(markdown)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

/// Detail level for document summarization
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
pub enum SummaryDetailLevel {
    Low,
    #[default]
    Medium,
    High,
}

impl SummaryDetailLevel {
    #[allow(dead_code)]
    fn target_ratio(&self) -> f32 {
        match self {
            Self::Low => 0.05,
            Self::Medium => 0.15,
            Self::High => 0.30,
        }
    }

    #[cfg(feature = "execute")]
    fn system_prompt(&self, include_toc: bool) -> String {
        let detail_instruction = match self {
            Self::Low => {
                "Create a very concise summary capturing only the most essential points. Focus on the main thesis, key conclusions, and critical takeaways. Omit details, examples, and supporting evidence."
            }
            Self::Medium => {
                "Create a balanced summary that covers main topics, key arguments, and important details. Include significant examples and supporting points while maintaining brevity."
            }
            Self::High => {
                "Create a comprehensive summary preserving most important information, including main topics, key arguments, supporting evidence, examples, and nuances. Maintain logical flow and relationships between concepts."
            }
        };

        let toc_instruction = if include_toc {
            "\n\nInclude a table of contents at the beginning with page references where each topic can be found. Format as:\n## Table of Contents\n- [Topic Name](#topic) (Pages X-Y)\n"
        } else {
            ""
        };

        format!(
            "You are a document summarization expert. {detail_instruction}{toc_instruction}\n\n\
            Guidelines:\n\
            - Preserve key terminology and domain-specific language\n\
            - Maintain factual accuracy\n\
            - Use clear, professional language\n\
            - Structure the summary logically\n\
            - Extract and highlight keywords that characterize the document's focus areas"
        )
    }
}

/// Result of document summarization containing the summary text and extracted keywords
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentSummary {
    pub summary: String,
    pub keywords: Vec<String>,
    pub page_references: Vec<PageReference>,
}

/// Reference to content location within the document
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PageReference {
    pub topic: String,
    pub pages: Vec<u32>,
}

/// A content section with semantic grouping across pages
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContentSection {
    pub title: String,
    pub summary: String,
    pub keywords: Vec<String>,
    pub page_references: Vec<u32>,
}

#[cfg(feature = "execute")]
fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

#[cfg(feature = "execute")]
fn chunk_pages_for_context(pages: &[DocumentPage], max_tokens: usize) -> Vec<Vec<&DocumentPage>> {
    let mut chunks = Vec::new();
    let mut current_chunk = Vec::new();
    let mut current_tokens = 0;

    for page in pages {
        let page_tokens = estimate_tokens(&page.content);
        if current_tokens + page_tokens > max_tokens && !current_chunk.is_empty() {
            chunks.push(current_chunk);
            current_chunk = Vec::new();
            current_tokens = 0;
        }
        current_chunk.push(page);
        current_tokens += page_tokens;
    }

    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }

    chunks
}

#[cfg(feature = "execute")]
fn format_pages_for_prompt(pages: &[&DocumentPage]) -> String {
    pages
        .iter()
        .map(|p| format!("[Page {}]\n{}", p.page_number, p.content))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}

#[cfg(feature = "execute")]
async fn invoke_model_simple(
    context: &ExecutionContext,
    model_bit: &Bit,
    system_prompt: &str,
    user_prompt: &str,
) -> flow_like_types::Result<String> {
    invoke_model_simple_standalone(
        &context.app_state,
        &context.token,
        context.model_usage_context(),
        model_bit,
        system_prompt,
        user_prompt,
    )
    .await
}

#[cfg(feature = "execute")]
async fn invoke_model_simple_standalone(
    app_state: &std::sync::Arc<flow_like::state::FlowLikeState>,
    access_token: &Option<String>,
    usage_context: Option<flow_like::models::llm::ModelUsageContext>,
    model_bit: &Bit,
    system_prompt: &str,
    user_prompt: &str,
) -> flow_like_types::Result<String> {
    use flow_like_model_provider::history::{History, HistoryMessage, Role};
    use flow_like_model_provider::llm::LLMCallback;
    use flow_like_model_provider::response_chunk::ResponseChunk;

    let model_factory = app_state.model_factory.clone();
    let model = model_factory
        .build(
            model_bit,
            app_state.clone(),
            access_token.clone(),
            usage_context,
        )
        .await?;

    let model_name = model_bit
        .meta
        .get("name")
        .map(|m| m.name.clone())
        .unwrap_or_else(|| model_bit.id.clone());

    let mut history = History::new(model_name, vec![]);
    history.set_system_prompt(system_prompt.to_string());
    history.push_message(HistoryMessage::from_string(Role::User, user_prompt));

    // Use a noop streaming callback to force the streaming code path, which is
    // more robust across providers (avoids JsonError when providers default to streaming).
    let callback: LLMCallback =
        std::sync::Arc::new(move |_chunk: ResponseChunk| Box::pin(async move { Ok(()) }));

    let response = model.invoke(&history, Some(callback)).await?;

    response
        .last_message()
        .and_then(|m| m.content.clone())
        .ok_or_else(|| flow_like_types::anyhow!("No response from model"))
}

#[crate::register_node]
#[derive(Default)]
pub struct SummarizeDocumentNode {}

impl SummarizeDocumentNode {
    pub fn new() -> Self {
        SummarizeDocumentNode {}
    }
}

#[async_trait]
impl NodeLogic for SummarizeDocumentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_processing_summarize_document",
            "Summarize Document",
            "Creates an intelligent summary of document pages using AI with configurable strategies and detail levels. Handles long documents via chunked summarization with multiple strategy options.",
            "AI/Processing",
        );
        node.set_flowscript_name("ai.processing", "summarizeDocument");
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_version(4);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(4)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to start summarization.",
            VariableType::Execution,
        );

        node.add_input_pin(
            "pages",
            "Pages",
            "Document pages to summarize.",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<DocumentPage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "model",
            "Model",
            "AI model to use for summarization.",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "detail_level",
            "Detail Level",
            "Summary detail level: Low (very concise), Medium (balanced), High (comprehensive).",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Low".to_string(),
                    "Medium".to_string(),
                    "High".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Medium")));

        node.add_input_pin(
            "include_toc",
            "Include TOC",
            "Whether to include a table of contents with page references.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "strategy",
            "Strategy",
            "Summarization strategy:\n\
             • Refine — sequential, best coherence, no parallelism\n\
             • MapReduce — parallel chunking, fast, may lose cross-chunk context\n\
             • Hierarchical — structure-aware tree, best for headed documents\n\
             • Hybrid — MapReduce speed + Refine coherence polish\n\
             • SlidingWindow — fixed memory buffer, best for very long documents",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Refine".to_string(),
                    "MapReduce".to_string(),
                    "Hierarchical".to_string(),
                    "Hybrid".to_string(),
                    "SlidingWindow".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Refine")));

        node.add_input_pin(
            "densification",
            "Densification",
            "Post-processing to increase information density:\n\
             • None — use the strategy output as-is\n\
             • ChainOfDensity — iteratively compress to optimal density",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["None".to_string(), "ChainOfDensity".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("None")));

        node.add_input_pin(
            "max_context_tokens",
            "Max Context Tokens",
            "Maximum characters per summarization chunk (adjust based on model context window).",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(8000)));

        node.add_input_pin(
            "chunk_overlap",
            "Chunk Overlap %",
            "Overlap between adjacent chunks as percentage (0-50). Prevents information loss at boundaries (default: 10).",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_input_pin(
            "track_entities",
            "Track Entities",
            "Extract and track named entities across chunks to prevent information loss.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "parallel_requests",
            "Parallel Requests",
            "Number of chunks to process in parallel for MapReduce/Hybrid strategies. 0 = unlimited (default: 4).",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(4)));

        node.add_input_pin(
            "density_steps",
            "Density Steps",
            "Number of Chain of Density refinement steps when densification is enabled (1-5, default: 3).",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(3)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution output after summarization completes.",
            VariableType::Execution,
        );

        node.add_output_pin(
            "summary",
            "Summary",
            "The generated document summary.",
            VariableType::Struct,
        )
        .set_schema::<DocumentSummary>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let pages: Vec<DocumentPage> = context.evaluate_pin("pages").await?;
        let model_bit: Bit = context.evaluate_pin("model").await?;
        let detail_str: String = context.evaluate_pin("detail_level").await?;
        let include_toc: bool = context.evaluate_pin("include_toc").await?;
        let strategy_str: String = context.evaluate_pin("strategy").await?;
        let densification_str: String = context.evaluate_pin("densification").await?;
        let max_context_tokens: i64 = context.evaluate_pin("max_context_tokens").await?;
        let chunk_overlap: i64 = context.evaluate_pin("chunk_overlap").await?;
        let track_entities: bool = context.evaluate_pin("track_entities").await?;
        let parallel_requests: i64 = context.evaluate_pin("parallel_requests").await?;
        let density_steps: i64 = context.evaluate_pin("density_steps").await?;

        let detail_level = match detail_str.as_str() {
            "Low" => SummaryDetailLevel::Low,
            "High" => SummaryDetailLevel::High,
            _ => SummaryDetailLevel::Medium,
        };

        if pages.is_empty() {
            let empty_summary = DocumentSummary {
                summary: String::new(),
                keywords: vec![],
                page_references: vec![],
            };
            context
                .set_pin_value("summary", json!(empty_summary))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        let strategy = SummarizationStrategy::try_from(strategy_str.as_str()).unwrap_or_default();
        let densification =
            DensificationStrategy::try_from(densification_str.as_str()).unwrap_or_default();

        let instructions = detail_level.system_prompt(include_toc);

        let mut model_name = model_bit.id.clone();
        if let Some(meta) = model_bit.meta.get("en") {
            model_name = meta.name.clone();
        }

        let model_factory = context.app_state.model_factory.clone();
        let model = model_factory
            .build(
                &model_bit,
                context.app_state.clone(),
                context.token.clone(),
                context.model_usage_context(),
            )
            .await?;

        let chunks: Vec<TextChunk> = pages
            .iter()
            .map(|p| {
                let content = format!("[Page {}]\n{}", p.page_number, p.content);
                TextChunk::new(content, p.page_number as usize)
                    .with_metadata(format!("Page {}", p.page_number))
            })
            .collect();

        let config = SummarizationConfig {
            strategy,
            densification,
            chunking: ChunkingMethod::Markdown,
            chunk_size: max_context_tokens as usize,
            chunk_overlap_percent: (chunk_overlap as u8).min(50),
            max_iterations: 5,
            track_entities,
            instructions,
            prior_summary: String::new(),
            concurrency: parallel_requests as usize,
            density_steps: density_steps as u32,
            memory_budget_ratio: 0.4,
        };

        let result = flow_like_model_provider::summarization::summarize_chunks(
            &chunks,
            &config,
            model.as_ref(),
            &model_name,
        )
        .await?;

        let (summary_text, mut keywords, page_refs) = parse_summary_response(&result.summary);

        if !result.entities.is_empty() {
            for entity in &result.entities {
                if !keywords.contains(entity) {
                    keywords.push(entity.clone());
                }
            }
        }

        let doc_summary = DocumentSummary {
            summary: summary_text,
            keywords,
            page_references: page_refs,
        };

        context.set_pin_value("summary", json!(doc_summary)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Document summarization requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn parse_summary_response(response: &str) -> (String, Vec<String>, Vec<PageReference>) {
    let mut keywords = Vec::new();
    let mut page_refs = Vec::new();
    let mut summary_lines = Vec::new();

    for line in response.lines() {
        let trimmed = line.trim();
        if trimmed.to_lowercase().starts_with("keywords:") {
            let kw_str = trimmed
                .strip_prefix("keywords:")
                .or_else(|| trimmed.strip_prefix("Keywords:"))
                .unwrap_or("");
            keywords.extend(
                kw_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
            );
        } else if let Some(page_ref) = parse_page_reference_line(trimmed) {
            page_refs.push(page_ref);
        } else {
            summary_lines.push(line);
        }
    }

    (
        summary_lines.join("\n").trim().to_string(),
        keywords,
        page_refs,
    )
}

#[cfg(feature = "execute")]
fn parse_page_reference_line(line: &str) -> Option<PageReference> {
    if !line.contains("Page") && !line.contains("page") {
        return None;
    }

    let page_pattern = regex::Regex::new(r"[Pp]ages?\s*(\d+(?:\s*[-,]\s*\d+)*)").ok()?;
    let captures = page_pattern.captures(line)?;
    let page_str = captures.get(1)?.as_str();

    let pages: Vec<u32> = page_str
        .split([',', '-'])
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    if pages.is_empty() {
        return None;
    }

    let topic = line
        .split(['(', '['])
        .next()
        .unwrap_or(line)
        .trim()
        .trim_start_matches('-')
        .trim()
        .to_string();

    Some(PageReference { topic, pages })
}

#[crate::register_node]
#[derive(Default)]
pub struct ExtractContentSectionsNode {}

impl ExtractContentSectionsNode {
    pub fn new() -> Self {
        ExtractContentSectionsNode {}
    }
}

#[async_trait]
impl NodeLogic for ExtractContentSectionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_processing_extract_content_sections",
            "Extract Content Sections",
            "Intelligently segments document into thematic sections with summaries, tracking content across non-contiguous pages. Ideal for large document corpora.",
            "AI/Processing",
        );
        node.set_flowscript_name("ai.processing", "extractContentSections");
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_version(2);

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(5)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(3)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to start section extraction.",
            VariableType::Execution,
        );

        node.add_input_pin(
            "pages",
            "Pages",
            "Document pages to segment into sections.",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<DocumentPage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "model",
            "Model",
            "AI model for semantic analysis.",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "max_context_tokens",
            "Max Context Tokens",
            "Maximum tokens per analysis chunk.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(8000)));

        node.add_input_pin(
            "parallel_requests",
            "Parallel Requests",
            "Number of chunks to process in parallel. Set to 0 or chunks count to process all at once.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(4)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution output after extraction completes.",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sections",
            "Sections",
            "Array of thematic content sections with cross-page tracking.",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<ContentSection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let pages: Vec<DocumentPage> = context.evaluate_pin("pages").await?;
        let model_bit: Bit = context.evaluate_pin("model").await?;
        let max_context_tokens: i64 = context.evaluate_pin("max_context_tokens").await?;
        let parallel_requests: i64 = context.evaluate_pin("parallel_requests").await?;

        if pages.is_empty() {
            context
                .set_pin_value("sections", json!(Vec::<ContentSection>::new()))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        let system_prompt = r#"You are a document analysis expert specializing in semantic segmentation.
Your task is to identify distinct thematic sections within document content, even when topics span non-contiguous pages.

For each distinct topic or theme you identify, provide:
1. SECTION_TITLE: A concise, descriptive title
2. SECTION_SUMMARY: A brief summary of that topic's content
3. SECTION_KEYWORDS: Comma-separated keywords specific to this section
4. SECTION_PAGES: Page numbers where this topic appears

Format each section exactly as:
===SECTION===
TITLE: [title]
SUMMARY: [summary]
KEYWORDS: [keyword1, keyword2, ...]
PAGES: [page numbers]
===END_SECTION===

Group related content even if it appears on different pages. Focus on semantic coherence, not page order.
Extract specific, domain-relevant keywords that would help identify this content in a large corpus."#;

        let chunks = chunk_pages_for_context(&pages, max_context_tokens as usize);
        let num_chunks = chunks.len();

        let concurrency = if parallel_requests <= 0 {
            num_chunks
        } else {
            (parallel_requests as usize).min(num_chunks)
        };

        let all_raw_sections: Vec<ContentSection> = if concurrency <= 1 {
            // Sequential processing - no cross-chunk merge needed for single chunk
            let mut results = Vec::new();
            for chunk in &chunks {
                let content = format_pages_for_prompt(chunk);
                let user_prompt = format!(
                    "Analyze the following document content and extract thematic sections:\n\n{}",
                    content
                );

                let response =
                    invoke_model_simple(context, &model_bit, system_prompt, &user_prompt).await?;
                results.extend(parse_sections_response(&response));
            }
            results
        } else {
            // Parallel processing
            use futures::stream::{self, StreamExt};
            use std::sync::Arc;

            let model_bit = Arc::new(model_bit.clone());
            let system_prompt = Arc::new(system_prompt.to_string());
            let app_state = context.app_state.clone();
            let token = context.token.clone();
            let usage_context = context.model_usage_context();

            let tasks: Vec<_> = chunks
                .iter()
                .map(|chunk| {
                    let content = format_pages_for_prompt(chunk);
                    let user_prompt = format!(
                        "Analyze the following document content and extract thematic sections:\n\n{}",
                        content
                    );

                    let model_bit = Arc::clone(&model_bit);
                    let system_prompt = Arc::clone(&system_prompt);
                    let app_state = app_state.clone();
                    let token = token.clone();
                    let usage_context = usage_context.clone();

                    async move {
                        let response = invoke_model_simple_standalone(
                            &app_state,
                            &token,
                            usage_context,
                            &model_bit,
                            &system_prompt,
                            &user_prompt,
                        )
                        .await?;
                        Ok::<_, flow_like_types::Error>(parse_sections_response(&response))
                    }
                })
                .collect();

            let chunk_results: Vec<Vec<ContentSection>> = stream::iter(tasks)
                .buffer_unordered(concurrency)
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?;

            chunk_results.into_iter().flatten().collect()
        };

        // Only merge if we have multiple chunks AND parallel processing was used
        let sections = if concurrency > 1 && num_chunks > 1 && !all_raw_sections.is_empty() {
            use std::sync::Arc;
            merge_related_sections(context, &Arc::new(model_bit), all_raw_sections).await?
        } else {
            all_raw_sections
        };

        context.set_pin_value("sections", json!(sections)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Content section extraction requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn parse_sections_response(response: &str) -> Vec<ContentSection> {
    let mut sections = Vec::new();
    let section_pattern = regex::Regex::new(r"===SECTION===(.*?)===END_SECTION===").ok();

    if let Some(pattern) = section_pattern {
        for cap in pattern.captures_iter(response) {
            if let Some(content) = cap.get(1)
                && let Some(section) = parse_single_section(content.as_str())
            {
                sections.push(section);
            }
        }
    }

    if sections.is_empty()
        && let Some(section) = parse_single_section(response)
    {
        sections.push(section);
    }

    sections
}

#[cfg(feature = "execute")]
fn parse_single_section(content: &str) -> Option<ContentSection> {
    let mut title = String::new();
    let mut summary = String::new();
    let mut keywords = Vec::new();
    let mut pages = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(t) = trimmed.strip_prefix("TITLE:") {
            title = t.trim().to_string();
        } else if let Some(s) = trimmed.strip_prefix("SUMMARY:") {
            summary = s.trim().to_string();
        } else if let Some(k) = trimmed.strip_prefix("KEYWORDS:") {
            keywords = k
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        } else if let Some(p) = trimmed.strip_prefix("PAGES:") {
            pages = p
                .split(|c: char| !c.is_ascii_digit())
                .filter_map(|s| s.trim().parse().ok())
                .collect();
        }
    }

    if title.is_empty() && summary.is_empty() {
        return None;
    }

    Some(ContentSection {
        title: if title.is_empty() {
            "Untitled Section".to_string()
        } else {
            title
        },
        summary,
        keywords,
        page_references: pages,
    })
}

#[cfg(feature = "execute")]
async fn merge_related_sections(
    context: &ExecutionContext,
    model_bit: &Bit,
    sections: Vec<ContentSection>,
) -> flow_like_types::Result<Vec<ContentSection>> {
    if sections.len() <= 1 {
        return Ok(sections);
    }

    let sections_json = sections
        .iter()
        .enumerate()
        .map(|(i, s)| {
            format!(
                "{{\"id\": {}, \"title\": \"{}\", \"keywords\": {:?}, \"pages\": {:?}}}",
                i, s.title, s.keywords, s.page_references
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");

    let system_prompt = r#"You are a document analysis expert. Given a list of content sections, identify which sections should be merged because they cover the same topic appearing on different pages.

Return a JSON array of merge groups. Each group contains the IDs of sections that should be merged together.
Only group sections that are clearly about the same specific topic.

Example output:
[[0, 3, 7], [1, 5], [2], [4], [6]]

This means: sections 0, 3, 7 should be merged; sections 1, 5 should be merged; others remain separate."#;

    let user_prompt = format!(
        "Analyze these sections and identify merge groups:\n{}",
        sections_json
    );

    let response = invoke_model_simple(context, model_bit, system_prompt, &user_prompt).await?;

    let merge_groups = parse_merge_groups(&response, sections.len());

    let mut merged = Vec::new();
    let mut used = vec![false; sections.len()];

    for group in merge_groups {
        if group.is_empty() {
            continue;
        }

        let mut combined_title = String::new();
        let mut combined_summary = String::new();
        let mut combined_keywords = Vec::new();
        let mut combined_pages = Vec::new();

        for &idx in &group {
            if idx < sections.len() && !used[idx] {
                used[idx] = true;
                let s = &sections[idx];

                if combined_title.is_empty() {
                    combined_title = s.title.clone();
                }

                if !combined_summary.is_empty() {
                    combined_summary.push(' ');
                }
                combined_summary.push_str(&s.summary);

                for kw in &s.keywords {
                    if !combined_keywords.contains(kw) {
                        combined_keywords.push(kw.clone());
                    }
                }

                combined_pages.extend(&s.page_references);
            }
        }

        if !combined_title.is_empty() {
            combined_pages.sort();
            combined_pages.dedup();

            merged.push(ContentSection {
                title: combined_title,
                summary: combined_summary,
                keywords: combined_keywords,
                page_references: combined_pages,
            });
        }
    }

    for (i, section) in sections.into_iter().enumerate() {
        if !used[i] {
            merged.push(section);
        }
    }

    Ok(merged)
}

#[cfg(feature = "execute")]
fn parse_merge_groups(response: &str, max_idx: usize) -> Vec<Vec<usize>> {
    let cleaned = response
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();

    if let Ok(groups) = flow_like_types::json::from_str::<Vec<Vec<usize>>>(cleaned) {
        return groups
            .into_iter()
            .map(|g| g.into_iter().filter(|&i| i < max_idx).collect())
            .collect();
    }

    let bracket_pattern = regex::Regex::new(r"\[([^\[\]]+)\]").ok();
    if let Some(pattern) = bracket_pattern {
        let mut groups = Vec::new();
        for cap in pattern.captures_iter(cleaned) {
            if let Some(inner) = cap.get(1) {
                let indices: Vec<usize> = inner
                    .as_str()
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .filter(|&i| i < max_idx)
                    .collect();
                if !indices.is_empty() {
                    groups.push(indices);
                }
            }
        }
        if !groups.is_empty() {
            return groups;
        }
    }

    (0..max_idx).map(|i| vec![i]).collect()
}

#[cfg(test)]
mod prompt_preset_tests {
    use super::*;

    fn node_with(preset: &str, page_prompt: &str) -> Node {
        let mut node = Node::new("test", "Test", "Test", "Test");
        add_prompt_pins(&mut node);
        node.get_pin_mut_by_name("prompt_preset")
            .unwrap()
            .set_default_value(Some(json!(preset)));
        node.get_pin_mut_by_name("page_prompt")
            .unwrap()
            .set_default_value(Some(json!(page_prompt)));
        node
    }

    fn unlimited_ocr_page() -> &'static str {
        PROMPT_PRESETS
            .iter()
            .find(|preset| preset.name == "Unlimited-OCR")
            .unwrap()
            .page
    }

    #[test]
    fn preset_fills_an_untouched_prompt_pin() {
        let mut node = node_with("Unlimited-OCR", "");
        apply_preset_to_pins(&mut node);
        assert_eq!(pin_literal(&node, "page_prompt"), unlimited_ocr_page());
    }

    #[test]
    fn reapplying_the_same_preset_is_a_no_op() {
        let mut node = node_with("Unlimited-OCR", unlimited_ocr_page());
        let before = node.get_pin_by_name("page_prompt").unwrap().id.clone();
        apply_preset_to_pins(&mut node);
        assert_eq!(pin_literal(&node, "page_prompt"), unlimited_ocr_page());
        assert_eq!(node.get_pin_by_name("page_prompt").unwrap().id, before);
    }

    #[test]
    fn switching_presets_replaces_the_previous_preset_prompt() {
        let mut node = node_with("DeepSeek-OCR", unlimited_ocr_page());
        apply_preset_to_pins(&mut node);
        assert!(pin_literal(&node, "page_prompt").contains("<|grounding|>"));
    }

    #[test]
    fn default_replaces_a_preset_prompt_with_the_tuned_default() {
        let mut node = node_with("Default", unlimited_ocr_page());
        apply_preset_to_pins(&mut node);
        assert_eq!(pin_literal(&node, "page_prompt"), DEFAULT_PAGE_PROMPT);
    }

    #[test]
    fn default_fills_an_untouched_node_on_first_parse() {
        let mut node = node_with("Default", "");
        apply_preset_to_pins(&mut node);
        assert_eq!(pin_literal(&node, "page_prompt"), DEFAULT_PAGE_PROMPT);
        assert_eq!(pin_literal(&node, "image_prompt"), DEFAULT_IMAGE_PROMPT);
        assert_eq!(pin_literal(&node, "batch_prompt"), DEFAULT_BATCH_PROMPT);
    }

    #[test]
    fn a_page_reader_preset_clears_the_batch_prompt() {
        let mut node = node_with("Default", "");
        apply_preset_to_pins(&mut node);
        assert_eq!(pin_literal(&node, "batch_prompt"), DEFAULT_BATCH_PROMPT);

        node.get_pin_mut_by_name("prompt_preset")
            .unwrap()
            .set_default_value(Some(json!("Unlimited-OCR")));
        apply_preset_to_pins(&mut node);
        assert_eq!(pin_literal(&node, "batch_prompt"), "");
    }

    #[test]
    fn round_tripping_through_a_preset_restores_the_defaults() {
        let mut node = node_with("Default", "");
        apply_preset_to_pins(&mut node);

        for preset in ["olmOCR", "Default"] {
            node.get_pin_mut_by_name("prompt_preset")
                .unwrap()
                .set_default_value(Some(json!(preset)));
            apply_preset_to_pins(&mut node);
        }

        assert_eq!(pin_literal(&node, "page_prompt"), DEFAULT_PAGE_PROMPT);
        assert_eq!(pin_literal(&node, "image_prompt"), DEFAULT_IMAGE_PROMPT);
        assert_eq!(pin_literal(&node, "batch_prompt"), DEFAULT_BATCH_PROMPT);
    }

    #[test]
    fn a_hand_written_prompt_survives_every_preset_change() {
        let custom = "Transcribe only the invoice line items.";

        let mut node = node_with("Default", custom);
        apply_preset_to_pins(&mut node);
        assert_eq!(pin_literal(&node, "page_prompt"), custom);

        let mut node = node_with("Unlimited-OCR", custom);
        apply_preset_to_pins(&mut node);
        assert_eq!(pin_literal(&node, "page_prompt"), custom);
    }

    #[test]
    fn an_unknown_preset_leaves_the_prompt_pins_alone() {
        let mut node = node_with("Not-A-Preset", unlimited_ocr_page());
        apply_preset_to_pins(&mut node);
        assert_eq!(pin_literal(&node, "page_prompt"), unlimited_ocr_page());
    }

    #[test]
    fn preset_names_are_unique_and_default_is_first() {
        let names = preset_names();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "duplicate preset name");
        assert_eq!(names.first().map(String::as_str), Some("Default"));
    }
}

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct PdfCreateFromMarkdownNode;

impl PdfCreateFromMarkdownNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PdfCreateFromMarkdownNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "pdf_create_from_markdown",
            "Create PDF from Markdown",
            "Typesets Markdown into a paginated PDF with selectable text, tables, code blocks, charts and embedded images",
            "Document/PDF",
        );
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(6)
                .set_governance(8)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "markdown",
            "Markdown",
            "Markdown source to typeset",
            VariableType::String,
        );

        node.add_input_pin("output", "Output Path", "Save path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "page_size",
            "Page Size",
            "Page geometry",
            VariableType::String,
        )
        .set_default_value(Some(json!("a4")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "a4".to_string(),
                    "letter".to_string(),
                    "legal".to_string(),
                    "a5".to_string(),
                    "a3".to_string(),
                ])
                .build(),
        );

        node.add_input_pin(
            "embed_images",
            "Embed Images",
            "Download and embed images referenced by the Markdown. Disable to render placeholders instead.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "page_numbers",
            "Page Numbers",
            "Print a page number in the footer",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "title",
            "Title",
            "Document title. Also sets the running header and the cover block.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "subtitle",
            "Subtitle",
            "Secondary line under the title on the cover block",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "cover",
            "Cover Block",
            "Open the document with the accent title block",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "author",
            "Author",
            "Document author metadata",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Done", "Continues", VariableType::Execution);
        node.add_output_pin("result", "Result", "Output file path", VariableType::Struct)
            .set_schema::<FlowPath>();
        node.add_output_pin(
            "pages",
            "Pages",
            "Number of pages written",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::render::{
            Block, PdfLayout, PdfMetadata, build_pdf, parse_markdown, render_document,
        };
        use std::collections::HashMap;

        context.deactivate_exec_pin("exec_out").await?;

        let markdown: String = context.evaluate_pin("markdown").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;
        let page_size: String = context.evaluate_pin("page_size").await?;
        let embed_images: bool = context.evaluate_pin("embed_images").await?;
        let page_numbers: bool = context.evaluate_pin("page_numbers").await?;
        let title: String = context.evaluate_pin("title").await?;
        let subtitle: String = context.evaluate_pin("subtitle").await?;
        let cover: bool = context.evaluate_pin("cover").await?;
        let author: String = context.evaluate_pin("author").await?;

        let layout = PdfLayout::for_page_size(&page_size);
        let blocks = parse_markdown(&markdown);

        let mut images = HashMap::new();
        if embed_images {
            let urls: Vec<String> = blocks
                .iter()
                .filter_map(|block| match block {
                    Block::Image { url, .. } if !url.is_empty() => Some(url.clone()),
                    _ => None,
                })
                .collect();
            images = resolve_images(context, &urls).await;
        }

        let metadata = PdfMetadata {
            title: (!title.is_empty()).then_some(title),
            author: (!author.is_empty()).then_some(author),
            subject: (!subtitle.is_empty()).then_some(subtitle),
            page_numbers,
            cover,
        };

        let (pages, image_keys) = render_document(&blocks, &layout, &images, &metadata);
        let page_count = pages.len() as i64;

        let bytes = build_pdf(pages, &image_keys, &images, &layout, &metadata)?;

        output.put(context, bytes, false).await?;
        context.set_pin_value("result", json!(output)).await?;
        context.set_pin_value("pages", json!(page_count)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!("Requires the 'execute' feature"))
    }
}

/// Cap on a single embedded image, so a hostile or mistaken URL cannot exhaust memory.
#[cfg(feature = "execute")]
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;

/// Fetch and decode every referenced image.
///
/// A failure is logged and skipped rather than propagated — one unreachable asset should degrade
/// to a placeholder box, not lose the whole document.
#[cfg(feature = "execute")]
async fn resolve_images(
    context: &mut ExecutionContext,
    urls: &[String],
) -> std::collections::HashMap<String, super::render::EmbeddedImage> {
    use flow_like::flow::execution::LogLevel;
    use std::collections::HashMap;

    let mut resolved = HashMap::new();
    let client = flow_like_types::reqwest::Client::new();

    for url in urls {
        if resolved.contains_key(url) {
            continue;
        }
        let bytes = match fetch_image_bytes(context, &client, url).await {
            Ok(bytes) => bytes,
            Err(err) => {
                context.log_message(
                    &format!("Skipping image \"{url}\": {err}"),
                    LogLevel::Warn,
                );
                continue;
            }
        };
        match super::render::decode_image(&bytes) {
            Ok(image) => {
                resolved.insert(url.clone(), image);
            }
            Err(err) => context.log_message(
                &format!("Skipping undecodable image \"{url}\": {err}"),
                LogLevel::Warn,
            ),
        }
    }

    resolved
}

/// Resolve one image reference to bytes.
///
/// Supports `http(s)://` downloads, `data:` URIs, and app storage paths (`storage://…` or a bare
/// relative path), which is what the rich text editor writes for uploaded images.
#[cfg(feature = "execute")]
async fn fetch_image_bytes(
    context: &mut ExecutionContext,
    client: &flow_like_types::reqwest::Client,
    url: &str,
) -> flow_like_types::Result<Vec<u8>> {
    if let Some(rest) = url.strip_prefix("data:") {
        let payload = rest
            .split_once(";base64,")
            .map(|(_, data)| data)
            .ok_or_else(|| flow_like_types::anyhow!("only base64 data URIs are supported"))?;
        use flow_like_types::base64::Engine;
        return flow_like_types::base64::engine::general_purpose::STANDARD
            .decode(payload)
            .map_err(|err| flow_like_types::anyhow!("invalid base64 payload: {err}"));
    }

    if url.starts_with("http://") || url.starts_with("https://") {
        let response = client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(flow_like_types::anyhow!(
                "download failed with status {}",
                response.status()
            ));
        }
        if let Some(length) = response.content_length()
            && length as usize > MAX_IMAGE_BYTES
        {
            return Err(flow_like_types::anyhow!(
                "image exceeds the {MAX_IMAGE_BYTES} byte limit"
            ));
        }
        let bytes = response.bytes().await?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(flow_like_types::anyhow!(
                "image exceeds the {MAX_IMAGE_BYTES} byte limit"
            ));
        }
        return Ok(bytes.to_vec());
    }

    let relative = url.strip_prefix("storage://").unwrap_or(url);
    if relative.contains("..") {
        return Err(flow_like_types::anyhow!(
            "storage paths may not traverse upwards"
        ));
    }
    let mut path = FlowPath::from_storage_dir(context, false).await?;
    path.path = format!("{}/{}", path.path.trim_end_matches('/'), relative);
    path.get(context, false).await
}

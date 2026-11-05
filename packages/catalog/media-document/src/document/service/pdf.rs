use std::collections::HashMap;

use crate::document::pdf::render::{
    EmbeddedImage, PdfLayout, PdfMetadata, build_pdf, decode_image, parse_markdown,
    render_document,
};

use super::{Built, Images};

#[derive(Debug, Clone, Default)]
pub struct PdfOptions {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub author: Option<String>,
    pub cover: bool,
    /// a4 (default), letter, legal, a5 or a3.
    pub page_size: Option<String>,
    pub page_numbers: bool,
}

fn non_empty(text: &Option<String>) -> Option<String> {
    text.as_ref().filter(|t| !t.trim().is_empty()).cloned()
}

/// Typesets the markdown. `images` maps the URL written in the markdown to the image bytes; an
/// image without bytes, or with undecodable bytes, degrades to a placeholder and a warning.
pub fn pdf_from_markdown(
    markdown: &str,
    options: &PdfOptions,
    images: &Images,
) -> flow_like_types::Result<Built> {
    let mut warnings = Vec::new();
    let mut decoded = HashMap::new();
    for (url, bytes) in images {
        match decode_image(bytes) {
            Ok(image) => {
                decoded.insert(url.clone(), image);
            }
            Err(error) => warnings.push(format!("Image '{url}' was skipped: {error}")),
        }
    }
    let mut built = pdf_from_images(markdown, options, &decoded)?;
    built.warnings.extend(warnings);
    Ok(built)
}

pub fn pdf_from_images(
    markdown: &str,
    options: &PdfOptions,
    images: &HashMap<String, EmbeddedImage>,
) -> flow_like_types::Result<Built> {
    let layout = PdfLayout::for_page_size(options.page_size.as_deref().unwrap_or("a4"));
    let blocks = parse_markdown(markdown);
    let metadata = PdfMetadata {
        title: non_empty(&options.title),
        author: non_empty(&options.author),
        subject: non_empty(&options.subtitle),
        page_numbers: options.page_numbers,
        cover: options.cover,
    };
    let (pages, image_keys) = render_document(&blocks, &layout, images, &metadata);
    let count = pages.len();
    let bytes = build_pdf(pages, &image_keys, images, &layout, &metadata)?;
    Ok(Built {
        bytes,
        count: Some(count),
        warnings: Vec::new(),
    })
}

//! Context-free document services: build and fill Office and PDF files from plain data.
//!
//! The nodes in this crate are bound to an `ExecutionContext`; these functions take bytes and
//! return bytes, so headless callers (bot tools) share the same typesetting.

pub mod chart;
pub mod docx;
pub mod fill;
pub mod pdf;
pub mod pptx;

use std::collections::HashMap;

pub type Images = HashMap<String, Vec<u8>>;

#[derive(Debug, Clone, Default)]
pub struct Built {
    pub bytes: Vec<u8>,
    /// Pages for a PDF, slides for a deck; unknown for Word (it paginates when opened).
    pub count: Option<usize>,
    pub warnings: Vec<String>,
}

pub(crate) fn xml_text(text: &str) -> String {
    quick_xml::escape::escape(text).into_owned()
}

pub(crate) fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpeg")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else {
        None
    }
}

/// PNG, JPEG and GIF pass through; anything else the `image` crate reads becomes PNG.
pub(crate) fn office_image(bytes: &[u8]) -> flow_like_types::Result<(Vec<u8>, &'static str, u32, u32)> {
    use flow_like_types::images::image;
    let decoded = image::load_from_memory(bytes)
        .map_err(|error| flow_like_types::anyhow!("The image cannot be decoded: {error}"))?;
    let (width, height) = image::GenericImageView::dimensions(&decoded);
    if let Some(extension) = image_extension(bytes) {
        return Ok((bytes.to_vec(), extension, width, height));
    }
    let mut png = std::io::Cursor::new(Vec::new());
    decoded
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|error| flow_like_types::anyhow!("The image cannot be converted: {error}"))?;
    Ok((png.into_inner(), "png", width, height))
}

pub(crate) fn content_type_for(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "gif" => "image/gif",
        _ => "image/jpeg",
    }
}

pub(crate) fn ensure_default_content_type(
    files: &mut HashMap<String, Vec<u8>>,
    extension: &str,
) {
    ensure_default_content_type_as(files, extension, content_type_for(extension));
}

pub(crate) fn ensure_default_content_type_as(
    files: &mut HashMap<String, Vec<u8>>,
    extension: &str,
    content_type: &str,
) {
    let Some(types) = files.get("[Content_Types].xml") else {
        return;
    };
    let mut xml = String::from_utf8_lossy(types).into_owned();
    if xml.contains(&format!("Extension=\"{extension}\"")) {
        return;
    }
    let entry = format!("<Default Extension=\"{extension}\" ContentType=\"{content_type}\"/>");
    if let Some(position) = xml.find("<Override") {
        xml.insert_str(position, &entry);
    } else if let Some(position) = xml.find("</Types>") {
        xml.insert_str(position, &entry);
    }
    files.insert("[Content_Types].xml".to_string(), xml.into_bytes());
}

pub(crate) fn add_override(files: &mut HashMap<String, Vec<u8>>, part: &str, content_type: &str) {
    let Some(types) = files.get("[Content_Types].xml") else {
        return;
    };
    let mut xml = String::from_utf8_lossy(types).into_owned();
    let entry = format!("<Override PartName=\"/{part}\" ContentType=\"{content_type}\"/>");
    if let Some(position) = xml.rfind("</Types>") {
        xml.insert_str(position, &entry);
    }
    files.insert("[Content_Types].xml".to_string(), xml.into_bytes());
}

pub(crate) fn add_relationship(
    files: &mut HashMap<String, Vec<u8>>,
    rels_path: &str,
    id: &str,
    kind: &str,
    target: &str,
) {
    let entry = format!("<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{target}\"/>");
    let mut xml = files.get(rels_path).map_or_else(
        || {
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>".to_string()
        },
        |bytes| String::from_utf8_lossy(bytes).into_owned(),
    );
    if let Some(position) = xml.rfind("</Relationships>") {
        xml.insert_str(position, &entry);
    }
    files.insert(rels_path.to_string(), xml.into_bytes());
}

pub(crate) const REL_IMAGE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";

#[cfg(test)]
mod tests;

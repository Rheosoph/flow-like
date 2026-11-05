use std::collections::{BTreeMap, HashMap};

use lopdf::{Document, Object};

use crate::document::ImageScaleMode;
use crate::document::docx::replace_table_row::replace_table_rows;
use crate::document::docx::replace_text::replace_docx_text;
use crate::document::openxml::{read_zip, replace_image_in_archive, write_zip};
use crate::document::pdf::fill_form::set_acroform_field;
use crate::document::pptx::replace_image::replace_pptx_image;
use crate::document::pptx::replace_table_data::replace_table_on_slide;
use crate::document::pptx::replace_text::replace_pptx_text;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    Docx,
    Pptx,
    Pdf,
}

impl TemplateKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Pptx => "pptx",
            Self::Pdf => "pdf",
        }
    }

    pub fn detect(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"%PDF") {
            return Some(Self::Pdf);
        }
        let files = read_zip(bytes).ok()?;
        if files.contains_key("word/document.xml") {
            Some(Self::Docx)
        } else if files.contains_key("ppt/presentation.xml") {
            Some(Self::Pptx)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub enum TableRows {
    /// Word: one object per row, its keys fill the `{{column}}` placeholders of the template row.
    Records(Vec<BTreeMap<String, String>>),
    /// PowerPoint: the whole table content, header row first.
    Grid(Vec<Vec<String>>),
}

#[derive(Debug, Clone, Default)]
pub struct FillInput {
    /// Placeholder name (`{{name}}` in the file) or PDF form field name, and its text.
    pub values: Vec<(String, String)>,
    /// Read the values as inline markdown (Word and PowerPoint).
    pub markdown: bool,
    /// Picture alt text or shape name, and the new image.
    pub images: Vec<(String, Vec<u8>)>,
    /// A placeholder inside a template table row (Word) or table (PowerPoint), and its rows.
    pub tables: Vec<(String, TableRows)>,
}

#[derive(Debug, Clone, Default)]
pub struct Filled {
    pub bytes: Vec<u8>,
    pub filled: Vec<String>,
    /// Names that matched nothing in the template.
    pub missing: Vec<String>,
}

pub fn placeholder(key: &str) -> String {
    let key = key.trim();
    if key.starts_with("{{") {
        key.to_string()
    } else {
        format!("{{{{{key}}}}}")
    }
}

fn text_parts(files: &HashMap<String, Vec<u8>>) -> Vec<(String, Vec<u8>)> {
    let mut parts: Vec<(String, Vec<u8>)> = files
        .iter()
        .filter(|(name, _)| {
            name.ends_with(".xml")
                && (name.starts_with("word/")
                    || name.starts_with("ppt/slides/")
                    || name.starts_with("ppt/slideLayouts/")
                    || name.starts_with("ppt/slideMasters/"))
        })
        .map(|(name, bytes)| (name.clone(), bytes.clone()))
        .collect();
    parts.sort();
    parts
}

fn slide_numbers(files: &HashMap<String, Vec<u8>>) -> Vec<i64> {
    let mut numbers: Vec<i64> = files
        .keys()
        .filter_map(|name| {
            name.strip_prefix("ppt/slides/slide")?
                .strip_suffix(".xml")?
                .parse()
                .ok()
        })
        .collect();
    numbers.sort_unstable();
    numbers
}

fn fill_text(
    kind: TemplateKind,
    files: &mut HashMap<String, Vec<u8>>,
    input: &FillInput,
    report: &mut Filled,
) -> flow_like_types::Result<()> {
    for (key, value) in &input.values {
        let before = text_parts(files);
        match kind {
            TemplateKind::Docx => replace_docx_text(files, &placeholder(key), value, input.markdown)?,
            _ => replace_pptx_text(files, &placeholder(key), value, input.markdown)?,
        }
        if before == text_parts(files) {
            report.missing.push(key.clone());
        } else {
            report.filled.push(key.clone());
        }
    }
    Ok(())
}

fn fill_images(
    kind: TemplateKind,
    files: &mut HashMap<String, Vec<u8>>,
    input: &FillInput,
    report: &mut Filled,
) -> flow_like_types::Result<()> {
    for (name, bytes) in &input.images {
        let replaced = match kind {
            TemplateKind::Docx => replace_image_in_archive(
                files,
                "word/document.xml",
                "word/_rels/document.xml.rels",
                name,
                bytes,
                &ImageScaleMode::KeepWidth,
            )?,
            _ => replace_pptx_image(files, name, bytes, &ImageScaleMode::KeepWidth)?,
        };
        if replaced {
            report.filled.push(name.clone());
        } else {
            report.missing.push(name.clone());
        }
    }
    Ok(())
}

fn fill_tables(
    kind: TemplateKind,
    files: &mut HashMap<String, Vec<u8>>,
    input: &FillInput,
    report: &mut Filled,
) -> flow_like_types::Result<()> {
    for (key, rows) in &input.tables {
        let marker = placeholder(key);
        let done = match (kind, rows) {
            (TemplateKind::Docx, TableRows::Records(records)) => {
                let xml = files
                    .get("word/document.xml")
                    .map(|b| String::from_utf8_lossy(b).into_owned())
                    .ok_or_else(|| flow_like_types::anyhow!("The template has no word/document.xml"))?;
                let maps: Vec<HashMap<String, String>> = records
                    .iter()
                    .map(|record| record.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .collect();
                match replace_table_rows(&xml, &marker, &maps) {
                    Ok(updated) => {
                        files.insert("word/document.xml".to_string(), updated.into_bytes());
                        true
                    }
                    Err(_) => false,
                }
            }
            (TemplateKind::Pptx, TableRows::Grid(grid)) => {
                let mut done = false;
                for slide in slide_numbers(files) {
                    if replace_table_on_slide(files, slide, &marker, grid).is_ok() {
                        done = true;
                        break;
                    }
                }
                done
            }
            (TemplateKind::Docx, TableRows::Grid(_)) => {
                return Err(flow_like_types::anyhow!(
                    "Table '{key}': a Word template takes rows as objects, one per row ({{{{column}}}} placeholders in the template row)"
                ));
            }
            (_, TableRows::Records(_)) => {
                return Err(flow_like_types::anyhow!(
                    "Table '{key}': a PowerPoint template takes rows as lists of cell texts, header row first"
                ));
            }
            (TemplateKind::Pdf, _) => false,
        };
        if done {
            report.filled.push(key.clone());
        } else {
            report.missing.push(key.clone());
        }
    }
    Ok(())
}

fn fill_pdf(template: &[u8], input: &FillInput) -> flow_like_types::Result<Filled> {
    if !input.images.is_empty() || !input.tables.is_empty() {
        return Err(flow_like_types::anyhow!(
            "A PDF form takes only values: images and tables are not supported"
        ));
    }
    let mut document = Document::load_mem(template)?;
    let mut report = Filled::default();
    for (name, value) in &input.values {
        match set_acroform_field(&mut document, name, value) {
            Ok(()) => report.filled.push(name.clone()),
            Err(_) => report.missing.push(name.clone()),
        }
    }
    if !report.filled.is_empty() {
        request_appearances(&mut document);
    }
    let mut bytes = Vec::new();
    document.save_to(&mut bytes)?;
    report.bytes = bytes;
    Ok(report)
}

fn request_appearances(document: &mut Document) {
    let Some(acroform) = document
        .catalog()
        .ok()
        .and_then(|catalog| catalog.get(b"AcroForm").ok())
        .and_then(|object| object.as_reference().ok())
    else {
        return;
    };
    if let Ok(dict) = document
        .get_object_mut(acroform)
        .and_then(|object| object.as_dict_mut())
    {
        dict.set("NeedAppearances", Object::Boolean(true));
    }
}

/// Fills a template and returns a new file; the template is not touched.
pub fn fill_template(
    kind: TemplateKind,
    template: &[u8],
    input: &FillInput,
) -> flow_like_types::Result<Filled> {
    if kind == TemplateKind::Pdf {
        return fill_pdf(template, input);
    }
    let mut files = read_zip(template)?;
    let mut report = Filled::default();
    fill_text(kind, &mut files, input, &mut report)?;
    fill_images(kind, &mut files, input, &mut report)?;
    fill_tables(kind, &mut files, input, &mut report)?;
    report.bytes = write_zip(&files)?;
    Ok(report)
}

use quick_xml::{escape::resolve_predefined_entity, events::Event, reader::Reader};
use serde::Serialize;
use std::io::{Cursor, Read, Seek};
use zip::{ZipArchive, result::ZipError};

/// Property parts are a few kilobytes; the cap keeps a crafted archive from inflating more.
const MAX_PART_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct OfficeDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub creator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keywords: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slides: Option<u64>,
}

/// Document properties of a DOCX, XLSX or PPTX package. Only `docProps/core.xml` and
/// `docProps/app.xml` are inflated; every other part stays compressed.
pub fn office_details(package: &[u8]) -> flow_like_types::Result<OfficeDetails> {
    let mut archive = ZipArchive::new(Cursor::new(package))?;
    let mut details = OfficeDetails::default();

    if let Some(core) = read_part(&mut archive, "docProps/core.xml")? {
        let [
            title,
            creator,
            subject,
            keywords,
            created,
            modified,
            last_modified_by,
        ] = element_texts(
            &core,
            [
                "title",
                "creator",
                "subject",
                "keywords",
                "created",
                "modified",
                "lastModifiedBy",
            ],
        )?;
        details = OfficeDetails {
            title,
            creator,
            subject,
            keywords,
            created,
            modified,
            last_modified_by,
            ..details
        };
    }

    if let Some(app) = read_part(&mut archive, "docProps/app.xml")? {
        let [pages, words, slides] = element_texts(&app, ["Pages", "Words", "Slides"])?;
        let count = |value: Option<String>| value.and_then(|value| value.parse().ok());
        details.pages = count(pages);
        details.words = count(words);
        details.slides = count(slides);
    }

    Ok(details)
}

fn read_part<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> flow_like_types::Result<Option<Vec<u8>>> {
    let mut part = match archive.by_name(name) {
        Ok(part) => part,
        Err(ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(flow_like_types::anyhow!("Reading {name} failed: {error}")),
    };
    let mut bytes = Vec::new();
    (&mut part).take(MAX_PART_BYTES).read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// The trimmed text of the first element with each local `name`, ignoring namespace prefixes.
fn element_texts<const N: usize>(
    xml: &[u8],
    names: [&str; N],
) -> flow_like_types::Result<[Option<String>; N]> {
    let mut reader = Reader::from_reader(xml);
    let mut values: [Option<String>; N] = std::array::from_fn(|_| None);
    let mut open: Option<(usize, String)> = None;
    let mut buffer = Vec::new();

    loop {
        match reader.read_event_into(&mut buffer)? {
            Event::Start(element) => {
                let local = element.local_name();
                open = names
                    .iter()
                    .position(|name| name.as_bytes() == local.as_ref())
                    .filter(|index| values[*index].is_none())
                    .map(|index| (index, String::new()));
            }
            Event::Text(text) => {
                if let Some((_, value)) = open.as_mut() {
                    value.push_str(&text.decode()?);
                }
            }
            Event::CData(text) => {
                if let Some((_, value)) = open.as_mut() {
                    value.push_str(&text.decode()?);
                }
            }
            Event::GeneralRef(reference) => {
                if let Some((_, value)) = open.as_mut() {
                    if let Some(character) = reference.resolve_char_ref()? {
                        value.push(character);
                    } else if let Some(entity) = resolve_predefined_entity(&reference.decode()?) {
                        value.push_str(entity);
                    }
                }
            }
            Event::End(_) => {
                if let Some((index, value)) = open.take() {
                    let value = value.trim();
                    if !value.is_empty() {
                        values[index] = Some(value.to_string());
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }

    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::openxml::write_zip;
    use std::collections::HashMap;

    const CORE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <dc:title>Budget &amp; Plan</dc:title>
  <dc:creator>Ada Lovelace</dc:creator>
  <cp:keywords>finance, 2024</cp:keywords>
  <cp:lastModifiedBy>Charles Babbage</cp:lastModifiedBy>
  <dcterms:created xsi:type="dcterms:W3CDTF">2024-01-31T12:00:00Z</dcterms:created>
  <dcterms:modified xsi:type="dcterms:W3CDTF">2024-02-01T08:30:00Z</dcterms:modified>
</cp:coreProperties>"#;

    fn package(parts: &[(&str, &str)]) -> Vec<u8> {
        let files: HashMap<String, Vec<u8>> = parts
            .iter()
            .map(|(name, xml)| (name.to_string(), xml.as_bytes().to_vec()))
            .collect();
        write_zip(&files).unwrap()
    }

    #[test]
    fn docx_core_and_app_properties() {
        let docx = package(&[
            ("[Content_Types].xml", "<Types/>"),
            ("word/document.xml", "<w:document/>"),
            ("docProps/core.xml", CORE_XML),
            (
                "docProps/app.xml",
                "<Properties><Pages>3</Pages><Words>412</Words><Application>Microsoft Office Word</Application></Properties>",
            ),
        ]);
        assert_eq!(
            office_details(&docx).unwrap(),
            OfficeDetails {
                title: Some("Budget & Plan".into()),
                creator: Some("Ada Lovelace".into()),
                keywords: Some("finance, 2024".into()),
                created: Some("2024-01-31T12:00:00Z".into()),
                modified: Some("2024-02-01T08:30:00Z".into()),
                last_modified_by: Some("Charles Babbage".into()),
                pages: Some(3),
                words: Some(412),
                ..OfficeDetails::default()
            }
        );
    }

    #[test]
    fn xlsx_without_app_properties_reports_core_fields_only() {
        let xlsx = package(&[
            ("xl/workbook.xml", "<workbook/>"),
            ("docProps/core.xml", CORE_XML),
        ]);
        let details = office_details(&xlsx).unwrap();
        assert_eq!(details.title.as_deref(), Some("Budget & Plan"));
        assert_eq!(details.pages, None);
        let json = flow_like_types::json::to_value(&details).unwrap();
        assert!(json.get("words").is_none());
    }

    #[test]
    fn a_package_without_properties_has_no_details() {
        let bare = package(&[("ppt/presentation.xml", "<p:presentation/>")]);
        assert_eq!(office_details(&bare).unwrap(), OfficeDetails::default());
        assert!(office_details(b"PK\x03\x04 truncated").is_err());
    }
}

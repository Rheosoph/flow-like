use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use zip::write::SimpleFileOptions;

/// Read all files from a ZIP archive into memory.
pub fn read_zip(data: &[u8]) -> Result<HashMap<String, Vec<u8>>> {
    let cursor = Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)?;
    let mut files: HashMap<String, Vec<u8>> = HashMap::with_capacity(archive.len());

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();
        let mut buf = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut buf)?;
        files.insert(name, buf);
    }

    Ok(files)
}

/// Write a map of files back into a ZIP archive.
pub fn write_zip(files: &HashMap<String, Vec<u8>>) -> Result<Vec<u8>> {
    let buf = Vec::new();
    let cursor = Cursor::new(buf);
    let mut zip_writer = zip::ZipWriter::new(cursor);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for (name, data) in files {
        zip_writer.start_file(name, options)?;
        zip_writer.write_all(data)?;
    }

    let cursor = zip_writer.finish()?;
    Ok(cursor.into_inner())
}

/// Replace all occurrences of a placeholder in text within OpenXML body content.
///
/// OpenXML (DOCX/PPTX) often splits text across multiple `<w:t>` or `<a:t>` elements
/// within a paragraph. This function concatenates text within a paragraph, finds
/// placeholders, and rebuilds the runs with the replacement.
pub fn replace_text_in_xml(
    xml_bytes: &[u8],
    placeholder: &str,
    replacement: &str,
    text_element: &str,
    run_element: &str,
    paragraph_element: &str,
) -> Result<Vec<u8>> {
    let xml_str = std::str::from_utf8(xml_bytes)?;
    let result = replace_placeholder_in_runs(
        xml_str,
        placeholder,
        replacement,
        text_element,
        run_element,
        paragraph_element,
    )?;
    Ok(result.into_bytes())
}

/// Replace all occurrences of a placeholder in rich text (markdown-converted) within OpenXML.
///
/// Parses markdown into formatted runs and replaces the placeholder paragraph content
/// with the rendered text, preserving the base formatting of the first run.
pub fn replace_text_in_xml_markdown(
    xml_bytes: &[u8],
    placeholder: &str,
    markdown: &str,
    text_element: &str,
    run_element: &str,
    run_props_element: &str,
    paragraph_element: &str,
    format: OpenXmlFormat,
) -> Result<Vec<u8>> {
    let xml_str = std::str::from_utf8(xml_bytes)?;
    let formatted_runs = markdown_to_runs(markdown, format);
    let result = replace_placeholder_with_runs(
        xml_str,
        placeholder,
        &formatted_runs,
        text_element,
        run_element,
        run_props_element,
        paragraph_element,
    )?;
    Ok(result.into_bytes())
}

/// Identifies the OpenXML variant for formatting differences.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OpenXmlFormat {
    /// Word Processing ML (DOCX) — uses `w:` prefix
    Docx,
    /// Presentation ML (PPTX) — uses `a:` prefix
    Pptx,
}

/// Semantic block type for a formatted run.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum BlockType {
    #[default]
    Normal,
    Heading(u8),
    BlockQuote,
    TableHeader,
    TableCell,
    TableRowEnd,
    Image {
        url: String,
        alt: String,
    },
    CodeBlock {
        language: Option<String>,
    },
}

/// A single run of formatted text for OpenXML insertion.
#[derive(Debug, Clone)]
pub struct FormattedRun {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strikethrough: bool,
    pub block_type: BlockType,
}

/// Convert markdown text to a series of FormattedRun structs.
pub fn markdown_to_runs(markdown: &str, _format: OpenXmlFormat) -> Vec<FormattedRun> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(markdown, options);

    let mut runs: Vec<FormattedRun> = Vec::new();
    let mut bold = false;
    let mut italic = false;
    let mut code = false;
    let mut strikethrough = false;
    let mut in_paragraph = false;
    let mut paragraph_count = 0u32;
    let mut heading_level: Option<u8> = None;
    let mut in_blockquote = false;
    let mut in_table_header = false;
    let mut in_table_cell = false;
    let mut code_language: Option<String> = None;

    let newline_run = || FormattedRun {
        text: "\n".to_string(),
        bold: false,
        italic: false,
        code: false,
        strikethrough: false,
        block_type: BlockType::Normal,
    };

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if in_paragraph && paragraph_count > 0 {
                        runs.push(newline_run());
                    }
                    in_paragraph = true;
                    paragraph_count += 1;
                }
                Tag::Strong => bold = true,
                Tag::Emphasis => italic = true,
                Tag::Strikethrough => strikethrough = true,
                Tag::CodeBlock(kind) => {
                    code = true;
                    code_language = match kind {
                        pulldown_cmark::CodeBlockKind::Fenced(lang) => {
                            let l = lang.trim().to_string();
                            if l.is_empty() { None } else { Some(l) }
                        }
                        _ => None,
                    };
                }
                Tag::List(_) => {}
                Tag::Item => {
                    runs.push(FormattedRun {
                        text: "• ".to_string(),
                        bold: false,
                        italic: false,
                        code: false,
                        strikethrough: false,
                        block_type: BlockType::Normal,
                    });
                }
                Tag::Heading { level, .. } => {
                    heading_level = Some(level as u8);
                    bold = true;
                }
                Tag::BlockQuote(_) => {
                    in_blockquote = true;
                }
                Tag::Table(_) => {}
                Tag::TableHead => {
                    in_table_header = true;
                }
                Tag::TableRow => {}
                Tag::TableCell => {
                    in_table_cell = true;
                }
                Tag::Image {
                    dest_url, title, ..
                } => {
                    let url = dest_url.to_string();
                    let alt = title.to_string();
                    runs.push(FormattedRun {
                        text: String::new(),
                        bold: false,
                        italic: false,
                        code: false,
                        strikethrough: false,
                        block_type: BlockType::Image { url, alt },
                    });
                }
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Paragraph => {
                    in_paragraph = false;
                }
                TagEnd::Strong => bold = false,
                TagEnd::Emphasis => italic = false,
                TagEnd::Strikethrough => strikethrough = false,
                TagEnd::CodeBlock => {
                    code = false;
                    code_language = None;
                    runs.push(newline_run());
                }
                TagEnd::Heading(_) => {
                    bold = false;
                    heading_level = None;
                    runs.push(newline_run());
                }
                TagEnd::Item => {
                    runs.push(newline_run());
                }
                TagEnd::BlockQuote(_) => {
                    in_blockquote = false;
                }
                TagEnd::TableHead => {
                    in_table_header = false;
                }
                TagEnd::TableCell => {
                    in_table_cell = false;
                }
                TagEnd::TableRow => {
                    runs.push(FormattedRun {
                        text: "\n".to_string(),
                        bold: false,
                        italic: false,
                        code: false,
                        strikethrough: false,
                        block_type: BlockType::TableRowEnd,
                    });
                }
                TagEnd::Table => {
                    runs.push(newline_run());
                }
                _ => {}
            },
            Event::Text(text) => {
                let block_type = if let Some(level) = heading_level {
                    BlockType::Heading(level)
                } else if in_blockquote {
                    BlockType::BlockQuote
                } else if in_table_header {
                    BlockType::TableHeader
                } else if in_table_cell {
                    BlockType::TableCell
                } else if code {
                    BlockType::CodeBlock {
                        language: code_language.clone(),
                    }
                } else {
                    BlockType::Normal
                };

                // For image alt text, update the last Image run
                if let Some(last) = runs.last_mut() {
                    if let BlockType::Image { .. } = &last.block_type {
                        if last.text.is_empty() {
                            last.text = text.to_string();
                            continue;
                        }
                    }
                }

                runs.push(FormattedRun {
                    text: text.to_string(),
                    bold,
                    italic,
                    code,
                    strikethrough,
                    block_type,
                });
            }
            Event::Code(text) => {
                let block_type = if in_table_header {
                    BlockType::TableHeader
                } else if in_table_cell {
                    BlockType::TableCell
                } else {
                    BlockType::Normal
                };
                runs.push(FormattedRun {
                    text: text.to_string(),
                    bold,
                    italic,
                    code: true,
                    strikethrough,
                    block_type,
                });
            }
            Event::SoftBreak | Event::HardBreak => {
                runs.push(newline_run());
            }
            _ => {}
        }
    }

    // Trim trailing newlines
    while runs.last().is_some_and(|r| r.text == "\n") {
        runs.pop();
    }

    if runs.is_empty() {
        runs.push(FormattedRun {
            text: markdown.to_string(),
            bold: false,
            italic: false,
            code: false,
            strikethrough: false,
            block_type: BlockType::Normal,
        });
    }

    runs
}

/// Build a formatted run as an XML string for DOCX (`w:r` / `w:rPr`) or PPTX (`a:r` / `a:rPr`).
fn build_formatted_run_xml(
    run: &FormattedRun,
    run_element: &str,
    run_props_element: &str,
    text_element: &str,
    base_rpr: Option<&str>,
    format: OpenXmlFormat,
) -> String {
    let mut xml = String::new();
    xml.push_str(&format!("<{}>", run_element));

    let has_formatting =
        run.bold || run.italic || run.code || run.strikethrough || base_rpr.is_some();
    if has_formatting {
        match format {
            OpenXmlFormat::Docx => {
                xml.push_str(&format!("<{}>", run_props_element));
                if let Some(base) = base_rpr {
                    xml.push_str(base);
                }
                if run.bold {
                    xml.push_str("<w:b/>");
                }
                if run.italic {
                    xml.push_str("<w:i/>");
                }
                if run.strikethrough {
                    xml.push_str("<w:strike/>");
                }
                xml.push_str(&format!("</{}>", run_props_element));
            }
            OpenXmlFormat::Pptx => {
                // PPTX uses attributes on the rPr element, not child elements
                let mut attrs = String::new();
                if run.bold {
                    attrs.push_str(" b=\"1\"");
                }
                if run.italic {
                    attrs.push_str(" i=\"1\"");
                }
                if run.strikethrough {
                    attrs.push_str(" strike=\"sngStrike\"");
                }
                if let Some(base) = base_rpr {
                    // base_rpr may contain child elements, so use full open/close
                    xml.push_str(&format!("<{}{}>{}", run_props_element, attrs, base));
                    xml.push_str(&format!("</{}>", run_props_element));
                } else if attrs.is_empty() {
                    xml.push_str(&format!("<{}/>", run_props_element));
                } else {
                    xml.push_str(&format!("<{}{}/>", run_props_element, attrs));
                }
            }
        }
    }

    let preserve = if run.text.contains(' ') || run.text.contains('\t') {
        " xml:space=\"preserve\""
    } else {
        ""
    };

    xml.push_str(&format!(
        "<{}{}>{}</{}>",
        text_element,
        preserve,
        quick_xml::escape::escape(&run.text),
        text_element
    ));
    xml.push_str(&format!("</{}>", run_element));

    xml
}

/// Replace placeholder text across split runs within paragraphs.
///
/// Collects all text elements within a paragraph, looks for the placeholder in the
/// concatenated text, and replaces the matching runs with the replacement text.
fn replace_placeholder_in_runs(
    xml: &str,
    placeholder: &str,
    replacement: &str,
    text_element: &str,
    _run_element: &str,
    _paragraph_element: &str,
) -> Result<String> {
    // Simple approach: gather all text, handle split placeholders
    // We work at the raw string level since placeholders may span across XML runs
    let result = replace_across_text_elements(xml, placeholder, replacement, text_element)?;
    Ok(result)
}

/// Replace placeholder with formatted runs.
fn replace_placeholder_with_runs(
    xml: &str,
    placeholder: &str,
    runs: &[FormattedRun],
    text_element: &str,
    run_element: &str,
    run_props_element: &str,
    _paragraph_element: &str,
) -> Result<String> {
    if runs.len() == 1
        && !runs[0].bold
        && !runs[0].italic
        && !runs[0].code
        && !runs[0].strikethrough
    {
        return replace_across_text_elements(xml, placeholder, &runs[0].text, text_element);
    }

    // For formatted replacement, first try simple text element replacement
    // If the placeholder is entirely within one text element, replace the whole run
    let format = if run_element.starts_with("w:") {
        OpenXmlFormat::Docx
    } else {
        OpenXmlFormat::Pptx
    };

    let runs_xml: String = runs
        .iter()
        .map(|r| {
            build_formatted_run_xml(
                r,
                run_element,
                run_props_element,
                text_element,
                None,
                format,
            )
        })
        .collect();

    // Try to find the placeholder within text elements and replace the enclosing run
    let result =
        replace_run_containing_placeholder(xml, placeholder, &runs_xml, text_element, run_element)?;
    Ok(result)
}

/// Replace text across potentially split XML text elements.
///
/// Handles the common case where a placeholder like `{{name}}` is split across
/// multiple `<w:t>` or `<a:t>` elements. Operates on the raw XML string.
fn replace_across_text_elements(
    xml: &str,
    placeholder: &str,
    replacement: &str,
    text_element: &str,
) -> Result<String> {
    let open_tag_prefix = format!("<{}", text_element);
    let close_tag = format!("</{}>", text_element);

    // Collect all text content with positions
    let mut segments: Vec<(usize, usize, String)> = Vec::new(); // (content_start, content_end, text)
    let mut search_from = 0;

    while let Some(tag_start) = xml[search_from..].find(&open_tag_prefix) {
        let tag_start = search_from + tag_start;
        let tag_end_pos = match xml[tag_start..].find('>') {
            Some(p) => tag_start + p + 1,
            None => break,
        };

        // Check for self-closing tag
        if xml[tag_start..tag_end_pos].ends_with("/>") {
            search_from = tag_end_pos;
            continue;
        }

        let close_pos = match xml[tag_end_pos..].find(&close_tag) {
            Some(p) => tag_end_pos + p,
            None => break,
        };

        let content = &xml[tag_end_pos..close_pos];
        segments.push((tag_end_pos, close_pos, content.to_string()));
        search_from = close_pos + close_tag.len();
    }

    // Now try to find the placeholder spanning across consecutive text elements
    let full_text: String = segments.iter().map(|(_, _, t)| t.as_str()).collect();
    if !full_text.contains(placeholder) {
        return Ok(xml.to_string());
    }

    // Rebuild: replace in the concatenated text and redistribute
    let mut result = xml.to_string();
    // Process from the end to preserve indices
    let mut offset = 0i64;

    // Find all matches in the full text
    let mut char_positions: Vec<(usize, usize)> = Vec::new(); // map from full_text char_idx -> (seg_idx, offset_in_seg)

    for (si, (_, _, text)) in segments.iter().enumerate() {
        for ci in 0..text.len() {
            char_positions.push((si, ci));
        }
    }

    // Find placeholder in full_text
    let mut search_start = 0;
    while let Some(match_pos) = full_text[search_start..].find(placeholder) {
        let match_pos = search_start + match_pos;
        let match_end = match_pos + placeholder.len();

        if match_pos >= char_positions.len() || match_end > char_positions.len() {
            break;
        }

        let (first_seg, first_offset) = char_positions[match_pos];
        let (last_seg, last_offset) = char_positions[match_end - 1];

        if first_seg == last_seg {
            // Placeholder is within a single text element — simple replacement
            let (content_start, content_end, ref _text) = segments[first_seg];
            let adjusted_start = (content_start as i64 + offset) as usize;
            let adjusted_end = (content_end as i64 + offset) as usize;
            let current_content = &result[adjusted_start..adjusted_end];
            let new_content = current_content.replacen(placeholder, replacement, 1);
            let len_diff = new_content.len() as i64 - current_content.len() as i64;
            result.replace_range(adjusted_start..adjusted_end, &new_content);
            offset += len_diff;
        } else {
            // Placeholder spans multiple text elements
            // Put replacement in the first element, clear the rest
            let (cs1, ce1, _) = segments[first_seg];
            let adj_cs1 = (cs1 as i64 + offset) as usize;
            let adj_ce1 = (ce1 as i64 + offset) as usize;
            let old1 = result[adj_cs1..adj_ce1].to_string();

            // Replace from first_offset to end in first segment
            let mut new1 = old1[..first_offset].to_string();
            new1.push_str(replacement);
            let diff1 = new1.len() as i64 - old1.len() as i64;
            result.replace_range(adj_cs1..adj_ce1, &new1);
            offset += diff1;

            // Clear content in middle and last segments that were part of the placeholder
            for seg_i in (first_seg + 1)..=last_seg {
                let (cs, ce, ref _text) = segments[seg_i];
                let adj_cs = (cs as i64 + offset) as usize;
                let adj_ce = (ce as i64 + offset) as usize;
                let old = result[adj_cs..adj_ce].to_string();

                let new_text = if seg_i == last_seg {
                    // Keep text after the placeholder end
                    if last_offset + 1 < old.len() {
                        old[last_offset + 1..].to_string()
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };

                let diff = new_text.len() as i64 - old.len() as i64;
                result.replace_range(adj_cs..adj_ce, &new_text);
                offset += diff;
            }
        }

        search_start = match_end;
    }

    Ok(result)
}

/// Replace a run that contains the placeholder with formatted runs XML.
fn replace_run_containing_placeholder(
    xml: &str,
    placeholder: &str,
    runs_xml: &str,
    text_element: &str,
    run_element: &str,
) -> Result<String> {
    // First try: if placeholder is within a single text element, replace the enclosing run
    let open_tag = format!("<{}", text_element);
    let close_tag = format!("</{}>", text_element);
    let run_open = format!("<{}", run_element);
    let run_close = format!("</{}>", run_element);

    let mut result = xml.to_string();
    let mut search_from = 0;

    loop {
        // Find next text element
        let text_start = match result[search_from..].find(&open_tag) {
            Some(p) => search_from + p,
            None => break,
        };

        let tag_end = match result[text_start..].find('>') {
            Some(p) => text_start + p + 1,
            None => break,
        };

        if result[text_start..tag_end].ends_with("/>") {
            search_from = tag_end;
            continue;
        }

        let content_end = match result[tag_end..].find(&close_tag) {
            Some(p) => tag_end + p,
            None => break,
        };

        let content = &result[tag_end..content_end];
        if !content.contains(placeholder) {
            search_from = content_end + close_tag.len();
            continue;
        }

        // Find the enclosing run element (not rPr or other tags starting with run_element prefix)
        let before_text = &result[..text_start];
        let run_start = {
            let mut found = None;
            let mut pos = before_text.len();
            while let Some(p) = before_text[..pos].rfind(&run_open) {
                // Verify this is actually the run element, not e.g. <w:rPr> when searching for <w:r
                let after = &before_text[p + run_open.len()..];
                if after.starts_with('>') || after.starts_with(' ') || after.starts_with('/') {
                    found = Some(p);
                    break;
                }
                pos = p;
            }
            match found {
                Some(p) => p,
                None => {
                    search_from = content_end + close_tag.len();
                    continue;
                }
            }
        };

        let after_text_start = content_end + close_tag.len();
        let run_end = match result[after_text_start..].find(&run_close) {
            Some(p) => after_text_start + p + run_close.len(),
            None => {
                search_from = after_text_start;
                continue;
            }
        };

        // Replace the entire run with the formatted runs
        result.replace_range(run_start..run_end, runs_xml);
        search_from = run_start + runs_xml.len();
    }

    // If no run-level replacement happened, fall back to simple text replacement
    if result == xml {
        return replace_across_text_elements(
            xml,
            placeholder,
            &runs_xml_to_text(runs_xml),
            text_element,
        );
    }

    Ok(result)
}

/// Extract plain text from formatted runs XML (fallback).
fn runs_xml_to_text(runs_xml: &str) -> String {
    let mut reader = Reader::from_str(runs_xml);
    let mut text = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Text(e)) => {
                if let Ok(s) = std::str::from_utf8(e.as_ref()) {
                    text.push_str(s);
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }

    text
}

/// Find image relationship IDs and alt text in OpenXML.
/// Returns (relationship_id, alt_text_or_name, image_path_in_rels).
pub fn find_images_in_xml(
    xml_bytes: &[u8],
    rels_bytes: &[u8],
) -> Result<Vec<(String, String, String)>> {
    let xml_str = std::str::from_utf8(xml_bytes)?;
    let rels_str = std::str::from_utf8(rels_bytes)?;

    // Parse relationships to get rId -> target mapping
    let rel_map = parse_relationships(rels_str)?;

    // Find image references in the XML
    let mut images = Vec::new();

    // Parse for drawing elements with image references
    let mut reader = Reader::from_str(xml_str);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();

    let mut current_name = String::new();
    let mut current_descr = String::new();
    let mut current_embed = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                let name = std::str::from_utf8(local.as_ref()).unwrap_or_default();

                match name {
                    "cNvPr" | "docPr" => {
                        for attr in e.attributes().flatten() {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or_default();
                            let val = std::str::from_utf8(&attr.value).unwrap_or_default();
                            match key {
                                "name" => current_name = val.to_string(),
                                "descr" => current_descr = val.to_string(),
                                _ => {}
                            }
                        }
                    }
                    "blip" => {
                        for attr in e.attributes().flatten() {
                            let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or_default();
                            let val = std::str::from_utf8(&attr.value).unwrap_or_default();
                            if key == "r:embed" || key == "embed" {
                                current_embed = val.to_string();
                            }
                        }

                        if !current_embed.is_empty() {
                            let identifier = if !current_descr.is_empty() {
                                current_descr.clone()
                            } else {
                                current_name.clone()
                            };

                            if let Some(target) = rel_map.get(&current_embed) {
                                images.push((current_embed.clone(), identifier, target.clone()));
                            }

                            current_embed.clear();
                            current_name.clear();
                            current_descr.clear();
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(flow_like_types::anyhow!("XML parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(images)
}

/// Parse an OpenXML .rels file into a map of rId -> Target.
pub fn parse_relationships(rels_xml: &str) -> Result<HashMap<String, String>> {
    let mut reader = Reader::from_str(rels_xml);
    let mut buf = Vec::new();
    let mut map = HashMap::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                let local = e.local_name();
                let name = std::str::from_utf8(local.as_ref()).unwrap_or_default();
                if name == "Relationship" {
                    let mut id = String::new();
                    let mut target = String::new();
                    for attr in e.attributes().flatten() {
                        let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or_default();
                        let val = std::str::from_utf8(&attr.value).unwrap_or_default();
                        match key {
                            "Id" => id = val.to_string(),
                            "Target" => target = val.to_string(),
                            _ => {}
                        }
                    }
                    if !id.is_empty() {
                        map.insert(id, target);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(flow_like_types::anyhow!("Rels parse error: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(map)
}

use super::ImageScaleMode;

/// Replace an image file in an OpenXML archive by matching alt text or shape name.
pub fn replace_image_in_archive(
    files: &mut HashMap<String, Vec<u8>>,
    xml_path: &str,
    rels_path: &str,
    identifier: &str,
    new_image_bytes: &[u8],
    _scale_mode: &ImageScaleMode,
) -> Result<bool> {
    let xml_bytes = files
        .get(xml_path)
        .ok_or_else(|| flow_like_types::anyhow!("XML file not found: {}", xml_path))?
        .clone();

    let rels_bytes = files
        .get(rels_path)
        .ok_or_else(|| flow_like_types::anyhow!("Rels file not found: {}", rels_path))?
        .clone();

    let images = find_images_in_xml(&xml_bytes, &rels_bytes)?;

    let mut replaced = false;
    for (_rel_id, img_identifier, img_path) in &images {
        if img_identifier == identifier {
            // Determine the full path of the image in the archive
            let base_dir = xml_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            let full_img_path = if img_path.starts_with('/') {
                img_path[1..].to_string()
            } else {
                if base_dir.is_empty() {
                    img_path.clone()
                } else {
                    format!("{}/{}", base_dir, img_path)
                }
            };

            // Normalize path (resolve ../)
            let full_img_path = normalize_path(&full_img_path);

            if files.contains_key(&full_img_path) {
                files.insert(full_img_path, new_image_bytes.to_vec());
                replaced = true;
            }
        }
    }

    Ok(replaced)
}

/// Normalize a path by resolving `../` segments.
fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            other => parts.push(other),
        }
    }
    parts.join("/")
}

type Result<T> = flow_like_types::Result<T>;

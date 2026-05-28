//! E2E tests that generate real DOCX, PPTX, and PDF files for visual inspection.
//!
//! Run with:
//!   cargo test -p flow-like-catalog-media --features execute --test document_generation -- --nocapture
//!
//! Output files are written to `packages/catalog/media/tests/output/`.

#![cfg(feature = "execute")]

use std::collections::HashMap;
use std::path::PathBuf;

use flow_like_catalog_media::document::chart::{
    ChartType, OfficeChartData, chart_input_to_office_data, parse_chart_block,
};
use flow_like_catalog_media::document::openxml::{
    BlockType, FormattedRun, OpenXmlFormat, markdown_to_runs, read_zip, replace_text_in_xml,
    replace_text_in_xml_markdown, write_zip,
};
use flow_like_catalog_media::document::styles::{
    self, ParagraphStyle, TextAlignment, cm_to_emu, cm_to_twips, defaults, hex_to_ooxml,
    pt_to_half_points, pt_to_hundredths,
};

fn output_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/output");
    std::fs::create_dir_all(&dir).expect("create output dir");
    dir
}

// ---------------------------------------------------------------------------
// DOCX helpers (replicate private node logic using only public APIs)
// ---------------------------------------------------------------------------

fn create_empty_docx(font: &str, font_size_pt: f32, theme_color: &str) -> Vec<u8> {
    let accent = hex_to_ooxml(theme_color);
    let half_pts = pt_to_half_points(font_size_pt);
    let heading_color = hex_to_ooxml(defaults::HEADING);
    let margin_twips = cm_to_twips(defaults::MARGIN_CM);

    let content_types = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
</Types>"#;

    let rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
</Relationships>"#;

    let word_rels = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;

    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:wpc="http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:wp14="http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:w10="urn:schemas-microsoft-com:office:word" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup" xmlns:wpi="http://schemas.microsoft.com/office/word/2010/wordprocessingInk" xmlns:wne="http://schemas.microsoft.com/office/word/2006/wordml" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" mc:Ignorable="w14 w15 wp14">
<w:body>
<w:sectPr>
<w:pgSz w:w="11906" w:h="16838"/>
<w:pgMar w:top="{margin_twips}" w:right="{margin_twips}" w:bottom="{margin_twips}" w:left="{margin_twips}" w:header="720" w:footer="720" w:gutter="0"/>
</w:sectPr>
</w:body>
</w:document>"#
    );

    let styles_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<w:docDefaults>
<w:rPrDefault><w:rPr>
<w:rFonts w:ascii="{font}" w:hAnsi="{font}" w:cs="{font}"/>
<w:sz w:val="{sz}"/><w:szCs w:val="{sz}"/>
<w:color w:val="{text}"/>
</w:rPr></w:rPrDefault>
<w:pPrDefault><w:pPr>
<w:spacing w:after="160" w:line="259" w:lineRule="auto"/>
</w:pPr></w:pPrDefault>
</w:docDefaults>
<w:style w:type="paragraph" w:styleId="Normal" w:default="1"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/>
<w:pPr><w:spacing w:before="240" w:after="80"/><w:outlineLvl w:val="0"/></w:pPr>
<w:rPr><w:b/><w:sz w:val="48"/><w:szCs w:val="48"/><w:color w:val="{heading}"/></w:rPr>
</w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/>
<w:pPr><w:spacing w:before="200" w:after="60"/><w:outlineLvl w:val="1"/></w:pPr>
<w:rPr><w:b/><w:sz w:val="36"/><w:szCs w:val="36"/><w:color w:val="{heading}"/></w:rPr>
</w:style>
<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/>
<w:pPr><w:spacing w:before="160" w:after="40"/><w:outlineLvl w:val="2"/></w:pPr>
<w:rPr><w:b/><w:sz w:val="28"/><w:szCs w:val="28"/><w:color w:val="{heading}"/></w:rPr>
</w:style>
<w:style w:type="paragraph" w:styleId="Heading4"><w:name w:val="heading 4"/>
<w:pPr><w:spacing w:before="120" w:after="40"/><w:outlineLvl w:val="3"/></w:pPr>
<w:rPr><w:b/><w:sz w:val="24"/><w:szCs w:val="24"/><w:color w:val="{accent}"/></w:rPr>
</w:style>
<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/>
<w:pPr><w:spacing w:after="80"/></w:pPr>
<w:rPr><w:b/><w:sz w:val="56"/><w:szCs w:val="56"/><w:color w:val="{heading}"/></w:rPr>
</w:style>
<w:style w:type="paragraph" w:styleId="Subtitle"><w:name w:val="Subtitle"/>
<w:pPr><w:spacing w:after="120"/></w:pPr>
<w:rPr><w:sz w:val="28"/><w:szCs w:val="28"/><w:color w:val="{accent}"/></w:rPr>
</w:style>
<w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/>
<w:pPr><w:pBdr><w:left w:val="single" w:sz="12" w:space="4" w:color="{accent}"/></w:pBdr><w:ind w:left="480"/><w:spacing w:before="120" w:after="120"/></w:pPr>
<w:rPr><w:i/><w:color w:val="{muted}"/></w:rPr>
</w:style>
</w:styles>"#,
        font = font,
        sz = half_pts,
        text = hex_to_ooxml(defaults::TEXT),
        heading = heading_color,
        accent = accent,
        muted = hex_to_ooxml(defaults::TEXT_MUTED),
    );

    let core = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
</cp:coreProperties>"#;

    let mut files = HashMap::new();
    files.insert(
        "[Content_Types].xml".to_string(),
        content_types.as_bytes().to_vec(),
    );
    files.insert("_rels/.rels".to_string(), rels.as_bytes().to_vec());
    files.insert(
        "word/_rels/document.xml.rels".to_string(),
        word_rels.as_bytes().to_vec(),
    );
    files.insert("word/document.xml".to_string(), document.into_bytes());
    files.insert("word/styles.xml".to_string(), styles_xml.into_bytes());
    files.insert("docProps/core.xml".to_string(), core.as_bytes().to_vec());

    write_zip(&files).expect("write_zip for DOCX")
}

fn build_paragraph(
    text: &str,
    style: &ParagraphStyle,
    alignment: &TextAlignment,
    font_family: Option<&str>,
    font_size_pt: Option<f32>,
    font_color: Option<&str>,
    bold: bool,
    italic: bool,
) -> String {
    let escaped = xml_escape(text);

    let mut p_pr = String::from("<w:pPr>");
    let style_id = style.to_style_id();
    if style_id != "Normal" {
        p_pr.push_str(&format!(r#"<w:pStyle w:val="{}"/>"#, style_id));
    }
    p_pr.push_str(&format!(r#"<w:jc w:val="{}"/>"#, alignment.to_ooxml_docx()));
    p_pr.push_str("</w:pPr>");

    let mut r_pr = String::from("<w:rPr>");
    let font = font_family.unwrap_or(defaults::FONT_SANS);
    r_pr.push_str(&format!(
        r#"<w:rFonts w:ascii="{f}" w:hAnsi="{f}"/>"#,
        f = xml_escape(font)
    ));

    let size = font_size_pt.unwrap_or_else(|| style.font_size_pt());
    let half_pts = pt_to_half_points(size);
    r_pr.push_str(&format!(
        r#"<w:sz w:val="{}"/><w:szCs w:val="{}"/>"#,
        half_pts, half_pts
    ));

    let color = font_color.unwrap_or(defaults::TEXT);
    r_pr.push_str(&format!(r#"<w:color w:val="{}"/>"#, hex_to_ooxml(color)));

    if bold {
        r_pr.push_str("<w:b/>");
    }
    if italic {
        r_pr.push_str("<w:i/>");
    }
    r_pr.push_str("</w:rPr>");

    format!(
        "<w:p>{}<w:r>{}<w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
        p_pr, r_pr, escaped
    )
}

fn build_table(
    data: &[Vec<String>],
    header_row: bool,
    alternate_rows: bool,
    border_color: &str,
    font_size_pt: f32,
) -> String {
    let col_count = data.iter().map(|r| r.len()).max().unwrap_or(1);
    let total_width = cm_to_twips(16.0);
    let col_width = total_width / col_count as i32;
    let border = hex_to_ooxml(border_color);
    let half_pts = pt_to_half_points(font_size_pt);

    let mut xml = String::from("<w:tbl>");

    xml.push_str("<w:tblPr>");
    xml.push_str(r#"<w:tblW w:w="0" w:type="auto"/>"#);
    xml.push_str(&format!(
        r#"<w:tblBorders>
<w:top w:val="single" w:sz="4" w:space="0" w:color="{b}"/>
<w:left w:val="single" w:sz="4" w:space="0" w:color="{b}"/>
<w:bottom w:val="single" w:sz="4" w:space="0" w:color="{b}"/>
<w:right w:val="single" w:sz="4" w:space="0" w:color="{b}"/>
<w:insideH w:val="single" w:sz="4" w:space="0" w:color="{b}"/>
<w:insideV w:val="single" w:sz="4" w:space="0" w:color="{b}"/>
</w:tblBorders>"#,
        b = border
    ));
    xml.push_str(r#"<w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" w:noHBand="0" w:noVBand="1"/>"#);
    xml.push_str("</w:tblPr>");

    xml.push_str("<w:tblGrid>");
    for _ in 0..col_count {
        xml.push_str(&format!("<w:gridCol w:w=\"{}\"/>", col_width));
    }
    xml.push_str("</w:tblGrid>");

    for (row_idx, row) in data.iter().enumerate() {
        let is_header = header_row && row_idx == 0;
        let bg_color = if is_header {
            hex_to_ooxml(defaults::PRIMARY)
        } else if alternate_rows && row_idx % 2 == 0 {
            hex_to_ooxml(defaults::SURFACE)
        } else {
            hex_to_ooxml(defaults::BACKGROUND)
        };
        let text_color = if is_header {
            "FFFFFF".to_string()
        } else {
            hex_to_ooxml(defaults::TEXT)
        };

        xml.push_str("<w:tr>");
        for col_idx in 0..col_count {
            let cell_text = row.get(col_idx).map(|s| s.as_str()).unwrap_or("");
            xml.push_str("<w:tc>");
            xml.push_str(&format!(
                r#"<w:tcPr><w:shd w:val="clear" w:color="auto" w:fill="{}"/></w:tcPr>"#,
                bg_color
            ));
            xml.push_str("<w:p><w:r><w:rPr>");
            xml.push_str(&format!(
                r#"<w:sz w:val="{}"/><w:szCs w:val="{}"/>"#,
                half_pts, half_pts
            ));
            xml.push_str(&format!(r#"<w:color w:val="{}"/>"#, text_color));
            if is_header {
                xml.push_str("<w:b/>");
            }
            xml.push_str("</w:rPr>");
            xml.push_str(&format!(
                "<w:t xml:space=\"preserve\">{}</w:t>",
                xml_escape(cell_text)
            ));
            xml.push_str("</w:r></w:p>");
            xml.push_str("</w:tc>");
        }
        xml.push_str("</w:tr>");
    }

    xml.push_str("</w:tbl>");
    xml
}

fn insert_before_sect_pr(files: &mut HashMap<String, Vec<u8>>, xml_fragment: &str) {
    let doc_key = "word/document.xml";
    if let Some(doc_data) = files.get(doc_key).cloned() {
        let mut doc_xml = String::from_utf8_lossy(&doc_data).to_string();
        if let Some(pos) = doc_xml.rfind("<w:sectPr") {
            doc_xml.insert_str(pos, xml_fragment);
        } else if let Some(pos) = doc_xml.rfind("</w:body>") {
            doc_xml.insert_str(pos, xml_fragment);
        }
        files.insert(doc_key.to_string(), doc_xml.into_bytes());
    }
}

// ---------------------------------------------------------------------------
// PPTX helpers
// ---------------------------------------------------------------------------

fn create_empty_pptx() -> Vec<u8> {
    let mut files = HashMap::new();

    files.insert(
        "[Content_Types].xml".to_string(),
        PPTX_CONTENT_TYPES.as_bytes().to_vec(),
    );
    files.insert("_rels/.rels".to_string(), PPTX_TOP_RELS.as_bytes().to_vec());
    files.insert(
        "ppt/presentation.xml".to_string(),
        PPTX_PRESENTATION.as_bytes().to_vec(),
    );
    files.insert(
        "ppt/_rels/presentation.xml.rels".to_string(),
        PPTX_PRESENTATION_RELS.as_bytes().to_vec(),
    );
    files.insert(
        "ppt/slideMasters/slideMaster1.xml".to_string(),
        PPTX_SLIDE_MASTER.as_bytes().to_vec(),
    );
    files.insert(
        "ppt/slideMasters/_rels/slideMaster1.xml.rels".to_string(),
        PPTX_SLIDE_MASTER_RELS.as_bytes().to_vec(),
    );
    files.insert(
        "ppt/slideLayouts/slideLayout1.xml".to_string(),
        PPTX_SLIDE_LAYOUT.as_bytes().to_vec(),
    );
    files.insert(
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels".to_string(),
        PPTX_SLIDE_LAYOUT_RELS.as_bytes().to_vec(),
    );
    files.insert(
        "ppt/theme/theme1.xml".to_string(),
        PPTX_THEME.as_bytes().to_vec(),
    );
    files.insert(
        "docProps/app.xml".to_string(),
        PPTX_APP_XML.as_bytes().to_vec(),
    );
    files.insert(
        "docProps/core.xml".to_string(),
        PPTX_CORE_XML.as_bytes().to_vec(),
    );

    write_zip(&files).expect("write_zip for PPTX")
}

fn pptx_add_slide(files: &mut HashMap<String, Vec<u8>>) -> u32 {
    let slide_num = pptx_next_slide_number(files);
    let layout_target = pptx_find_slide_layout_target(files);

    let blank_slide = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
  xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr/>
    </p:spTree>
  </p:cSld>
</p:sld>"#;

    let slide_rels = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="{layout_target}"/>
</Relationships>"#
    );

    files.insert(
        format!("ppt/slides/slide{}.xml", slide_num),
        blank_slide.as_bytes().to_vec(),
    );
    files.insert(
        format!("ppt/slides/_rels/slide{}.xml.rels", slide_num),
        slide_rels.into_bytes(),
    );

    pptx_update_presentation_xml(files, slide_num);
    pptx_update_content_types(files, slide_num);

    slide_num
}

fn pptx_next_slide_number(files: &HashMap<String, Vec<u8>>) -> u32 {
    let mut max = 0u32;
    for key in files.keys() {
        if let Some(rest) = key.strip_prefix("ppt/slides/slide") {
            if let Some(num_str) = rest.strip_suffix(".xml") {
                if let Ok(n) = num_str.parse::<u32>() {
                    max = max.max(n);
                }
            }
        }
    }
    max + 1
}

fn pptx_find_slide_layout_target(files: &HashMap<String, Vec<u8>>) -> String {
    for key in files.keys() {
        if key.starts_with("ppt/slides/_rels/slide") && key.ends_with(".xml.rels") {
            if let Some(data) = files.get(key) {
                let content = String::from_utf8_lossy(data);
                if let Some(pos) = content.find("slideLayout") {
                    if let Some(start) = content[..pos].rfind("Target=\"") {
                        let target_start = start + 8;
                        if let Some(end) = content[target_start..].find('"') {
                            return content[target_start..target_start + end].to_string();
                        }
                    }
                }
            }
        }
    }
    "../slideLayouts/slideLayout1.xml".to_string()
}

fn pptx_update_presentation_xml(files: &mut HashMap<String, Vec<u8>>, slide_num: u32) {
    if let Some(pres_data) = files.get("ppt/presentation.xml") {
        let mut content = String::from_utf8_lossy(pres_data).to_string();
        let pres_rels_data = files.get("ppt/_rels/presentation.xml.rels").cloned();

        let new_rid = pptx_next_rid(&pres_rels_data);
        let new_sld_id = pptx_next_sld_id(&content);

        let sld_entry = format!(r#"<p:sldId id="{}" r:id="{}"/>"#, new_sld_id, new_rid);

        if let Some(pos) = content.find("</p:sldIdLst>") {
            content.insert_str(pos, &sld_entry);
        } else if let Some(pos) = content.find("</p:sldMasterIdLst>") {
            let insert_pos = pos + "</p:sldMasterIdLst>".len();
            content.insert_str(
                insert_pos,
                &format!("<p:sldIdLst>{}</p:sldIdLst>", sld_entry),
            );
        }

        files.insert("ppt/presentation.xml".to_string(), content.into_bytes());

        let rel_entry = format!(
            r#"<Relationship Id="{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide{}.xml"/>"#,
            new_rid, slide_num
        );
        if let Some(rels_data) = files.get("ppt/_rels/presentation.xml.rels") {
            let mut rels = String::from_utf8_lossy(rels_data).to_string();
            if let Some(pos) = rels.find("</Relationships>") {
                rels.insert_str(pos, &rel_entry);
            }
            files.insert(
                "ppt/_rels/presentation.xml.rels".to_string(),
                rels.into_bytes(),
            );
        }
    }
}

fn pptx_next_rid(rels_data: &Option<Vec<u8>>) -> String {
    let mut max = 0u32;
    if let Some(data) = rels_data {
        let content = String::from_utf8_lossy(data);
        for cap in content.match_indices("rId") {
            let rest = &content[cap.0 + 3..];
            if let Some(end) = rest.find('"') {
                if let Ok(n) = rest[..end].parse::<u32>() {
                    max = max.max(n);
                }
            }
        }
    }
    format!("rId{}", max + 1)
}

fn pptx_next_sld_id(pres_content: &str) -> u32 {
    let mut max = 256u32;
    let section = if let Some(start) = pres_content.find("<p:sldIdLst>") {
        let offset = start + "<p:sldIdLst>".len();
        pres_content[offset..]
            .find("</p:sldIdLst>")
            .map(|end| &pres_content[start..offset + end])
            .unwrap_or("")
    } else {
        ""
    };
    for cap in section.match_indices("id=\"") {
        let rest = &section[cap.0 + 4..];
        if let Some(end) = rest.find('"') {
            if let Ok(n) = rest[..end].parse::<u32>() {
                if n >= 256 {
                    max = max.max(n);
                }
            }
        }
    }
    max + 1
}

fn pptx_update_content_types(files: &mut HashMap<String, Vec<u8>>, slide_num: u32) {
    if let Some(ct_data) = files.get("[Content_Types].xml") {
        let mut content = String::from_utf8_lossy(ct_data).to_string();
        let entry = format!(
            r#"<Override PartName="/ppt/slides/slide{}.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>"#,
            slide_num
        );
        if let Some(pos) = content.find("</Types>") {
            content.insert_str(pos, &entry);
        }
        files.insert("[Content_Types].xml".to_string(), content.into_bytes());
    }
}

fn pptx_add_text_box(
    files: &mut HashMap<String, Vec<u8>>,
    slide_num: u32,
    text: &str,
    x_cm: f32,
    y_cm: f32,
    w_cm: f32,
    h_cm: f32,
    font_size: f32,
    font_color: &str,
    bold: bool,
) {
    let slide_path = format!("ppt/slides/slide{}.xml", slide_num);
    let slide_data = files.get(&slide_path).expect("slide exists").clone();
    let mut slide_xml = String::from_utf8_lossy(&slide_data).to_string();

    let next_id = max_id(&slide_xml) + 1;
    let bold_attr = if bold { r#" b="1""# } else { "" };
    let color_val = hex_to_ooxml(font_color);
    let font_hundredths = pt_to_hundredths(font_size) as i64;

    let shape_xml = format!(
        r#"<p:sp>
  <p:nvSpPr>
    <p:cNvPr id="{id}" name="TextBox {id}"/>
    <p:cNvSpPr txBox="1"/>
    <p:nvPr/>
  </p:nvSpPr>
  <p:spPr>
    <a:xfrm>
      <a:off x="{ox}" y="{oy}"/>
      <a:ext cx="{cx}" cy="{cy}"/>
    </a:xfrm>
    <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
    <a:noFill/>
  </p:spPr>
  <p:txBody>
    <a:bodyPr wrap="square" rtlCol="0"/>
    <a:lstStyle/>
    <a:p>
      <a:r>
        <a:rPr lang="en-US" sz="{sz}" dirty="0"{bold}>
          <a:solidFill><a:srgbClr val="{clr}"/></a:solidFill>
          <a:latin typeface="Calibri"/>
        </a:rPr>
        <a:t>{text}</a:t>
      </a:r>
    </a:p>
  </p:txBody>
</p:sp>"#,
        id = next_id,
        ox = cm_to_emu(x_cm),
        oy = cm_to_emu(y_cm),
        cx = cm_to_emu(w_cm),
        cy = cm_to_emu(h_cm),
        sz = font_hundredths,
        bold = bold_attr,
        clr = color_val,
        text = xml_escape(text),
    );

    if let Some(pos) = slide_xml.find("</p:spTree>") {
        slide_xml.insert_str(pos, &shape_xml);
    }
    files.insert(slide_path, slide_xml.into_bytes());
}

fn pptx_add_text_box_aligned(
    files: &mut HashMap<String, Vec<u8>>,
    slide_num: u32,
    text: &str,
    x_cm: f32,
    y_cm: f32,
    w_cm: f32,
    h_cm: f32,
    font_size: f32,
    font_color: &str,
    bold: bool,
    align: &str,
    anchor: &str,
) {
    let slide_path = format!("ppt/slides/slide{}.xml", slide_num);
    let slide_data = files.get(&slide_path).expect("slide exists").clone();
    let mut slide_xml = String::from_utf8_lossy(&slide_data).to_string();

    let next_id = max_id(&slide_xml) + 1;
    let bold_attr = if bold { r#" b="1""# } else { "" };
    let color_val = hex_to_ooxml(font_color);
    let font_hundredths = pt_to_hundredths(font_size) as i64;
    let algn_attr = if align.is_empty() {
        String::new()
    } else {
        format!(r#"<a:pPr algn="{}"/>"#, align)
    };
    let anchor_attr = if anchor.is_empty() {
        String::new()
    } else {
        format!(r#" anchor="{}""#, anchor)
    };

    let shape_xml = format!(
        r#"<p:sp>
  <p:nvSpPr>
    <p:cNvPr id="{id}" name="TextBox {id}"/>
    <p:cNvSpPr txBox="1"/>
    <p:nvPr/>
  </p:nvSpPr>
  <p:spPr>
    <a:xfrm>
      <a:off x="{ox}" y="{oy}"/>
      <a:ext cx="{cx}" cy="{cy}"/>
    </a:xfrm>
    <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
    <a:noFill/>
  </p:spPr>
  <p:txBody>
    <a:bodyPr wrap="square" rtlCol="0"{anchor}/>
    <a:lstStyle/>
    <a:p>
      {algn}
      <a:r>
        <a:rPr lang="en-US" sz="{sz}" dirty="0"{bold}>
          <a:solidFill><a:srgbClr val="{clr}"/></a:solidFill>
          <a:latin typeface="Calibri"/>
        </a:rPr>
        <a:t>{text}</a:t>
      </a:r>
    </a:p>
  </p:txBody>
</p:sp>"#,
        id = next_id,
        ox = cm_to_emu(x_cm),
        oy = cm_to_emu(y_cm),
        cx = cm_to_emu(w_cm),
        cy = cm_to_emu(h_cm),
        sz = font_hundredths,
        bold = bold_attr,
        clr = color_val,
        text = xml_escape(text),
        algn = algn_attr,
        anchor = anchor_attr,
    );

    if let Some(pos) = slide_xml.find("</p:spTree>") {
        slide_xml.insert_str(pos, &shape_xml);
    }
    files.insert(slide_path, slide_xml.into_bytes());
}

fn pptx_add_shape(
    files: &mut HashMap<String, Vec<u8>>,
    slide_num: u32,
    preset: &str,
    x_cm: f32,
    y_cm: f32,
    w_cm: f32,
    h_cm: f32,
    fill_color: &str,
    line_color: &str,
    text: &str,
) {
    let slide_path = format!("ppt/slides/slide{}.xml", slide_num);
    let slide_data = files.get(&slide_path).expect("slide exists").clone();
    let mut slide_xml = String::from_utf8_lossy(&slide_data).to_string();

    let next_id = max_id(&slide_xml) + 1;
    let fill_val = hex_to_ooxml(fill_color);

    let line_xml = if line_color.is_empty() {
        "<a:ln><a:noFill/></a:ln>".to_string()
    } else {
        let lc = hex_to_ooxml(line_color);
        format!(
            r#"<a:ln w="12700"><a:solidFill><a:srgbClr val="{}"/></a:solidFill></a:ln>"#,
            lc
        )
    };

    let text_body = if text.is_empty() {
        r#"<p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang="en-US"/></a:p></p:txBody>"#
            .to_string()
    } else {
        format!(
            r#"<p:txBody><a:bodyPr anchor="ctr"/><a:lstStyle/><a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-US" sz="1400" dirty="0"><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill><a:latin typeface="Calibri"/></a:rPr><a:t>{}</a:t></a:r></a:p></p:txBody>"#,
            xml_escape(text)
        )
    };

    let shape_xml = format!(
        r#"<p:sp>
  <p:nvSpPr>
    <p:cNvPr id="{id}" name="Shape {id}"/>
    <p:cNvSpPr/>
    <p:nvPr/>
  </p:nvSpPr>
  <p:spPr>
    <a:xfrm>
      <a:off x="{ox}" y="{oy}"/>
      <a:ext cx="{cx}" cy="{cy}"/>
    </a:xfrm>
    <a:prstGeom prst="{prst}"><a:avLst/></a:prstGeom>
    <a:solidFill><a:srgbClr val="{fill}"/></a:solidFill>
    {line}
  </p:spPr>
  {txBody}
</p:sp>"#,
        id = next_id,
        ox = cm_to_emu(x_cm),
        oy = cm_to_emu(y_cm),
        cx = cm_to_emu(w_cm),
        cy = cm_to_emu(h_cm),
        prst = preset,
        fill = fill_val,
        line = line_xml,
        txBody = text_body,
    );

    if let Some(pos) = slide_xml.find("</p:spTree>") {
        slide_xml.insert_str(pos, &shape_xml);
    }
    files.insert(slide_path, slide_xml.into_bytes());
}

fn pptx_add_table(
    files: &mut HashMap<String, Vec<u8>>,
    slide_num: u32,
    headers: &[&str],
    rows: &[Vec<&str>],
    x_cm: f32,
    y_cm: f32,
    width_cm: f32,
    row_height_cm: f32,
) {
    let slide_path = format!("ppt/slides/slide{}.xml", slide_num);
    let slide_data = files.get(&slide_path).expect("slide exists").clone();
    let mut slide_xml = String::from_utf8_lossy(&slide_data).to_string();

    let col_count = headers.len().max(1);
    let col_width = cm_to_emu(width_cm) / col_count as i64;
    let rh = cm_to_emu(row_height_cm);
    let total_rows = 1 + rows.len();
    let total_height = rh * total_rows as i64;
    let next_id = max_id(&slide_xml) + 1;

    let mut grid_cols = String::new();
    for _ in 0..col_count {
        grid_cols.push_str(&format!(r#"<a:gridCol w="{}"/>"#, col_width));
    }

    let mut rows_xml = String::new();

    // Header row
    rows_xml.push_str(&format!(r#"<a:tr h="{}">"#, rh));
    for h in headers {
        rows_xml.push_str(&pptx_build_cell(&xml_escape(h), "FF4343", "FFFFFF", true));
    }
    rows_xml.push_str("</a:tr>");

    // Data rows
    for (row_idx, row_data) in rows.iter().enumerate() {
        let bg = if row_idx % 2 == 0 { "F9FAFB" } else { "FFFFFF" };
        rows_xml.push_str(&format!(r#"<a:tr h="{}">"#, rh));
        for i in 0..col_count {
            let val = row_data.get(i).copied().unwrap_or("");
            rows_xml.push_str(&pptx_build_cell(&xml_escape(val), bg, "1A1A1A", false));
        }
        rows_xml.push_str("</a:tr>");
    }

    let table_xml = format!(
        r#"<p:graphicFrame>
  <p:nvGraphicFramePr>
    <p:cNvPr id="{id}" name="Table {id}"/>
    <p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr>
    <p:nvPr/>
  </p:nvGraphicFramePr>
  <p:xfrm>
    <a:off x="{ox}" y="{oy}"/>
    <a:ext cx="{cx}" cy="{cy}"/>
  </p:xfrm>
  <a:graphic>
    <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table">
      <a:tbl>
        <a:tblPr firstRow="1" bandRow="1"/>
        <a:tblGrid>{grid}</a:tblGrid>
        {rows}
      </a:tbl>
    </a:graphicData>
  </a:graphic>
</p:graphicFrame>"#,
        id = next_id,
        ox = cm_to_emu(x_cm),
        oy = cm_to_emu(y_cm),
        cx = cm_to_emu(width_cm),
        cy = total_height,
        grid = grid_cols,
        rows = rows_xml,
    );

    if let Some(pos) = slide_xml.find("</p:spTree>") {
        slide_xml.insert_str(pos, &table_xml);
    }
    files.insert(slide_path, slide_xml.into_bytes());
}

fn pptx_build_cell(text: &str, bg_color: &str, text_color: &str, bold: bool) -> String {
    let bold_attr = if bold { r#" b="1""# } else { "" };
    format!(
        r#"<a:tc>
  <a:txBody>
    <a:bodyPr/>
    <a:lstStyle/>
    <a:p>
      <a:r>
        <a:rPr lang="en-US" sz="1400" dirty="0"{bold}>
          <a:solidFill><a:srgbClr val="{tc}"/></a:solidFill>
          <a:latin typeface="Calibri"/>
        </a:rPr>
        <a:t>{text}</a:t>
      </a:r>
    </a:p>
  </a:txBody>
  <a:tcPr>
    <a:solidFill><a:srgbClr val="{bg}"/></a:solidFill>
  </a:tcPr>
</a:tc>"#,
        bold = bold_attr,
        tc = text_color,
        bg = bg_color,
        text = text,
    )
}

fn pptx_add_multi_chart(
    files: &mut HashMap<String, Vec<u8>>,
    slide_num: u32,
    chart_type: &str,
    categories: &[&str],
    all_values: &[&[f64]],
    series_names: &[&str],
    colors: &[&str],
    x_cm: f32,
    y_cm: f32,
    w_cm: f32,
    h_cm: f32,
) {
    let chart_num = pptx_next_chart_number(files);
    let chart_path = format!("ppt/charts/chart{}.xml", chart_num);
    let chart_rels_path = format!("ppt/charts/_rels/chart{}.xml.rels", chart_num);

    let chart_xml =
        build_multi_series_chart_xml(chart_type, categories, all_values, series_names, colors);
    files.insert(chart_path, chart_xml.into_bytes());
    files.insert(
        chart_rels_path,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#
            .as_bytes()
            .to_vec(),
    );

    let slide_path = format!("ppt/slides/slide{}.xml", slide_num);
    let slide_rels_path = format!("ppt/slides/_rels/slide{}.xml.rels", slide_num);

    let slide_data = files.get(&slide_path).expect("slide exists").clone();

    let rid = pptx_next_rel_id(files, &slide_rels_path);
    pptx_add_relationship(
        files,
        &slide_rels_path,
        &rid,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart",
        &format!("../charts/chart{}.xml", chart_num),
    );

    pptx_update_content_types_for_chart(files, chart_num);

    let mut slide_xml = String::from_utf8_lossy(&slide_data).to_string();
    let next_id = max_id(&slide_xml) + 1;

    let frame_xml = format!(
        r#"<p:graphicFrame>
  <p:nvGraphicFramePr>
    <p:cNvPr id="{id}" name="Chart {id}"/>
    <p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr>
    <p:nvPr/>
  </p:nvGraphicFramePr>
  <p:xfrm>
    <a:off x="{ox}" y="{oy}"/>
    <a:ext cx="{cx}" cy="{cy}"/>
  </p:xfrm>
  <a:graphic>
    <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">
      <c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="{rid}"/>
    </a:graphicData>
  </a:graphic>
</p:graphicFrame>"#,
        id = next_id,
        ox = cm_to_emu(x_cm),
        oy = cm_to_emu(y_cm),
        cx = cm_to_emu(w_cm),
        cy = cm_to_emu(h_cm),
        rid = rid,
    );

    if let Some(pos) = slide_xml.find("</p:spTree>") {
        slide_xml.insert_str(pos, &frame_xml);
    }
    files.insert(slide_path, slide_xml.into_bytes());
}

fn build_multi_series_chart_xml(
    chart_type: &str,
    categories: &[&str],
    all_values: &[&[f64]],
    series_names: &[&str],
    colors: &[&str],
) -> String {
    let count = categories.len();
    let mut cat_xml = String::new();
    for (i, cat) in categories.iter().enumerate() {
        cat_xml.push_str(&format!(
            r#"<c:pt idx="{}"><c:v>{}</c:v></c:pt>"#,
            i,
            xml_escape(cat)
        ));
    }

    let mut series_xml = String::new();
    for (si, values) in all_values.iter().enumerate() {
        let sn = series_names.get(si).copied().unwrap_or("Series");
        let color = colors.get(si).copied().unwrap_or("FF4343");

        let mut val_xml = String::new();
        for (i, val) in values.iter().enumerate() {
            val_xml.push_str(&format!(r#"<c:pt idx="{}"><c:v>{}</c:v></c:pt>"#, i, val));
        }

        // For pie charts, color each data point individually
        let fill_section = if chart_type == "pie" {
            let mut dpt_xml = String::new();
            for (i, _) in values.iter().enumerate() {
                let pt_color = colors.get(i).copied().unwrap_or("CCCCCC");
                dpt_xml.push_str(&format!(
                    r#"<c:dPt><c:idx val="{i}"/><c:spPr><a:solidFill><a:srgbClr val="{c}"/></a:solidFill></c:spPr></c:dPt>"#,
                    i = i, c = pt_color,
                ));
            }
            dpt_xml
        } else {
            match chart_type {
                "line" => format!(
                    r#"<c:spPr><a:ln w="28575"><a:solidFill><a:srgbClr val="{c}"/></a:solidFill></a:ln></c:spPr>"#,
                    c = color
                ),
                _ => format!(
                    r#"<c:spPr><a:solidFill><a:srgbClr val="{c}"/></a:solidFill></c:spPr>"#,
                    c = color
                ),
            }
        };

        series_xml.push_str(&format!(
            r#"<c:ser>
    <c:idx val="{si}"/><c:order val="{si}"/>
    <c:tx><c:v>{sn}</c:v></c:tx>
    {fill}
    <c:cat><c:strRef><c:strCache><c:ptCount val="{cnt}"/>{cats}</c:strCache></c:strRef></c:cat>
    <c:val><c:numRef><c:numCache><c:ptCount val="{cnt}"/>{vals}</c:numCache></c:numRef></c:val>
  </c:ser>"#,
            si = si,
            sn = xml_escape(sn),
            fill = fill_section,
            cnt = count,
            cats = cat_xml,
            vals = val_xml,
        ));
    }

    let axes_xml = r#"<c:axId val="111111111"/><c:axId val="222222222"/>"#;
    let axis_defs = r#"<c:catAx>
      <c:axId val="111111111"/><c:scaling><c:orientation val="minMax"/></c:scaling>
      <c:delete val="0"/><c:axPos val="b"/><c:crossAx val="222222222"/>
    </c:catAx>
    <c:valAx>
      <c:axId val="222222222"/><c:scaling><c:orientation val="minMax"/></c:scaling>
      <c:delete val="0"/><c:axPos val="l"/><c:crossAx val="111111111"/>
    </c:valAx>"#;

    let chart_element = match chart_type {
        "pie" => format!(
            r#"<c:pieChart><c:varyColors val="1"/>{}</c:pieChart>"#,
            series_xml
        ),
        "line" => format!(
            r#"<c:lineChart><c:grouping val="standard"/>{}{}</c:lineChart>{}"#,
            series_xml, axes_xml, axis_defs
        ),
        _ => format!(
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="clustered"/>{}{}</c:barChart>{}"#,
            series_xml, axes_xml, axis_defs
        ),
    };

    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
  xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <c:chart>
    <c:plotArea>
      <c:layout/>
      {chart}
    </c:plotArea>
    <c:legend><c:legendPos val="b"/></c:legend>
  </c:chart>
</c:chartSpace>"#,
        chart = chart_element,
    )
}

fn pptx_next_chart_number(files: &HashMap<String, Vec<u8>>) -> u32 {
    let mut max = 0u32;
    for key in files.keys() {
        if let Some(rest) = key.strip_prefix("ppt/charts/chart") {
            if let Some(num_str) = rest.strip_suffix(".xml") {
                if let Ok(n) = num_str.parse::<u32>() {
                    max = max.max(n);
                }
            }
        }
    }
    max + 1
}

fn pptx_next_rel_id(files: &HashMap<String, Vec<u8>>, rels_path: &str) -> String {
    let mut max = 0u32;
    if let Some(data) = files.get(rels_path) {
        let content = String::from_utf8_lossy(data);
        for cap in content.match_indices("rId") {
            let rest = &content[cap.0 + 3..];
            if let Some(end) = rest.find('"') {
                if let Ok(n) = rest[..end].parse::<u32>() {
                    max = max.max(n);
                }
            }
        }
    }
    format!("rId{}", max + 1)
}

fn pptx_add_relationship(
    files: &mut HashMap<String, Vec<u8>>,
    rels_path: &str,
    rid: &str,
    rel_type: &str,
    target: &str,
) {
    let entry = format!(
        r#"<Relationship Id="{}" Type="{}" Target="{}"/>"#,
        rid, rel_type, target
    );
    if let Some(data) = files.get(rels_path) {
        let mut content = String::from_utf8_lossy(data).to_string();
        if let Some(pos) = content.find("</Relationships>") {
            content.insert_str(pos, &entry);
        }
        files.insert(rels_path.to_string(), content.into_bytes());
    } else {
        let content = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  {}
</Relationships>"#,
            entry
        );
        files.insert(rels_path.to_string(), content.into_bytes());
    }
}

fn pptx_update_content_types_for_chart(files: &mut HashMap<String, Vec<u8>>, chart_num: u32) {
    if let Some(ct_data) = files.get("[Content_Types].xml") {
        let mut content = String::from_utf8_lossy(ct_data).to_string();
        let entry = format!(
            r#"<Override PartName="/ppt/charts/chart{}.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>"#,
            chart_num,
        );
        if let Some(pos) = content.find("</Types>") {
            content.insert_str(pos, &entry);
        }
        files.insert("[Content_Types].xml".to_string(), content.into_bytes());
    }
}

// ---------------------------------------------------------------------------
// Chart helpers: extract chart blocks from runs, render to native formats
// ---------------------------------------------------------------------------

fn is_chart_language(lang: &Option<String>) -> bool {
    lang.as_deref()
        .is_some_and(|l| l == "nivo" || l == "plotly")
}

/// Collect code-block text for a chart block, advancing `i` past the block.
fn collect_chart_code_text(runs: &[FormattedRun], i: &mut usize) -> String {
    let mut code_text = String::new();
    while *i < runs.len() {
        let r = &runs[*i];
        if !matches!(r.block_type, BlockType::CodeBlock { .. }) && r.text != "\n" {
            break;
        }
        if r.text == "\n" && !matches!(r.block_type, BlockType::CodeBlock { .. }) {
            *i += 1;
            break;
        }
        code_text.push_str(&r.text);
        *i += 1;
    }
    code_text
}

/// Extract all chart blocks from runs, returning (index, OfficeChartData) pairs.
fn extract_charts_from_runs(runs: &[FormattedRun]) -> Vec<OfficeChartData> {
    let mut charts = Vec::new();
    let mut i = 0;
    while i < runs.len() {
        if let BlockType::CodeBlock { ref language } = runs[i].block_type {
            if is_chart_language(language) {
                let mut j = i;
                let code_text = collect_chart_code_text(runs, &mut j);
                if let Some(input) = parse_chart_block(&code_text) {
                    if let Some(office) = chart_input_to_office_data(&input) {
                        charts.push(office);
                    }
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    charts
}

/// Build DrawingML chart XML from OfficeChartData.
fn build_office_chart_xml(data: &OfficeChartData) -> String {
    let count = data.categories.len();

    let mut cat_xml = String::new();
    for (i, cat) in data.categories.iter().enumerate() {
        cat_xml.push_str(&format!(
            r#"<c:pt idx="{}"><c:v>{}</c:v></c:pt>"#,
            i,
            xml_escape(cat)
        ));
    }

    let mut series_xml = String::new();
    for (si, series) in data.series.iter().enumerate() {
        let color = data.colors.get(si).map(|s| s.as_str()).unwrap_or("FF4343");

        let mut val_xml = String::new();
        for (i, val) in series.values.iter().enumerate() {
            val_xml.push_str(&format!(r#"<c:pt idx="{}"><c:v>{}</c:v></c:pt>"#, i, val));
        }

        let fill_section = if data.chart_type == ChartType::Pie {
            let mut dpt = String::new();
            for (i, _) in series.values.iter().enumerate() {
                let pt_color = data.colors.get(i).map(|s| s.as_str()).unwrap_or("CCCCCC");
                dpt.push_str(&format!(
                    r#"<c:dPt><c:idx val="{i}"/><c:spPr><a:solidFill><a:srgbClr val="{c}"/></a:solidFill></c:spPr></c:dPt>"#,
                    i = i,
                    c = pt_color,
                ));
            }
            dpt
        } else if data.chart_type == ChartType::Line {
            format!(
                r#"<c:spPr><a:ln w="28575"><a:solidFill><a:srgbClr val="{c}"/></a:solidFill></a:ln></c:spPr>"#,
                c = color
            )
        } else {
            format!(
                r#"<c:spPr><a:solidFill><a:srgbClr val="{c}"/></a:solidFill></c:spPr>"#,
                c = color
            )
        };

        series_xml.push_str(&format!(
            r#"<c:ser>
    <c:idx val="{si}"/><c:order val="{si}"/>
    <c:tx><c:v>{sn}</c:v></c:tx>
    {fill}
    <c:cat><c:strRef><c:strCache><c:ptCount val="{cnt}"/>{cats}</c:strCache></c:strRef></c:cat>
    <c:val><c:numRef><c:numCache><c:ptCount val="{cnt}"/>{vals}</c:numCache></c:numRef></c:val>
  </c:ser>"#,
            si = si,
            sn = xml_escape(&series.name),
            fill = fill_section,
            cnt = count,
            cats = cat_xml,
            vals = val_xml,
        ));
    }

    let axes_xml = r#"<c:axId val="111111111"/><c:axId val="222222222"/>"#;
    let axis_defs = r#"<c:catAx>
      <c:axId val="111111111"/><c:scaling><c:orientation val="minMax"/></c:scaling>
      <c:delete val="0"/><c:axPos val="b"/><c:crossAx val="222222222"/>
    </c:catAx>
    <c:valAx>
      <c:axId val="222222222"/><c:scaling><c:orientation val="minMax"/></c:scaling>
      <c:delete val="0"/><c:axPos val="l"/><c:crossAx val="111111111"/>
    </c:valAx>"#;

    let grouping = if data.stacked { "stacked" } else { "clustered" };
    let chart_element = match data.chart_type {
        ChartType::Pie => format!(
            r#"<c:pieChart><c:varyColors val="1"/>{}</c:pieChart>"#,
            series_xml
        ),
        ChartType::Line => format!(
            r#"<c:lineChart><c:grouping val="standard"/>{}{}</c:lineChart>{}"#,
            series_xml, axes_xml, axis_defs
        ),
        _ => format!(
            r#"<c:barChart><c:barDir val="col"/><c:grouping val="{g}"/>{}{}</c:barChart>{}"#,
            series_xml,
            axes_xml,
            axis_defs,
            g = grouping,
        ),
    };

    let title_xml = if let Some(ref title) = data.title {
        format!(
            r#"<c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:t>{}</a:t></a:r></a:p></c:rich></c:tx></c:title>"#,
            xml_escape(title)
        )
    } else {
        String::new()
    };

    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"
  xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <c:chart>
    {title}
    <c:plotArea>
      <c:layout/>
      {chart}
    </c:plotArea>
    <c:legend><c:legendPos val="b"/></c:legend>
  </c:chart>
</c:chartSpace>"#,
        title = title_xml,
        chart = chart_element,
    )
}

/// Embed all chart code blocks from markdown as native PPTX charts on a slide.
fn pptx_embed_charts_from_markdown(
    files: &mut HashMap<String, Vec<u8>>,
    slide_num: u32,
    markdown: &str,
    x_cm: f32,
    y_cm: f32,
    w_cm: f32,
    h_cm: f32,
) -> usize {
    let runs = markdown_to_runs(markdown, OpenXmlFormat::Pptx);
    let charts = extract_charts_from_runs(&runs);
    let chart_count = charts.len();

    // Stack charts vertically, dividing available height equally
    let per_chart_h = if chart_count > 0 {
        h_cm / chart_count as f32
    } else {
        h_cm
    };

    for (ci, chart_data) in charts.iter().enumerate() {
        if !chart_data.chart_type.supported_in_office() {
            continue;
        }

        let chart_num = pptx_next_chart_number(files);
        let chart_path = format!("ppt/charts/chart{}.xml", chart_num);
        let chart_rels_path = format!("ppt/charts/_rels/chart{}.xml.rels", chart_num);

        let chart_xml = build_office_chart_xml(chart_data);
        files.insert(chart_path, chart_xml.into_bytes());
        files.insert(
            chart_rels_path,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#
                .as_bytes()
                .to_vec(),
        );

        let slide_path = format!("ppt/slides/slide{}.xml", slide_num);
        let slide_rels_path = format!("ppt/slides/_rels/slide{}.xml.rels", slide_num);

        let rid = pptx_next_rel_id(files, &slide_rels_path);
        pptx_add_relationship(
            files,
            &slide_rels_path,
            &rid,
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart",
            &format!("../charts/chart{}.xml", chart_num),
        );
        pptx_update_content_types_for_chart(files, chart_num);

        let slide_data = files.get(&slide_path).expect("slide exists").clone();
        let mut slide_xml = String::from_utf8_lossy(&slide_data).to_string();
        let next_id = max_id(&slide_xml) + 1;
        let cy = y_cm + ci as f32 * per_chart_h;

        let frame_xml = format!(
            r#"<p:graphicFrame>
  <p:nvGraphicFramePr>
    <p:cNvPr id="{id}" name="Chart {id}"/>
    <p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr>
    <p:nvPr/>
  </p:nvGraphicFramePr>
  <p:xfrm>
    <a:off x="{ox}" y="{oy}"/>
    <a:ext cx="{cx}" cy="{cy}"/>
  </p:xfrm>
  <a:graphic>
    <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">
      <c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:id="{rid}"/>
    </a:graphicData>
  </a:graphic>
</p:graphicFrame>"#,
            id = next_id,
            ox = cm_to_emu(x_cm),
            oy = cm_to_emu(cy),
            cx = cm_to_emu(w_cm),
            cy = cm_to_emu(per_chart_h),
            rid = rid,
        );

        if let Some(pos) = slide_xml.find("</p:spTree>") {
            slide_xml.insert_str(pos, &frame_xml);
        }
        files.insert(slide_path, slide_xml.into_bytes());
    }

    chart_count
}

/// Build a DOCX chart `w:drawing` element that references an embedded chart part.
fn docx_chart_drawing_xml(rid: &str, w_emu: i64, h_emu: i64, id: u32) -> String {
    format!(
        r#"<w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">
  <wp:extent cx="{w}" cy="{h}"/>
  <wp:effectExtent l="0" t="0" r="0" b="0"/>
  <wp:docPr id="{id}" name="Chart {id}"/>
  <wp:cNvGraphicFramePr/>
  <a:graphic>
    <a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart">
      <c:chart r:id="{rid}"/>
    </a:graphicData>
  </a:graphic>
</wp:inline></w:drawing></w:r></w:p>"#,
        w = w_emu,
        h = h_emu,
        id = id,
        rid = rid,
    )
}

/// Embed all chart code blocks from markdown as native DOCX charts.
/// Replaces `[Chart: ...]` placeholder paragraphs in `paragraphs_xml` with
/// inline chart drawing elements. Mutates `files` with chart parts.
fn docx_embed_charts_from_markdown(
    files: &mut HashMap<String, Vec<u8>>,
    markdown: &str,
    paragraphs_xml: &str,
) -> String {
    let runs = markdown_to_runs(markdown, OpenXmlFormat::Docx);
    let charts = extract_charts_from_runs(&runs);
    let mut result = paragraphs_xml.to_string();

    for chart_data in &charts {
        if !chart_data.chart_type.supported_in_office() {
            continue;
        }

        // Determine next chart number
        let chart_num = {
            let mut max = 0u32;
            for key in files.keys() {
                if let Some(rest) = key.strip_prefix("word/charts/chart") {
                    if let Some(num_str) = rest.strip_suffix(".xml") {
                        if let Ok(n) = num_str.parse::<u32>() {
                            max = max.max(n);
                        }
                    }
                }
            }
            max + 1
        };

        let chart_path = format!("word/charts/chart{}.xml", chart_num);
        let chart_xml = build_office_chart_xml(chart_data);
        files.insert(chart_path.clone(), chart_xml.into_bytes());

        // Add chart rels
        let chart_rels = format!("word/charts/_rels/chart{}.xml.rels", chart_num);
        files.insert(
            chart_rels,
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#
                .as_bytes()
                .to_vec(),
        );

        // Add relationship in word/_rels/document.xml.rels
        let doc_rels_path = "word/_rels/document.xml.rels";
        let rid = {
            let mut max = 0u32;
            if let Some(data) = files.get(doc_rels_path) {
                let content = String::from_utf8_lossy(data);
                for cap in content.match_indices("rId") {
                    let rest = &content[cap.0 + 3..];
                    if let Some(end) = rest.find('"') {
                        if let Ok(n) = rest[..end].parse::<u32>() {
                            max = max.max(n);
                        }
                    }
                }
            }
            format!("rId{}", max + 1)
        };

        let rel_entry = format!(
            r#"<Relationship Id="{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="charts/chart{}.xml"/>"#,
            rid, chart_num,
        );
        if let Some(data) = files.get(doc_rels_path) {
            let mut content = String::from_utf8_lossy(data).to_string();
            if let Some(pos) = content.find("</Relationships>") {
                content.insert_str(pos, &rel_entry);
            }
            files.insert(doc_rels_path.to_string(), content.into_bytes());
        }

        // Add content type
        if let Some(ct_data) = files.get("[Content_Types].xml") {
            let mut content = String::from_utf8_lossy(ct_data).to_string();
            let entry = format!(
                r#"<Override PartName="/word/charts/chart{}.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>"#,
                chart_num,
            );
            if let Some(pos) = content.find("</Types>") {
                content.insert_str(pos, &entry);
            }
            files.insert("[Content_Types].xml".to_string(), content.into_bytes());
        }

        // Generate w:drawing XML and replace placeholder paragraph
        let w_emu = cm_to_emu(15.0);
        let h_emu = cm_to_emu(8.0);
        let drawing = docx_chart_drawing_xml(&rid, w_emu, h_emu, chart_num);

        let title = chart_data.title.as_deref().unwrap_or("Chart");
        let placeholder = format!("[Chart: {}]", title);
        if let Some(pos) = result.find(&placeholder) {
            // Find enclosing <w:p> and </w:p>
            if let Some(p_start) = result[..pos]
                .rfind("<w:p>")
                .or_else(|| result[..pos].rfind("<w:p "))
            {
                if let Some(end_offset) = result[pos..].find("</w:p>") {
                    let p_end = pos + end_offset + 6;
                    result.replace_range(p_start..p_end, &drawing);
                    continue;
                }
            }
        }
        // Fallback: append at end if placeholder not found
        result.push_str(&drawing);
    }

    result
}

/// Render chart data directly into PDF content stream operators.
fn pdf_render_chart(data: &OfficeChartData, y: &mut f64, page_width: f64) -> String {
    let mut ops = String::new();
    let left_margin = 50.0;
    let chart_w = page_width - 100.0;
    let chart_h = 140.0;
    let bottom_margin = 40.0;

    if *y < bottom_margin + chart_h + 40.0 {
        return ops;
    }

    let colors: Vec<(f32, f32, f32)> = data
        .colors
        .iter()
        .map(|c| {
            let hex = c.trim_start_matches('#');
            let r = u8::from_str_radix(&hex[..2.min(hex.len())], 16).unwrap_or(200) as f32 / 255.0;
            let g = u8::from_str_radix(&hex[2..4.min(hex.len())], 16).unwrap_or(200) as f32 / 255.0;
            let b = u8::from_str_radix(&hex[4..6.min(hex.len())], 16).unwrap_or(200) as f32 / 255.0;
            (r, g, b)
        })
        .collect();

    // Title
    if let Some(ref title) = data.title {
        let escaped = title
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        ops.push_str(&format!(
            "BT /F2 12 Tf 0.067 0.067 0.067 rg {} {} Td ({}) Tj ET\n",
            left_margin, *y, escaped,
        ));
        *y -= 20.0;
    }

    let chart_top = *y;
    let chart_bottom = chart_top - chart_h;

    // Light background
    ops.push_str(&format!(
        "q 0.973 0.976 0.98 rg {} {} {} {} re f Q\n",
        left_margin, chart_bottom, chart_w, chart_h,
    ));

    match data.chart_type {
        ChartType::Bar => {
            let max_val = data
                .series
                .iter()
                .flat_map(|s| &s.values)
                .copied()
                .fold(0.0_f64, f64::max)
                .max(1.0);

            let n_cats = data.categories.len().max(1);
            let n_series = data.series.len().max(1);
            let group_w = chart_w / n_cats as f64;
            let bar_w = (group_w * 0.7) / n_series as f64;
            let group_pad = group_w * 0.15;

            for (ci, _cat) in data.categories.iter().enumerate() {
                for (si, series) in data.series.iter().enumerate() {
                    let val = series.values.get(ci).copied().unwrap_or(0.0);
                    let bar_h = (val / max_val) * (chart_h - 20.0);
                    let bx = left_margin + ci as f64 * group_w + group_pad + si as f64 * bar_w;
                    let by = chart_bottom;
                    let (r, g, b) = colors.get(si).copied().unwrap_or((0.8, 0.2, 0.2));
                    ops.push_str(&format!(
                        "q {} {} {} rg {} {} {} {} re f Q\n",
                        r, g, b, bx, by, bar_w, bar_h,
                    ));
                }
            }

            // Category labels
            let label_y = chart_bottom - 18.0;
            for (ci, cat) in data.categories.iter().enumerate() {
                let lx = left_margin + ci as f64 * group_w + group_pad;
                let escaped = cat
                    .replace('\\', "\\\\")
                    .replace('(', "\\(")
                    .replace(')', "\\)");
                ops.push_str(&format!(
                    "BT /F1 8 Tf 0.4 0.4 0.4 rg {} {} Td ({}) Tj ET\n",
                    lx, label_y, escaped,
                ));
            }
        }

        ChartType::Line => {
            let max_val = data
                .series
                .iter()
                .flat_map(|s| &s.values)
                .copied()
                .fold(0.0_f64, f64::max)
                .max(1.0);

            let n_cats = data.categories.len().max(1);
            let step = chart_w / (n_cats.max(2) - 1).max(1) as f64;

            for (si, series) in data.series.iter().enumerate() {
                let (r, g, b) = colors.get(si).copied().unwrap_or((0.8, 0.2, 0.2));
                ops.push_str(&format!("q {} {} {} RG 1.5 w\n", r, g, b,));
                for (pi, val) in series.values.iter().enumerate() {
                    let px = left_margin + pi as f64 * step;
                    let py = chart_bottom + (val / max_val) * (chart_h - 20.0);
                    if pi == 0 {
                        ops.push_str(&format!("{} {} m\n", px, py));
                    } else {
                        ops.push_str(&format!("{} {} l\n", px, py));
                    }
                }
                ops.push_str("S Q\n");

                // Draw data points
                for (pi, val) in series.values.iter().enumerate() {
                    let px = left_margin + pi as f64 * step;
                    let py = chart_bottom + (val / max_val) * (chart_h - 20.0);
                    ops.push_str(&format!(
                        "q {} {} {} rg {} {} 3 0 360 arc f Q\n",
                        r, g, b, px, py,
                    ));
                    // Fallback: use a small square since arc isn't a PDF operator
                    ops.push_str(&format!(
                        "q {} {} {} rg {} {} 4 4 re f Q\n",
                        r,
                        g,
                        b,
                        px - 2.0,
                        py - 2.0,
                    ));
                }
            }

            // Category labels
            let label_y = chart_bottom - 18.0;
            let step = chart_w / (n_cats.max(2) - 1).max(1) as f64;
            for (ci, cat) in data.categories.iter().enumerate() {
                let lx = left_margin + ci as f64 * step;
                let escaped = cat
                    .replace('\\', "\\\\")
                    .replace('(', "\\(")
                    .replace(')', "\\)");
                ops.push_str(&format!(
                    "BT /F1 7 Tf 0.4 0.4 0.4 rg {} {} Td ({}) Tj ET\n",
                    lx, label_y, escaped,
                ));
            }
        }

        ChartType::Pie => {
            let total: f64 = data
                .series
                .first()
                .map(|s| s.values.iter().sum())
                .unwrap_or(1.0);
            let cx = left_margin + chart_w / 2.0;
            let cy = chart_bottom + chart_h / 2.0;
            let radius = (chart_h / 2.0 - 10.0).min(chart_w / 4.0);

            // PDF doesn't have native arc operations, so draw pie segments as filled triangular fan
            let values = data
                .series
                .first()
                .map(|s| s.values.as_slice())
                .unwrap_or(&[]);
            let mut start_angle: f64 = 0.0;

            for (vi, val) in values.iter().enumerate() {
                let sweep = (*val / total.max(0.001)) * 360.0;
                let (r, g, b) = colors.get(vi).copied().unwrap_or((0.8, 0.8, 0.8));

                // Approximate arc with line segments
                ops.push_str(&format!("q {} {} {} rg\n", r, g, b));
                ops.push_str(&format!("{} {} m\n", cx, cy));

                let segments = 20;
                for seg in 0..=segments {
                    let angle = (start_angle + sweep * seg as f64 / segments as f64).to_radians();
                    let px = cx + radius * angle.cos();
                    let py = cy + radius * angle.sin();
                    ops.push_str(&format!("{} {} l\n", px, py));
                }
                ops.push_str("f Q\n");

                // Legend label
                let mid_angle = (start_angle + sweep / 2.0).to_radians();
                let lx = cx + (radius + 15.0) * mid_angle.cos();
                let ly = cy + (radius + 15.0) * mid_angle.sin();
                if let Some(cat) = data.categories.get(vi) {
                    let escaped = cat
                        .replace('\\', "\\\\")
                        .replace('(', "\\(")
                        .replace(')', "\\)");
                    ops.push_str(&format!(
                        "BT /F1 7 Tf {} {} {} rg {} {} Td ({}) Tj ET\n",
                        r, g, b, lx, ly, escaped,
                    ));
                }

                start_angle += sweep;
            }
        }

        _ => {
            // Unsupported chart type → render as text placeholder
            let escaped = format!("[Chart: {} not supported in PDF]", data.chart_type);
            ops.push_str(&format!(
                "BT /F1 10 Tf 0.4 0.4 0.4 rg {} {} Td ({}) Tj ET\n",
                left_margin,
                chart_bottom + chart_h / 2.0,
                escaped,
            ));
        }
    }

    *y = chart_bottom - 30.0;
    ops
}

// ---------------------------------------------------------------------------
// PDF helpers
// ---------------------------------------------------------------------------

fn create_simple_pdf() -> Vec<u8> {
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.7");

    let pages_id = doc.new_object_id();
    let font_id = doc.new_object_id();
    let font_bold_id = doc.new_object_id();
    let resources_id = doc.new_object_id();

    doc.objects.insert(
        font_id,
        Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        }),
    );

    doc.objects.insert(
        font_bold_id,
        Object::Dictionary(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica-Bold",
        }),
    );

    doc.objects.insert(
        resources_id,
        Object::Dictionary(dictionary! {
            "Font" => dictionary! {
                "F1" => Object::Reference(font_id),
                "F2" => Object::Reference(font_bold_id),
            },
        }),
    );

    let page_contents = [
        // Page 1: Title page
        concat!(
            "q 1 0.263 0.263 rg 0 692 612 100 re f Q\n",
            "BT /F2 36 Tf 1 1 1 rg 50 732 Td (FLOW LIKE) Tj ET\n",
            "BT /F1 14 Tf 1 1 1 rg 50 708 Td (Document Generation Platform) Tj ET\n",
            "q 0.067 0.067 0.067 rg 50 640 512 1 re f Q\n",
            "BT /F2 20 Tf 0.067 0.067 0.067 rg 50 600 Td (Capability Overview) Tj ET\n",
            "BT /F1 11 Tf 0.416 0.416 0.416 rg 50 575 Td (Q4 2024  |  Version 1.0  |  Confidential) Tj ET\n",
            "BT /F1 11 Tf 0.102 0.102 0.102 rg 50 500 Td (Flow Like is a next-generation document automation platform that enables) Tj ET\n",
            "BT /F1 11 Tf 0.102 0.102 0.102 rg 50 485 Td (teams to programmatically generate, transform, and manage documents at) Tj ET\n",
            "BT /F1 11 Tf 0.102 0.102 0.102 rg 50 470 Td (scale. With 50+ automation nodes spanning four document formats, the) Tj ET\n",
            "BT /F1 11 Tf 0.102 0.102 0.102 rg 50 455 Td (platform eliminates manual workflows and reduces production time by 90%.) Tj ET\n",
            "q 1 0.263 0.263 rg 50 400 160 40 re f Q\n",
            "q 0.231 0.510 0.965 rg 220 400 160 40 re f Q\n",
            "q 0.063 0.725 0.506 rg 390 400 160 40 re f Q\n",
            "BT /F2 12 Tf 1 1 1 rg 88 416 Td (DOCX  10) Tj ET\n",
            "BT /F2 12 Tf 1 1 1 rg 258 416 Td (PPTX  12) Tj ET\n",
            "BT /F2 12 Tf 1 1 1 rg 432 416 Td (PDF  18) Tj ET\n",
            "q 1 0.263 0.263 rg 0 0 612 4 re f Q\n",
        ),
        // Page 2: Capabilities
        concat!(
            "q 1 0.263 0.263 rg 0 780 612 12 re f Q\n",
            "BT /F2 22 Tf 0.067 0.067 0.067 rg 50 730 Td (Core Capabilities) Tj ET\n",
            "q 0.067 0.067 0.067 rg 50 722 200 1 re f Q\n",
            "BT /F2 13 Tf 1 0.263 0.263 rg 50 690 Td (Document Generation) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 673 Td (Create DOCX, PPTX, and PDF documents from scratch with full theming) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 659 Td (support. Every document starts from a branded template with custom fonts,) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 645 Td (colors, and styles pre-configured.) Tj ET\n",
            "BT /F2 13 Tf 0.231 0.510 0.965 rg 50 615 Td (Content Automation) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 598 Td (Insert paragraphs, tables, charts, shapes, and text boxes through composable) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 584 Td (node pipelines. Supports multi-series charts, styled tables with alternating) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 570 Td (rows, and geometric shapes with text overlays.) Tj ET\n",
            "BT /F2 13 Tf 0.063 0.725 0.506 rg 50 540 Td (PDF Operations) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 523 Td (Merge, split, encrypt, watermark, and add page numbers to PDF files.) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 509 Td (Digital signature support and metadata extraction for enterprise workflows.) Tj ET\n",
            "q 0.976 0.976 0.976 rg 50 440 512 50 re f Q\n",
            "q 1 0.263 0.263 rg 50 440 4 50 re f Q\n",
            "BT /F1 10 Tf 0.416 0.416 0.416 rg 64 462 Td (\"Build the future of documents, one node at a time.\") Tj ET\n",
            "q 0.067 0.067 0.067 rg 50 380 512 1 re f Q\n",
            "BT /F2 14 Tf 0.067 0.067 0.067 rg 50 350 Td (Performance Benchmarks) Tj ET\n",
            "q 1 0.263 0.263 rg 50 310 512 25 re f Q\n",
            "BT /F2 9 Tf 1 1 1 rg 60 318 Td (Operation) Tj ET\n",
            "BT /F2 9 Tf 1 1 1 rg 250 318 Td (Latency) Tj ET\n",
            "BT /F2 9 Tf 1 1 1 rg 400 318 Td (Status) Tj ET\n",
            "BT /F1 9 Tf 0.102 0.102 0.102 rg 60 293 Td (DOCX Generation) Tj ET\n",
            "BT /F1 9 Tf 0.102 0.102 0.102 rg 250 293 Td (< 50ms) Tj ET\n",
            "BT /F1 9 Tf 0.063 0.725 0.506 rg 400 293 Td (Exceeds) Tj ET\n",
            "q 0.976 0.976 0.976 rg 50 275 512 18 re f Q\n",
            "BT /F1 9 Tf 0.102 0.102 0.102 rg 60 280 Td (PPTX with Charts) Tj ET\n",
            "BT /F1 9 Tf 0.102 0.102 0.102 rg 250 280 Td (< 120ms) Tj ET\n",
            "BT /F1 9 Tf 0.063 0.725 0.506 rg 400 280 Td (Exceeds) Tj ET\n",
            "BT /F1 9 Tf 0.102 0.102 0.102 rg 60 263 Td (PDF Merge \\(10 files\\)) Tj ET\n",
            "BT /F1 9 Tf 0.102 0.102 0.102 rg 250 263 Td (< 200ms) Tj ET\n",
            "BT /F1 9 Tf 0.063 0.725 0.506 rg 400 263 Td (Exceeds) Tj ET\n",
            "q 0.976 0.976 0.976 rg 50 245 512 18 re f Q\n",
            "BT /F1 9 Tf 0.102 0.102 0.102 rg 60 250 Td (PDF Watermark) Tj ET\n",
            "BT /F1 9 Tf 0.102 0.102 0.102 rg 250 250 Td (< 80ms) Tj ET\n",
            "BT /F1 9 Tf 0.063 0.725 0.506 rg 400 250 Td (Exceeds) Tj ET\n",
            "q 1 0.263 0.263 rg 0 0 612 4 re f Q\n",
        ),
        // Page 3: Architecture
        concat!(
            "q 1 0.263 0.263 rg 0 780 612 12 re f Q\n",
            "BT /F2 22 Tf 0.067 0.067 0.067 rg 50 730 Td (Architecture) Tj ET\n",
            "q 0.067 0.067 0.067 rg 50 722 200 1 re f Q\n",
            "BT /F2 13 Tf 0.067 0.067 0.067 rg 50 690 Td (Node-Based Pipeline Design) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 673 Td (Flow Like uses a directed acyclic graph \\(DAG\\) execution model where each) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 659 Td (node represents an atomic document operation. Nodes are composed into) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 645 Td (pipelines that transform documents through multiple stages.) Tj ET\n",
            "BT /F2 13 Tf 0.067 0.067 0.067 rg 50 615 Td (Supported Operations) Tj ET\n",
            "q 0.976 0.976 0.976 rg 60 548 230 55 re f Q\n",
            "q 1 0.263 0.263 rg 60 548 3 55 re f Q\n",
            "BT /F2 10 Tf 0.067 0.067 0.067 rg 72 588 Td (DOCX Operations) Tj ET\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 72 574 Td (Create, Add Paragraph,) Tj ET\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 72 562 Td (Add Table, Replace Text) Tj ET\n",
            "q 0.976 0.976 0.976 rg 310 548 230 55 re f Q\n",
            "q 0.231 0.510 0.965 rg 310 548 3 55 re f Q\n",
            "BT /F2 10 Tf 0.067 0.067 0.067 rg 322 588 Td (PPTX Operations) Tj ET\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 322 574 Td (Create, Add Slide, Text Box,) Tj ET\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 322 562 Td (Shape, Table, Chart) Tj ET\n",
            "q 0.976 0.976 0.976 rg 60 480 230 55 re f Q\n",
            "q 0.063 0.725 0.506 rg 60 480 3 55 re f Q\n",
            "BT /F2 10 Tf 0.067 0.067 0.067 rg 72 520 Td (PDF Operations) Tj ET\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 72 506 Td (Merge, Split, Encrypt, Sign,) Tj ET\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 72 494 Td (Watermark, Page Numbers) Tj ET\n",
            "q 0.976 0.976 0.976 rg 310 480 230 55 re f Q\n",
            "q 0.961 0.620 0.043 rg 310 480 3 55 re f Q\n",
            "BT /F2 10 Tf 0.067 0.067 0.067 rg 322 520 Td (Image Operations) Tj ET\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 322 506 Td (Convert, Resize, Crop,) Tj ET\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 322 494 Td (Rotate, Thumbnail) Tj ET\n",
            "q 0.067 0.067 0.067 rg 50 430 512 1 re f Q\n",
            "BT /F1 10 Tf 0.416 0.416 0.416 rg 50 400 Td (flow-like.com  |  github.com/Rheosoph/flow-like) Tj ET\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 50 385 Td (Copyright 2024 Flow Like. All rights reserved.) Tj ET\n",
            "q 1 0.263 0.263 rg 0 0 612 4 re f Q\n",
        ),
        // Page 4: Use cases
        concat!(
            "q 1 0.263 0.263 rg 0 780 612 12 re f Q\n",
            "BT /F2 22 Tf 0.067 0.067 0.067 rg 50 730 Td (Use Cases) Tj ET\n",
            "q 0.067 0.067 0.067 rg 50 722 200 1 re f Q\n",
            "BT /F2 13 Tf 1 0.263 0.263 rg 50 690 Td (Enterprise Reporting) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 673 Td (Automatically generate quarterly reports with live data, charts, and) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 659 Td (branded formatting. Reduce report generation time from hours to seconds.) Tj ET\n",
            "BT /F2 13 Tf 0.231 0.510 0.965 rg 50 630 Td (Proposal Automation) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 613 Td (Create client proposals with dynamic pricing tables, custom cover pages,) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 599 Td (and tailored content. Scale from 10 to 10,000 proposals per month.) Tj ET\n",
            "BT /F2 13 Tf 0.063 0.725 0.506 rg 50 570 Td (Compliance Documents) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 553 Td (Generate audit-ready documents with watermarks, encryption, and digital) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 539 Td (signatures. Ensure every document meets regulatory requirements.) Tj ET\n",
            "BT /F2 13 Tf 0.961 0.620 0.043 rg 50 510 Td (Training Materials) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 493 Td (Build presentations and handouts for onboarding and training programs.) Tj ET\n",
            "BT /F1 10 Tf 0.102 0.102 0.102 rg 50 479 Td (Consistent branding across all materials with zero manual formatting.) Tj ET\n",
            "q 0.067 0.067 0.067 rg 50 430 512 1 re f Q\n",
            "q 0.976 0.976 0.976 rg 50 370 512 50 re f Q\n",
            "q 1 0.263 0.263 rg 50 370 4 50 re f Q\n",
            "BT /F1 10 Tf 0.416 0.416 0.416 rg 64 392 Td (\"Automate what you repeat. Create what matters.\") Tj ET\n",
            "q 1 0.263 0.263 rg 0 0 612 4 re f Q\n",
        ),
        // Page 5: Summary
        concat!(
            "q 1 0.263 0.263 rg 0 692 612 100 re f Q\n",
            "BT /F2 30 Tf 1 1 1 rg 50 732 Td (Thank You) Tj ET\n",
            "BT /F1 14 Tf 1 1 1 rg 50 708 Td (The future of document automation starts here.) Tj ET\n",
            "q 0.067 0.067 0.067 rg 50 640 512 1 re f Q\n",
            "BT /F2 16 Tf 0.067 0.067 0.067 rg 50 600 Td (Get Started) Tj ET\n",
            "BT /F1 11 Tf 0.102 0.102 0.102 rg 50 578 Td (Visit flow-like.com to explore the platform.) Tj ET\n",
            "BT /F1 11 Tf 0.102 0.102 0.102 rg 50 560 Td (Review the source at github.com/Rheosoph/flow-like.) Tj ET\n",
            "BT /F2 16 Tf 0.067 0.067 0.067 rg 50 520 Td (Key Numbers) Tj ET\n",
            "q 1 0.263 0.263 rg 50 475 100 35 re f Q\n",
            "BT /F2 14 Tf 1 1 1 rg 70 488 Td (50+) Tj ET\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 55 460 Td (Nodes) Tj ET\n",
            "q 0.231 0.510 0.965 rg 180 475 100 35 re f Q\n",
            "BT /F2 14 Tf 1 1 1 rg 206 488 Td (4) Tj ET\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 185 460 Td (Formats) Tj ET\n",
            "q 0.063 0.725 0.506 rg 310 475 100 35 re f Q\n",
            "BT /F2 14 Tf 1 1 1 rg 333 488 Td (90%) Tj ET\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 315 460 Td (Time Saved) Tj ET\n",
            "q 0.961 0.620 0.043 rg 440 475 100 35 re f Q\n",
            "BT /F2 14 Tf 1 1 1 rg 462 488 Td (<1s) Tj ET\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 445 460 Td (Avg Latency) Tj ET\n",
            "q 0.067 0.067 0.067 rg 50 410 512 1 re f Q\n",
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 50 385 Td (Copyright 2024 Flow Like. All rights reserved.) Tj ET\n",
            "q 1 0.263 0.263 rg 0 0 612 4 re f Q\n",
        ),
    ];

    let mut page_ids = Vec::new();
    for content_str in &page_contents {
        let content_bytes = content_str.as_bytes().to_vec();
        let stream = Stream::new(dictionary! {}, content_bytes);
        let content_id = doc.add_object(stream);

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => Object::Reference(pages_id),
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => Object::Reference(resources_id),
            "Contents" => Object::Reference(content_id),
        });
        page_ids.push(page_id);
    }

    let kids: Vec<Object> = page_ids.iter().map(|id| Object::Reference(*id)).collect();
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => Object::Integer(page_ids.len() as i64),
        }),
    );

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => Object::Reference(pages_id),
    });

    doc.trailer.set("Root", Object::Reference(catalog_id));

    let mut buf = Vec::new();
    doc.save_to(&mut buf).expect("save PDF");
    buf
}

fn pdf_add_watermark(pdf_bytes: &[u8], text: &str, color: &str, opacity: f64) -> Vec<u8> {
    use lopdf::{Document, Object, Stream, dictionary};

    let (r, g, b) = styles::hex_to_rgb(color);
    let mut doc = Document::load_mem(pdf_bytes).expect("load PDF");
    let angle_rad = 45.0_f64 * std::f64::consts::PI / 180.0;
    let cos_a = angle_rad.cos();
    let sin_a = angle_rad.sin();

    let page_ids: Vec<lopdf::ObjectId> = doc.page_iter().collect();

    for page_id in &page_ids {
        let (width, height) = pdf_get_page_size(&doc, *page_id);
        let x = width / 2.0;
        let y = height / 2.0;
        let escaped_text = text
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");

        let content_str = format!(
            "q\n/GS0 gs\nBT\n{cos_a:.6} {sin_a:.6} -{sin_a:.6} {cos_a:.6} {x:.2} {y:.2} Tm\n/F1 60 Tf\n{r} {g} {b} rg\n({escaped_text}) Tj\nET\nQ\n"
        );

        let ext_gstate = dictionary! {
            "Type" => "ExtGState",
            "ca" => Object::Real(opacity as f32),
            "CA" => Object::Real(opacity as f32),
        };
        let gs_id = doc.add_object(ext_gstate);

        let font_dict = dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        };
        let font_id = doc.add_object(font_dict);

        let stream = Stream::new(dictionary! {}, content_str.into_bytes());
        let content_id = doc.add_object(stream);

        if let Ok(page) = doc.get_object_mut(*page_id) {
            if let Object::Dictionary(dict) = page {
                let resources = dict.get_mut(b"Resources");
                match resources {
                    Ok(Object::Dictionary(res)) => {
                        let ext_g = dictionary! { "GS0" => Object::Reference(gs_id) };
                        res.set("ExtGState", Object::Dictionary(ext_g));
                        let fonts = dictionary! { "F1" => Object::Reference(font_id) };
                        res.set("Font", Object::Dictionary(fonts));
                    }
                    _ => {
                        let res = dictionary! {
                            "ExtGState" => dictionary! { "GS0" => Object::Reference(gs_id) },
                            "Font" => dictionary! { "F1" => Object::Reference(font_id) },
                        };
                        dict.set("Resources", Object::Dictionary(res));
                    }
                }

                let existing_contents = dict.get(b"Contents").ok().cloned();
                match existing_contents {
                    Some(Object::Array(mut arr)) => {
                        arr.push(Object::Reference(content_id));
                        dict.set("Contents", Object::Array(arr));
                    }
                    Some(Object::Reference(existing_ref)) => {
                        dict.set(
                            "Contents",
                            Object::Array(vec![
                                Object::Reference(existing_ref),
                                Object::Reference(content_id),
                            ]),
                        );
                    }
                    _ => {
                        dict.set("Contents", Object::Reference(content_id));
                    }
                }
            }
        }
    }

    let mut buf = Vec::new();
    doc.save_to(&mut buf).expect("save PDF");
    buf
}

fn pdf_add_page_numbers(pdf_bytes: &[u8]) -> Vec<u8> {
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::load_mem(pdf_bytes).expect("load PDF");
    let page_ids: Vec<lopdf::ObjectId> = doc.page_iter().collect();
    let total = page_ids.len();

    for (idx, page_id) in page_ids.iter().enumerate() {
        let page_num = idx + 1;
        let text = format!("Page {} of {}", page_num, total);
        let escaped = text
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");

        let content_str = format!(
            "BT /F1 9 Tf 0.416 0.416 0.416 rg 280 30 Td ({}) Tj ET\n",
            escaped,
        );

        let font_dict = dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        };
        let font_id = doc.add_object(font_dict);

        let stream = Stream::new(dictionary! {}, content_str.into_bytes());
        let content_id = doc.add_object(stream);

        if let Ok(page) = doc.get_object_mut(*page_id) {
            if let Object::Dictionary(dict) = page {
                let resources = dict.get_mut(b"Resources");
                match resources {
                    Ok(Object::Dictionary(res)) => {
                        let fonts = dictionary! { "F1" => Object::Reference(font_id) };
                        res.set("Font", Object::Dictionary(fonts));
                    }
                    _ => {
                        let res = dictionary! {
                            "Font" => dictionary! { "F1" => Object::Reference(font_id) },
                        };
                        dict.set("Resources", Object::Dictionary(res));
                    }
                }

                let existing_contents = dict.get(b"Contents").ok().cloned();
                match existing_contents {
                    Some(Object::Array(mut arr)) => {
                        arr.push(Object::Reference(content_id));
                        dict.set("Contents", Object::Array(arr));
                    }
                    Some(Object::Reference(existing_ref)) => {
                        dict.set(
                            "Contents",
                            Object::Array(vec![
                                Object::Reference(existing_ref),
                                Object::Reference(content_id),
                            ]),
                        );
                    }
                    _ => {
                        dict.set("Contents", Object::Reference(content_id));
                    }
                }
            }
        }
    }

    let mut buf = Vec::new();
    doc.save_to(&mut buf).expect("save PDF");
    buf
}

fn pdf_get_page_size(doc: &lopdf::Document, page_id: lopdf::ObjectId) -> (f64, f64) {
    use lopdf::Object;
    if let Ok(page) = doc.get_object(page_id) {
        if let Object::Dictionary(dict) = page {
            if let Ok(Object::Array(media_box)) = dict.get(b"MediaBox") {
                if media_box.len() == 4 {
                    let w = match &media_box[2] {
                        Object::Integer(n) => *n as f64,
                        Object::Real(n) => *n as f64,
                        _ => 612.0,
                    };
                    let h = match &media_box[3] {
                        Object::Integer(n) => *n as f64,
                        Object::Real(n) => *n as f64,
                        _ => 792.0,
                    };
                    return (w, h);
                }
            }
        }
    }
    (612.0, 792.0)
}

// ---------------------------------------------------------------------------
// Shared utilities
// ---------------------------------------------------------------------------

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn max_id(xml: &str) -> u32 {
    let mut max = 0u32;
    for cap in xml.match_indices("id=\"") {
        let rest = &xml[cap.0 + 4..];
        if let Some(end) = rest.find('"') {
            if let Ok(n) = rest[..end].parse::<u32>() {
                max = max.max(n);
            }
        }
    }
    max
}

// ---------------------------------------------------------------------------
// PPTX XML constants (identical to create.rs)
// ---------------------------------------------------------------------------

const PPTX_CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>
  <Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>
  <Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>
  <Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
  <Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>
</Types>"#;

const PPTX_TOP_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
  <Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>
</Relationships>"#;

const PPTX_PRESENTATION: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
  xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldMasterIdLst>
    <p:sldMasterId id="2147483648" r:id="rId1"/>
  </p:sldMasterIdLst>
  <p:sldSz cx="12192000" cy="6858000"/>
  <p:notesSz cx="6858000" cy="9144000"/>
</p:presentation>"#;

const PPTX_PRESENTATION_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/>
</Relationships>"#;

const PPTX_SLIDE_MASTER: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
  xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:bg>
      <p:bgPr>
        <a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill>
        <a:effectLst/>
      </p:bgPr>
    </p:bg>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr/>
    </p:spTree>
  </p:cSld>
  <p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>
  <p:sldLayoutIdLst>
    <p:sldLayoutId id="2147483649" r:id="rId1"/>
  </p:sldLayoutIdLst>
</p:sldMaster>"#;

const PPTX_SLIDE_MASTER_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/>
</Relationships>"#;

const PPTX_SLIDE_LAYOUT: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
  xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"
  type="blank">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
      <p:grpSpPr/>
    </p:spTree>
  </p:cSld>
</p:sldLayout>"#;

const PPTX_SLIDE_LAYOUT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>
</Relationships>"#;

const PPTX_THEME: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Flow Like">
  <a:themeElements>
    <a:clrScheme name="Flow Like">
      <a:dk1><a:srgbClr val="111111"/></a:dk1>
      <a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>
      <a:dk2><a:srgbClr val="1A1A1A"/></a:dk2>
      <a:lt2><a:srgbClr val="F9FAFB"/></a:lt2>
      <a:accent1><a:srgbClr val="FF4343"/></a:accent1>
      <a:accent2><a:srgbClr val="3B82F6"/></a:accent2>
      <a:accent3><a:srgbClr val="10B981"/></a:accent3>
      <a:accent4><a:srgbClr val="F59E0B"/></a:accent4>
      <a:accent5><a:srgbClr val="8B5CF6"/></a:accent5>
      <a:accent6><a:srgbClr val="EC4899"/></a:accent6>
      <a:hlink><a:srgbClr val="FF4343"/></a:hlink>
      <a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
    </a:clrScheme>
    <a:fontScheme name="Flow Like">
      <a:majorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont>
      <a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont>
    </a:fontScheme>
    <a:fmtScheme name="Office">
      <a:fillStyleLst>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
      </a:fillStyleLst>
      <a:lnStyleLst>
        <a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>
        <a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>
        <a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln>
      </a:lnStyleLst>
      <a:effectStyleLst>
        <a:effectStyle><a:effectLst/></a:effectStyle>
        <a:effectStyle><a:effectLst/></a:effectStyle>
        <a:effectStyle><a:effectLst/></a:effectStyle>
      </a:effectStyleLst>
      <a:bgFillStyleLst>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
        <a:solidFill><a:schemeClr val="phClr"/></a:solidFill>
      </a:bgFillStyleLst>
    </a:fmtScheme>
  </a:themeElements>
</a:theme>"#;

const PPTX_APP_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">
  <Application>Flow Like</Application>
</Properties>"#;

const PPTX_CORE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties"
  xmlns:dc="http://purl.org/dc/elements/1.1/"
  xmlns:dcterms="http://purl.org/dc/terms/"
  xmlns:dcmitype="http://purl.org/dc/dcmitype/"
  xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <dc:title>Flow Like Presentation</dc:title>
  <dc:creator>Flow Like</dc:creator>
</cp:coreProperties>"#;

// ===========================================================================
// TESTS
// ===========================================================================

#[test]
fn generate_docx_with_content() {
    let docx_bytes = create_empty_docx(
        defaults::FONT_SANS,
        defaults::DOCX_FONT_SIZE,
        defaults::PRIMARY,
    );
    let mut files = read_zip(&docx_bytes).expect("read_zip");

    // ── Title Page ───────────────────────────────────────────────────────
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Flow Like",
            &ParagraphStyle::Title,
            &TextAlignment::Left,
            None,
            None,
            None,
            false,
            false,
        ),
    );
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Document Generation Platform",
            &ParagraphStyle::Subtitle,
            &TextAlignment::Left,
            None,
            None,
            None,
            false,
            false,
        ),
    );
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Capability Overview  \u{2022}  Q4 2024",
            &ParagraphStyle::Normal,
            &TextAlignment::Left,
            None,
            Some(12.0),
            Some(defaults::TEXT_MUTED),
            false,
            false,
        ),
    );

    // ── Executive Summary ────────────────────────────────────────────────
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Executive Summary",
            &ParagraphStyle::Heading1,
            &TextAlignment::Left,
            None,
            None,
            None,
            false,
            false,
        ),
    );
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Flow Like is a next-generation document automation platform that enables teams to \
             programmatically generate, transform, and manage documents at scale. With over 50 \
             purpose-built automation nodes spanning DOCX, PPTX, PDF, and image formats, Flow Like \
             eliminates manual document workflows and reduces production time by up to 90%.",
            &ParagraphStyle::Normal,
            &TextAlignment::Justify,
            None,
            None,
            None,
            false,
            false,
        ),
    );
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "The future of document workflows is automated, intelligent, and seamless. \
             Flow Like makes that future a reality today.",
            &ParagraphStyle::Quote,
            &TextAlignment::Left,
            None,
            None,
            None,
            false,
            false,
        ),
    );

    // ── Core Capabilities ────────────────────────────────────────────────
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Core Capabilities",
            &ParagraphStyle::Heading1,
            &TextAlignment::Left,
            None,
            None,
            None,
            false,
            false,
        ),
    );

    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Document Formats",
            &ParagraphStyle::Heading2,
            &TextAlignment::Left,
            None,
            None,
            None,
            false,
            false,
        ),
    );
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "The platform supports four primary document formats, each with dedicated creation, \
             manipulation, and transformation nodes. Documents are generated as valid OOXML \
             (Office Open XML) packages with full support for themes, styles, charts, and \
             embedded content.",
            &ParagraphStyle::Normal,
            &TextAlignment::Justify,
            None,
            None,
            None,
            false,
            false,
        ),
    );

    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Node Architecture",
            &ParagraphStyle::Heading2,
            &TextAlignment::Left,
            None,
            None,
            None,
            false,
            false,
        ),
    );
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Each node is a self-contained automation unit that accepts typed inputs and produces \
             deterministic outputs. Nodes can be composed into directed acyclic graphs (DAGs) to \
             create complex document workflows. The architecture supports parallel execution, \
             error recovery, and incremental updates.",
            &ParagraphStyle::Normal,
            &TextAlignment::Justify,
            None,
            None,
            None,
            false,
            false,
        ),
    );

    // ── Performance Metrics ──────────────────────────────────────────────
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Performance Metrics",
            &ParagraphStyle::Heading3,
            &TextAlignment::Left,
            None,
            None,
            None,
            false,
            false,
        ),
    );
    let table_data = vec![
        vec![
            "Metric".into(),
            "Value".into(),
            "Benchmark".into(),
            "Status".into(),
        ],
        vec![
            "DOCX Generation".into(),
            "< 50ms".into(),
            "200ms".into(),
            "\u{2705} Exceeds".into(),
        ],
        vec![
            "PPTX with Charts".into(),
            "< 120ms".into(),
            "500ms".into(),
            "\u{2705} Exceeds".into(),
        ],
        vec![
            "PDF Merge (10 files)".into(),
            "< 200ms".into(),
            "1000ms".into(),
            "\u{2705} Exceeds".into(),
        ],
        vec![
            "PDF Watermark".into(),
            "< 80ms".into(),
            "300ms".into(),
            "\u{2705} Exceeds".into(),
        ],
        vec![
            "Image Conversion".into(),
            "< 150ms".into(),
            "400ms".into(),
            "\u{2705} Exceeds".into(),
        ],
        vec![
            "Full Pipeline (6 nodes)".into(),
            "< 400ms".into(),
            "2000ms".into(),
            "\u{2705} Exceeds".into(),
        ],
    ];
    insert_before_sect_pr(
        &mut files,
        &build_table(
            &table_data,
            true,
            true,
            defaults::BORDER,
            defaults::DOCX_FONT_SIZE,
        ),
    );

    // ── Typography & Styling ─────────────────────────────────────────────
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Typography & Styling Showcase",
            &ParagraphStyle::Heading2,
            &TextAlignment::Left,
            None,
            None,
            None,
            false,
            false,
        ),
    );
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Bold text demonstrates emphasis for key concepts and important terms.",
            &ParagraphStyle::Normal,
            &TextAlignment::Left,
            None,
            None,
            None,
            true,
            false,
        ),
    );
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Italic text is used for technical terms, citations, and subtle emphasis.",
            &ParagraphStyle::Normal,
            &TextAlignment::Left,
            None,
            None,
            None,
            false,
            true,
        ),
    );
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Combined bold and italic for maximum emphasis on critical information.",
            &ParagraphStyle::Normal,
            &TextAlignment::Left,
            None,
            None,
            None,
            true,
            true,
        ),
    );
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Accent-colored text draws attention to brand-relevant content and calls to action.",
            &ParagraphStyle::Normal,
            &TextAlignment::Left,
            None,
            Some(13.0),
            Some(defaults::PRIMARY),
            false,
            false,
        ),
    );
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Centered alignment is ideal for section dividers and key statements.",
            &ParagraphStyle::Normal,
            &TextAlignment::Center,
            None,
            None,
            None,
            false,
            false,
        ),
    );
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Right-aligned text for dates, signatures, and supplementary notes.",
            &ParagraphStyle::Normal,
            &TextAlignment::Right,
            None,
            Some(11.0),
            Some(defaults::TEXT_MUTED),
            false,
            false,
        ),
    );

    // ── Accent Heading ───────────────────────────────────────────────────
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "What Makes Flow Like Different",
            &ParagraphStyle::Heading4,
            &TextAlignment::Left,
            None,
            None,
            None,
            false,
            false,
        ),
    );
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Unlike traditional template-based solutions, Flow Like generates documents from first \
             principles using a composable node architecture. Every element\u{2014}from paragraph \
             styles to chart data\u{2014}is controlled programmatically, enabling true automation \
             without manual intervention.",
            &ParagraphStyle::Normal,
            &TextAlignment::Justify,
            None,
            None,
            None,
            false,
            false,
        ),
    );

    // ── Node Inventory Table ─────────────────────────────────────────────
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Complete Node Inventory",
            &ParagraphStyle::Heading2,
            &TextAlignment::Left,
            None,
            None,
            None,
            false,
            false,
        ),
    );
    let inventory = vec![
        vec!["Category".into(), "Node Name".into(), "Description".into()],
        vec![
            "DOCX".into(),
            "Create Document".into(),
            "Generate themed DOCX from scratch".into(),
        ],
        vec![
            "DOCX".into(),
            "Add Paragraph".into(),
            "Insert styled paragraph with formatting".into(),
        ],
        vec![
            "DOCX".into(),
            "Add Table".into(),
            "Create branded data tables".into(),
        ],
        vec![
            "DOCX".into(),
            "Replace Text".into(),
            "Template-style text substitution".into(),
        ],
        vec![
            "PPTX".into(),
            "Create Presentation".into(),
            "Generate PPTX with theme and master".into(),
        ],
        vec![
            "PPTX".into(),
            "Add Slide".into(),
            "Append blank or formatted slides".into(),
        ],
        vec![
            "PPTX".into(),
            "Add Text Box".into(),
            "Positioned text with alignment".into(),
        ],
        vec![
            "PPTX".into(),
            "Add Shape".into(),
            "Geometric shapes with fill and text".into(),
        ],
        vec![
            "PPTX".into(),
            "Add Table".into(),
            "Tabular data on slides".into(),
        ],
        vec![
            "PPTX".into(),
            "Add Chart".into(),
            "Bar, Line, and Pie charts".into(),
        ],
        vec![
            "PDF".into(),
            "Add Watermark".into(),
            "Overlay semi-transparent text".into(),
        ],
        vec![
            "PDF".into(),
            "Add Page Numbers".into(),
            "Auto-numbered page footers".into(),
        ],
        vec![
            "PDF".into(),
            "Merge Documents".into(),
            "Combine multiple PDFs".into(),
        ],
        vec![
            "PDF".into(),
            "Encrypt".into(),
            "Password-protect PDF files".into(),
        ],
    ];
    insert_before_sect_pr(
        &mut files,
        &build_table(&inventory, true, true, defaults::BORDER, 9.0),
    );

    // ── Closing ──────────────────────────────────────────────────────────
    insert_before_sect_pr(
        &mut files,
        &build_paragraph(
            "Build the future of documents, one node at a time.",
            &ParagraphStyle::Quote,
            &TextAlignment::Left,
            None,
            None,
            None,
            false,
            false,
        ),
    );

    let result = write_zip(&files).expect("final write_zip");

    let re_read = read_zip(&result).expect("re-read");
    assert!(re_read.contains_key("word/document.xml"));
    assert!(re_read.contains_key("word/styles.xml"));

    let doc_xml = String::from_utf8_lossy(re_read.get("word/document.xml").unwrap());
    assert!(doc_xml.contains("Flow Like"));
    assert!(doc_xml.contains("<w:tbl>"));

    let path = output_dir().join("test_document.docx");
    std::fs::write(&path, &result).expect("write DOCX");
    println!("DOCX written to: {}", path.display());
}

#[test]
fn generate_pptx_with_slides() {
    let pptx_bytes = create_empty_pptx();
    let mut files = read_zip(&pptx_bytes).expect("read_zip");

    // ── Slide 1: Hero Title ──────────────────────────────────────────────
    let s1 = pptx_add_slide(&mut files);
    // Red hero band spanning top 55%
    pptx_add_shape(
        &mut files,
        s1,
        "rect",
        0.0,
        0.0,
        33.87,
        10.5,
        defaults::PRIMARY,
        "",
        "",
    );
    // Thin dark accent strip below hero
    pptx_add_shape(
        &mut files, s1, "rect", 0.0, 10.5, 33.87, 0.25, "#111111", "", "",
    );
    // Title on hero
    pptx_add_text_box_aligned(
        &mut files,
        s1,
        "FLOW LIKE",
        2.0,
        2.5,
        29.87,
        4.0,
        54.0,
        "#FFFFFF",
        true,
        "ctr",
        "ctr",
    );
    // Subtitle on hero
    pptx_add_text_box_aligned(
        &mut files,
        s1,
        "Next-Generation Document Automation Platform",
        4.0,
        6.5,
        25.87,
        2.0,
        22.0,
        "#FFFFFF",
        false,
        "ctr",
        "",
    );
    // Lower section
    pptx_add_text_box_aligned(
        &mut files,
        s1,
        "Q4 2024  \u{2022}  Platform Capability Overview",
        2.0,
        12.0,
        29.87,
        1.5,
        14.0,
        defaults::TEXT_MUTED,
        false,
        "ctr",
        "",
    );
    // Small decorative shapes
    pptx_add_shape(
        &mut files, s1, "ellipse", 1.0, 14.5, 0.8, 0.8, "#FF4343", "", "",
    );
    pptx_add_shape(
        &mut files, s1, "ellipse", 2.2, 14.5, 0.8, 0.8, "#3B82F6", "", "",
    );
    pptx_add_shape(
        &mut files, s1, "ellipse", 3.4, 14.5, 0.8, 0.8, "#10B981", "", "",
    );
    pptx_add_shape(
        &mut files, s1, "ellipse", 4.6, 14.5, 0.8, 0.8, "#F59E0B", "", "",
    );

    // ── Slide 2: Platform at a Glance ────────────────────────────────────
    let s2 = pptx_add_slide(&mut files);
    pptx_add_shape(
        &mut files,
        s2,
        "rect",
        0.0,
        0.0,
        33.87,
        0.6,
        defaults::PRIMARY,
        "",
        "",
    );
    pptx_add_text_box(
        &mut files,
        s2,
        "Platform at a Glance",
        2.0,
        1.2,
        29.87,
        1.8,
        32.0,
        defaults::HEADING,
        true,
    );
    pptx_add_text_box(
        &mut files,
        s2,
        "Four document engines \u{2022} 50+ automation nodes \u{2022} Zero dependencies",
        2.0,
        3.2,
        29.87,
        1.2,
        14.0,
        defaults::TEXT_MUTED,
        false,
    );
    // Feature cards
    pptx_add_shape(
        &mut files,
        s2,
        "roundRect",
        1.5,
        5.5,
        7.0,
        7.5,
        "#FF4343",
        "",
        "",
    );
    pptx_add_text_box_aligned(
        &mut files, s2, "DOCX", 1.5, 6.5, 7.0, 2.0, 28.0, "#FFFFFF", true, "ctr", "",
    );
    pptx_add_text_box_aligned(
        &mut files, s2, "10 Nodes", 1.5, 8.5, 7.0, 1.2, 14.0, "#FFFFFF", false, "ctr", "",
    );
    pptx_add_text_box_aligned(
        &mut files,
        s2,
        "Create \u{2022} Style \u{2022} Table",
        1.5,
        10.0,
        7.0,
        1.2,
        11.0,
        "#FFFFFF",
        false,
        "ctr",
        "",
    );

    pptx_add_shape(
        &mut files,
        s2,
        "roundRect",
        9.5,
        5.5,
        7.0,
        7.5,
        "#3B82F6",
        "",
        "",
    );
    pptx_add_text_box_aligned(
        &mut files, s2, "PPTX", 9.5, 6.5, 7.0, 2.0, 28.0, "#FFFFFF", true, "ctr", "",
    );
    pptx_add_text_box_aligned(
        &mut files, s2, "12 Nodes", 9.5, 8.5, 7.0, 1.2, 14.0, "#FFFFFF", false, "ctr", "",
    );
    pptx_add_text_box_aligned(
        &mut files,
        s2,
        "Slides \u{2022} Charts \u{2022} Shapes",
        9.5,
        10.0,
        7.0,
        1.2,
        11.0,
        "#FFFFFF",
        false,
        "ctr",
        "",
    );

    pptx_add_shape(
        &mut files,
        s2,
        "roundRect",
        17.5,
        5.5,
        7.0,
        7.5,
        "#10B981",
        "",
        "",
    );
    pptx_add_text_box_aligned(
        &mut files, s2, "PDF", 17.5, 6.5, 7.0, 2.0, 28.0, "#FFFFFF", true, "ctr", "",
    );
    pptx_add_text_box_aligned(
        &mut files, s2, "18 Nodes", 17.5, 8.5, 7.0, 1.2, 14.0, "#FFFFFF", false, "ctr", "",
    );
    pptx_add_text_box_aligned(
        &mut files,
        s2,
        "Merge \u{2022} Encrypt \u{2022} Sign",
        17.5,
        10.0,
        7.0,
        1.2,
        11.0,
        "#FFFFFF",
        false,
        "ctr",
        "",
    );

    pptx_add_shape(
        &mut files,
        s2,
        "roundRect",
        25.5,
        5.5,
        7.0,
        7.5,
        "#F59E0B",
        "",
        "",
    );
    pptx_add_text_box_aligned(
        &mut files, s2, "IMAGE", 25.5, 6.5, 7.0, 2.0, 28.0, "#FFFFFF", true, "ctr", "",
    );
    pptx_add_text_box_aligned(
        &mut files, s2, "8 Nodes", 25.5, 8.5, 7.0, 1.2, 14.0, "#FFFFFF", false, "ctr", "",
    );
    pptx_add_text_box_aligned(
        &mut files,
        s2,
        "Convert \u{2022} Resize \u{2022} Crop",
        25.5,
        10.0,
        7.0,
        1.2,
        11.0,
        "#FFFFFF",
        false,
        "ctr",
        "",
    );

    // ── Slide 3: Node Capabilities Table ─────────────────────────────────
    let s3 = pptx_add_slide(&mut files);
    pptx_add_shape(
        &mut files,
        s3,
        "rect",
        0.0,
        0.0,
        33.87,
        0.6,
        defaults::PRIMARY,
        "",
        "",
    );
    pptx_add_text_box(
        &mut files,
        s3,
        "Node Capabilities",
        2.0,
        1.2,
        29.87,
        1.8,
        32.0,
        defaults::HEADING,
        true,
    );
    pptx_add_text_box(
        &mut files,
        s3,
        "Complete inventory of document automation nodes",
        2.0,
        3.2,
        29.87,
        1.2,
        14.0,
        defaults::TEXT_MUTED,
        false,
    );
    pptx_add_table(
        &mut files,
        s3,
        &["Category", "Node", "Description", "Status"],
        &[
            vec![
                "DOCX",
                "Create Document",
                "Generate blank DOCX with theme",
                "Ready",
            ],
            vec![
                "DOCX",
                "Add Paragraph",
                "Insert styled text with formatting",
                "Ready",
            ],
            vec![
                "DOCX",
                "Add Table",
                "Data tables with branded headers",
                "Ready",
            ],
            vec![
                "PPTX",
                "Create Presentation",
                "Generate themed PPTX shell",
                "Ready",
            ],
            vec![
                "PPTX",
                "Add Slide",
                "Append blank or templated slides",
                "Ready",
            ],
            vec![
                "PPTX",
                "Add Chart",
                "Bar, Line, Pie chart insertion",
                "Ready",
            ],
            vec![
                "PDF",
                "Merge Documents",
                "Combine multiple PDFs into one",
                "Ready",
            ],
            vec![
                "PDF",
                "Add Watermark",
                "Overlay branded watermark text",
                "Ready",
            ],
        ],
        1.5,
        5.0,
        30.87,
        0.9,
    );

    // ── Slide 4: Revenue Bar Chart (multi-series) ────────────────────────
    let s4 = pptx_add_slide(&mut files);
    pptx_add_shape(
        &mut files,
        s4,
        "rect",
        0.0,
        0.0,
        33.87,
        0.6,
        defaults::PRIMARY,
        "",
        "",
    );
    pptx_add_text_box(
        &mut files,
        s4,
        "Quarterly Revenue",
        2.0,
        1.2,
        20.0,
        1.8,
        32.0,
        defaults::HEADING,
        true,
    );
    pptx_add_text_box(
        &mut files,
        s4,
        "Year-over-year growth across product lines",
        2.0,
        3.2,
        29.87,
        1.2,
        14.0,
        defaults::TEXT_MUTED,
        false,
    );
    pptx_add_multi_chart(
        &mut files,
        s4,
        "bar",
        &["Q1", "Q2", "Q3", "Q4"],
        &[&[85.0, 130.0, 195.0, 260.0], &[120.0, 180.0, 250.0, 310.0]],
        &["2023 Revenue ($K)", "2024 Revenue ($K)"],
        &["3B82F6", "FF4343"],
        2.5,
        5.0,
        28.87,
        12.5,
    );

    // ── Slide 5: Growth Line Chart (multi-series) ────────────────────────
    let s5 = pptx_add_slide(&mut files);
    pptx_add_shape(
        &mut files,
        s5,
        "rect",
        0.0,
        0.0,
        33.87,
        0.6,
        defaults::PRIMARY,
        "",
        "",
    );
    pptx_add_text_box(
        &mut files,
        s5,
        "User Adoption Trends",
        2.0,
        1.2,
        20.0,
        1.8,
        32.0,
        defaults::HEADING,
        true,
    );
    pptx_add_text_box(
        &mut files,
        s5,
        "Monthly active users across regions (thousands)",
        2.0,
        3.2,
        29.87,
        1.2,
        14.0,
        defaults::TEXT_MUTED,
        false,
    );
    pptx_add_multi_chart(
        &mut files,
        s5,
        "line",
        &["Jan", "Feb", "Mar", "Apr", "May", "Jun"],
        &[
            &[12.0, 19.0, 28.0, 35.0, 48.0, 62.0],
            &[8.0, 14.0, 18.0, 24.0, 31.0, 40.0],
            &[5.0, 8.0, 12.0, 18.0, 25.0, 34.0],
        ],
        &["North America", "Europe", "Asia Pacific"],
        &["FF4343", "3B82F6", "10B981"],
        2.5,
        5.0,
        28.87,
        12.5,
    );

    // ── Slide 6: Distribution Pie Chart ──────────────────────────────────
    let s6 = pptx_add_slide(&mut files);
    pptx_add_shape(
        &mut files,
        s6,
        "rect",
        0.0,
        0.0,
        33.87,
        0.6,
        defaults::PRIMARY,
        "",
        "",
    );
    pptx_add_text_box(
        &mut files,
        s6,
        "Node Distribution by Format",
        2.0,
        1.2,
        20.0,
        1.8,
        32.0,
        defaults::HEADING,
        true,
    );
    pptx_add_text_box(
        &mut files,
        s6,
        "Breakdown of 50+ automation nodes across document formats",
        2.0,
        3.2,
        29.87,
        1.2,
        14.0,
        defaults::TEXT_MUTED,
        false,
    );
    // Stats on the left
    pptx_add_shape(
        &mut files,
        s6,
        "roundRect",
        2.0,
        5.5,
        8.0,
        3.0,
        "#FEF2F2",
        "#FF4343",
        "",
    );
    pptx_add_text_box_aligned(
        &mut files, s6, "50+", 2.0, 5.8, 8.0, 1.8, 36.0, "#FF4343", true, "ctr", "",
    );
    pptx_add_text_box_aligned(
        &mut files,
        s6,
        "Total Nodes",
        2.0,
        7.6,
        8.0,
        1.0,
        12.0,
        defaults::TEXT_MUTED,
        false,
        "ctr",
        "",
    );
    pptx_add_shape(
        &mut files,
        s6,
        "roundRect",
        2.0,
        9.5,
        8.0,
        3.0,
        "#EFF6FF",
        "#3B82F6",
        "",
    );
    pptx_add_text_box_aligned(
        &mut files, s6, "4", 2.0, 9.8, 8.0, 1.8, 36.0, "#3B82F6", true, "ctr", "",
    );
    pptx_add_text_box_aligned(
        &mut files,
        s6,
        "Formats",
        2.0,
        11.6,
        8.0,
        1.0,
        12.0,
        defaults::TEXT_MUTED,
        false,
        "ctr",
        "",
    );
    // Pie chart on the right
    pptx_add_multi_chart(
        &mut files,
        s6,
        "pie",
        &[
            "DOCX (10)",
            "PPTX (12)",
            "PDF (18)",
            "Image (8)",
            "Utility (4)",
        ],
        &[&[10.0, 12.0, 18.0, 8.0, 4.0]],
        &["Nodes by Format"],
        &["FF4343", "3B82F6", "10B981", "F59E0B", "8B5CF6"],
        11.0,
        5.0,
        21.0,
        12.5,
    );

    // ── Slide 7: Thank You / Closing ─────────────────────────────────────
    let s7 = pptx_add_slide(&mut files);
    pptx_add_shape(
        &mut files,
        s7,
        "rect",
        0.0,
        13.0,
        33.87,
        6.05,
        defaults::PRIMARY,
        "",
        "",
    );
    pptx_add_text_box_aligned(
        &mut files,
        s7,
        "Thank You",
        2.0,
        3.0,
        29.87,
        4.0,
        48.0,
        defaults::HEADING,
        true,
        "ctr",
        "",
    );
    pptx_add_text_box_aligned(
        &mut files,
        s7,
        "Powering the next generation of document workflows",
        2.0,
        7.0,
        29.87,
        2.0,
        18.0,
        defaults::TEXT_MUTED,
        false,
        "ctr",
        "",
    );
    pptx_add_text_box_aligned(
        &mut files,
        s7,
        "flow-like.com  \u{2022}  github.com/Rheosoph/flow-like",
        2.0,
        14.5,
        29.87,
        1.5,
        14.0,
        "#FFFFFF",
        false,
        "ctr",
        "ctr",
    );

    let result = write_zip(&files).expect("final write_zip");

    // Verification
    let re_read = read_zip(&result).expect("re-read");
    assert!(re_read.contains_key("ppt/slides/slide1.xml"));
    assert!(re_read.contains_key("ppt/slides/slide7.xml"));
    assert!(re_read.contains_key("ppt/charts/chart1.xml"));
    assert!(re_read.contains_key("ppt/theme/theme1.xml"));

    let pres = String::from_utf8_lossy(re_read.get("ppt/presentation.xml").unwrap());
    assert!(
        !pres.contains("id=\"2147483649\""),
        "sldId must not exceed ST_SlideId max"
    );

    let theme = String::from_utf8_lossy(re_read.get("ppt/theme/theme1.xml").unwrap());
    assert!(theme.contains("FF4343"));

    let path = output_dir().join("test_presentation.pptx");
    std::fs::write(&path, &result).expect("write PPTX");
    println!("PPTX written to: {}", path.display());
}

#[test]
fn generate_pdf_with_watermark_and_page_numbers() {
    let pdf_bytes = create_simple_pdf();
    assert!(!pdf_bytes.is_empty());

    let watermarked = pdf_add_watermark(&pdf_bytes, "CONFIDENTIAL", defaults::PRIMARY, 0.12);
    assert!(!watermarked.is_empty());

    let final_pdf = pdf_add_page_numbers(&watermarked);
    assert!(!final_pdf.is_empty());

    let doc = lopdf::Document::load_mem(&final_pdf).expect("reload PDF");
    let page_count = doc.page_iter().count();
    assert_eq!(page_count, 5);

    let path = output_dir().join("test_document.pdf");
    std::fs::write(&path, &final_pdf).expect("write PDF");
    println!("PDF written to: {}", path.display());
}

#[test]
fn generate_plain_pdf() {
    let pdf_bytes = create_simple_pdf();
    let doc = lopdf::Document::load_mem(&pdf_bytes).expect("load plain PDF");
    assert_eq!(doc.page_iter().count(), 5);

    let path = output_dir().join("test_plain.pdf");
    std::fs::write(&path, &pdf_bytes).expect("write plain PDF");
    println!("Plain PDF written to: {}", path.display());
}

#[test]
fn generate_empty_docx() {
    let bytes = create_empty_docx(
        defaults::FONT_SANS,
        defaults::DOCX_FONT_SIZE,
        defaults::PRIMARY,
    );
    let files = read_zip(&bytes).expect("read_zip");

    assert!(files.contains_key("[Content_Types].xml"));
    assert!(files.contains_key("word/document.xml"));
    assert!(files.contains_key("word/styles.xml"));

    let styles = String::from_utf8_lossy(files.get("word/styles.xml").unwrap());
    assert!(styles.contains("Calibri"));
    assert!(styles.contains("FF4343"));

    let path = output_dir().join("test_empty.docx");
    std::fs::write(&path, &bytes).expect("write empty DOCX");
    println!("Empty DOCX written to: {}", path.display());
}

#[test]
fn generate_empty_pptx() {
    let bytes = create_empty_pptx();
    let files = read_zip(&bytes).expect("read_zip");

    assert!(files.contains_key("ppt/presentation.xml"));
    assert!(files.contains_key("ppt/theme/theme1.xml"));
    assert!(files.contains_key("ppt/slideMasters/slideMaster1.xml"));

    let theme = String::from_utf8_lossy(files.get("ppt/theme/theme1.xml").unwrap());
    assert!(theme.contains("FF4343"));
    assert!(theme.contains("Flow Like"));

    let path = output_dir().join("test_empty.pptx");
    std::fs::write(&path, &bytes).expect("write empty PPTX");
    println!("Empty PPTX written to: {}", path.display());
}

// ---------------------------------------------------------------------------
// Markdown → OpenXML tests
// ---------------------------------------------------------------------------

#[test]
fn markdown_to_runs_basic_formatting() {
    let runs = markdown_to_runs("Hello **bold** and *italic* world", OpenXmlFormat::Docx);
    assert!(
        runs.len() >= 4,
        "expected multiple runs, got {}",
        runs.len()
    );

    let bold_run = runs.iter().find(|r| r.text == "bold").expect("bold run");
    assert!(bold_run.bold);
    assert!(!bold_run.italic);

    let italic_run = runs
        .iter()
        .find(|r| r.text == "italic")
        .expect("italic run");
    assert!(italic_run.italic);
    assert!(!italic_run.bold);

    let plain = runs
        .iter()
        .find(|r| r.text.contains("Hello"))
        .expect("plain run");
    assert!(!plain.bold);
    assert!(!plain.italic);
}

#[test]
fn markdown_to_runs_code_and_strikethrough() {
    let runs = markdown_to_runs("Use `code` and ~~deleted~~ text", OpenXmlFormat::Pptx);

    let code_run = runs.iter().find(|r| r.text == "code").expect("code run");
    assert!(code_run.code);

    let strike_run = runs
        .iter()
        .find(|r| r.text == "deleted")
        .expect("strikethrough run");
    assert!(strike_run.strikethrough);
}

#[test]
fn markdown_to_runs_nested_formatting() {
    let runs = markdown_to_runs("***bold italic***", OpenXmlFormat::Docx);
    let run = runs
        .iter()
        .find(|r| r.text == "bold italic")
        .expect("nested run");
    assert!(run.bold);
    assert!(run.italic);
}

#[test]
fn markdown_to_runs_list_items() {
    let md = "- First\n- Second\n- Third";
    let runs = markdown_to_runs(md, OpenXmlFormat::Docx);
    let bullets: Vec<_> = runs.iter().filter(|r| r.text.contains('•')).collect();
    assert!(bullets.len() >= 2, "expected bullet items");
}

#[test]
fn replace_text_plain_in_docx() {
    let docx_bytes = create_empty_docx(
        defaults::FONT_SANS,
        defaults::DOCX_FONT_SIZE,
        defaults::PRIMARY,
    );
    let mut files = read_zip(&docx_bytes).expect("read_zip");

    let template_body = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<w:body>
<w:p><w:r><w:t>Hello {{NAME}}, welcome to {{COMPANY}}!</w:t></w:r></w:p>
<w:p><w:r><w:t>Your role is {{ROLE}}.</w:t></w:r></w:p>
<w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
</w:body>
</w:document>"#;
    files.insert(
        "word/document.xml".to_string(),
        template_body.as_bytes().to_vec(),
    );

    let doc = files.get("word/document.xml").unwrap().clone();
    let replaced = replace_text_in_xml(&doc, "{{NAME}}", "Alice", "w:t", "w:r", "w:p").unwrap();
    let replaced =
        replace_text_in_xml(&replaced, "{{COMPANY}}", "Flow Like", "w:t", "w:r", "w:p").unwrap();
    let replaced =
        replace_text_in_xml(&replaced, "{{ROLE}}", "Developer", "w:t", "w:r", "w:p").unwrap();
    files.insert("word/document.xml".to_string(), replaced);

    let result = write_zip(&files).expect("write_zip");
    let re_read = read_zip(&result).expect("re-read");
    let body = String::from_utf8_lossy(re_read.get("word/document.xml").unwrap());
    assert!(body.contains("Alice"));
    assert!(body.contains("Flow Like"));
    assert!(body.contains("Developer"));
    assert!(!body.contains("{{NAME}}"));
    assert!(!body.contains("{{COMPANY}}"));
    assert!(!body.contains("{{ROLE}}"));

    let path = output_dir().join("test_replace_plain.docx");
    std::fs::write(&path, &result).expect("write");
    println!("Plain-replaced DOCX written to: {}", path.display());
}

#[test]
fn replace_text_markdown_in_docx() {
    let docx_bytes = create_empty_docx(
        defaults::FONT_SANS,
        defaults::DOCX_FONT_SIZE,
        defaults::PRIMARY,
    );
    let mut files = read_zip(&docx_bytes).expect("read_zip");

    let template_body = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<w:body>
<w:p><w:r><w:rPr><w:sz w:val="24"/></w:rPr><w:t>{{CONTENT}}</w:t></w:r></w:p>
<w:sectPr><w:pgSz w:w="11906" w:h="16838"/></w:sectPr>
</w:body>
</w:document>"#;
    files.insert(
        "word/document.xml".to_string(),
        template_body.as_bytes().to_vec(),
    );

    let markdown =
        "**Flow Like** is a *powerful* platform for `automation` with ~~legacy~~ modern workflows.";
    let doc = files.get("word/document.xml").unwrap().clone();
    let replaced = replace_text_in_xml_markdown(
        &doc,
        "{{CONTENT}}",
        markdown,
        "w:t",
        "w:r",
        "w:rPr",
        "w:p",
        OpenXmlFormat::Docx,
    )
    .unwrap();
    files.insert("word/document.xml".to_string(), replaced);

    let result = write_zip(&files).expect("write_zip");
    let re_read = read_zip(&result).expect("re-read");
    let body = String::from_utf8_lossy(re_read.get("word/document.xml").unwrap());
    assert!(body.contains("Flow Like"));
    assert!(body.contains("powerful"));
    assert!(body.contains("automation"));
    assert!(body.contains("<w:b/>"), "bold formatting expected");
    assert!(body.contains("<w:i/>"), "italic formatting expected");
    assert!(!body.contains("{{CONTENT}}"));

    let path = output_dir().join("test_replace_markdown.docx");
    std::fs::write(&path, &result).expect("write");
    println!("Markdown-replaced DOCX written to: {}", path.display());
}

#[test]
fn replace_text_plain_in_pptx() {
    let pptx_bytes = create_empty_pptx();
    let mut files = read_zip(&pptx_bytes).expect("read_zip");

    let slide_num = pptx_add_slide(&mut files);
    pptx_add_text_box(
        &mut files,
        slide_num,
        "Hello {{NAME}}, your title is {{TITLE}}.",
        2.0,
        3.0,
        28.0,
        3.0,
        18.0,
        defaults::TEXT,
        false,
    );

    let slide_path = format!("ppt/slides/slide{}.xml", slide_num);
    let slide_data = files.get(&slide_path).unwrap().clone();
    let replaced =
        replace_text_in_xml(&slide_data, "{{NAME}}", "Bob", "a:t", "a:r", "a:p").unwrap();
    let replaced = replace_text_in_xml(&replaced, "{{TITLE}}", "CTO", "a:t", "a:r", "a:p").unwrap();
    files.insert(slide_path.clone(), replaced);

    let result = write_zip(&files).expect("write_zip");
    let re_read = read_zip(&result).expect("re-read");
    let slide = String::from_utf8_lossy(re_read.get(&slide_path).unwrap());
    assert!(slide.contains("Bob"));
    assert!(slide.contains("CTO"));
    assert!(!slide.contains("{{NAME}}"));
    assert!(!slide.contains("{{TITLE}}"));

    let path = output_dir().join("test_replace_plain.pptx");
    std::fs::write(&path, &result).expect("write");
    println!("Plain-replaced PPTX written to: {}", path.display());
}

#[test]
fn replace_text_markdown_in_pptx() {
    let pptx_bytes = create_empty_pptx();
    let mut files = read_zip(&pptx_bytes).expect("read_zip");

    let slide_num = pptx_add_slide(&mut files);
    pptx_add_text_box(
        &mut files,
        slide_num,
        "{{CONTENT}}",
        2.0,
        2.0,
        28.0,
        14.0,
        16.0,
        defaults::TEXT,
        false,
    );

    let slide_path = format!("ppt/slides/slide{}.xml", slide_num);
    let slide_data = files.get(&slide_path).unwrap().clone();

    let markdown = "**Key Features:**\n\n- *Visual* workflow builder\n- `Code` execution nodes\n- ~~Deprecated~~ **modern** API design";
    let replaced = replace_text_in_xml_markdown(
        &slide_data,
        "{{CONTENT}}",
        markdown,
        "a:t",
        "a:r",
        "a:rPr",
        "a:p",
        OpenXmlFormat::Pptx,
    )
    .unwrap();
    files.insert(slide_path.clone(), replaced);

    let result = write_zip(&files).expect("write_zip");
    let re_read = read_zip(&result).expect("re-read");
    let slide = String::from_utf8_lossy(re_read.get(&slide_path).unwrap());
    assert!(slide.contains("Key Features:"));
    assert!(slide.contains("Visual"));
    assert!(slide.contains("Code"));
    assert!(slide.contains("modern"));
    assert!(!slide.contains("{{CONTENT}}"));

    let path = output_dir().join("test_replace_markdown.pptx");
    std::fs::write(&path, &result).expect("write");
    println!("Markdown-replaced PPTX written to: {}", path.display());
}

// ---------------------------------------------------------------------------
// Markdown → Document conversion helpers
// ---------------------------------------------------------------------------

fn heading_size_half_pts(level: u8) -> i32 {
    match level {
        1 => 48, // 24pt
        2 => 36, // 18pt
        3 => 28, // 14pt
        4 => 24, // 12pt
        _ => 22, // 11pt
    }
}

fn heading_size_hundredths(level: u8) -> i64 {
    match level {
        1 => 3600, // 36pt
        2 => 2800, // 28pt
        3 => 2200, // 22pt
        4 => 1800, // 18pt
        _ => 1400, // 14pt
    }
}

fn heading_size_pdf(level: u8) -> f64 {
    match level {
        1 => 22.0,
        2 => 18.0,
        3 => 15.0,
        4 => 13.0,
        _ => 12.0,
    }
}

fn docx_run_xml(
    text: &str,
    font: &str,
    size_half_pts: i32,
    color: &str,
    bold: bool,
    italic: bool,
    strikethrough: bool,
    is_code: bool,
) -> String {
    let rpr = if is_code {
        format!(
            r#"<w:rFonts w:ascii="Courier New" w:hAnsi="Courier New"/><w:sz w:val="{sz}"/><w:szCs w:val="{sz}"/><w:color w:val="D63384"/><w:shd w:val="clear" w:color="auto" w:fill="F8F9FA"/>"#,
            sz = size_half_pts,
        )
    } else {
        let mut rpr = format!(
            r#"<w:rFonts w:ascii="{f}" w:hAnsi="{f}"/><w:sz w:val="{sz}"/><w:szCs w:val="{sz}"/><w:color w:val="{c}"/>"#,
            f = font,
            sz = size_half_pts,
            c = color,
        );
        if bold {
            rpr.push_str("<w:b/>");
        }
        if italic {
            rpr.push_str("<w:i/>");
        }
        if strikethrough {
            rpr.push_str("<w:strike/>");
        }
        rpr
    };
    let preserve = if text.contains(' ') || text.contains('\t') {
        r#" xml:space="preserve""#
    } else {
        ""
    };
    format!(
        "<w:r><w:rPr>{rpr}</w:rPr><w:t{preserve}>{text}</w:t></w:r>",
        rpr = rpr,
        preserve = preserve,
        text = xml_escape(text),
    )
}

/// Full GFM-aware DOCX renderer: headings with sizes, bullet lists, blockquotes,
/// tables as real `<w:tbl>`, code blocks with shading, and images as placeholders.
fn markdown_runs_to_docx_xml(
    runs: &[FormattedRun],
    font: &str,
    base_size_half_pts: i32,
    base_color: &str,
) -> String {
    let mut xml = String::new();
    let color = hex_to_ooxml(base_color);
    let heading_color = hex_to_ooxml(defaults::HEADING);

    // Collect table data first (we need to scan ahead for table rows)
    let mut i = 0;
    while i < runs.len() {
        let run = &runs[i];

        // -- Image placeholder --
        if let BlockType::Image { ref url, ref alt } = run.block_type {
            let label = if alt.is_empty() && run.text.is_empty() {
                format!("[Image: {}]", url)
            } else if !run.text.is_empty() {
                run.text.clone()
            } else {
                alt.clone()
            };
            xml.push_str(&format!(
                "<w:p><w:pPr><w:spacing w:after=\"120\"/><w:pBdr><w:top w:val=\"single\" w:sz=\"4\" w:space=\"4\" w:color=\"CCCCCC\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"4\" w:color=\"CCCCCC\"/></w:pBdr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F0F0F0\"/></w:pPr>{}</w:p>",
                docx_run_xml(&label, font, base_size_half_pts, "666666", false, true, false, false),
            ));
            i += 1;
            continue;
        }

        // -- Heading --
        if let BlockType::Heading(level) = run.block_type {
            let sz = heading_size_half_pts(level);
            let spacing_before = if level <= 2 { "240" } else { "160" };
            let spacing_after = if level <= 2 { "120" } else { "80" };
            let mut heading_runs = String::new();
            // Collect all runs for this heading (until we hit a newline with Normal block)
            while i < runs.len() {
                let r = &runs[i];
                if r.text == "\n" && !matches!(r.block_type, BlockType::Heading(_)) {
                    i += 1;
                    break;
                }
                if let BlockType::Heading(_) = r.block_type {
                    heading_runs.push_str(&docx_run_xml(
                        &r.text,
                        font,
                        sz,
                        &heading_color,
                        true,
                        r.italic,
                        false,
                        r.code,
                    ));
                }
                i += 1;
            }
            xml.push_str(&format!(
                "<w:p><w:pPr><w:spacing w:before=\"{}\" w:after=\"{}\"/></w:pPr>{}</w:p>",
                spacing_before, spacing_after, heading_runs,
            ));
            // Add a separator line under h1/h2
            if level <= 2 {
                xml.push_str(&format!(
                    "<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"1\" w:color=\"{}\"/></w:pBdr><w:spacing w:after=\"120\"/></w:pPr></w:p>",
                    hex_to_ooxml(defaults::PRIMARY),
                ));
            }
            continue;
        }

        // -- BlockQuote --
        if run.block_type == BlockType::BlockQuote {
            let mut quote_runs = String::new();
            while i < runs.len() {
                let r = &runs[i];
                if r.block_type != BlockType::BlockQuote && r.text != "\n" {
                    break;
                }
                if r.text == "\n" {
                    i += 1;
                    continue;
                }
                quote_runs.push_str(&docx_run_xml(
                    &r.text,
                    font,
                    base_size_half_pts,
                    "666666",
                    r.bold,
                    true,
                    r.strikethrough,
                    r.code,
                ));
                i += 1;
            }
            xml.push_str(&format!(
                "<w:p><w:pPr><w:pBdr><w:left w:val=\"single\" w:sz=\"18\" w:space=\"8\" w:color=\"{}\"/></w:pBdr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"FFF5F5\"/><w:spacing w:after=\"120\"/><w:ind w:left=\"400\"/></w:pPr>{}</w:p>",
                hex_to_ooxml(defaults::PRIMARY), quote_runs,
            ));
            continue;
        }

        // -- Table --
        if run.block_type == BlockType::TableHeader || run.block_type == BlockType::TableCell {
            let mut rows: Vec<Vec<(String, bool)>> = Vec::new();
            let mut current_row: Vec<(String, bool)> = Vec::new();
            let mut is_header_row = run.block_type == BlockType::TableHeader;
            let mut cell_text = String::new();
            let mut cell_is_header = is_header_row;

            let mut j = i;
            while j < runs.len() {
                let r = &runs[j];
                match r.block_type {
                    BlockType::TableHeader => {
                        cell_text.push_str(&r.text);
                        cell_is_header = true;
                    }
                    BlockType::TableCell => {
                        cell_text.push_str(&r.text);
                    }
                    BlockType::TableRowEnd => {
                        if !cell_text.is_empty() {
                            current_row.push((cell_text.clone(), cell_is_header));
                            cell_text.clear();
                        }
                        if !current_row.is_empty() {
                            rows.push(current_row.clone());
                            current_row.clear();
                        }
                        is_header_row = false;
                        cell_is_header = false;
                    }
                    _ => {
                        // We've left the table
                        if !cell_text.is_empty() {
                            current_row.push((cell_text.clone(), cell_is_header));
                            cell_text.clear();
                        }
                        if !current_row.is_empty() {
                            rows.push(current_row.clone());
                        }
                        break;
                    }
                }
                j += 1;
            }
            if !cell_text.is_empty() {
                current_row.push((cell_text.clone(), cell_is_header));
            }
            if !current_row.is_empty() {
                rows.push(current_row);
            }
            i = j;

            // Determine column count
            let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(1);
            let col_width = 9000 / col_count as i32;

            let border_color = hex_to_ooxml(defaults::PRIMARY);
            let mut tbl = format!(
                r#"<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="9000" w:type="dxa"/><w:tblBorders><w:top w:val="single" w:sz="6" w:space="0" w:color="{bc}"/><w:left w:val="single" w:sz="6" w:space="0" w:color="{bc}"/><w:bottom w:val="single" w:sz="6" w:space="0" w:color="{bc}"/><w:right w:val="single" w:sz="6" w:space="0" w:color="{bc}"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="{bc}"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="{bc}"/></w:tblBorders></w:tblPr><w:tblGrid>{grid}</w:tblGrid>"#,
                bc = border_color,
                grid = (0..col_count)
                    .map(|_| format!("<w:gridCol w:w=\"{}\"/>", col_width))
                    .collect::<String>(),
            );

            for row in &rows {
                tbl.push_str("<w:tr>");
                for (cell_text_val, is_hdr) in row {
                    let cell_fill = if *is_hdr {
                        format!(
                            "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
                            hex_to_ooxml(defaults::PRIMARY)
                        )
                    } else {
                        String::new()
                    };
                    let text_color = if *is_hdr { "FFFFFF" } else { &color };
                    let cell_bold = *is_hdr;
                    tbl.push_str(&format!(
                        "<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"dxa\"/>{}</w:tcPr><w:p>{}</w:p></w:tc>",
                        col_width, cell_fill,
                        docx_run_xml(cell_text_val.trim(), font, base_size_half_pts, text_color, cell_bold, false, false, false),
                    ));
                }
                tbl.push_str("</w:tr>");
            }
            tbl.push_str("</w:tbl>");
            xml.push_str(&tbl);

            // Skip any trailing newline after table
            if i < runs.len() && runs[i].text == "\n" {
                i += 1;
            }
            continue;
        }

        // -- CodeBlock --
        if let BlockType::CodeBlock { ref language } = run.block_type {
            if is_chart_language(language) {
                // Chart code block → placeholder text (actual chart embedded separately)
                let mut j = i;
                let code_text = collect_chart_code_text(runs, &mut j);
                let label = if let Some(input) = parse_chart_block(&code_text) {
                    input.config.title.unwrap_or_else(|| "Chart".into())
                } else {
                    "Chart".into()
                };
                xml.push_str(&format!(
                    "<w:p><w:pPr><w:spacing w:after=\"120\"/></w:pPr>{}</w:p>",
                    docx_run_xml(
                        &format!("[Chart: {}]", label),
                        font,
                        base_size_half_pts,
                        "888888",
                        false,
                        true,
                        false,
                        false
                    ),
                ));
                i = j;
                continue;
            }

            let mut code_text = String::new();
            while i < runs.len() {
                let r = &runs[i];
                if !matches!(r.block_type, BlockType::CodeBlock { .. }) && r.text != "\n" {
                    break;
                }
                if r.text == "\n" && !matches!(r.block_type, BlockType::CodeBlock { .. }) {
                    i += 1;
                    break;
                }
                code_text.push_str(&r.text);
                i += 1;
            }
            for line in code_text.lines() {
                xml.push_str(&format!(
                    "<w:p><w:pPr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F8F9FA\"/><w:spacing w:after=\"0\" w:line=\"260\" w:lineRule=\"exact\"/></w:pPr>{}</w:p>",
                    docx_run_xml(line, "Courier New", 20, "333333", false, false, false, false),
                ));
            }
            continue;
        }

        // -- Normal paragraph / list items --
        if run.text == "\n" {
            // Paragraph break — flush
            xml.push_str(&format!(
                "<w:p><w:pPr><w:spacing w:after=\"120\"/></w:pPr></w:p>"
            ));
            i += 1;
            continue;
        }

        // Collect runs for this paragraph
        let mut para_runs = String::new();
        while i < runs.len() {
            let r = &runs[i];
            if r.text == "\n" {
                i += 1;
                break;
            }
            if !matches!(r.block_type, BlockType::Normal) {
                break;
            }
            para_runs.push_str(&docx_run_xml(
                &r.text,
                font,
                base_size_half_pts,
                &color,
                r.bold,
                r.italic,
                r.strikethrough,
                r.code,
            ));
            i += 1;
        }
        if !para_runs.is_empty() {
            xml.push_str(&format!(
                "<w:p><w:pPr><w:spacing w:after=\"120\"/></w:pPr>{}</w:p>",
                para_runs
            ));
        }
    }

    xml
}

/// GFM-aware PPTX body renderer: headings with sizes, tables as text grids, blockquotes styled.
fn markdown_runs_to_pptx_body(
    runs: &[FormattedRun],
    font_size_pt: f32,
    font_color: &str,
) -> String {
    let color = hex_to_ooxml(font_color);
    let sz = pt_to_hundredths(font_size_pt) as i64;
    let mut paragraphs: Vec<String> = Vec::new();

    fn pptx_run(
        text: &str,
        sz: i64,
        color: &str,
        bold: bool,
        italic: bool,
        strikethrough: bool,
        code: bool,
    ) -> String {
        let mut attrs = format!(r#" lang="en-US" sz="{}" dirty="0""#, sz);
        if bold {
            attrs.push_str(r#" b="1""#);
        }
        if italic {
            attrs.push_str(r#" i="1""#);
        }
        if strikethrough {
            attrs.push_str(r#" strike="sngStrike""#);
        }
        let font_face = if code { "Courier New" } else { "Calibri" };
        let fill_color = if code { "D63384" } else { color };
        let preserve = if text.contains(' ') || text.contains('\t') {
            r#" xml:space="preserve""#
        } else {
            ""
        };
        format!(
            r#"<a:r><a:rPr{attrs}><a:solidFill><a:srgbClr val="{clr}"/></a:solidFill><a:latin typeface="{font}"/></a:rPr><a:t{preserve}>{text}</a:t></a:r>"#,
            attrs = attrs,
            clr = fill_color,
            font = font_face,
            preserve = preserve,
            text = xml_escape(text),
        )
    }

    let mut i = 0;
    while i < runs.len() {
        let run = &runs[i];

        // Heading
        if let BlockType::Heading(level) = run.block_type {
            let hsz = heading_size_hundredths(level);
            let heading_clr = hex_to_ooxml(defaults::HEADING);
            let mut heading_content = String::new();
            while i < runs.len() {
                let r = &runs[i];
                if r.text == "\n" && !matches!(r.block_type, BlockType::Heading(_)) {
                    i += 1;
                    break;
                }
                if let BlockType::Heading(_) = r.block_type {
                    heading_content.push_str(&pptx_run(
                        &r.text,
                        hsz,
                        &heading_clr,
                        true,
                        r.italic,
                        false,
                        r.code,
                    ));
                }
                i += 1;
            }
            paragraphs.push(format!(
                "<a:p><a:pPr><a:spcBef><a:spcPts val=\"800\"/></a:spcBef></a:pPr>{}</a:p>",
                heading_content
            ));
            continue;
        }

        // BlockQuote
        if run.block_type == BlockType::BlockQuote {
            let quote_clr = "888888";
            let mut quote_content = String::new();
            while i < runs.len() {
                let r = &runs[i];
                if r.block_type != BlockType::BlockQuote && r.text != "\n" {
                    break;
                }
                if r.text == "\n" {
                    i += 1;
                    continue;
                }
                quote_content.push_str(&pptx_run(
                    &r.text,
                    sz,
                    quote_clr,
                    r.bold,
                    true,
                    r.strikethrough,
                    r.code,
                ));
                i += 1;
            }
            // Add a bar prefix run
            let bar = pptx_run(
                "┃ ",
                sz,
                &hex_to_ooxml(defaults::PRIMARY),
                true,
                false,
                false,
                false,
            );
            paragraphs.push(format!("<a:p>{}{}</a:p>", bar, quote_content));
            continue;
        }

        // Image placeholder
        if let BlockType::Image { ref url, ref alt } = run.block_type {
            let label = if run.text.is_empty() && alt.is_empty() {
                format!("[Image: {}]", url)
            } else if !run.text.is_empty() {
                run.text.clone()
            } else {
                alt.clone()
            };
            paragraphs.push(format!(
                "<a:p>{}</a:p>",
                pptx_run(
                    &format!("🖼 {}", label),
                    sz,
                    "888888",
                    false,
                    true,
                    false,
                    false
                )
            ));
            i += 1;
            continue;
        }

        // Table → render as grid with tab alignment
        if run.block_type == BlockType::TableHeader || run.block_type == BlockType::TableCell {
            let mut rows: Vec<Vec<(String, bool)>> = Vec::new();
            let mut current_row: Vec<(String, bool)> = Vec::new();
            let mut cell_text = String::new();
            let mut cell_is_header = run.block_type == BlockType::TableHeader;
            let mut j = i;
            while j < runs.len() {
                let r = &runs[j];
                match r.block_type {
                    BlockType::TableHeader => {
                        cell_text.push_str(&r.text);
                        cell_is_header = true;
                    }
                    BlockType::TableCell => {
                        cell_text.push_str(&r.text);
                    }
                    BlockType::TableRowEnd => {
                        if !cell_text.is_empty() {
                            current_row.push((cell_text.clone(), cell_is_header));
                            cell_text.clear();
                        }
                        if !current_row.is_empty() {
                            rows.push(current_row.clone());
                            current_row.clear();
                        }
                        cell_is_header = false;
                    }
                    _ => {
                        if !cell_text.is_empty() {
                            current_row.push((cell_text.clone(), cell_is_header));
                            cell_text.clear();
                        }
                        if !current_row.is_empty() {
                            rows.push(current_row.clone());
                        }
                        break;
                    }
                }
                j += 1;
            }
            if !cell_text.is_empty() {
                current_row.push((cell_text, cell_is_header));
            }
            if !current_row.is_empty() {
                rows.push(current_row);
            }
            i = j;

            for row in &rows {
                let row_text: String = row
                    .iter()
                    .map(|(t, _)| t.trim().to_string())
                    .collect::<Vec<_>>()
                    .join("  │  ");
                let is_header = row.first().is_some_and(|(_, h)| *h);
                if is_header {
                    paragraphs.push(format!(
                        "<a:p>{}</a:p>",
                        pptx_run(
                            &row_text,
                            sz,
                            &hex_to_ooxml(defaults::HEADING),
                            true,
                            false,
                            false,
                            false
                        )
                    ));
                    // Separator
                    let sep: String = row.iter().map(|_| "────").collect::<Vec<_>>().join("──┼──");
                    paragraphs.push(format!(
                        "<a:p>{}</a:p>",
                        pptx_run(&sep, sz - 200, "AAAAAA", false, false, false, true)
                    ));
                } else {
                    paragraphs.push(format!(
                        "<a:p>{}</a:p>",
                        pptx_run(&row_text, sz, &color, false, false, false, false)
                    ));
                }
            }
            if i < runs.len() && runs[i].text == "\n" {
                i += 1;
            }
            continue;
        }

        // Code block
        if let BlockType::CodeBlock { ref language } = run.block_type {
            if is_chart_language(language) {
                // Chart code block → skip in text box (native chart added separately)
                let mut j = i;
                let _code_text = collect_chart_code_text(runs, &mut j);
                i = j;
                continue;
            }

            let mut code_text = String::new();
            while i < runs.len() {
                let r = &runs[i];
                if !matches!(r.block_type, BlockType::CodeBlock { .. }) && r.text != "\n" {
                    break;
                }
                if r.text == "\n" && !matches!(r.block_type, BlockType::CodeBlock { .. }) {
                    i += 1;
                    break;
                }
                code_text.push_str(&r.text);
                i += 1;
            }
            for line in code_text.lines() {
                paragraphs.push(format!(
                    "<a:p>{}</a:p>",
                    pptx_run(line, sz - 200, "D63384", false, false, false, true)
                ));
            }
            continue;
        }
        let mut current_runs = String::new();
        while i < runs.len() {
            let r = &runs[i];
            if r.text == "\n" {
                i += 1;
                break;
            }
            if !matches!(r.block_type, BlockType::Normal) {
                break;
            }
            current_runs.push_str(&pptx_run(
                &r.text,
                sz,
                &color,
                r.bold,
                r.italic,
                r.strikethrough,
                r.code,
            ));
            i += 1;
        }
        if !current_runs.is_empty() {
            paragraphs.push(format!("<a:p>{}</a:p>", current_runs));
        }
    }

    if paragraphs.is_empty() {
        "<a:p><a:endParaRPr lang=\"en-US\"/></a:p>".to_string()
    } else {
        paragraphs.join("")
    }
}

fn pptx_add_markdown_text_box(
    files: &mut HashMap<String, Vec<u8>>,
    slide_num: u32,
    markdown: &str,
    x_cm: f32,
    y_cm: f32,
    w_cm: f32,
    h_cm: f32,
    font_size_pt: f32,
    font_color: &str,
) {
    let runs = markdown_to_runs(markdown, OpenXmlFormat::Pptx);
    let body_content = markdown_runs_to_pptx_body(&runs, font_size_pt, font_color);

    let slide_path = format!("ppt/slides/slide{}.xml", slide_num);
    let slide_data = files.get(&slide_path).expect("slide exists").clone();
    let mut slide_xml = String::from_utf8_lossy(&slide_data).to_string();
    let next_id = max_id(&slide_xml) + 1;

    let shape_xml = format!(
        r#"<p:sp>
  <p:nvSpPr>
    <p:cNvPr id="{id}" name="MarkdownBox {id}"/>
    <p:cNvSpPr txBox="1"/>
    <p:nvPr/>
  </p:nvSpPr>
  <p:spPr>
    <a:xfrm>
      <a:off x="{ox}" y="{oy}"/>
      <a:ext cx="{cx}" cy="{cy}"/>
    </a:xfrm>
    <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
    <a:noFill/>
  </p:spPr>
  <p:txBody>
    <a:bodyPr wrap="square" rtlCol="0"/>
    <a:lstStyle/>
    {body}
  </p:txBody>
</p:sp>"#,
        id = next_id,
        ox = cm_to_emu(x_cm),
        oy = cm_to_emu(y_cm),
        cx = cm_to_emu(w_cm),
        cy = cm_to_emu(h_cm),
        body = body_content,
    );

    if let Some(pos) = slide_xml.find("</p:spTree>") {
        slide_xml.insert_str(pos, &shape_xml);
    }
    files.insert(slide_path, slide_xml.into_bytes());
}

/// GFM-aware PDF renderer: proper line wrapping, heading sizes, tables, blockquotes.
fn markdown_to_pdf_content(markdown: &str, start_y: f64, page_width: f64) -> String {
    let runs = markdown_to_runs(markdown, OpenXmlFormat::Docx);
    let mut ops = String::new();
    let mut y = start_y;
    let left_margin = 50.0;
    let right_margin = page_width - 50.0;
    let base_size = 11.0;
    let line_height = base_size * 1.4;
    let bottom_margin = 40.0;

    let char_width = |size: f64| size * 0.52;

    fn pdf_text(font: &str, size: f64, color: &str, x: f64, y: f64, text: &str) -> String {
        let escaped = text
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        format!(
            "BT {} {} Tf {} {} {} Td ({}) Tj ET\n",
            font, size, color, x, y, escaped
        )
    }

    fn pdf_rect(x: f64, y: f64, w: f64, h: f64, color: &str, fill: bool) -> String {
        let op = if fill { "f" } else { "S" };
        format!("q {} {} {} {} {} re {} Q\n", color, x, y, w, h, op)
    }

    // Represents a styled span of text within a paragraph
    #[derive(Clone)]
    struct Span {
        text: String,
        bold: bool,
        italic: bool,
        code: bool,
    }

    // Break a list of spans into wrapped visual lines respecting max_width.
    // Each returned line is a Vec of (text_chunk, bold, italic, code) within that line.
    fn wrap_spans(spans: &[Span], max_width: f64, cw: f64) -> Vec<Vec<Span>> {
        let mut lines: Vec<Vec<Span>> = Vec::new();
        let mut cur_line: Vec<Span> = Vec::new();
        let mut cur_w: f64 = 0.0;

        for span in spans {
            let words: Vec<&str> = span.text.split(' ').collect();
            let mut span_buf = String::new();

            for (wi, word) in words.iter().enumerate() {
                if word.is_empty() && wi < words.len() - 1 {
                    // Consecutive spaces — just add a space
                    if !span_buf.is_empty() || !cur_line.is_empty() {
                        span_buf.push(' ');
                        cur_w += cw;
                    }
                    continue;
                }

                let word_w = word.len() as f64 * cw;
                let space_needed = if span_buf.is_empty() && cur_line.is_empty() {
                    0.0
                } else if span_buf.is_empty() {
                    cw
                } else {
                    cw
                };

                if cur_w + space_needed + word_w > max_width
                    && (cur_w > 0.0 || !cur_line.is_empty())
                {
                    // Flush current span buffer into the line
                    if !span_buf.is_empty() {
                        cur_line.push(Span {
                            text: span_buf.clone(),
                            bold: span.bold,
                            italic: span.italic,
                            code: span.code,
                        });
                        span_buf.clear();
                    }
                    if !cur_line.is_empty() {
                        lines.push(cur_line.clone());
                        cur_line.clear();
                    }
                    cur_w = 0.0;
                }

                if !span_buf.is_empty() {
                    span_buf.push(' ');
                    cur_w += cw;
                } else if !cur_line.is_empty() && cur_w > 0.0 {
                    // Need a space between the previous span's last word and this word
                    span_buf.push(' ');
                    cur_w += cw;
                }
                span_buf.push_str(word);
                cur_w += word_w;
            }

            if !span_buf.is_empty() {
                cur_line.push(Span {
                    text: span_buf,
                    bold: span.bold,
                    italic: span.italic,
                    code: span.code,
                });
            }
        }
        if !cur_line.is_empty() {
            lines.push(cur_line);
        }
        if lines.is_empty() {
            lines.push(Vec::new());
        }
        lines
    }

    fn wrap_plain(text: &str, max_width: f64, cw: f64) -> Vec<String> {
        let mut lines = Vec::new();
        let mut current = String::new();
        let mut current_w = 0.0;
        for word in text.split_whitespace() {
            let word_w = word.len() as f64 * cw;
            if current_w + word_w > max_width && !current.is_empty() {
                lines.push(current.trim().to_string());
                current = String::new();
                current_w = 0.0;
            }
            if !current.is_empty() {
                current.push(' ');
                current_w += cw;
            }
            current.push_str(word);
            current_w += word_w;
        }
        if !current.is_empty() {
            lines.push(current.trim().to_string());
        }
        if lines.is_empty() {
            lines.push(String::new());
        }
        lines
    }

    let text_color = "0.102 0.102 0.102 rg";
    let muted_color = "0.416 0.416 0.416 rg";
    let code_color = "0.839 0.192 0.518 rg";
    let max_text_w = right_margin - left_margin;

    let mut i = 0;
    while i < runs.len() {
        if y < bottom_margin {
            break;
        }
        let run = &runs[i];

        // Heading
        if let BlockType::Heading(level) = run.block_type {
            y -= 8.0;
            let hsz = heading_size_pdf(level);
            let cw = char_width(hsz);
            let mut heading_text = String::new();
            while i < runs.len() {
                let r = &runs[i];
                if r.text == "\n" && !matches!(r.block_type, BlockType::Heading(_)) {
                    i += 1;
                    break;
                }
                if matches!(r.block_type, BlockType::Heading(_)) {
                    heading_text.push_str(&r.text);
                }
                i += 1;
            }
            for line in wrap_plain(&heading_text, max_text_w, cw) {
                if y < bottom_margin {
                    break;
                }
                ops.push_str(&pdf_text(
                    "/F2",
                    hsz,
                    "0.067 0.067 0.067 rg",
                    left_margin,
                    y,
                    &line,
                ));
                y -= hsz * 1.3;
            }
            if level <= 2 {
                ops.push_str(&pdf_rect(
                    left_margin,
                    y + 4.0,
                    max_text_w,
                    0.5,
                    "0.067 0.067 0.067 rg",
                    true,
                ));
                y -= 6.0;
            }
            continue;
        }

        // BlockQuote
        if run.block_type == BlockType::BlockQuote {
            let cw = char_width(base_size);
            let indent = 20.0;
            let bar_x = left_margin + 6.0;
            let q_start_y = y;
            let mut quote_text = String::new();
            while i < runs.len() {
                let r = &runs[i];
                if r.block_type != BlockType::BlockQuote && r.text != "\n" {
                    break;
                }
                if r.text == "\n" {
                    i += 1;
                    continue;
                }
                quote_text.push_str(&r.text);
                i += 1;
            }
            for line in wrap_plain(&quote_text, max_text_w - indent, cw) {
                if y < bottom_margin {
                    break;
                }
                ops.push_str(&pdf_text(
                    "/F1",
                    base_size,
                    muted_color,
                    left_margin + indent,
                    y,
                    &line,
                ));
                y -= line_height;
            }
            let bar_height = q_start_y - y;
            if bar_height > 0.0 {
                ops.push_str(&pdf_rect(
                    bar_x,
                    y + base_size * 0.5,
                    3.0,
                    bar_height,
                    "1 0.263 0.263 rg",
                    true,
                ));
            }
            y -= 4.0;
            continue;
        }

        // Image placeholder
        if let BlockType::Image { ref url, ref alt } = run.block_type {
            let label = if run.text.is_empty() && alt.is_empty() {
                format!("[Image: {}]", url)
            } else if !run.text.is_empty() {
                run.text.clone()
            } else {
                alt.clone()
            };
            if y >= bottom_margin + 40.0 {
                ops.push_str(&pdf_rect(
                    left_margin,
                    y - 20.0,
                    max_text_w,
                    30.0,
                    "0.941 0.941 0.941 rg",
                    true,
                ));
                ops.push_str(&pdf_text(
                    "/F1",
                    10.0,
                    muted_color,
                    left_margin + 10.0,
                    y - 10.0,
                    &format!("Image: {}", label),
                ));
                y -= 40.0;
            }
            i += 1;
            continue;
        }

        // Table
        if run.block_type == BlockType::TableHeader || run.block_type == BlockType::TableCell {
            let mut rows: Vec<Vec<(String, bool)>> = Vec::new();
            let mut current_row: Vec<(String, bool)> = Vec::new();
            let mut cell_text = String::new();
            let mut cell_is_header = run.block_type == BlockType::TableHeader;
            let mut j = i;
            while j < runs.len() {
                let r = &runs[j];
                match r.block_type {
                    BlockType::TableHeader => {
                        cell_text.push_str(&r.text);
                        cell_is_header = true;
                    }
                    BlockType::TableCell => {
                        cell_text.push_str(&r.text);
                    }
                    BlockType::TableRowEnd => {
                        if !cell_text.is_empty() {
                            current_row.push((cell_text.clone(), cell_is_header));
                            cell_text.clear();
                        }
                        if !current_row.is_empty() {
                            rows.push(current_row.clone());
                            current_row.clear();
                        }
                        cell_is_header = false;
                    }
                    _ => {
                        if !cell_text.is_empty() {
                            current_row.push((cell_text.clone(), cell_is_header));
                            cell_text.clear();
                        }
                        if !current_row.is_empty() {
                            rows.push(current_row.clone());
                        }
                        break;
                    }
                }
                j += 1;
            }
            if !cell_text.is_empty() {
                current_row.push((cell_text, cell_is_header));
            }
            if !current_row.is_empty() {
                rows.push(current_row);
            }
            i = j;

            let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(1);
            let col_w = max_text_w / col_count as f64;
            let row_h = 18.0;
            let cell_cw = char_width(10.0);
            let cell_pad = 4.0;
            let max_cell_chars = ((col_w - cell_pad * 2.0) / cell_cw).max(1.0) as usize;

            for row in &rows {
                if y < bottom_margin {
                    break;
                }
                let is_header = row.first().is_some_and(|(_, h)| *h);
                if is_header {
                    ops.push_str(&pdf_rect(
                        left_margin,
                        y - row_h + 4.0,
                        max_text_w,
                        row_h,
                        "1 0.263 0.263 rg",
                        true,
                    ));
                }
                for (ci, (cell_t, is_hdr)) in row.iter().enumerate() {
                    let cx = left_margin + ci as f64 * col_w + cell_pad;
                    let font = if *is_hdr { "/F2" } else { "/F1" };
                    let clr = if *is_hdr { "1 1 1 rg" } else { text_color };
                    let trimmed = cell_t.trim();
                    let display: &str = if trimmed.len() > max_cell_chars {
                        &trimmed[..max_cell_chars.saturating_sub(1)]
                    } else {
                        trimmed
                    };
                    ops.push_str(&pdf_text(font, 10.0, clr, cx, y, display));
                }
                // Row border
                ops.push_str(&format!(
                    "q 0.8 0.8 0.8 RG 0.5 w {} {} m {} {} l S Q\n",
                    left_margin,
                    y - row_h + 4.0,
                    left_margin + max_text_w,
                    y - row_h + 4.0
                ));
                y -= row_h;
            }
            y -= 6.0;
            if i < runs.len() && runs[i].text == "\n" {
                i += 1;
            }
            continue;
        }

        // Code block
        if let BlockType::CodeBlock { ref language } = run.block_type {
            if is_chart_language(language) {
                // Chart code block → render native chart shapes in PDF
                let mut j = i;
                let code_text = collect_chart_code_text(&runs, &mut j);
                if let Some(input) = parse_chart_block(&code_text) {
                    if let Some(office) = chart_input_to_office_data(&input) {
                        ops.push_str(&pdf_render_chart(&office, &mut y, page_width));
                    }
                }
                i = j;
                continue;
            }

            let mut code_text = String::new();
            while i < runs.len() {
                let r = &runs[i];
                if !matches!(r.block_type, BlockType::CodeBlock { .. }) && r.text != "\n" {
                    break;
                }
                if r.text == "\n" && !matches!(r.block_type, BlockType::CodeBlock { .. }) {
                    i += 1;
                    break;
                }
                code_text.push_str(&r.text);
                i += 1;
            }
            let lines: Vec<&str> = code_text.lines().collect();
            let code_line_h = 13.0;
            let block_h = lines.len() as f64 * code_line_h + 8.0;
            if y >= bottom_margin + block_h {
                ops.push_str(&pdf_rect(
                    left_margin,
                    y - block_h + 4.0,
                    max_text_w,
                    block_h,
                    "0.973 0.976 0.98 rg",
                    true,
                ));
                for line in &lines {
                    let code_cw = char_width(10.0);
                    let max_code_chars = ((max_text_w - 16.0) / code_cw).max(1.0) as usize;
                    let display = if line.len() > max_code_chars {
                        &line[..max_code_chars]
                    } else {
                        line
                    };
                    ops.push_str(&pdf_text(
                        "/F3",
                        10.0,
                        code_color,
                        left_margin + 8.0,
                        y,
                        display,
                    ));
                    y -= code_line_h;
                }
                y -= 8.0;
            }
            continue;
        }

        // Newline → paragraph spacing
        if run.text == "\n" {
            y -= base_size * 0.6;
            i += 1;
            continue;
        }

        // Normal paragraph: collect formatted spans and word-wrap preserving styles
        let cw = char_width(base_size);
        let mut spans: Vec<Span> = Vec::new();
        while i < runs.len() {
            let r = &runs[i];
            if r.text == "\n" {
                i += 1;
                break;
            }
            if !matches!(r.block_type, BlockType::Normal) {
                break;
            }
            spans.push(Span {
                text: r.text.clone(),
                bold: r.bold,
                italic: r.italic,
                code: r.code,
            });
            i += 1;
        }

        let wrapped_lines = wrap_spans(&spans, max_text_w, cw);
        for visual_line in &wrapped_lines {
            if y < bottom_margin {
                break;
            }
            let mut lx = left_margin;
            for span in visual_line {
                if span.text.is_empty() {
                    continue;
                }
                let font = if span.code {
                    "/F3"
                } else if span.bold {
                    "/F2"
                } else {
                    "/F1"
                };
                let clr = if span.code { code_color } else { text_color };
                let sz = if span.code { 10.0 } else { base_size };
                let span_cw = char_width(sz);
                ops.push_str(&pdf_text(font, sz, clr, lx, y, &span.text));
                lx += span.text.len() as f64 * span_cw;
            }
            y -= line_height;
        }
    }
    ops
}

fn pdf_escape_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

// ---------------------------------------------------------------------------
// MD → DOCX test (GFM: headings, tables, blockquotes, code, images)
// ---------------------------------------------------------------------------

#[test]
fn convert_markdown_to_docx() {
    let md = r##"# Flow Like Platform

**Flow Like** is a *next-generation* document automation platform.

## Key Features

- **Visual Workflows**: Drag-and-drop node editor for building pipelines
- **Multi-Format**: Support for `DOCX`, `PPTX`, `PDF`, and images
- ~~Legacy templates~~ **Modern** markdown-based content injection
- `code_execution()` nodes for dynamic data

## Architecture

The platform uses a ***composable node graph*** where each node performs a single document operation. Nodes can be chained to build complex pipelines.

> This document was generated entirely from markdown using Flow Like's document automation nodes.

### Performance Metrics

| Metric | Value |
|--------|-------|
| Nodes Available | 50+ |
| Formats Supported | 4 |
| Build Time | <100ms |

## Code Example

```rust
let doc = create_empty_docx("Calibri", 11.0, "#FF4343");
let runs = markdown_to_runs(md, OpenXmlFormat::Docx);
```

![Flow Like Logo](https://example.com/logo.png "Platform Logo")

### Summary

This showcases **headings**, *italic*, ~~strikethrough~~, `inline code`, block quotes, tables, fenced code blocks, and image placeholders — all from GFM markdown.
"##;

    let runs = markdown_to_runs(md, OpenXmlFormat::Docx);
    assert!(!runs.is_empty());

    let docx_bytes = create_empty_docx(
        defaults::FONT_SANS,
        defaults::DOCX_FONT_SIZE,
        defaults::PRIMARY,
    );
    let mut files = read_zip(&docx_bytes).expect("read_zip");

    let paragraphs_xml = markdown_runs_to_docx_xml(&runs, "Calibri", 22, defaults::TEXT);

    // Verify GFM elements are present in the XML
    assert!(paragraphs_xml.contains("<w:b/>"), "bold expected");
    assert!(paragraphs_xml.contains("<w:i/>"), "italic expected");
    assert!(paragraphs_xml.contains("<w:tbl>"), "table expected");
    assert!(paragraphs_xml.contains("Nodes Available"), "table content");
    assert!(paragraphs_xml.contains("Courier New"), "code font");
    assert!(paragraphs_xml.contains("F8F9FA"), "code block shading");
    assert!(paragraphs_xml.contains("FFF5F5"), "blockquote shading");
    assert!(
        paragraphs_xml.contains("Flow Like Logo"),
        "image placeholder"
    );

    insert_before_sect_pr(&mut files, &paragraphs_xml);
    let result = write_zip(&files).expect("write_zip");

    let re_read = read_zip(&result).expect("re-read");
    let doc = String::from_utf8_lossy(re_read.get("word/document.xml").unwrap());
    assert!(doc.contains("Flow Like"));
    assert!(doc.contains("<w:tbl>"));

    let path = output_dir().join("test_md_to_docx.docx");
    std::fs::write(&path, &result).expect("write");
    println!("MD→DOCX written to: {}", path.display());
}

// ---------------------------------------------------------------------------
// MD → PPTX slide text box test (GFM comprehensive)
// ---------------------------------------------------------------------------

#[test]
fn convert_markdown_to_pptx_slide() {
    let md = r#"# Flow Like Platform

**Flow Like** is a *visual workflow automation* platform for document generation.

## Supported Formats

- **DOCX**: Full Word document creation with `styled paragraphs`, tables, and headers
- **PPTX**: Presentation slides with charts, shapes, and ~~placeholder~~ **markdown** text boxes
- **PDF**: Merge, watermark, encrypt, and add `page numbers`

## Performance

| Metric | Value |
|--------|-------|
| Nodes Available | 50+ |
| Formats Supported | 4 |
| Build Time | <100ms |

> Built with Rust for maximum performance and reliability.

```rust
let pptx = create_empty_pptx();
markdown_to_runs(md, OpenXmlFormat::Pptx);
```

![Architecture Diagram](https://example.com/arch.png "System Architecture")

***50+ automation nodes*** available across all formats."#;

    let pptx_bytes = create_empty_pptx();
    let mut files = read_zip(&pptx_bytes).expect("read_zip");

    // Slide 1: Title
    let s1 = pptx_add_slide(&mut files);
    pptx_add_shape(
        &mut files,
        s1,
        "rect",
        0.0,
        0.0,
        33.87,
        4.0,
        defaults::PRIMARY,
        "",
        "",
    );
    pptx_add_text_box_aligned(
        &mut files,
        s1,
        "Markdown → PPTX",
        2.0,
        0.8,
        29.87,
        2.5,
        36.0,
        "#FFFFFF",
        true,
        "ctr",
        "",
    );
    pptx_add_text_box_aligned(
        &mut files,
        s1,
        "GFM rendering: headings, tables, blockquotes, code, images",
        2.0,
        3.0,
        29.87,
        1.0,
        14.0,
        "#FFB3B3",
        false,
        "ctr",
        "",
    );

    // Slide 2: Full markdown text box
    let s2 = pptx_add_slide(&mut files);
    pptx_add_shape(
        &mut files,
        s2,
        "rect",
        0.0,
        0.0,
        33.87,
        0.6,
        defaults::PRIMARY,
        "",
        "",
    );
    pptx_add_markdown_text_box(
        &mut files,
        s2,
        md,
        2.0,
        1.5,
        29.87,
        17.0,
        12.0,
        defaults::TEXT,
    );

    let result = write_zip(&files).expect("write_zip");
    let re_read = read_zip(&result).expect("re-read");

    let slide2 = String::from_utf8_lossy(re_read.get("ppt/slides/slide2.xml").unwrap());
    assert!(slide2.contains("Flow Like Platform"), "heading");
    assert!(slide2.contains("b=\"1\""), "bold formatting");
    assert!(slide2.contains("i=\"1\""), "italic formatting");
    assert!(slide2.contains("Courier New"), "code font");
    assert!(slide2.contains("Nodes Available"), "table content");
    assert!(slide2.contains("┃"), "blockquote marker");
    assert!(slide2.contains("────"), "table separator");

    let path = output_dir().join("test_md_to_pptx.pptx");
    std::fs::write(&path, &result).expect("write");
    println!("MD→PPTX written to: {}", path.display());
}

// ---------------------------------------------------------------------------
// MD → PDF test (GFM comprehensive)
// ---------------------------------------------------------------------------

#[test]
fn convert_markdown_to_pdf() {
    use lopdf::{Document, Object, Stream, dictionary};

    let md = r#"# Flow Like Document Generation

The platform enables *automated* document creation across multiple formats.

## Features

- **50+ Nodes**: Covering DOCX, PPTX, PDF, and image operations
- `code_execution()` for dynamic content generation
- ~~Manual processes~~ replaced by **Automated** pipelines with visual workflows

## Performance Metrics

| Metric | Value |
|--------|-------|
| Nodes Available | 50+ |
| Formats Supported | 4 |
| Build Time | <100ms |

> This document was generated entirely from markdown using Flow Like's document automation nodes. The blockquote rendering demonstrates proper left-bar styling.

```rust
let pdf = create_simple_pdf();
markdown_to_pdf_content(md, 700.0, 612.0);
```

![Flow Like Logo](https://example.com/logo.png "Platform Logo")

Built with ***Rust*** for maximum performance and reliability."#;

    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font_id = doc.new_object_id();
    let font_bold_id = doc.new_object_id();
    let font_mono_id = doc.new_object_id();
    let resources_id = doc.new_object_id();

    doc.objects.insert(
        font_id,
        Object::Dictionary(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        }),
    );
    doc.objects.insert(
        font_bold_id,
        Object::Dictionary(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica-Bold",
        }),
    );
    doc.objects.insert(
        font_mono_id,
        Object::Dictionary(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
        }),
    );
    doc.objects.insert(
        resources_id,
        Object::Dictionary(dictionary! {
            "Font" => dictionary! {
                "F1" => Object::Reference(font_id),
                "F2" => Object::Reference(font_bold_id),
                "F3" => Object::Reference(font_mono_id),
            },
        }),
    );

    // Title bar
    let mut content = String::new();
    content.push_str("q 1 0.263 0.263 rg 0 780 612 12 re f Q\n");

    // Render markdown content
    content.push_str(&markdown_to_pdf_content(md, 750.0, 612.0));

    // Footer
    content.push_str("q 1 0.263 0.263 rg 0 0 612 4 re f Q\n");
    content.push_str(
        "BT /F1 8 Tf 0.416 0.416 0.416 rg 200 15 Td (Generated from Markdown by Flow Like) Tj ET\n",
    );

    let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => Object::Reference(pages_id),
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Contents" => Object::Reference(content_id),
        "Resources" => Object::Reference(resources_id),
    });

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => Object::Reference(pages_id),
    });
    doc.trailer.set("Root", Object::Reference(catalog_id));

    let mut pdf_bytes = Vec::new();
    doc.save_to(&mut pdf_bytes).expect("save PDF");
    assert!(!pdf_bytes.is_empty());

    let path = output_dir().join("test_md_to_pdf.pdf");
    std::fs::write(&path, &pdf_bytes).expect("write");
    println!("MD→PDF written to: {}", path.display());
}

// ---------------------------------------------------------------------------
// Chart rendering tests
// ---------------------------------------------------------------------------

const CHART_MARKDOWN: &str = r#"# Revenue Report

Below is a bar chart showing quarterly revenue:

```nivo
type: bar
title: Revenue by Quarter
colors: [#FF4343, #4B5563]
stacked: false
---
Quarter,Product A,Product B
Q1,120,80
Q2,150,95
Q3,180,110
Q4,200,130
```

And a line chart showing trends:

```nivo
type: line
title: Monthly Trends
---
Month,Sales,Expenses
Jan,100,80
Feb,120,85
Mar,140,90
Apr,160,95
```

Finally a pie chart:

```plotly
type: pie
title: Market Share
---
Segment,Share
Desktop,65
Mobile,25
Tablet,10
```

Regular code blocks should still render normally:

```rust
let x = 42;
println!("{}", x);
```
"#;

#[test]
fn chart_blocks_detected_in_runs() {
    let runs = markdown_to_runs(CHART_MARKDOWN, OpenXmlFormat::Docx);
    let charts = extract_charts_from_runs(&runs);
    assert_eq!(charts.len(), 3, "expected 3 charts, got {}", charts.len());

    assert_eq!(charts[0].chart_type, ChartType::Bar);
    assert_eq!(charts[0].title.as_deref(), Some("Revenue by Quarter"));
    assert_eq!(charts[0].categories, vec!["Q1", "Q2", "Q3", "Q4"]);
    assert_eq!(charts[0].series.len(), 2);

    assert_eq!(charts[1].chart_type, ChartType::Line);
    assert_eq!(charts[1].title.as_deref(), Some("Monthly Trends"));

    assert_eq!(charts[2].chart_type, ChartType::Pie);
    assert_eq!(charts[2].title.as_deref(), Some("Market Share"));
    assert_eq!(charts[2].categories, vec!["Desktop", "Mobile", "Tablet"]);
}

#[test]
fn chart_nivo_to_pptx() {
    let pptx_bytes = create_empty_pptx();
    let mut files = read_zip(&pptx_bytes).expect("read_zip");

    let s1 = pptx_add_slide(&mut files);
    pptx_add_shape(
        &mut files,
        s1,
        "rect",
        0.0,
        0.0,
        33.87,
        4.0,
        defaults::PRIMARY,
        "",
        "",
    );
    pptx_add_text_box_aligned(
        &mut files,
        s1,
        "Charts in PPTX",
        2.0,
        0.8,
        29.87,
        2.5,
        36.0,
        "#FFFFFF",
        true,
        "ctr",
        "",
    );

    let s2 = pptx_add_slide(&mut files);
    pptx_add_shape(
        &mut files,
        s2,
        "rect",
        0.0,
        0.0,
        33.87,
        0.6,
        defaults::PRIMARY,
        "",
        "",
    );

    // Add markdown text box (chart blocks are skipped in text)
    pptx_add_markdown_text_box(
        &mut files,
        s2,
        CHART_MARKDOWN,
        2.0,
        1.5,
        29.87,
        3.5,
        12.0,
        defaults::TEXT,
    );

    // Embed native charts on the slide (more vertical space)
    let chart_count =
        pptx_embed_charts_from_markdown(&mut files, s2, CHART_MARKDOWN, 2.0, 5.5, 29.87, 13.0);
    assert_eq!(chart_count, 3, "expected 3 charts embedded");

    let result = write_zip(&files).expect("write_zip");
    let re_read = read_zip(&result).expect("re-read");

    // Verify chart files exist
    assert!(
        re_read.contains_key("ppt/charts/chart1.xml"),
        "chart1 missing"
    );
    assert!(
        re_read.contains_key("ppt/charts/chart2.xml"),
        "chart2 missing"
    );
    assert!(
        re_read.contains_key("ppt/charts/chart3.xml"),
        "chart3 missing"
    );

    // Verify chart XML contains correct chart types
    let chart1 = String::from_utf8_lossy(re_read.get("ppt/charts/chart1.xml").unwrap());
    assert!(chart1.contains("<c:barChart>"), "chart1 should be bar");
    assert!(chart1.contains("Revenue by Quarter"), "chart1 title");
    assert!(chart1.contains("Product A"), "chart1 series name");

    let chart2 = String::from_utf8_lossy(re_read.get("ppt/charts/chart2.xml").unwrap());
    assert!(chart2.contains("<c:lineChart>"), "chart2 should be line");

    let chart3 = String::from_utf8_lossy(re_read.get("ppt/charts/chart3.xml").unwrap());
    assert!(chart3.contains("<c:pieChart>"), "chart3 should be pie");

    // Verify slide has chart references
    let slide2 = String::from_utf8_lossy(re_read.get("ppt/slides/slide2.xml").unwrap());
    assert!(
        slide2.contains("p:graphicFrame"),
        "chart graphic frame on slide"
    );

    // Verify regular code block still rendered as text
    assert!(slide2.contains("let x = 42"), "regular code block as text");

    // Verify content types updated
    let ct = String::from_utf8_lossy(re_read.get("[Content_Types].xml").unwrap());
    assert!(ct.contains("drawingml.chart+xml"), "chart content type");

    let path = output_dir().join("test_chart_pptx.pptx");
    std::fs::write(&path, &result).expect("write");
    println!("Chart PPTX written to: {}", path.display());
}

#[test]
fn chart_nivo_to_docx() {
    let docx_bytes = create_empty_docx(
        defaults::FONT_SANS,
        defaults::DOCX_FONT_SIZE,
        defaults::PRIMARY,
    );
    let mut files = read_zip(&docx_bytes).expect("read_zip");

    let runs = markdown_to_runs(CHART_MARKDOWN, OpenXmlFormat::Docx);
    let paragraphs_xml = markdown_runs_to_docx_xml(&runs, "Calibri", 22, defaults::TEXT);

    // Embed native charts (replaces placeholder paragraphs inline)
    let final_xml = docx_embed_charts_from_markdown(&mut files, CHART_MARKDOWN, &paragraphs_xml);

    insert_before_sect_pr(&mut files, &final_xml);

    let result = write_zip(&files).expect("write_zip");
    let re_read = read_zip(&result).expect("re-read");

    // Verify chart files exist
    assert!(
        re_read.contains_key("word/charts/chart1.xml"),
        "chart1 missing"
    );
    assert!(
        re_read.contains_key("word/charts/chart2.xml"),
        "chart2 missing"
    );
    assert!(
        re_read.contains_key("word/charts/chart3.xml"),
        "chart3 missing"
    );

    let chart1 = String::from_utf8_lossy(re_read.get("word/charts/chart1.xml").unwrap());
    assert!(chart1.contains("<c:barChart>"), "chart1 should be bar");
    assert!(chart1.contains("Revenue by Quarter"), "chart1 title");

    // Verify document contains chart drawing inline with text
    let doc = String::from_utf8_lossy(re_read.get("word/document.xml").unwrap());
    assert!(doc.contains("Revenue Report"), "heading present");
    assert!(doc.contains("wp:inline"), "chart inline drawing present");

    // Verify regular code block still rendered
    assert!(
        paragraphs_xml.contains("let x = 42"),
        "regular code block kept"
    );

    // Verify placeholders were replaced (not present in final document)
    assert!(
        !doc.contains("[Chart:"),
        "placeholders should be replaced by chart drawings"
    );

    let path = output_dir().join("test_chart_docx.docx");
    std::fs::write(&path, &result).expect("write");
    println!("Chart DOCX written to: {}", path.display());
}

#[test]
fn chart_nivo_to_pdf() {
    use lopdf::{Document, Object, Stream, dictionary};

    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let font_id = doc.new_object_id();
    let font_bold_id = doc.new_object_id();
    let font_mono_id = doc.new_object_id();
    let resources_id = doc.new_object_id();

    doc.objects.insert(
        font_id,
        Object::Dictionary(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        }),
    );
    doc.objects.insert(
        font_bold_id,
        Object::Dictionary(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica-Bold",
        }),
    );
    doc.objects.insert(
        font_mono_id,
        Object::Dictionary(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
        }),
    );
    doc.objects.insert(
        resources_id,
        Object::Dictionary(dictionary! {
            "Font" => dictionary! {
                "F1" => Object::Reference(font_id),
                "F2" => Object::Reference(font_bold_id),
                "F3" => Object::Reference(font_mono_id),
            },
        }),
    );

    let mut content = String::new();
    content.push_str("q 1 0.263 0.263 rg 0 780 612 12 re f Q\n");
    content.push_str(&markdown_to_pdf_content(CHART_MARKDOWN, 750.0, 612.0));
    content.push_str("q 1 0.263 0.263 rg 0 0 612 4 re f Q\n");

    // Verify chart rendering produced bar/line/pie shapes
    assert!(content.contains("re f"), "bars or rectangles drawn");
    assert!(content.contains("Revenue by Quarter"), "bar chart title");
    assert!(content.contains("Monthly Trends"), "line chart title");
    assert!(content.contains("Market Share"), "pie chart title");

    let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => Object::Reference(pages_id),
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Contents" => Object::Reference(content_id),
        "Resources" => Object::Reference(resources_id),
    });

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![Object::Reference(page_id)],
            "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => Object::Reference(pages_id),
    });
    doc.trailer.set("Root", Object::Reference(catalog_id));

    let mut pdf_bytes = Vec::new();
    doc.save_to(&mut pdf_bytes).expect("save PDF");
    assert!(!pdf_bytes.is_empty());

    let path = output_dir().join("test_chart_pdf.pdf");
    std::fs::write(&path, &pdf_bytes).expect("write");
    println!("Chart PDF written to: {}", path.display());
}

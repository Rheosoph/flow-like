use std::collections::HashMap;

use crate::document::docx::add_table::build_table;
use crate::document::docx::create::docx_package;
use crate::document::pdf::render::{Block, Span, parse_markdown};
use crate::document::openxml::write_zip;
use crate::document::styles::{defaults, hex_to_ooxml};

use super::{
    Built, Images, REL_IMAGE, add_override, add_relationship, ensure_default_content_type,
    office_image, xml_text,
};

#[derive(Debug, Clone, Default)]
pub struct DocxOptions {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub author: Option<String>,
    /// Put the title block on its own first page.
    pub cover: bool,
    /// a4 (default), letter, legal, a5 or a3.
    pub page_size: Option<String>,
    pub page_numbers: bool,
}

const EMU_PER_PIXEL: i64 = 9525;
const MAX_IMAGE_WIDTH_EMU: i64 = 15 * 360_000;
const MAX_IMAGE_HEIGHT_EMU: i64 = 20 * 360_000;
const FOOTER_REL_ID: &str = "rIdFooterPageNumbers";

const EXTRA_STYLES: &str = r#"<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:pPr><w:spacing w:after="60"/></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="CodeBlock"><w:name w:val="Code Block"/><w:pPr><w:shd w:val="clear" w:color="auto" w:fill="F8F8F8"/><w:spacing w:before="0" w:after="0" w:line="240" w:lineRule="auto"/><w:ind w:left="240" w:right="240"/></w:pPr><w:rPr><w:rFonts w:ascii="Consolas" w:hAnsi="Consolas" w:cs="Consolas"/><w:sz w:val="19"/><w:szCs w:val="19"/></w:rPr></w:style>
"#;

fn page_geometry(name: Option<&str>) -> (u32, u32) {
    match name.unwrap_or("a4").to_ascii_lowercase().as_str() {
        "letter" => (12240, 15840),
        "legal" => (12240, 20160),
        "a5" => (8391, 11906),
        "a3" => (16838, 23811),
        _ => (11906, 16838),
    }
}

fn run_xml(span: &Span, base_bold: bool) -> String {
    if span.text.is_empty() {
        return String::new();
    }
    let mut props = String::new();
    if span.code {
        props.push_str("<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:cs=\"Consolas\"/>");
    }
    if span.bold || base_bold {
        props.push_str("<w:b/>");
    }
    if span.italic {
        props.push_str("<w:i/>");
    }
    if span.strikethrough {
        props.push_str("<w:strike/>");
    }
    if span.code {
        props.push_str("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F3F4F6\"/>");
    }
    let props = if props.is_empty() {
        String::new()
    } else {
        format!("<w:rPr>{props}</w:rPr>")
    };
    let mut runs = String::new();
    for (index, line) in span.text.split('\n').enumerate() {
        if index > 0 {
            runs.push_str("<w:r><w:br/></w:r>");
        }
        runs.push_str(&format!(
            "<w:r>{props}<w:t xml:space=\"preserve\">{}</w:t></w:r>",
            xml_text(line)
        ));
    }
    runs
}

fn runs_xml(spans: &[Span]) -> String {
    spans.iter().map(|span| run_xml(span, false)).collect()
}

fn paragraph(style: Option<&str>, properties: &str, runs: &str) -> String {
    let style = style.map_or_else(String::new, |id| format!("<w:pStyle w:val=\"{id}\"/>"));
    format!("<w:p><w:pPr>{style}{properties}</w:pPr>{runs}</w:p>")
}

fn plain(text: &str) -> String {
    format!(
        "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>",
        xml_text(text)
    )
}

fn image_alts(markdown: &str) -> HashMap<String, String> {
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut alts = HashMap::new();
    let mut current: Option<(String, String)> = None;
    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                current = Some((dest_url.to_string(), String::new()));
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, alt)) = current.as_mut() {
                    alt.push_str(&text);
                }
            }
            Event::End(TagEnd::Image) => {
                if let Some((url, alt)) = current.take() {
                    alts.entry(url).or_insert(alt);
                }
            }
            _ => {}
        }
    }
    alts
}

fn plain_text(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

struct Builder<'a> {
    files: HashMap<String, Vec<u8>>,
    images: &'a Images,
    body: String,
    next_image: u32,
    warnings: Vec<String>,
}

impl Builder<'_> {
    fn image(&mut self, url: &str, alt: &str) {
        let Some(source) = self.images.get(url) else {
            self.warnings.push(format!(
                "Image '{url}' was not provided, so only its description is shown"
            ));
            self.body
                .push_str(&paragraph(None, "", &plain(&format!("[{alt}]"))));
            return;
        };
        let (bytes, extension, width, height) = match office_image(source) {
            Ok(image) => image,
            Err(error) => {
                self.warnings.push(format!("Image '{url}' was skipped: {error}"));
                return;
            }
        };
        self.next_image += 1;
        let index = self.next_image;
        let name = format!("image{index}.{extension}");
        self.files.insert(format!("word/media/{name}"), bytes);
        ensure_default_content_type(&mut self.files, extension);
        let rel_id = format!("rIdImg{index}");
        add_relationship(
            &mut self.files,
            "word/_rels/document.xml.rels",
            &rel_id,
            REL_IMAGE,
            &format!("media/{name}"),
        );
        let mut cx = i64::from(width.max(1)) * EMU_PER_PIXEL;
        let mut cy = i64::from(height.max(1)) * EMU_PER_PIXEL;
        for (limit, along_x) in [(MAX_IMAGE_WIDTH_EMU, true), (MAX_IMAGE_HEIGHT_EMU, false)] {
            let current = if along_x { cx } else { cy };
            if current > limit {
                cx = cx * limit / current;
                cy = cy * limit / current;
            }
        }
        let alt = xml_text(alt);
        let drawing = format!(
            "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:docPr id=\"{index}\" name=\"{alt}\" descr=\"{alt}\"/><a:graphic xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\"><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:nvPicPr><pic:cNvPr id=\"{index}\" name=\"{alt}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{rel_id}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"
        );
        self.body.push_str(&paragraph(None, "", &drawing));
    }

    fn block(&mut self, block: &Block) {
        match block {
            Block::Heading { level, spans } => {
                let style = format!("Heading{}", (*level).clamp(1, 4));
                self.body
                    .push_str(&paragraph(Some(&style), "", &runs_xml(spans)));
            }
            Block::Paragraph(spans) => self.body.push_str(&paragraph(None, "", &runs_xml(spans))),
            Block::ListItem {
                depth,
                marker,
                spans,
            } => {
                let left = 720 + 360 * *depth;
                let properties = format!("<w:ind w:left=\"{left}\" w:hanging=\"360\"/>");
                let runs = format!("{}<w:r><w:tab/></w:r>{}", plain(marker), runs_xml(spans));
                self.body
                    .push_str(&paragraph(Some("ListParagraph"), &properties, &runs));
            }
            Block::Quote(spans) => self
                .body
                .push_str(&paragraph(Some("Quote"), "", &runs_xml(spans))),
            Block::Code { lines, .. } => {
                for line in lines {
                    self.body
                        .push_str(&paragraph(Some("CodeBlock"), "", &plain(line)));
                }
                self.body.push_str(&paragraph(None, "", ""));
            }
            Block::Rule => self.body.push_str(&paragraph(
                None,
                &format!(
                    "<w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"{}\"/></w:pBdr>",
                    hex_to_ooxml(defaults::BORDER)
                ),
                "",
            )),
            Block::Image { url, alt } => self.image(url, alt),
            Block::Table { header, rows } => {
                let mut grid = vec![header.clone()];
                grid.extend(rows.iter().cloned());
                self.body
                    .push_str(&build_table(&grid, true, true, defaults::BORDER, 10.0));
                self.body.push_str(&paragraph(None, "", ""));
            }
            Block::Chart(_) => self.warnings.push(
                "A chart block was left out: Word documents do not render markdown charts"
                    .to_string(),
            ),
        }
    }
}

fn core_properties(options: &DocxOptions) -> String {
    let field = |tag: &str, value: &Option<String>| {
        value
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .map(|v| format!("<{tag}>{}</{tag}>", xml_text(v)))
            .unwrap_or_default()
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">{}{}{}</cp:coreProperties>",
        field("dc:title", &options.title),
        field("dc:subject", &options.subtitle),
        field("dc:creator", &options.author),
    )
}

fn add_page_number_footer(files: &mut HashMap<String, Vec<u8>>) {
    let footer = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:ftr xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:p><w:pPr><w:jc w:val=\"center\"/></w:pPr><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p></w:ftr>";
    files.insert("word/footer1.xml".to_string(), footer.as_bytes().to_vec());
    add_override(
        files,
        "word/footer1.xml",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml",
    );
    add_relationship(
        files,
        "word/_rels/document.xml.rels",
        FOOTER_REL_ID,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer",
        "footer1.xml",
    );
}

fn apply_section(document: &str, options: &DocxOptions) -> String {
    let (width, height) = page_geometry(options.page_size.as_deref());
    let mut section = document.to_string();
    if let Some(start) = section.find("<w:pgSz")
        && let Some(length) = section[start..].find("/>")
    {
        section.replace_range(
            start..start + length + 2,
            &format!("<w:pgSz w:w=\"{width}\" w:h=\"{height}\"/>"),
        );
    }
    if options.page_numbers {
        section = section.replacen(
            "<w:sectPr>",
            &format!("<w:sectPr><w:footerReference w:type=\"default\" r:id=\"{FOOTER_REL_ID}\"/>"),
            1,
        );
    }
    section
}

/// Builds a Word document with real heading, list, table, code and image structure from markdown.
/// `images` maps the URL written in the markdown to the image bytes.
pub fn docx_from_markdown(
    markdown: &str,
    options: &DocxOptions,
    images: &Images,
) -> flow_like_types::Result<Built> {
    let files = docx_package(defaults::FONT_SANS, f64::from(defaults::DOCX_FONT_SIZE), defaults::PRIMARY)?;
    let mut builder = Builder {
        files,
        images,
        body: String::new(),
        next_image: 0,
        warnings: Vec::new(),
    };

    let title = options.title.as_deref().filter(|t| !t.trim().is_empty());
    let subtitle = options.subtitle.as_deref().filter(|t| !t.trim().is_empty());
    if let Some(title) = title {
        builder
            .body
            .push_str(&paragraph(Some("Title"), "", &plain(title)));
    }
    if let Some(subtitle) = subtitle {
        builder
            .body
            .push_str(&paragraph(Some("Subtitle"), "", &plain(subtitle)));
    }
    if options.cover && (title.is_some() || subtitle.is_some()) {
        builder
            .body
            .push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
    }
    let alts = image_alts(markdown);
    let mut caption: Option<String> = None;
    for block in parse_markdown(markdown) {
        if let (Some(alt), Block::Paragraph(spans)) = (caption.take(), &block)
            && plain_text(spans) == alt
        {
            continue;
        }
        if let Block::Image { url, alt } = &block {
            let alt = alts.get(url).cloned().unwrap_or_else(|| alt.clone());
            caption = (!alt.is_empty()).then(|| alt.clone());
            builder.image(url, &alt);
            continue;
        }
        builder.block(&block);
    }

    let Builder {
        mut files,
        body,
        warnings,
        ..
    } = builder;

    let document = String::from_utf8_lossy(&files["word/document.xml"]).into_owned();
    let mut document = apply_section(&document, options);
    if let Some(position) = document.rfind("<w:sectPr") {
        document.insert_str(position, &body);
    }
    files.insert("word/document.xml".to_string(), document.into_bytes());

    let styles = String::from_utf8_lossy(&files["word/styles.xml"]).into_owned();
    files.insert(
        "word/styles.xml".to_string(),
        styles
            .replacen("</w:styles>", &format!("{EXTRA_STYLES}</w:styles>"), 1)
            .into_bytes(),
    );
    files.insert(
        "docProps/core.xml".to_string(),
        core_properties(options).into_bytes(),
    );
    if options.page_numbers {
        add_page_number_footer(&mut files);
    }

    Ok(Built {
        bytes: write_zip(&files)?,
        count: None,
        warnings,
    })
}

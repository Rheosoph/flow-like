use std::collections::HashMap;

use crate::document::openxml::write_zip;
use crate::document::pptx::create::{THEME, pptx_package};
use crate::document::styles::{defaults, hex_to_ooxml};

use super::chart::add_chart_parts;
use super::{
    Built, add_override, add_relationship, ensure_default_content_type, office_image,
    xml_text,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlideLayout {
    Title,
    Bullets,
    Image,
    Table,
    Chart,
}

pub use super::chart::{ChartKind, ChartSpec as SlideChart, WorkbookFn};

#[derive(Debug, Clone)]
pub struct Slide {
    pub layout: SlideLayout,
    pub title: String,
    pub bullets: Vec<String>,
    pub notes: Option<String>,
    pub image: Option<Vec<u8>>,
    pub table: Option<(Vec<String>, Vec<Vec<String>>)>,
    pub chart: Option<SlideChart>,
}

#[derive(Debug, Clone, Default)]
pub struct DeckOptions {
    pub title: Option<String>,
    pub author: Option<String>,
    /// Embeds each chart's data as a workbook so "Edit data" works.
    pub workbook: Option<WorkbookFn>,
}

const SLIDE_WIDTH: i64 = 12_192_000;
const MARGIN: i64 = 609_600;
const CONTENT_TOP: i64 = 1_550_000;
const CONTENT_HEIGHT: i64 = 4_700_000;
const CONTENT_WIDTH: i64 = SLIDE_WIDTH - 2 * MARGIN;
const EMU_PER_PIXEL: i64 = 9525;
const NS: &str = "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"";
const REL_BASE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

struct SlideXml {
    shapes: String,
    next_id: u32,
    rels: Vec<(String, &'static str, String)>,
}

impl SlideXml {
    fn new() -> Self {
        Self {
            shapes: String::new(),
            next_id: 2,
            rels: vec![(
                "rId1".to_string(),
                "slideLayout",
                "../slideLayouts/slideLayout1.xml".to_string(),
            )],
        }
    }

    fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    fn relate(&mut self, kind: &'static str, target: String) -> String {
        let id = format!("rId{}", self.rels.len() + 1);
        self.rels.push((id.clone(), kind, target));
        id
    }

    fn rect(&mut self, x: i64, y: i64, cx: i64, cy: i64, color: &str) {
        let id = self.id();
        self.shapes.push_str(&format!(
            "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Accent {id}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill><a:ln><a:noFill/></a:ln></p:spPr></p:sp>"
        ));
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        name: &str,
        (x, y, cx, cy): (i64, i64, i64, i64),
        paragraphs: &[String],
        size: u32,
        bold: bool,
        color: &str,
        bullets: bool,
        anchor: &str,
    ) {
        let id = self.id();
        let weight = if bold { " b=\"1\"" } else { "" };
        let body: String = paragraphs
            .iter()
            .map(|text| {
                let bullet = if bullets {
                    "<a:pPr marL=\"342900\" indent=\"-342900\"><a:spcBef><a:spcPts val=\"600\"/></a:spcBef><a:buFont typeface=\"Arial\"/><a:buChar char=\"&#8226;\"/></a:pPr>"
                } else {
                    "<a:pPr><a:buNone/></a:pPr>"
                };
                format!(
                    "<a:p>{bullet}<a:r><a:rPr lang=\"en-US\" sz=\"{}\"{weight}><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill></a:rPr><a:t>{}</a:t></a:r></a:p>",
                    size * 100,
                    xml_text(text)
                )
            })
            .collect();
        self.shapes.push_str(&format!(
            "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name} {id}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr wrap=\"square\" anchor=\"{anchor}\"><a:normAutofit/></a:bodyPr><a:lstStyle/>{body}</p:txBody></p:sp>"
        ));
    }

    fn picture(&mut self, rel_id: &str, x: i64, y: i64, cx: i64, cy: i64) {
        let id = self.id();
        self.shapes.push_str(&format!(
            "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"Image {id}\" descr=\"Slide image\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"{rel_id}\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>"
        ));
    }

    fn frame(&mut self, name: &str, graphic: &str) {
        let id = self.id();
        self.shapes.push_str(&format!(
            "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"{name} {id}\"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp=\"1\"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr>{graphic}</p:graphicFrame>"
        ));
    }

    fn xml(&self) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:sld {NS}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}</p:spTree></p:cSld></p:sld>",
            self.shapes
        )
    }

    fn rels_xml(&self) -> String {
        let items: String = self
            .rels
            .iter()
            .map(|(id, kind, target)| {
                format!("<Relationship Id=\"{id}\" Type=\"{REL_BASE}/{kind}\" Target=\"{target}\"/>")
            })
            .collect();
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{items}</Relationships>"
        )
    }
}

fn table_graphic(headers: &[String], rows: &[Vec<String>]) -> String {
    let columns = rows
        .iter()
        .map(Vec::len)
        .chain([headers.len()])
        .max()
        .unwrap_or(1)
        .max(1);
    let column_width = CONTENT_WIDTH / columns as i64;
    let row_height = 400_000i64;
    let accent = hex_to_ooxml(defaults::PRIMARY);
    let cell = |text: &str, header: bool, band: bool| {
        let (fill, color, weight) = if header {
            (accent.clone(), "FFFFFF".to_string(), " b=\"1\"")
        } else if band {
            (hex_to_ooxml(defaults::SURFACE), hex_to_ooxml(defaults::TEXT), "")
        } else {
            ("FFFFFF".to_string(), hex_to_ooxml(defaults::TEXT), "")
        };
        let border = hex_to_ooxml(defaults::BORDER);
        let line = |side: &str| {
            format!("<a:{side} w=\"6350\"><a:solidFill><a:srgbClr val=\"{border}\"/></a:solidFill></a:{side}>")
        };
        format!(
            "<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"en-US\" sz=\"1400\"{weight}><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill></a:rPr><a:t>{}</a:t></a:r></a:p></a:txBody><a:tcPr>{}{}{}{}<a:solidFill><a:srgbClr val=\"{fill}\"/></a:solidFill></a:tcPr></a:tc>",
            xml_text(text),
            line("lnL"),
            line("lnR"),
            line("lnT"),
            line("lnB"),
        )
    };
    let render_row = |values: &[String], header: bool, band: bool| {
        let cells: String = (0..columns)
            .map(|column| cell(values.get(column).map_or("", String::as_str), header, band))
            .collect();
        format!("<a:tr h=\"{row_height}\">{cells}</a:tr>")
    };
    let mut body = render_row(headers, true, false);
    for (index, row) in rows.iter().enumerate() {
        body.push_str(&render_row(row, false, index % 2 == 1));
    }
    let grid: String = (0..columns)
        .map(|_| format!("<a:gridCol w=\"{column_width}\"/>"))
        .collect();
    let height = row_height * (rows.len() as i64 + 1);
    format!(
        "<p:xfrm><a:off x=\"{MARGIN}\" y=\"{CONTENT_TOP}\"/><a:ext cx=\"{CONTENT_WIDTH}\" cy=\"{height}\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\"><a:tbl><a:tblPr firstRow=\"1\" bandRow=\"1\"/><a:tblGrid>{grid}</a:tblGrid>{body}</a:tbl></a:graphicData></a:graphic>"
    )
}

fn chart_graphic(rel_id: &str) -> String {
    format!(
        "<p:xfrm><a:off x=\"{MARGIN}\" y=\"{CONTENT_TOP}\"/><a:ext cx=\"{CONTENT_WIDTH}\" cy=\"{CONTENT_HEIGHT}\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:chart xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"{rel_id}\"/></a:graphicData></a:graphic>"
    )
}

fn notes_xml(text: &str) -> String {
    let paragraphs: String = text
        .lines()
        .map(|line| format!("<a:p><a:r><a:rPr lang=\"en-US\"/><a:t>{}</a:t></a:r></a:p>", xml_text(line)))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:notes {NS}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Notes Placeholder 1\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr><p:ph type=\"body\" idx=\"1\"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{paragraphs}</p:txBody></p:sp></p:spTree></p:cSld></p:notes>"
    )
}

fn notes_master() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:notesMaster {NS}><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/></p:notesMaster>"
    )
}

fn fit(width: u32, height: u32) -> (i64, i64, i64, i64) {
    let (w, h) = (i64::from(width.max(1)) * EMU_PER_PIXEL, i64::from(height.max(1)) * EMU_PER_PIXEL);
    let scale = (CONTENT_WIDTH as f64 / w as f64).min(CONTENT_HEIGHT as f64 / h as f64);
    let (cx, cy) = ((w as f64 * scale) as i64, (h as f64 * scale) as i64);
    (MARGIN + (CONTENT_WIDTH - cx) / 2, CONTENT_TOP + (CONTENT_HEIGHT - cy) / 2, cx, cy)
}

struct Deck {
    files: HashMap<String, Vec<u8>>,
    warnings: Vec<String>,
    charts: u32,
    images: u32,
    workbook: Option<WorkbookFn>,
}

impl Deck {
    fn slide(&mut self, number: usize, slide: &Slide) {
        let heading = hex_to_ooxml(defaults::HEADING);
        let muted = hex_to_ooxml(defaults::TEXT_MUTED);
        let text_color = hex_to_ooxml(defaults::TEXT);
        let accent = hex_to_ooxml(defaults::PRIMARY);
        let mut xml = SlideXml::new();
        if slide.layout == SlideLayout::Title {
            xml.rect(0, 0, 228_600, 6_858_000, &accent);
            xml.text(
                "Title",
                (MARGIN + 300_000, 2_150_000, CONTENT_WIDTH - 300_000, 1_500_000),
                std::slice::from_ref(&slide.title),
                44,
                true,
                &heading,
                false,
                "b",
            );
            if !slide.bullets.is_empty() {
                xml.text(
                    "Subtitle",
                    (MARGIN + 300_000, 3_750_000, CONTENT_WIDTH - 300_000, 1_300_000),
                    &slide.bullets,
                    24,
                    false,
                    &muted,
                    false,
                    "t",
                );
            }
        } else {
            xml.text(
                "Title",
                (MARGIN, 380_000, CONTENT_WIDTH, 900_000),
                std::slice::from_ref(&slide.title),
                32,
                true,
                &heading,
                false,
                "ctr",
            );
            xml.rect(MARGIN, 1_300_000, 900_000, 45_720, &accent);
            self.content(&mut xml, number, slide, &text_color);
        }

        let key = |part: &str, ext: &str| format!("ppt/{part}{number}.{ext}");
        if let Some(notes) = slide.notes.as_deref().filter(|n| !n.trim().is_empty()) {
            xml.relate("notesSlide", format!("../notesSlides/notesSlide{number}.xml"));
            self.files
                .insert(key("notesSlides/notesSlide", "xml"), notes_xml(notes).into_bytes());
            let rels = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"{REL_BASE}/notesMaster\" Target=\"../notesMasters/notesMaster1.xml\"/><Relationship Id=\"rId2\" Type=\"{REL_BASE}/slide\" Target=\"../slides/slide{number}.xml\"/></Relationships>"
            );
            self.files
                .insert(format!("ppt/notesSlides/_rels/notesSlide{number}.xml.rels"), rels.into_bytes());
            add_override(
                &mut self.files,
                &format!("ppt/notesSlides/notesSlide{number}.xml"),
                "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml",
            );
        }
        self.files
            .insert(format!("ppt/slides/slide{number}.xml"), xml.xml().into_bytes());
        self.files.insert(
            format!("ppt/slides/_rels/slide{number}.xml.rels"),
            xml.rels_xml().into_bytes(),
        );
        add_override(
            &mut self.files,
            &format!("ppt/slides/slide{number}.xml"),
            "application/vnd.openxmlformats-officedocument.presentationml.slide+xml",
        );
    }

    fn content(&mut self, xml: &mut SlideXml, number: usize, slide: &Slide, text_color: &str) {
        let area = (MARGIN, CONTENT_TOP, CONTENT_WIDTH, CONTENT_HEIGHT);
        let mut bullets_area = area;
        if let Some(bytes) = &slide.image {
            match office_image(bytes) {
                Ok((data, extension, width, height)) => {
                    self.images += 1;
                    let name = format!("image{}.{extension}", self.images);
                    self.files.insert(format!("ppt/media/{name}"), data);
                    ensure_default_content_type(&mut self.files, extension);
                    let rel = xml.relate("image", format!("../media/{name}"));
                    let (x, y, cx, cy) = fit(width, height);
                    xml.picture(&rel, x, y, cx, cy);
                    bullets_area = (MARGIN, CONTENT_TOP, CONTENT_WIDTH, 0);
                }
                Err(error) => self
                    .warnings
                    .push(format!("Slide {number}: the image was skipped: {error}")),
            }
        }
        if let Some((headers, rows)) = &slide.table {
            xml.frame("Table", &table_graphic(headers, rows));
            bullets_area.3 = 0;
        }
        if let Some(chart) = &slide.chart {
            self.charts += 1;
            match add_chart_parts(&mut self.files, "ppt", self.charts, chart, self.workbook) {
                Ok(part) => {
                    let rel = xml.relate("chart", format!("../{part}"));
                    xml.frame("Chart", &chart_graphic(&rel));
                }
                Err(error) => self
                    .warnings
                    .push(format!("Slide {number}: the chart was skipped: {error}")),
            }
            bullets_area.3 = 0;
        }
        if !slide.bullets.is_empty() && bullets_area.3 > 0 {
            xml.text("Content", bullets_area, &slide.bullets, 22, false, text_color, true, "t");
        } else if !slide.bullets.is_empty() {
            self.warnings.push(format!(
                "Slide {number}: bullets are not shown next to an image, table or chart; give them their own slide"
            ));
        }
    }
}

fn register_slides(files: &mut HashMap<String, Vec<u8>>, count: usize, has_notes: bool) {
    let mut ids = String::new();
    let mut rels = String::new();
    let first = if has_notes { 4 } else { 3 };
    for index in 0..count {
        ids.push_str(&format!("<p:sldId id=\"{}\" r:id=\"rId{}\"/>", 256 + index, first + index));
        rels.push_str(&format!(
            "<Relationship Id=\"rId{}\" Type=\"{REL_BASE}/slide\" Target=\"slides/slide{}.xml\"/>",
            first + index,
            index + 1
        ));
    }
    let presentation = String::from_utf8_lossy(&files["ppt/presentation.xml"]).into_owned();
    let notes_list = if has_notes {
        "<p:notesMasterIdLst><p:notesMasterId r:id=\"rId3\"/></p:notesMasterIdLst>"
    } else {
        ""
    };
    let presentation = presentation.replacen(
        "</p:sldMasterIdLst>",
        &format!("</p:sldMasterIdLst>{notes_list}<p:sldIdLst>{ids}</p:sldIdLst>"),
        1,
    );
    files.insert("ppt/presentation.xml".to_string(), presentation.into_bytes());
    if has_notes {
        rels.insert_str(
            0,
            &format!("<Relationship Id=\"rId3\" Type=\"{REL_BASE}/notesMaster\" Target=\"notesMasters/notesMaster1.xml\"/>"),
        );
    }
    let presentation_rels = String::from_utf8_lossy(&files["ppt/_rels/presentation.xml.rels"]).into_owned();
    files.insert(
        "ppt/_rels/presentation.xml.rels".to_string(),
        presentation_rels
            .replacen("</Relationships>", &format!("{rels}</Relationships>"), 1)
            .into_bytes(),
    );
}

fn add_notes_master(files: &mut HashMap<String, Vec<u8>>) {
    files.insert("ppt/notesMasters/notesMaster1.xml".to_string(), notes_master().into_bytes());
    files.insert("ppt/theme/theme2.xml".to_string(), THEME.as_bytes().to_vec());
    add_relationship(
        files,
        "ppt/notesMasters/_rels/notesMaster1.xml.rels",
        "rId1",
        &format!("{REL_BASE}/theme"),
        "../theme/theme2.xml",
    );
    add_override(
        files,
        "ppt/notesMasters/notesMaster1.xml",
        "application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml",
    );
    add_override(
        files,
        "ppt/theme/theme2.xml",
        "application/vnd.openxmlformats-officedocument.theme+xml",
    );
}

fn core_properties(options: &DeckOptions) -> String {
    let field = |tag: &str, value: &Option<String>| {
        value
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .map(|v| format!("<{tag}>{}</{tag}>", xml_text(v)))
            .unwrap_or_default()
    };
    let creator = field("dc:creator", &options.author);
    let creator = if creator.is_empty() { "<dc:creator>Flow Like</dc:creator>".to_string() } else { creator };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">{creator}{}</cp:coreProperties>",
        field("dc:title", &options.title)
    )
}

/// Builds a 16:9 deck in the Flow Like theme. Each slide carries its own layout content.
pub fn pptx_from_slides(options: &DeckOptions, slides: &[Slide]) -> flow_like_types::Result<Built> {
    let mut deck = Deck {
        files: pptx_package(),
        warnings: Vec::new(),
        charts: 0,
        images: 0,
        workbook: options.workbook,
    };
    let has_notes = slides
        .iter()
        .any(|s| s.notes.as_deref().is_some_and(|n| !n.trim().is_empty()));
    if has_notes {
        add_notes_master(&mut deck.files);
    }
    for (index, slide) in slides.iter().enumerate() {
        deck.slide(index + 1, slide);
    }
    register_slides(&mut deck.files, slides.len(), has_notes);
    deck.files
        .insert("docProps/core.xml".to_string(), core_properties(options).into_bytes());
    let app = String::from_utf8_lossy(&deck.files["docProps/app.xml"])
        .replace("<Slides>0</Slides>", &format!("<Slides>{}</Slides>", slides.len()));
    deck.files.insert("docProps/app.xml".to_string(), app.into_bytes());
    Ok(Built {
        bytes: write_zip(&deck.files)?,
        count: Some(slides.len()),
        warnings: deck.warnings,
    })
}

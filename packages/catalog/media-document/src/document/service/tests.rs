use std::collections::HashMap;

use flow_like_types::images::image;

use super::docx::{DocxOptions, docx_from_markdown};
use super::fill::{FillInput, TableRows, TemplateKind, fill_template};
use super::pdf::{PdfOptions, pdf_from_markdown};
use super::pptx::{
    ChartKind, DeckOptions, Slide, SlideChart, SlideLayout, pptx_from_slides,
};
use super::chart::{WorkbookCell, WorkbookFn};
use super::xml_text;
use crate::document::openxml::read_zip;

fn png(color: [u8; 3]) -> Vec<u8> {
    let picture = image::RgbImage::from_pixel(8, 4, image::Rgb(color));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(picture)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

fn text_of(files: &HashMap<String, Vec<u8>>, part: &str) -> String {
    String::from_utf8(files[part].clone()).unwrap()
}

fn assert_well_formed(files: &HashMap<String, Vec<u8>>) {
    for (name, bytes) in files {
        if !(name.ends_with(".xml") || name.ends_with(".rels")) {
            continue;
        }
        let mut reader = quick_xml::Reader::from_reader(bytes.as_slice());
        let mut buffer = Vec::new();
        loop {
            match reader.read_event_into(&mut buffer) {
                Ok(quick_xml::events::Event::Eof) => break,
                Ok(_) => {}
                Err(error) => panic!("{name} is not well-formed XML: {error}"),
            }
            buffer.clear();
        }
    }
}

fn slide(layout: SlideLayout, title: &str) -> Slide {
    Slide {
        layout,
        title: title.to_string(),
        bullets: Vec::new(),
        notes: None,
        image: None,
        table: None,
        chart: None,
    }
}

fn deck() -> Vec<Slide> {
    let mut opening = slide(SlideLayout::Title, "Hello {{who}} & co");
    opening.bullets = vec!["Quarterly review".into()];
    opening.notes = Some("Say hi\nSecond line".into());
    let mut points = slide(SlideLayout::Bullets, "Points");
    points.bullets = vec!["One".into(), "Two <b>".into()];
    let mut picture = slide(SlideLayout::Image, "Picture");
    picture.image = Some(png([200, 10, 10]));
    let mut table = slide(SlideLayout::Table, "Numbers");
    table.table = Some((
        vec!["{{rows}}".into(), "B".into()],
        vec![vec!["1".into(), "2".into()]],
    ));
    let mut chart = slide(SlideLayout::Chart, "Chart");
    chart.chart = Some(SlideChart {
        kind: ChartKind::Bar,
        categories: vec!["Q1".into(), "Q2".into()],
        series: vec![("Sales".into(), vec![1.0, 2.5]), ("Costs".into(), vec![0.5, 1.0])],
        ..SlideChart::default()
    });
    vec![opening, points, picture, table, chart]
}

#[test]
fn pdf_has_pages_and_the_requested_metadata() {
    let markdown = format!("# Title\n\n{}", "A paragraph of text. ".repeat(600));
    let options = PdfOptions {
        title: Some("Report".into()),
        page_numbers: true,
        cover: true,
        page_size: Some("letter".into()),
        ..PdfOptions::default()
    };
    let built = pdf_from_markdown(&markdown, &options, &HashMap::new()).unwrap();
    assert!(built.bytes.starts_with(b"%PDF"));
    let document = lopdf::Document::load_mem(&built.bytes).unwrap();
    assert_eq!(built.count, Some(document.get_pages().len()));
    assert!(document.get_pages().len() > 1);
}

#[test]
fn pdf_with_a_bad_image_warns_instead_of_failing() {
    let images = HashMap::from([("x.png".to_string(), b"not an image".to_vec())]);
    let built = pdf_from_markdown("![x](x.png)\n\ntext", &PdfOptions::default(), &images).unwrap();
    assert_eq!(built.count, Some(1));
    assert_eq!(built.warnings.len(), 1);
}

#[test]
fn docx_keeps_the_block_structure() {
    let markdown = "# Heading One\n\nSome **bold** and *italic* text.\n\n- first\n- second\n\n1. one\n2. two\n\n| Name | Qty |\n|---|---|\n| {{item}} | {{qty}} |\n\n```\nlet x = 1;\n```\n\n> quoted\n\n![logo](logo.png)\n";
    let images = HashMap::from([("logo.png".to_string(), png([0, 0, 255]))]);
    let options = DocxOptions {
        title: Some("My Doc".into()),
        subtitle: Some("Sub".into()),
        author: Some("Ann".into()),
        page_numbers: true,
        page_size: Some("letter".into()),
        ..DocxOptions::default()
    };
    let built = docx_from_markdown(markdown, &options, &images).unwrap();
    let files = read_zip(&built.bytes).unwrap();
    assert_well_formed(&files);
    let document = text_of(&files, "word/document.xml");
    assert!(document.contains("w:val=\"Title\""));
    assert!(document.contains("w:val=\"Heading1\""));
    assert!(document.contains("Heading One"));
    assert!(document.contains("<w:b/>") && document.contains("<w:i/>"));
    assert!(document.contains("w:val=\"ListParagraph\""));
    assert!(document.contains("<w:tbl>") && document.contains("{{item}}"));
    assert!(document.contains("w:val=\"CodeBlock\"") && document.contains("let x = 1;"));
    assert!(document.contains("w:val=\"Quote\""));
    assert!(document.contains("w:w=\"12240\""));
    assert!(document.contains("<w:footerReference"));
    assert!(document.contains("<w:drawing>") && document.contains("name=\"logo\""));
    assert!(document.find("Heading One").unwrap() < document.find("<w:sectPr").unwrap());
    assert!(files.contains_key("word/media/image1.png"));
    assert!(files.contains_key("word/footer1.xml"));
    assert!(text_of(&files, "docProps/core.xml").contains("<dc:creator>Ann</dc:creator>"));
    assert!(text_of(&files, "word/styles.xml").contains("ListParagraph"));
    assert!(built.warnings.is_empty(), "{:?}", built.warnings);
}

#[test]
fn docx_without_the_image_bytes_warns_and_keeps_the_description() {
    let built = docx_from_markdown("![chart](gone.png)", &DocxOptions::default(), &HashMap::new()).unwrap();
    assert_eq!(built.warnings.len(), 1);
    let files = read_zip(&built.bytes).unwrap();
    assert!(text_of(&files, "word/document.xml").contains("[chart]"));
}

#[test]
fn pptx_has_slides_chart_notes_and_valid_parts() {
    let built = pptx_from_slides(
        &DeckOptions {
            title: Some("Deck".into()),
            author: Some("Ann".into()),
            ..DeckOptions::default()
        },
        &deck(),
    )
    .unwrap();
    assert_eq!(built.count, Some(5));
    let files = read_zip(&built.bytes).unwrap();
    assert_well_formed(&files);
    let slides = files
        .keys()
        .filter(|k| k.starts_with("ppt/slides/slide") && k.ends_with(".xml"))
        .count();
    assert_eq!(slides, 5);
    assert_eq!(text_of(&files, "ppt/presentation.xml").matches("<p:sldId ").count(), 5);
    assert!(files.contains_key("ppt/charts/chart1.xml"));
    let chart = text_of(&files, "ppt/charts/chart1.xml");
    assert!(chart.contains("<c:barChart>") && chart.matches("<c:ser>").count() == 2);
    assert!(text_of(&files, "ppt/slides/_rels/slide5.xml.rels").contains("charts/chart1.xml"));
    assert!(text_of(&files, "[Content_Types].xml").contains("/ppt/charts/chart1.xml"));
    assert!(files.contains_key("ppt/media/image1.png"));
    assert!(text_of(&files, "ppt/slides/slide4.xml").contains("<a:tbl>"));
    assert!(text_of(&files, "ppt/notesSlides/notesSlide1.xml").contains("Second line"));
    assert!(text_of(&files, "ppt/slides/slide2.xml").contains("Two &lt;b&gt;"));
    assert!(text_of(&files, "docProps/app.xml").contains("<Slides>5</Slides>"));
}

#[test]
fn fill_replaces_docx_text_rows_and_images() {
    let markdown = "Dear {{name}}, welcome.\n\n![logo](logo.png)\n\n| Item | Qty |\n|---|---|\n| {{item}} | {{qty}} |\n";
    let images = HashMap::from([("logo.png".to_string(), png([0, 0, 255]))]);
    let template = docx_from_markdown(markdown, &DocxOptions::default(), &images).unwrap().bytes;
    assert_eq!(TemplateKind::detect(&template), Some(TemplateKind::Docx));

    let replacement = png([255, 0, 0]);
    let input = FillInput {
        values: vec![("name".into(), "Ada & Bob".into()), ("nothing".into(), "x".into())],
        images: vec![("logo".into(), replacement.clone()), ("ghost".into(), replacement.clone())],
        tables: vec![(
            "item".into(),
            TableRows::Records(vec![
                [("item".to_string(), "Apple".to_string()), ("qty".to_string(), "3".to_string())].into(),
                [("item".to_string(), "Pear".to_string()), ("qty".to_string(), "5".to_string())].into(),
            ]),
        )],
        ..FillInput::default()
    };
    let filled = fill_template(TemplateKind::Docx, &template, &input).unwrap();
    assert_eq!(filled.filled, vec!["name", "logo", "item"]);
    assert_eq!(filled.missing, vec!["nothing", "ghost"]);
    let files = read_zip(&filled.bytes).unwrap();
    assert_well_formed(&files);
    let document = text_of(&files, "word/document.xml");
    assert!(document.contains("Ada &amp; Bob"));
    assert!(!document.contains("{{name}}") && !document.contains("{{item}}"));
    assert!(document.contains("Apple") && document.contains("Pear"));
    assert_eq!(files["word/media/image1.png"], replacement);
}

#[test]
fn fill_replaces_pptx_text_and_table() {
    let template = pptx_from_slides(&DeckOptions::default(), &deck()).unwrap().bytes;
    assert_eq!(TemplateKind::detect(&template), Some(TemplateKind::Pptx));
    let input = FillInput {
        values: vec![("who".into(), "Ada".into())],
        tables: vec![(
            "rows".into(),
            TableRows::Grid(vec![
                vec!["H1".into(), "H2".into()],
                vec!["a".into(), "b".into()],
                vec!["c".into(), "d".into()],
            ]),
        )],
        ..FillInput::default()
    };
    let filled = fill_template(TemplateKind::Pptx, &template, &input).unwrap();
    assert!(filled.missing.is_empty(), "{:?}", filled.missing);
    let files = read_zip(&filled.bytes).unwrap();
    assert_well_formed(&files);
    assert!(text_of(&files, "ppt/slides/slide1.xml").contains("Hello Ada &amp; co"));
    let table = text_of(&files, "ppt/slides/slide4.xml");
    assert!(table.contains(">H1<") && table.contains(">d<") && !table.contains("{{rows}}"));
}

fn form_pdf() -> Vec<u8> {
    use lopdf::{Document, Object, StringFormat, dictionary};
    let mut document = Document::with_version("1.5");
    let pages = document.new_object_id();
    let field = document.add_object(dictionary! {
        "FT" => "Tx",
        "T" => Object::String(b"customer".to_vec(), StringFormat::Literal),
    });
    let acroform = document.add_object(dictionary! { "Fields" => vec![field.into()] });
    let page = document.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 200.into(), 200.into()],
    });
    document.objects.insert(
        pages,
        Object::Dictionary(dictionary! {
            "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1,
        }),
    );
    let catalog = document.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages, "AcroForm" => acroform,
    });
    document.trailer.set("Root", catalog);
    let mut out = Vec::new();
    document.save_to(&mut out).unwrap();
    out
}

#[test]
fn fill_sets_acroform_fields_and_reports_unknown_ones() {
    let template = form_pdf();
    assert_eq!(TemplateKind::detect(&template), Some(TemplateKind::Pdf));
    let input = FillInput {
        values: vec![("customer".into(), "Ada".into()), ("nope".into(), "x".into())],
        ..FillInput::default()
    };
    let filled = fill_template(TemplateKind::Pdf, &template, &input).unwrap();
    assert_eq!(filled.filled, vec!["customer"]);
    assert_eq!(filled.missing, vec!["nope"]);
    let document = lopdf::Document::load_mem(&filled.bytes).unwrap();
    let rendered = String::from_utf8_lossy(&filled.bytes).into_owned();
    assert!(rendered.contains("Ada") && rendered.contains("NeedAppearances"));
    assert_eq!(document.get_pages().len(), 1);
    let images_only = FillInput {
        images: vec![("a".into(), vec![1])],
        ..FillInput::default()
    };
    assert!(fill_template(TemplateKind::Pdf, &template, &images_only).is_err());
}

#[test]
fn xml_text_escapes_markup() {
    assert_eq!(xml_text("a<b&c"), "a&lt;b&amp;c");
}

fn stub_workbook(rows: &[Vec<WorkbookCell>]) -> flow_like_types::Result<Vec<u8>> {
    Ok(format!("XLSX-STUB rows={} first={:?}", rows.len(), rows[0]).into_bytes())
}

const STUB: Option<WorkbookFn> = Some(stub_workbook);

#[test]
fn docx_lists_use_numbering_xml_with_levels_and_restarts() {
    let markdown = "- apple\n  - green\n    - sour\n- pear\n\nIntro\n\n1. one\n2. two\n   1. two-a\n   2. two-b\n\nBreak\n\n3. three\n4. four\n";
    let built = docx_from_markdown(markdown, &DocxOptions::default(), &HashMap::new()).unwrap();
    let files = read_zip(&built.bytes).unwrap();
    assert_well_formed(&files);
    let document = text_of(&files, "word/document.xml");
    assert!(!document.contains('\u{2022}') && !document.contains("<w:tab/>"));
    assert!(document.contains("<w:ilvl w:val=\"2\"/><w:numId w:val=\"1\"/>"));
    assert!(document.contains("<w:ilvl w:val=\"1\"/><w:numId w:val=\"1\"/>"));
    let numbering = text_of(&files, "word/numbering.xml");
    assert_eq!(numbering.matches("<w:abstractNum ").count(), 2);
    assert_eq!(numbering.matches("<w:lvl ").count(), 18);
    assert!(numbering.contains("w:numFmt w:val=\"bullet\""));
    assert!(numbering.contains("w:numFmt w:val=\"decimal\""));
    assert!(numbering.contains("<w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/>"));
    assert_eq!(numbering.matches("<w:startOverride").count(), 3, "{numbering}");
    assert!(numbering.contains("<w:lvlOverride w:ilvl=\"0\">") && numbering.contains("<w:startOverride w:val=\"3\"/>"));
    let first_ordered = document.find("w:numId w:val=\"2\"").unwrap();
    assert!(document[first_ordered..].contains("w:numId w:val=\"4\""), "{document}");
    assert!(text_of(&files, "word/_rels/document.xml.rels").contains("relationships/numbering"));
    assert!(text_of(&files, "[Content_Types].xml").contains("/word/numbering.xml"));
}

#[test]
fn docx_without_lists_has_no_numbering_part() {
    let built = docx_from_markdown("plain", &DocxOptions::default(), &HashMap::new()).unwrap();
    assert!(!read_zip(&built.bytes).unwrap().contains_key("word/numbering.xml"));
}

#[test]
fn docx_charts_are_native_parts_with_an_embedded_workbook() {
    let markdown = "Before\n\n```nivo\ntype: bar\ntitle: Sales\nstacked: true\nlayout: horizontal\n---\nQuarter,Sales,Costs\nQ1,10,4\nQ2,12,5\n```\n\n```plotly\ntype: pie\n---\nPart,Share\nA,60\nB,40\n```\n";
    let options = DocxOptions { workbook: STUB, ..DocxOptions::default() };
    let built = docx_from_markdown(markdown, &options, &HashMap::new()).unwrap();
    assert!(built.warnings.is_empty(), "{:?}", built.warnings);
    let files = read_zip(&built.bytes).unwrap();
    assert_well_formed(&files);
    for n in [1, 2] {
        assert!(files.contains_key(&format!("word/charts/chart{n}.xml")));
        assert!(files.contains_key(&format!("word/embeddings/Microsoft_Excel_Sheet{n}.xlsx")));
        let rels = text_of(&files, &format!("word/charts/_rels/chart{n}.xml.rels"));
        assert!(rels.contains("relationships/package") && rels.contains(&format!("../embeddings/Microsoft_Excel_Sheet{n}.xlsx")));
    }
    let chart = text_of(&files, "word/charts/chart1.xml");
    assert!(chart.contains("<c:barDir val=\"bar\"/>") && chart.contains("<c:grouping val=\"stacked\"/>"));
    assert!(chart.contains("Sheet1!$B$1") && chart.contains("Sheet1!$C$2:$C$3"));
    assert!(chart.contains("<c:externalData r:id=\"rId1\">") && chart.contains(">Sales<"));
    assert!(text_of(&files, "word/charts/chart2.xml").contains("<c:pieChart>"));
    assert!(String::from_utf8_lossy(&files["word/embeddings/Microsoft_Excel_Sheet1.xlsx"]).contains("rows=3"));
    let document = text_of(&files, "word/document.xml");
    assert_eq!(document.matches("<c:chart ").count(), 2);
    let rels = text_of(&files, "word/_rels/document.xml.rels");
    assert!(rels.contains("relationships/chart\"") && rels.contains("charts/chart2.xml"));
    let types = text_of(&files, "[Content_Types].xml");
    assert!(types.contains("/word/charts/chart1.xml") && types.contains("Extension=\"xlsx\""));
}

#[test]
fn docx_charts_without_a_workbook_use_literal_data() {
    let markdown = "```nivo\ntype: line\n---\nX,Y\na,1\nb,2\n```\n";
    let built = docx_from_markdown(markdown, &DocxOptions::default(), &HashMap::new()).unwrap();
    let files = read_zip(&built.bytes).unwrap();
    assert!(!files.keys().any(|k| k.contains("embeddings")));
    let chart = text_of(&files, "word/charts/chart1.xml");
    assert!(chart.contains("<c:numLit>") && !chart.contains("externalData"));
}

#[test]
fn docx_charts_that_cannot_be_native_fall_back_to_a_table_with_a_warning() {
    let markdown = "```nivo\ntype: scatter\ntitle: Dots\n---\nX,Y\n1,2\n3,4\n```\n\n```nivo\n{\"data\": []}\n```\n";
    let built = docx_from_markdown(markdown, &DocxOptions::default(), &HashMap::new()).unwrap();
    assert_eq!(built.warnings.len(), 2, "{:?}", built.warnings);
    assert!(built.warnings[0].contains("scatter") && built.warnings[0].contains("table"));
    let files = read_zip(&built.bytes).unwrap();
    assert!(!files.keys().any(|k| k.starts_with("word/charts")));
    let document = text_of(&files, "word/document.xml");
    assert!(document.contains("<w:tbl>") && document.contains("Dots"));
    assert!(document.contains("&quot;data&quot;") || document.contains("\"data\""));
}

#[test]
fn pptx_charts_embed_their_data_when_a_workbook_builder_is_given() {
    let options = DeckOptions { workbook: STUB, ..DeckOptions::default() };
    let built = pptx_from_slides(&options, &deck()).unwrap();
    let files = read_zip(&built.bytes).unwrap();
    assert_well_formed(&files);
    assert!(files.contains_key("ppt/embeddings/Microsoft_Excel_Sheet1.xlsx"));
    let rels = text_of(&files, "ppt/charts/_rels/chart1.xml.rels");
    assert!(rels.contains("relationships/package") && rels.contains("../embeddings/Microsoft_Excel_Sheet1.xlsx"));
    let chart = text_of(&files, "ppt/charts/chart1.xml");
    assert!(chart.contains("Sheet1!$B$1") && chart.contains("Sheet1!$C$2:$C$3") && chart.contains("<c:externalData"));
    assert!(text_of(&files, "[Content_Types].xml").contains("Extension=\"xlsx\""));
    assert!(String::from_utf8_lossy(&files["ppt/embeddings/Microsoft_Excel_Sheet1.xlsx"]).contains("rows=3"));

    let plain = pptx_from_slides(&DeckOptions::default(), &deck()).unwrap();
    let plain = read_zip(&plain.bytes).unwrap();
    assert!(!plain.keys().any(|k| k.contains("embeddings")));
    assert!(text_of(&plain, "ppt/charts/chart1.xml").contains("<c:numLit>"));
}

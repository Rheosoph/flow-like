use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use crate::document::openxml::write_zip;
use crate::document::styles::defaults;
#[cfg(feature = "execute")]
use crate::document::styles::{hex_to_ooxml, pt_to_half_points, cm_to_twips};

#[crate::register_node]
#[derive(Default)]
pub struct DocxCreateNode;

impl DocxCreateNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DocxCreateNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "docx_create",
            "Create Document",
            "Create an empty DOCX with Flow Like branded theme (styled headings, Calibri font, modern spacing)",
            "Document/DOCX",
        );
        node.add_icon("/flow/icons/text.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "font_family",
            "Font Family",
            "Default body font",
            VariableType::String,
        )
        .set_default_value(Some(json!("Calibri")));
        node.add_input_pin(
            "font_size",
            "Font Size",
            "Body font size in points",
            VariableType::Float,
        )
        .set_default_value(Some(json!(11.0)));
        node.add_input_pin(
            "theme_color",
            "Theme Color",
            "Accent color for headings (hex)",
            VariableType::String,
        )
        .set_default_value(Some(json!(defaults::PRIMARY)));
        node.add_input_pin(
            "output",
            "Output Path",
            "Where to save the DOCX file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("exec_out", "Done", "Continues", VariableType::Execution);
        node.add_output_pin("result", "Result", "Output file path", VariableType::Struct)
            .set_schema::<FlowPath>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let font_family: String = context.evaluate_pin("font_family").await?;
        let font_size: f64 = context.evaluate_pin("font_size").await?;
        let theme_color: String = context.evaluate_pin("theme_color").await?;
        let output: FlowPath = context.evaluate_pin("output").await?;

        let accent = hex_to_ooxml(&theme_color);
        let half_pts = pt_to_half_points(font_size as f32);
        let heading_color = hex_to_ooxml(defaults::HEADING);

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

        let margin_twips = cm_to_twips(defaults::MARGIN_CM);
        let document = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:wpc="http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:wp14="http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:w10="urn:schemas-microsoft-com:office:word" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup" xmlns:wpi="http://schemas.microsoft.com/office/word/2010/wordprocessingInk" xmlns:wne="http://schemas.microsoft.com/office/word/2006/wordml" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" mc:Ignorable="w14 w15 wp14">
<w:body>
<w:sectPr>
<w:pgSz w:w="11906" w:h="16838"/>
<w:pgMar w:top="{margin_twips}" w:right="{margin_twips}" w:bottom="{margin_twips}" w:left="{margin_twips}" w:header="720" w:footer="720" w:gutter="0"/>
</w:sectPr>
</w:body>
</w:document>"#
        );

        let styles = format!(
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
            font = quick_xml::escape::escape(&font_family),
            sz = half_pts,
            text = hex_to_ooxml(defaults::TEXT),
            heading = heading_color,
            accent = accent,
            muted = hex_to_ooxml(defaults::TEXT_MUTED),
        );

        let core = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
</cp:coreProperties>"#;

        let mut files = std::collections::HashMap::new();
        files.insert("[Content_Types].xml".to_string(), content_types.as_bytes().to_vec());
        files.insert("_rels/.rels".to_string(), rels.as_bytes().to_vec());
        files.insert("word/_rels/document.xml.rels".to_string(), word_rels.as_bytes().to_vec());
        files.insert("word/document.xml".to_string(), document.into_bytes());
        files.insert("word/styles.xml".to_string(), styles.into_bytes());
        files.insert("docProps/core.xml".to_string(), core.as_bytes().to_vec());

        let result_bytes = write_zip(&files)?;
        output.put(context, result_bytes, false).await?;
        context.set_pin_value("result", json!(output)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!("Requires the 'execute' feature"))
    }
}

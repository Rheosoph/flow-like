#[cfg(feature = "execute")]
mod kind;
#[cfg(feature = "execute")]
mod office;
#[cfg(feature = "execute")]
mod pdf;

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::async_trait;

#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
#[cfg(feature = "execute")]
use flow_like_catalog_core::{FilePrefix, PROBE_PREFIX_BYTES, read_image_header};
#[cfg(feature = "execute")]
use flow_like_types::{Value, image::guess_format, json::json};
#[cfg(feature = "execute")]
use kind::{FileKind, detect_file_type, is_ooxml};

const DETAILS_DESCRIPTION: &str = "Metadata the file's format carries. image: width, height (upright, EXIF orientation applied), format, orientation. pdf: page_count, pdf_version, encrypted and whichever Info fields exist (title, author, subject, keywords, creator, producer, creation_date, modification_date as ISO 8601). office (docx, xlsx, pptx): title, creator, subject, keywords, created, modified, last_modified_by, pages, words, slides when present. Empty for every other kind or when the format cannot be parsed.";

#[crate::register_node]
#[derive(Default)]
pub struct FileGetMetadataNode;

impl FileGetMetadataNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for FileGetMetadataNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "file_get_metadata",
            "Get File Metadata",
            "Reports a file's name, size, timestamps, media type and kind, plus what its format says about it: dimensions for images, pages and document properties for PDFs and Office files. Type detection and image headers use ranged reads of the first bytes only; PDFs and Office files are read whole.",
            "Data/Files/Operations",
        );
        node.set_flowscript_name("files", "getMetadata");
        node.set_receiver("path");
        node.add_icon("/flow/icons/path.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(8)
                .set_governance(9)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );
        node.add_input_pin("path", "Path", "File to inspect", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );
        node.add_output_pin(
            "name",
            "Name",
            "File name including its extension",
            VariableType::String,
        );
        node.add_output_pin(
            "extension",
            "Extension",
            "Text after the last dot of the name, without the dot; empty when there is none",
            VariableType::String,
        );
        node.add_output_pin("size", "Size", "Size in bytes", VariableType::Integer);
        node.add_output_pin(
            "last_modified",
            "Last Modified",
            "When the file was last written",
            VariableType::Date,
        );
        node.add_output_pin(
            "e_tag",
            "ETag",
            "Entity tag the store reports; it changes whenever the content changes",
            VariableType::String,
        );
        node.add_output_pin(
            "mime_type",
            "MIME Type",
            "Media type detected from the file's leading bytes, falling back to its extension",
            VariableType::String,
        );
        node.add_output_pin(
            "kind",
            "Kind",
            "One of image, pdf, office, audio, video, archive, text or other",
            VariableType::String,
        );
        node.add_output_pin(
            "details",
            "Details",
            DETAILS_DESCRIPTION,
            VariableType::Struct,
        )
        .set_open_schema();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let path: FlowPath = context.evaluate_pin("path").await?;

        let mut file = FilePrefix::open(&path, context).await?;
        file.fill_to(PROBE_PREFIX_BYTES).await?;
        let name = file.file_name().unwrap_or_default();
        let extension = name
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_string())
            .unwrap_or_default();
        let (mime_type, kind) = detect_file_type(file.bytes(), &name);

        let details = match read_details(&mut file, kind, mime_type).await {
            Ok(details) => details,
            Err(error) => {
                context.log_message(
                    &format!(
                        "Leaving the {} details of '{}' empty: {error}",
                        kind.as_str(),
                        path.path
                    ),
                    LogLevel::Warn,
                );
                json!({})
            }
        };
        context.log_message(
            &format!(
                "Read {} of {} bytes of '{}' for its metadata",
                file.bytes().len(),
                file.meta().size,
                path.path
            ),
            LogLevel::Debug,
        );

        let meta = file.meta();
        let size = meta.size as i64;
        let last_modified = meta.last_modified;
        let e_tag = meta.e_tag.clone().unwrap_or_default();

        context.set_pin_value("name", json!(name)).await?;
        context.set_pin_value("extension", json!(extension)).await?;
        context.set_pin_value("size", json!(size)).await?;
        context
            .set_pin_value("last_modified", json!(last_modified))
            .await?;
        context.set_pin_value("e_tag", json!(e_tag)).await?;
        context.set_pin_value("mime_type", json!(mime_type)).await?;
        context.set_pin_value("kind", json!(kind.as_str())).await?;
        context.set_pin_value("details", details).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Get File Metadata requires the 'execute' feature"
        ))
    }
}

/// Only images whose leading bytes name a raster format get a header read; PDFs and Office
/// packages are fetched whole because their metadata sits behind cross-references.
#[cfg(feature = "execute")]
async fn read_details(
    file: &mut FilePrefix,
    kind: FileKind,
    mime_type: &str,
) -> flow_like_types::Result<Value> {
    Ok(match kind {
        FileKind::Image if guess_format(file.bytes()).is_ok() => {
            json!(read_image_header(file, true).await?)
        }
        FileKind::Pdf => json!(pdf::pdf_details(file.fill_all().await?)?),
        FileKind::Office if is_ooxml(mime_type) && file.bytes().starts_with(b"PK\x03\x04") => {
            json!(office::office_details(file.fill_all().await?)?)
        }
        _ => json!({}),
    })
}

#[cfg(test)]
mod tests {
    use super::FileGetMetadataNode;
    use flow_like::flow::{node::NodeLogic, pin::OPEN_OBJECT_SCHEMA, variable::VariableType};

    #[test]
    fn node_declares_generic_and_kind_specific_outputs() {
        let node = FileGetMetadataNode::new().get_node();
        assert_eq!(node.name, "file_get_metadata");
        for (name, data_type) in [
            ("name", VariableType::String),
            ("extension", VariableType::String),
            ("size", VariableType::Integer),
            ("last_modified", VariableType::Date),
            ("e_tag", VariableType::String),
            ("mime_type", VariableType::String),
            ("kind", VariableType::String),
            ("details", VariableType::Struct),
        ] {
            assert_eq!(node.get_pin_by_name(name).unwrap().data_type, data_type);
        }
        let details = node.get_pin_by_name("details").unwrap();
        assert_eq!(details.schema.as_deref(), Some(OPEN_OBJECT_SCHEMA));
    }
}

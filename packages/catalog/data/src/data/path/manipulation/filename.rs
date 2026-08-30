use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct FilenameNode {}

impl FilenameNode {
    pub fn new() -> Self {
        FilenameNode {}
    }
}

#[async_trait]
impl NodeLogic for FilenameNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "filename",
            "Filename",
            "Gets the filename from a path",
            "Data/Files/Path",
        );
        node.set_flowscript_name("path", "filename");
        node.set_receiver("path");
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin("path", "Path", "FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "remove_extension",
            "Remove Extension",
            "Remove Extension from the Path",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("filename", "Filename", "Filename", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let path: FlowPath = context.evaluate_pin("path").await?;
        let remove_extension: bool = context.evaluate_pin("remove_extension").await?;

        let pb = std::path::PathBuf::from(&path.path);

        let filename = if remove_extension {
            pb.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            pb.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        };

        context.set_pin_value("filename", json!(filename)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct SetFilenameNode {}

impl SetFilenameNode {
    pub fn new() -> Self {
        SetFilenameNode {}
    }
}

#[async_trait]
impl NodeLogic for SetFilenameNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "set_filename",
            "Set Filename",
            "Renames a file path, keeping the original extension when the new name has none",
            "Data/Files/Path",
        );
        node.set_flowscript_name("path", "setFilename");
        node.set_receiver("in_path");
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin("in_path", "Path", "FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("filename", "Filename", "Filename", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_output_pin("out_path", "Path", "FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut path: FlowPath = context.evaluate_pin("in_path").await?;
        let filename: String = context.evaluate_pin("filename").await?;

        if filename.is_empty() {
            context.set_pin_value("out_path", json!(path)).await?;
            return Ok(());
        }

        let mut pb = std::path::PathBuf::from(&path.path);

        // If the supplied filename already contains an extension, use it as-is.
        // Otherwise keep the original path's extension (if any).
        let new_name = if std::path::Path::new(&filename).extension().is_some() {
            filename
        } else if let Some(orig_ext) = pb.extension() {
            if orig_ext.is_empty() {
                filename
            } else {
                format!("{}.{}", filename, orig_ext.to_string_lossy())
            }
        } else {
            filename
        };

        pb.set_file_name(new_name);
        path.path = pb.to_string_lossy().into_owned();

        context.set_pin_value("out_path", json!(path)).await?;
        Ok(())
    }
}

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{
    JsonSchema, async_trait,
    json::{Deserialize, Serialize, json},
};

use crate::data::path::FlowPath;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TdmsChannelInfo {
    pub name: String,
    pub group: String,
    pub data_type: String,
    pub properties: Vec<TdmsPropertyInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TdmsPropertyInfo {
    pub name: String,
    pub data_type: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TdmsGroupInfo {
    pub name: String,
    pub channels: Vec<TdmsChannelInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TdmsFileMetadata {
    pub groups: Vec<TdmsGroupInfo>,
    pub segment_count: u64,
}

#[crate::register_node]
#[derive(Default)]
pub struct TdmsMetadataNode {}

impl TdmsMetadataNode {
    pub fn new() -> Self {
        TdmsMetadataNode {}
    }
}

#[cfg(feature = "execute")]
fn format_data_type(dt: &tdms_rs::DataType) -> String {
    format!("{:?}", dt)
}

#[cfg(feature = "execute")]
fn property_value_type(value: &tdms_rs::PropertyValue) -> String {
    match value {
        tdms_rs::PropertyValue::I8(_) => "I8".to_string(),
        tdms_rs::PropertyValue::I16(_) => "I16".to_string(),
        tdms_rs::PropertyValue::I32(_) => "I32".to_string(),
        tdms_rs::PropertyValue::I64(_) => "I64".to_string(),
        tdms_rs::PropertyValue::U8(_) => "U8".to_string(),
        tdms_rs::PropertyValue::U16(_) => "U16".to_string(),
        tdms_rs::PropertyValue::U32(_) => "U32".to_string(),
        tdms_rs::PropertyValue::U64(_) => "U64".to_string(),
        tdms_rs::PropertyValue::Float(_) => "Float".to_string(),
        tdms_rs::PropertyValue::Double(_) => "Double".to_string(),
        tdms_rs::PropertyValue::String(_) => "String".to_string(),
        tdms_rs::PropertyValue::Boolean(_) => "Boolean".to_string(),
        tdms_rs::PropertyValue::TimeStamp(_) => "TimeStamp".to_string(),
    }
}

#[cfg(feature = "execute")]
fn format_value(val: &tdms_rs::PropertyValue) -> String {
    format!("{}", val)
}

#[async_trait]
impl NodeLogic for TdmsMetadataNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "tdms_metadata",
            "TDMS Metadata",
            "Extracts metadata (groups, channels, properties) from a LabVIEW TDMS file.",
            "Data/TDMS",
        );
        node.set_flowscript_name("data", "tdmsMetadata");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "tdms_path",
            "TDMS File",
            "Path to the TDMS file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Finished extracting metadata",
            VariableType::Execution,
        );
        node.add_output_pin(
            "metadata",
            "Metadata",
            "TDMS file metadata struct",
            VariableType::Struct,
        )
        .set_schema::<TdmsFileMetadata>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let tdms_path: FlowPath = context.evaluate_pin("tdms_path").await?;
        let bytes = tdms_path.get(context, false).await?;

        let tmp_file = tempfile::NamedTempFile::new()?;
        std::fs::write(tmp_file.path(), &bytes)?;

        let file = tdms_rs::TdmsFile::open(tmp_file.path())
            .map_err(|e| flow_like_types::anyhow!("Failed to parse TDMS file: {:?}", e))?;

        let mut groups = Vec::new();

        for group in file.groups() {
            let group_name = group.name().to_string();
            let mut channels = Vec::new();

            for channel in group.channels() {
                let properties: Vec<TdmsPropertyInfo> = channel
                    .properties()
                    .map(|(name, value)| TdmsPropertyInfo {
                        name: name.to_string(),
                        data_type: property_value_type(value),
                        value: format_value(value),
                    })
                    .collect();

                channels.push(TdmsChannelInfo {
                    name: channel.name().to_string(),
                    group: group_name.clone(),
                    data_type: format_data_type(&channel.dtype()),
                    properties,
                });
            }

            groups.push(TdmsGroupInfo {
                name: group_name,
                channels,
            });
        }

        let metadata = TdmsFileMetadata {
            segment_count: file.segment_count() as u64,
            groups,
        };

        context.set_pin_value("metadata", json!(metadata)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "TDMS parsing requires the 'execute' feature"
        ))
    }
}

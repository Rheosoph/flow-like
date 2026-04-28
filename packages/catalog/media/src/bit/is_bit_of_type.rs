use flow_like::{
    bit::{Bit, BitTypes},
    flow::{
        execution::{LogLevel, context::ExecutionContext},
        node::{Node, NodeLogic},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_types::{
    Result, async_trait,
    json::{from_str, json},
};

#[crate::register_node]
#[derive(Default)]
pub struct IsBitOfTypeNode {}

impl IsBitOfTypeNode {
    pub fn new() -> Self {
        IsBitOfTypeNode {}
    }
}

#[async_trait]
impl NodeLogic for IsBitOfTypeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "is_bit_of_type",
            "Is Bit of Type",
            "Checks if the Bit is of the specified type and branches the execution flow accordingly.",
            "Bit",
        );
        node.add_icon("/flow/icons/bit.svg");
        node.set_version(1);

        // Input Pins
        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );
        node.add_input_pin("bit", "Bit", "Input Bit", VariableType::Struct)
            .set_schema::<Bit>();
        node.add_input_pin(
            "bit_type",
            "Bit Type",
            "Type to check (e.g., \"Llm\", \"Vlm\")",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Llm".to_string(),
                    "Vlm".to_string(),
                    "Embedding".to_string(),
                    "ImageEmbedding".to_string(),
                    "File".to_string(),
                    "Media".to_string(),
                    "ImageGeneration".to_string(),
                    "VideoGeneration".to_string(),
                    "Template".to_string(),
                    "Tokenizer".to_string(),
                    "TokenizerConfig".to_string(),
                    "SpecialTokensMap".to_string(),
                    "Config".to_string(),
                    "Course".to_string(),
                    "PreprocessorConfig".to_string(),
                    "Projection".to_string(),
                    "Project".to_string(),
                    "Board".to_string(),
                    "Other".to_string(),
                ])
                .build(),
        );

        // Output Pins
        node.add_output_pin("bit_out", "Bit", "Output Bit", VariableType::Struct)
            .set_schema::<Bit>();
        node.add_output_pin(
            "yes",
            "Yes",
            "Execution if Bit is of the specified type",
            VariableType::Execution,
        );
        node.add_output_pin(
            "no",
            "No",
            "Execution if Bit is not of the specified type",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("yes").await?;
        context.deactivate_exec_pin("no").await?;

        let bit: Bit = context.evaluate_pin("bit").await?;
        let bit_type_str: String = context.evaluate_pin("bit_type").await?;

        let bit_type = from_str::<BitTypes>(&format!("\"{}\"", bit_type_str));

        if bit_type.is_err() {
            context.log_message(
                &format!("Invalid Bit Type: {}", bit_type_str),
                LogLevel::Error,
            );
            context.activate_exec_pin("no").await?;
            return Ok(());
        }

        let bit_type = bit_type.unwrap();

        context.set_pin_value("bit_out", json!(bit)).await?;

        if bit.bit_type == bit_type {
            context.activate_exec_pin("yes").await?;
        } else {
            context.activate_exec_pin("no").await?;
        }

        Ok(())
    }
}

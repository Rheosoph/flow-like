use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

pub fn bytes_input(node: &mut Node, name: &str, label: &str, description: &str) {
    node.add_input_pin(name, label, description, VariableType::Byte)
        .set_value_type(ValueType::Array);
}

pub fn bytes_output(node: &mut Node, name: &str, label: &str, description: &str) {
    node.add_output_pin(name, label, description, VariableType::Byte)
        .set_value_type(ValueType::Array);
}

pub fn bytes_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils/Bytes");
    node.add_icon("/flow/icons/box.svg");
    node.set_scores(pure_scores());
    node
}

#[crate::register_node]
#[derive(Default)]
pub struct BytesLengthNode {}

impl BytesLengthNode {
    pub fn new() -> Self {
        BytesLengthNode {}
    }
}

#[async_trait]
impl NodeLogic for BytesLengthNode {
    fn get_node(&self) -> Node {
        let mut node = bytes_node(
            "bytes_length",
            "Byte Length",
            "How many bytes the buffer holds",
        );
        node.set_flowscript_name("bytes", "length");
        node.set_receiver("bytes");
        bytes_input(&mut node, "bytes", "Bytes", "Input Bytes");
        node.add_output_pin("length", "Length", "Number of bytes", VariableType::Integer);
        node.add_output_pin(
            "is_empty",
            "Is Empty",
            "True when the buffer holds nothing",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let bytes: Vec<u8> = context.evaluate_pin("bytes").await?;
        context
            .set_pin_value("length", json!(bytes.len() as i64))
            .await?;
        context
            .set_pin_value("is_empty", json!(bytes.is_empty()))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BytesConcatNode {}

impl BytesConcatNode {
    pub fn new() -> Self {
        BytesConcatNode {}
    }
}

#[async_trait]
impl NodeLogic for BytesConcatNode {
    fn get_node(&self) -> Node {
        let mut node = bytes_node(
            "bytes_concat",
            "Concat Bytes",
            "Appends byte buffers to each other",
        );
        node.set_flowscript_name("bytes", "concat");
        node.set_receiver("bytes");
        bytes_input(&mut node, "bytes", "Bytes", "Part to append");
        bytes_input(&mut node, "bytes", "Bytes", "Part to append");
        bytes_output(&mut node, "result", "Bytes", "All parts appended in order");

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let pins = context.get_pins_by_name("bytes").await?;

        let mut result: Vec<u8> = Vec::new();
        for pin in pins {
            let part: Vec<u8> = context.evaluate_pin_ref(pin).await?;
            result.extend(part);
        }

        context.set_pin_value("result", json!(result)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BytesSliceNode {}

impl BytesSliceNode {
    pub fn new() -> Self {
        BytesSliceNode {}
    }
}

#[async_trait]
impl NodeLogic for BytesSliceNode {
    fn get_node(&self) -> Node {
        let mut node = bytes_node(
            "bytes_slice",
            "Slice Bytes",
            "Takes a range out of a byte buffer",
        );
        node.set_flowscript_name("bytes", "slice");
        node.set_receiver("bytes");
        bytes_input(&mut node, "bytes", "Bytes", "Input Bytes");
        node.add_input_pin(
            "start",
            "Start",
            "First byte index, negative counts from the end",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "length",
            "Length",
            "Number of bytes to take, -1 for the rest",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));
        bytes_output(&mut node, "result", "Bytes", "The selected bytes");

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let bytes: Vec<u8> = context.evaluate_pin("bytes").await?;
        let start: i64 = context.evaluate_pin("start").await?;
        let length: i64 = context.evaluate_pin("length").await?;

        let total = bytes.len() as i64;
        let start_index = if start < 0 {
            (total + start).max(0)
        } else {
            start.min(total)
        };
        let take = if length < 0 {
            total - start_index
        } else {
            length.min(total - start_index)
        };

        let sliced: Vec<u8> = bytes
            .into_iter()
            .skip(start_index.max(0) as usize)
            .take(take.max(0) as usize)
            .collect();

        context.set_pin_value("result", json!(sliced)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BytesEqualNode {}

impl BytesEqualNode {
    pub fn new() -> Self {
        BytesEqualNode {}
    }
}

#[async_trait]
impl NodeLogic for BytesEqualNode {
    fn get_node(&self) -> Node {
        let mut node = bytes_node(
            "bytes_equal",
            "== (Bytes)",
            "Compares two byte buffers for equality",
        );
        node.set_flowscript_name("bytes", "equal");
        node.set_receiver("bytes");
        bytes_input(&mut node, "bytes", "Bytes", "Input Bytes");
        bytes_input(&mut node, "other", "Other", "Input Bytes");
        node.add_output_pin(
            "equal",
            "Is Equal?",
            "True when both buffers hold the same bytes",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let bytes: Vec<u8> = context.evaluate_pin("bytes").await?;
        let other: Vec<u8> = context.evaluate_pin("other").await?;
        context
            .set_pin_value("equal", json!(bytes == other))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BytesStartsWithNode {}

impl BytesStartsWithNode {
    pub fn new() -> Self {
        BytesStartsWithNode {}
    }
}

#[async_trait]
impl NodeLogic for BytesStartsWithNode {
    fn get_node(&self) -> Node {
        let mut node = bytes_node(
            "bytes_starts_with",
            "Starts With (Bytes)",
            "Checks a buffer against a leading byte sequence, for example a file signature",
        );
        node.set_flowscript_name("bytes", "startsWith");
        node.set_receiver("bytes");
        bytes_input(&mut node, "bytes", "Bytes", "Input Bytes");
        bytes_input(&mut node, "prefix", "Prefix", "Bytes to look for");
        node.add_output_pin(
            "starts_with",
            "Starts With?",
            "True when the buffer begins with the prefix",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let bytes: Vec<u8> = context.evaluate_pin("bytes").await?;
        let prefix: Vec<u8> = context.evaluate_pin("prefix").await?;
        context
            .set_pin_value(
                "starts_with",
                json!(!prefix.is_empty() && bytes.starts_with(&prefix)),
            )
            .await?;
        Ok(())
    }
}

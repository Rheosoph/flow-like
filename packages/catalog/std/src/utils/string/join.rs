use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct StringJoinNode {}

impl StringJoinNode {
    pub fn new() -> Self {
        StringJoinNode {}
    }
}

#[async_trait]
impl NodeLogic for StringJoinNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_join",
            "Join Strings",
            "Joins multiple strings together",
            "Utils/String",
        );
        node.set_flowscript_name("string", "join");
        node.set_receiver("strings");
        node.add_icon("/flow/icons/string.svg");

        node.add_input_pin(
            "strings",
            "Strings",
            "Strings to join",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node.add_input_pin(
            "separator",
            "Separator",
            "String to separate by",
            VariableType::String,
        );

        node.add_output_pin(
            "joined_string",
            "Joined String",
            "Concatenated string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let strings: Vec<String> = context.evaluate_pin("strings").await?;
        let separator: String = context.evaluate_pin("separator").await?;

        let joined_string = strings.join(&separator);

        context
            .set_pin_value("joined_string", json!(joined_string))
            .await?;
        Ok(())
    }
}

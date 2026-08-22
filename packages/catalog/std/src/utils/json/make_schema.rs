use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json};

#[crate::register_node]
#[derive(Default)]
pub struct SchemaFromExample {}

impl SchemaFromExample {
    pub fn new() -> Self {
        SchemaFromExample {}
    }
}

#[async_trait]
impl NodeLogic for SchemaFromExample {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_json_make_schema",
            "Make Tool Schema",
            "Generate Tool Definitions for Tool Calls",
            "Utils/JSON",
        );
        node.add_icon("/flow/icons/repair.svg");

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin(
            "example_json",
            "Example JSON",
            "Example JSON to infer schema from",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Execution Output",
            "Execution Output",
            VariableType::Execution,
        );

        node.add_output_pin(
            "schema",
            "Schema",
            "Generated JSON Schema / Tool Definition",
            VariableType::Struct,
        )
        .set_open_schema();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        // fetch inputs
        context.deactivate_exec_pin("exec_out").await?;
        let example_json: String = context.evaluate_pin("example_json").await?;

        let example_json: Value = json::from_str(&example_json)?;
        let schema = schemars::schema_for_value!(&example_json);
        let schema = json::to_string_pretty(&schema)?;
        let schema = json::from_str::<Value>(&schema)?;

        context.set_pin_value("schema", schema).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn uuid_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils");
    node.add_icon("/flow/icons/hash.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("exec_in", "In", "Trigger Pin", VariableType::Execution);
    node.add_input_pin(
        "uppercase",
        "Uppercase",
        "Write the hex digits in upper case",
        VariableType::Boolean,
    )
    .set_default_value(Some(json!(false)));

    node.add_output_pin("exec_out", "Out", "", VariableType::Execution);
    node.add_output_pin("uuid", "UUID", description, VariableType::String);

    node
}

async fn emit(context: &mut ExecutionContext, uuid: uuid::Uuid) -> flow_like_types::Result<()> {
    context.deactivate_exec_pin("exec_out").await?;
    let uppercase: bool = context.evaluate_pin("uppercase").await?;

    let text = if uppercase {
        uuid.to_string().to_uppercase()
    } else {
        uuid.to_string()
    };

    context.set_pin_value("uuid", json!(text)).await?;
    context.activate_exec_pin("exec_out").await?;
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct UuidV4Node {}

impl UuidV4Node {
    pub fn new() -> Self {
        UuidV4Node {}
    }
}

#[async_trait]
impl NodeLogic for UuidV4Node {
    fn get_node(&self) -> Node {
        let mut node = uuid_node("uuid_v4", "UUID v4", "A random identifier");
        node.set_flowscript_name("random", "uuidV4");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        emit(context, uuid::Uuid::new_v4()).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct UuidV7Node {}

impl UuidV7Node {
    pub fn new() -> Self {
        UuidV7Node {}
    }
}

#[async_trait]
impl NodeLogic for UuidV7Node {
    fn get_node(&self) -> Node {
        let mut node = uuid_node(
            "uuid_v7",
            "UUID v7",
            "A time ordered identifier — sorts by creation time, which keeps database indexes tidy",
        );
        node.set_flowscript_name("random", "uuidV7");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        emit(context, uuid::Uuid::now_v7()).await
    }
}

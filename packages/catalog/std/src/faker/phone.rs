use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use fake::{Fake, faker::phone_number::en::*};

#[crate::register_node]
#[derive(Default)]
pub struct FakePhoneNumber;

#[async_trait]
impl NodeLogic for FakePhoneNumber {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_phone_number",
            "Fake Phone Number",
            "Generates a random phone number for mocking data",
            "Utils/Faker/Phone",
        );
        node.set_flowscript_name("faker.phone", "number");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "phone",
            "Phone Number",
            "Generated phone number",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let phone: String = PhoneNumber().fake();
        context.set_pin_value("phone", json!(phone)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FakeCellNumber;

#[async_trait]
impl NodeLogic for FakeCellNumber {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_cell_number",
            "Fake Cell Number",
            "Generates a random cell/mobile phone number for mocking data",
            "Utils/Faker/Phone",
        );
        node.set_flowscript_name("faker.phone", "cellNumber");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "phone",
            "Cell Number",
            "Generated cell number",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let phone: String = CellNumber().fake();
        context.set_pin_value("phone", json!(phone)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

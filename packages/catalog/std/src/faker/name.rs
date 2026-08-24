use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use fake::{Fake, faker::name::en::*};

#[crate::register_node]
#[derive(Default)]
pub struct FakeFirstName;

#[async_trait]
impl NodeLogic for FakeFirstName {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_first_name",
            "Fake First Name",
            "Generates a random first name for mocking data",
            "Utils/Faker/Name",
        );
        node.set_flowscript_name("faker.name", "firstName");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "name",
            "First Name",
            "Generated first name",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let name: String = FirstName().fake();
        context.set_pin_value("name", json!(name)).await?;
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
pub struct FakeLastName;

#[async_trait]
impl NodeLogic for FakeLastName {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_last_name",
            "Fake Last Name",
            "Generates a random last name for mocking data",
            "Utils/Faker/Name",
        );
        node.set_flowscript_name("faker.name", "lastName");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "name",
            "Last Name",
            "Generated last name",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let name: String = LastName().fake();
        context.set_pin_value("name", json!(name)).await?;
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
pub struct FakeFullName;

#[async_trait]
impl NodeLogic for FakeFullName {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_full_name",
            "Fake Full Name",
            "Generates a random full name for mocking data",
            "Utils/Faker/Name",
        );
        node.set_flowscript_name("faker.name", "fullName");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "name",
            "Full Name",
            "Generated full name",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let name: String = Name().fake();
        context.set_pin_value("name", json!(name)).await?;
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
pub struct FakeTitle;

#[async_trait]
impl NodeLogic for FakeTitle {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_title",
            "Fake Title",
            "Generates a random name title (Mr., Mrs., Dr., etc.)",
            "Utils/Faker/Name",
        );
        node.set_flowscript_name("faker.name", "title");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin("title", "Title", "Generated title", VariableType::String);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let title: String = Title().fake();
        context.set_pin_value("title", json!(title)).await?;
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

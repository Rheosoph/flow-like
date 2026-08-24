use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use fake::{Fake, faker::company::en::*};

#[crate::register_node]
#[derive(Default)]
pub struct FakeCompanyName;

#[async_trait]
impl NodeLogic for FakeCompanyName {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_company_name",
            "Fake Company Name",
            "Generates a random company name for mocking data",
            "Utils/Faker/Company",
        );
        node.set_flowscript_name("faker.company", "name");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "company",
            "Company Name",
            "Generated company name",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let company: String = CompanyName().fake();
        context.set_pin_value("company", json!(company)).await?;
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
pub struct FakeBuzzword;

#[async_trait]
impl NodeLogic for FakeBuzzword {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_buzzword",
            "Fake Buzzword",
            "Generates a random business buzzword for mocking data",
            "Utils/Faker/Company",
        );
        node.set_flowscript_name("faker.company", "buzzword");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "buzzword",
            "Buzzword",
            "Generated buzzword",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let buzzword: String = Buzzword().fake();
        context.set_pin_value("buzzword", json!(buzzword)).await?;
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
pub struct FakeCatchPhrase;

#[async_trait]
impl NodeLogic for FakeCatchPhrase {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_catch_phrase",
            "Fake Catch Phrase",
            "Generates a random business catch phrase for mocking data",
            "Utils/Faker/Company",
        );
        node.set_flowscript_name("faker.company", "catchPhrase");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "phrase",
            "Catch Phrase",
            "Generated catch phrase",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let phrase: String = CatchPhrase().fake();
        context.set_pin_value("phrase", json!(phrase)).await?;
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
pub struct FakeIndustry;

#[async_trait]
impl NodeLogic for FakeIndustry {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_industry",
            "Fake Industry",
            "Generates a random industry name for mocking data",
            "Utils/Faker/Company",
        );
        node.set_flowscript_name("faker.company", "industry");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "industry",
            "Industry",
            "Generated industry name",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let industry: String = Industry().fake();
        context.set_pin_value("industry", json!(industry)).await?;
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
pub struct FakeProfession;

#[async_trait]
impl NodeLogic for FakeProfession {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_profession",
            "Fake Profession",
            "Generates a random profession/job title for mocking data",
            "Utils/Faker/Company",
        );
        node.set_flowscript_name("faker.company", "profession");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "profession",
            "Profession",
            "Generated profession",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let profession: String = Profession().fake();
        context
            .set_pin_value("profession", json!(profession))
            .await?;
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

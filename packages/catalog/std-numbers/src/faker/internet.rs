use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use fake::{Fake, faker::internet::en::*};

#[crate::register_node]
#[derive(Default)]
pub struct FakeEmail;

#[async_trait]
impl NodeLogic for FakeEmail {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_email",
            "Fake Email",
            "Generates a random email address for mocking data",
            "Utils/Faker/Internet",
        );
        node.set_flowscript_name("faker.internet", "email");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "email",
            "Email",
            "Generated email address",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let email: String = SafeEmail().fake();
        context.set_pin_value("email", json!(email)).await?;
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
pub struct FakeUsername;

#[async_trait]
impl NodeLogic for FakeUsername {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_username",
            "Fake Username",
            "Generates a random username for mocking data",
            "Utils/Faker/Internet",
        );
        node.set_flowscript_name("faker.internet", "username");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "username",
            "Username",
            "Generated username",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let username: String = Username().fake();
        context.set_pin_value("username", json!(username)).await?;
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
pub struct FakePassword;

#[async_trait]
impl NodeLogic for FakePassword {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_password",
            "Fake Password",
            "Generates a random password for mocking data",
            "Utils/Faker/Internet",
        );
        node.set_flowscript_name("faker.internet", "password");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "min_length",
            "Min Length",
            "Minimum password length",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(8)));
        node.add_input_pin(
            "max_length",
            "Max Length",
            "Maximum password length",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(16)));
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "password",
            "Password",
            "Generated password",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let min: i64 = context.evaluate_pin("min_length").await?;
        let max: i64 = context.evaluate_pin("max_length").await?;
        let password: String = Password(min as usize..max as usize).fake();
        context.set_pin_value("password", json!(password)).await?;
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
pub struct FakeIPv4;

#[async_trait]
impl NodeLogic for FakeIPv4 {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_ipv4",
            "Fake IPv4",
            "Generates a random IPv4 address for mocking data",
            "Utils/Faker/Internet",
        );
        node.set_flowscript_name("faker.internet", "ipv4");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin("ip", "IPv4", "Generated IPv4 address", VariableType::String);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let ip: String = IPv4().fake();
        context.set_pin_value("ip", json!(ip)).await?;
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
pub struct FakeIPv6;

#[async_trait]
impl NodeLogic for FakeIPv6 {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_ipv6",
            "Fake IPv6",
            "Generates a random IPv6 address for mocking data",
            "Utils/Faker/Internet",
        );
        node.set_flowscript_name("faker.internet", "ipv6");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin("ip", "IPv6", "Generated IPv6 address", VariableType::String);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let ip: String = IPv6().fake();
        context.set_pin_value("ip", json!(ip)).await?;
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
pub struct FakeUserAgent;

#[async_trait]
impl NodeLogic for FakeUserAgent {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_user_agent",
            "Fake User Agent",
            "Generates a random user agent string for mocking data",
            "Utils/Faker/Internet",
        );
        node.set_flowscript_name("faker.internet", "userAgent");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "user_agent",
            "User Agent",
            "Generated user agent",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let ua: String = UserAgent().fake();
        context.set_pin_value("user_agent", json!(ua)).await?;
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
pub struct FakeDomainSuffix;

#[async_trait]
impl NodeLogic for FakeDomainSuffix {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_domain_suffix",
            "Fake Domain Suffix",
            "Generates a random domain suffix (com, org, net, etc.)",
            "Utils/Faker/Internet",
        );
        node.set_flowscript_name("faker.internet", "domainSuffix");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "suffix",
            "Domain Suffix",
            "Generated domain suffix",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let suffix: String = DomainSuffix().fake();
        context.set_pin_value("suffix", json!(suffix)).await?;
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

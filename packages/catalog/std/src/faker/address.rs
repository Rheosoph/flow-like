use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use fake::{Fake, faker::address::en::*};

#[crate::register_node]
#[derive(Default)]
pub struct FakeStreetName;

#[async_trait]
impl NodeLogic for FakeStreetName {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_street_name",
            "Fake Street Name",
            "Generates a random street name for mocking data",
            "Utils/Faker/Address",
        );
        node.set_flowscript_name("faker.address", "streetName");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "street",
            "Street Name",
            "Generated street name",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let street: String = StreetName().fake();
        context.set_pin_value("street", json!(street)).await?;
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
pub struct FakeStreetAddress;

#[async_trait]
impl NodeLogic for FakeStreetAddress {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_street_address",
            "Fake Street Address",
            "Generates a random full street address for mocking data",
            "Utils/Faker/Address",
        );
        node.set_flowscript_name("faker.address", "streetAddress");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "address",
            "Street Address",
            "Generated street address",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let building: String = BuildingNumber().fake();
        let street: String = StreetName().fake();
        let address = format!("{} {}", building, street);
        context.set_pin_value("address", json!(address)).await?;
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
pub struct FakeCityName;

#[async_trait]
impl NodeLogic for FakeCityName {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_city_name",
            "Fake City Name",
            "Generates a random city name for mocking data",
            "Utils/Faker/Address",
        );
        node.set_flowscript_name("faker.address", "cityName");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "city",
            "City Name",
            "Generated city name",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let city: String = CityName().fake();
        context.set_pin_value("city", json!(city)).await?;
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
pub struct FakeStateName;

#[async_trait]
impl NodeLogic for FakeStateName {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_state_name",
            "Fake State Name",
            "Generates a random state/province name for mocking data",
            "Utils/Faker/Address",
        );
        node.set_flowscript_name("faker.address", "stateName");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "state",
            "State Name",
            "Generated state name",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let state: String = StateName().fake();
        context.set_pin_value("state", json!(state)).await?;
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
pub struct FakeCountryName;

#[async_trait]
impl NodeLogic for FakeCountryName {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_country_name",
            "Fake Country Name",
            "Generates a random country name for mocking data",
            "Utils/Faker/Address",
        );
        node.set_flowscript_name("faker.address", "countryName");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "country",
            "Country Name",
            "Generated country name",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let country: String = CountryName().fake();
        context.set_pin_value("country", json!(country)).await?;
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
pub struct FakeCountryCode;

#[async_trait]
impl NodeLogic for FakeCountryCode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_country_code",
            "Fake Country Code",
            "Generates a random country code (e.g., US, DE, FR) for mocking data",
            "Utils/Faker/Address",
        );
        node.set_flowscript_name("faker.address", "countryCode");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "code",
            "Country Code",
            "Generated country code",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let code: String = CountryCode().fake();
        context.set_pin_value("code", json!(code)).await?;
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
pub struct FakePostCode;

#[async_trait]
impl NodeLogic for FakePostCode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_post_code",
            "Fake Post Code",
            "Generates a random postal/zip code for mocking data",
            "Utils/Faker/Address",
        );
        node.set_flowscript_name("faker.address", "postCode");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "code",
            "Post Code",
            "Generated postal code",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let code: String = PostCode().fake();
        context.set_pin_value("code", json!(code)).await?;
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
pub struct FakeLatitude;

#[async_trait]
impl NodeLogic for FakeLatitude {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_latitude",
            "Fake Latitude",
            "Generates a random latitude coordinate for mocking data",
            "Utils/Faker/Address",
        );
        node.set_flowscript_name("faker.address", "latitude");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "latitude",
            "Latitude",
            "Generated latitude",
            VariableType::Float,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let lat: String = Latitude().fake();
        let lat_f: f64 = lat.parse().unwrap_or(0.0);
        context.set_pin_value("latitude", json!(lat_f)).await?;
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
pub struct FakeLongitude;

#[async_trait]
impl NodeLogic for FakeLongitude {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "faker_longitude",
            "Fake Longitude",
            "Generates a random longitude coordinate for mocking data",
            "Utils/Faker/Address",
        );
        node.set_flowscript_name("faker.address", "longitude");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_output_pin("exec_out", "Output", "Continue", VariableType::Execution);
        node.add_output_pin(
            "longitude",
            "Longitude",
            "Generated longitude",
            VariableType::Float,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let lon: String = Longitude().fake();
        let lon_f: f64 = lon.parse().unwrap_or(0.0);
        context.set_pin_value("longitude", json!(lon_f)).await?;
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

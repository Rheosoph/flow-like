use flow_like::{
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
        variable::VariableType,
    },
};
use flow_like_types::{async_trait, bail, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BitFromStringNode {}

impl BitFromStringNode {
    pub fn new() -> Self {
        BitFromStringNode {}
    }
}

#[async_trait]
impl NodeLogic for BitFromStringNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "bit_from_string",
            "Load Bit",
            "Loads a Bit from a string ID",
            "Bit",
        );
        node.set_flowscript_name("ai", "loadBit");
        node.add_icon("/flow/icons/bit.svg");
        node.set_version(3);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_input_pin("bit_id", "Bit ID", "Input String", VariableType::String);

        node.add_output_pin("output_bit", "Bit", "Output Bit", VariableType::Struct)
            .set_schema::<Bit>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let bit_id: String = context.evaluate_pin("bit_id").await?;
        let http_client = context.app_state.http_client.clone();

        // Parse the bit_id which might be in "hub:id" format
        // Hub URLs contain "://" so we need to find the last colon after any protocol
        let bit = if let Some(last_colon) = bit_id.rfind(':') {
            // Check if this colon is part of a URL scheme (http:// or https://)
            let before_colon = &bit_id[..last_colon];
            if before_colon.ends_with("http") || before_colon.ends_with("https") {
                // No hub:id separator, just a plain ID
                context.profile.find_bit(&bit_id, http_client).await
            } else {
                // Split into hub and id
                let hub = &bit_id[..last_colon];
                let id = &bit_id[last_colon + 1..];
                context
                    .profile
                    .get_bit(id.to_string(), Some(hub.to_string()), http_client)
                    .await
            }
        } else {
            // No colon at all, plain bit ID
            context.profile.find_bit(&bit_id, http_client).await
        };

        if let Ok(bit) = bit {
            context.set_pin_value("output_bit", json!(bit)).await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        let err = bit.err().unwrap();
        bail!("Bit not found: {}", err);
    }
}

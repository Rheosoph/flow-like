use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{
    async_trait, create_id,
    interaction::{ChoiceOption, InteractionRequest, InteractionStatus, InteractionType},
    json::json,
};
use std::time::{SystemTime, UNIX_EPOCH};

use super::wait::wait_for_interaction_response;

#[crate::register_node]
#[derive(Default)]
pub struct MultipleChoiceInteraction {}

#[async_trait]
impl NodeLogic for MultipleChoiceInteraction {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "interaction_multiple_choice",
            "Multiple Choice",
            "Request the user to pick one or more options. Pauses execution until a response or timeout.",
            "Events/Chat/Interaction",
        );
        node.add_icon("/flow/icons/interaction.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);
        node.add_output_pin(
            "exec_out",
            "Done",
            "Continues after response received",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_timeout",
            "Timeout",
            "Continues if no response within TTL",
            VariableType::Execution,
        );

        node.add_input_pin(
            "name",
            "Name",
            "Display name for this interaction",
            VariableType::String,
        )
        .set_default_value(Some(json!("Choose options")));

        node.add_input_pin(
            "description",
            "Description",
            "Prompt shown to the user",
            VariableType::String,
        )
        .set_default_value(Some(json!(
            "Please select one or more of the following options:"
        )));

        node.add_input_pin(
            "options",
            "Options",
            "Choice option labels",
            VariableType::String,
        )
        .set_default_value(Some(json!("Option A")));

        node.add_input_pin(
            "options",
            "Options",
            "Choice option labels",
            VariableType::String,
        )
        .set_default_value(Some(json!("Option B")));

        node.add_input_pin(
            "min_selections",
            "Min Selections",
            "Minimum number of options the user must select",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "max_selections",
            "Max Selections",
            "Maximum number of options the user can select (0 = unlimited)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "ttl_seconds",
            "Timeout (seconds)",
            "How long to wait for response",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(120)));

        node.add_output_pin(
            "response",
            "Response",
            "JSON array of selected option labels",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node.add_output_pin(
            "responded",
            "Responded",
            "Whether the user responded (vs timeout)",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_timeout").await?;

        let name: String = context.evaluate_pin("name").await?;
        let description: String = context.evaluate_pin("description").await?;
        let min_selections: i64 = context.evaluate_pin("min_selections").await?;
        let max_selections: i64 = context.evaluate_pin("max_selections").await?;
        let ttl_seconds: i64 = context.evaluate_pin("ttl_seconds").await?;
        let ttl_seconds = ttl_seconds.max(10) as u64;

        let max_selections = if max_selections <= 0 {
            usize::MAX
        } else {
            max_selections as usize
        };

        let option_pins = context.get_pins_by_name("options").await?;
        let mut options = Vec::new();
        for pin in option_pins {
            let label: String = context.evaluate_pin_ref(pin).await?;
            options.push(ChoiceOption {
                id: create_id(),
                label,
                description: None,
                freeform: false,
            });
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let interaction_id = create_id();

        let options_for_lookup = options.clone();
        let request = InteractionRequest {
            id: interaction_id.clone(),
            name,
            description,
            interaction_type: InteractionType::MultipleChoice {
                options,
                min_selections: min_selections.max(0) as usize,
                max_selections,
            },
            status: InteractionStatus::Pending,
            ttl_seconds,
            expires_at: now + ttl_seconds,
            run_id: None,
            app_id: None,
            responder_jwt: None,
        };

        let result = wait_for_interaction_response(context, request, ttl_seconds).await?;

        let responded = result.responded;

        // Map selected IDs back to labels
        let selected_labels: Vec<String> =
            if let Some(selected_ids) = result.value.get("selected_ids") {
                if let Some(ids_array) = selected_ids.as_array() {
                    ids_array
                        .iter()
                        .filter_map(|id| id.as_str())
                        .filter_map(|id| {
                            options_for_lookup
                                .iter()
                                .find(|opt| opt.id == id)
                                .map(|opt| opt.label.clone())
                        })
                        .collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

        let response_value = flow_like_types::json::to_string(&selected_labels).unwrap_or_default();

        context
            .set_pin_value("response", json!(response_value))
            .await?;
        context.set_pin_value("responded", json!(responded)).await?;

        if responded {
            context.activate_exec_pin("exec_out").await?;
        } else {
            context.log_message("Interaction timed out", LogLevel::Warn);
            context.activate_exec_pin("exec_timeout").await?;
        }

        Ok(())
    }
}

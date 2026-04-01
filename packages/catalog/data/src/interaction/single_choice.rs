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
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use super::wait::wait_for_interaction_response;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InteractionWaitResult {
    pub selected_id: Option<String>,
    pub freeform_value: Option<String>,
}

#[crate::register_node]
#[derive(Default)]
pub struct SingleChoiceInteraction {}

#[async_trait]
impl NodeLogic for SingleChoiceInteraction {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "interaction_single_choice",
            "Single Choice",
            "Request the user to pick one option. Pauses execution until a response or timeout.",
            "Events/Chat/Interaction",
        );
        node.add_icon("/flow/icons/interaction.svg");

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
        .set_default_value(Some(json!("Choose an option")));

        node.add_input_pin(
            "description",
            "Description",
            "Prompt shown to the user",
            VariableType::String,
        )
        .set_default_value(Some(json!("Please select one of the following options:")));

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
            "allow_freeform",
            "Allow Freeform",
            "Let user type a custom answer",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

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
            "The selected option label or freeform text",
            VariableType::String,
        );
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
        let allow_freeform: bool = context.evaluate_pin("allow_freeform").await?;
        let ttl_seconds: i64 = context.evaluate_pin("ttl_seconds").await?;
        let ttl_seconds = ttl_seconds.max(10) as u64;

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

        if allow_freeform {
            options.push(ChoiceOption {
                id: create_id(),
                label: "Other (type your answer)".to_string(),
                description: None,
                freeform: true,
            });
        }

        // Keep a copy for looking up selected label later
        let options_lookup = options.clone();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let interaction_id = create_id();

        let request = InteractionRequest {
            id: interaction_id.clone(),
            name,
            description,
            interaction_type: InteractionType::SingleChoice {
                options,
                allow_freeform,
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

        let response_value = if responded {
            let parsed: InteractionWaitResult = flow_like_types::json::from_value(result.value)?;
            if let Some(freeform) = parsed.freeform_value {
                freeform
            } else if let Some(selected_id) = parsed.selected_id {
                options_lookup
                    .iter()
                    .find(|opt| opt.id == selected_id)
                    .map(|opt| opt.label.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

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

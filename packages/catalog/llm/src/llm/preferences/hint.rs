use flow_like::{
    bit::BitModelPreference,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct SetModelHintNode {}

impl SetModelHintNode {
    pub fn new() -> Self {
        SetModelHintNode {}
    }
}

#[async_trait]
impl NodeLogic for SetModelHintNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_set_model_hint",
            "Set Model Hint",
            "Adds a soft preference hint for downstream model selection",
            "AI/Generative/Preferences",
        );
        node.set_flowscript_name("ai.preferences", "setModelHint");
        node.set_receiver("preferences_in");
        node.add_icon("/flow/icons/struct.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_reliability(10)
                .set_governance(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Begin execution when ready to update",
            VariableType::Execution,
        );

        node.add_input_pin(
            "preferences_in",
            "Preferences",
            "Current model preference state",
            VariableType::Struct,
        )
        .set_schema::<BitModelPreference>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "model_hint",
            "Model Hint",
            "Friendly hint describing the desired model family",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Signals completion after updating",
            VariableType::Execution,
        );

        node.add_output_pin(
            "preferences_out",
            "Preferences",
            "Preferences with the new hint",
            VariableType::Struct,
        )
        .set_schema::<BitModelPreference>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut preferences: BitModelPreference = context.evaluate_pin("preferences_in").await?;
        let model_hint: String = context.evaluate_pin("model_hint").await?;

        preferences.model_hint = Some(model_hint);

        context
            .set_pin_value("preferences_out", json!(preferences))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

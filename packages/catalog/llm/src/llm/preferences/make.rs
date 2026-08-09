use flow_like::{
    bit::BitModelPreference,
    flow::{
        execution::{LogLevel, context::ExecutionContext},
        node::{Node, NodeLogic, NodeScores},
        variable::VariableType,
    },
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct MakePreferencesNode {}

impl MakePreferencesNode {
    pub fn new() -> Self {
        MakePreferencesNode {}
    }
}

#[async_trait]
impl NodeLogic for MakePreferencesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_make_preferences",
            "Make Preferences",
            "Creates a BitModelPreference struct used to guide model selection",
            "AI/Generative/Preferences",
        );
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
            "multimodal",
            "Multimodal",
            "True if the target model must handle images",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "preferences",
            "Preferences",
            "Constructed BitModelPreference struct",
            VariableType::Struct,
        )
        .set_schema::<BitModelPreference>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut preferences = BitModelPreference::default();
        context.log_message(
            &format!("New Preferences: {:?}", preferences),
            LogLevel::Debug,
        );

        let multimodal = context.evaluate_pin::<bool>("multimodal").await;
        if let Ok(multimodal) = multimodal {
            preferences.multimodal = Some(multimodal);
        }

        context
            .set_pin_value("preferences", json!(preferences))
            .await?;

        Ok(())
    }
}

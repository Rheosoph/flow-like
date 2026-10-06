use std::collections::BTreeMap;

use flow_like::bit::Bit;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, PinType, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_model_provider::systemone::{
    NoulCriteria, SystemOneAnswer, SystemOneQuestion, SystemOneRequest, SystemOneResponse,
};
use flow_like_types::{Result, Value, anyhow, async_trait, json::json};

const QUESTION_ID: &str = "answer";

fn base_node(name: &str, label: &str, description: &str, method: &str) -> Node {
    let mut node = Node::new(name, label, description, "AI/Decisions");
    node.set_flowscript_name("ai.systemone", method);
    node.add_icon("/flow/icons/bot-invoke.svg");
    node.set_version(2);
    node.set_long_running(true);
    node.add_input_pin(
        "exec_in",
        "Input",
        "Run the decision model",
        VariableType::Execution,
    );
    node.add_input_pin(
        "model",
        "Model",
        "A SystemOne decision model Bit from Find Decision Model or Load Bit",
        VariableType::Struct,
    )
    .set_schema::<Bit>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node
}

#[cfg(feature = "execute")]
async fn invoke(
    context: &mut ExecutionContext,
    request: &SystemOneRequest,
) -> Result<SystemOneResponse> {
    request.validate()?;
    let bit: Bit = context.evaluate_pin("model").await?;
    let model = context
        .app_state
        .model_factory
        .build_systemone(
            &bit,
            context.app_state.clone(),
            context.token.clone(),
            context.model_usage_context(),
        )
        .await?;
    model.invoke(request).await
}

#[crate::register_node]
#[derive(Default)]
pub struct InvokeSystemOne {}

impl InvokeSystemOne {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for InvokeSystemOne {
    fn get_node(&self) -> Node {
        let mut node = base_node(
            "ai_systemone_invoke",
            "Invoke SystemOne",
            "Answers typed questions about text or data with a local or hosted decision model.",
            "invoke",
        );
        node.add_output_pin("done", "Done", "Decision complete", VariableType::Execution);
        node.add_input_pin(
            "request",
            "Questions",
            "The state to assess and named choice, score, or yes/no questions",
            VariableType::Struct,
        )
        .set_schema::<SystemOneRequest>()
        .set_default_value(Some(json!({
            "state": "The order arrived two days late.",
            "questions": {
                "sentiment": {
                    "type": "choice",
                    "instructions": "What is the customer's sentiment?",
                    "criteria": {"positive": null, "neutral": null, "negative": null}
                }
            }
        })))
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "result",
            "Result",
            "Typed answers and usage returned by the model",
            VariableType::Struct,
        )
        .set_schema::<SystemOneResponse>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        flow_like_catalog_core::run_with_execute_gate!(context, {
            context.deactivate_exec_pin("done").await?;
            let request: SystemOneRequest = context.evaluate_pin("request").await?;
            let response = invoke(context, &request).await?;
            context.set_pin_value("result", json!(response)).await?;
            context.activate_exec_pin("done").await?;
            Ok(())
        })
    }
}

#[derive(Clone, Copy)]
enum DecisionKind {
    Noul,
    Choice,
    Score,
}

fn decision_node(kind: DecisionKind) -> Node {
    let (name, label, description, method, instructions) = match kind {
        DecisionKind::Noul => (
            "ai_systemone_noul",
            "SystemOne Noul",
            "Returns the probability that a yes/no assessment is true.",
            "noul",
            "Does the customer request a refund?",
        ),
        DecisionKind::Choice => (
            "ai_systemone_choice",
            "SystemOne Choice",
            "Selects a named option and returns its probability distribution.",
            "choice",
            "Which team should handle this message?",
        ),
        DecisionKind::Score => (
            "ai_systemone_score",
            "SystemOne Score",
            "Scores a state against ordered levels and returns the weighted level index.",
            "score",
            "How urgent is this message?",
        ),
    };
    let mut node = base_node(name, label, description, method);
    if !matches!(kind, DecisionKind::Noul) {
        node.add_output_pin("done", "Done", "Decision complete", VariableType::Execution);
    }
    node.add_input_pin("state", "State", "Text to assess", VariableType::String)
        .set_default_value(Some(json!("I was charged twice for my order.")));
    node.add_input_pin(
        "instructions",
        "Instructions",
        "The question to answer about the state",
        VariableType::String,
    )
    .set_default_value(Some(json!(instructions)));
    node.add_input_pin(
        "images",
        "Images",
        "Optional images; requires an image-capable decision model and projector",
        VariableType::Struct,
    )
    .set_schema::<NodeImage>()
    .set_options(PinOptions::new().set_enforce_schema(true).build())
    .set_value_type(ValueType::Array)
    .set_default_value(Some(json!([])));
    match kind {
        DecisionKind::Noul => {
            node.add_input_pin(
                "threshold",
                "Threshold",
                "Branch True when the probability is at least this value, from 0 to 1",
                VariableType::Float,
            )
            .set_default_value(Some(json!(0.5)));
            node.add_output_pin(
                "true",
                "True",
                "Probability meets or exceeds the threshold",
                VariableType::Execution,
            );
            node.add_output_pin(
                "false",
                "False",
                "Probability is below the threshold",
                VariableType::Execution,
            );
            for (name, label, description) in [
                (
                    "true_description",
                    "True Description",
                    "Optional description of a true answer",
                ),
                (
                    "false_description",
                    "False Description",
                    "Optional description of a false answer",
                ),
            ] {
                node.add_input_pin(name, label, description, VariableType::String)
                    .set_default_value(Some(json!("")));
            }
            node.add_output_pin(
                "probability",
                "Probability",
                "Probability that the assessment is true, from 0 to 1",
                VariableType::Float,
            );
        }
        DecisionKind::Choice => {
            node.add_input_pin(
                "criteria",
                "Choices",
                "Map each option name to its description",
                VariableType::String,
            )
            .set_value_type(ValueType::HashMap)
            .set_default_value(Some(
                json!({"billing":"Payments and refunds", "shipping":"Delivery and tracking"}),
            ));
            node.add_output_pin(
                "choice",
                "Choice",
                "The selected option name",
                VariableType::String,
            );
        }
        DecisionKind::Score => {
            node.add_input_pin(
                "criteria",
                "Levels",
                "Two to ten descriptions ordered from lowest to highest",
                VariableType::String,
            )
            .set_value_type(ValueType::Array)
            .set_default_value(Some(json!(["Not urgent", "Urgent", "Critical"])));
            node.add_output_pin(
                "score",
                "Score",
                "Weighted level index, from 0 to the number of levels minus 1",
                VariableType::Float,
            );
            node.add_output_pin(
                "legend",
                "Legend",
                "Maps zero-based level indices to their descriptions",
                VariableType::String,
            )
            .set_value_type(ValueType::HashMap);
        }
    }
    if !matches!(kind, DecisionKind::Noul) {
        node.add_output_pin(
            "probabilities",
            "Probabilities",
            "Probability of each choice or zero-based level index",
            VariableType::Float,
        )
        .set_value_type(ValueType::HashMap);
        node.add_output_pin(
            "confidence",
            "Confidence",
            "The model's confidence value, from 0 to 1",
            VariableType::Float,
        );
    }
    node
}

fn single_request(
    state: String,
    images: Vec<String>,
    question: SystemOneQuestion,
) -> Result<SystemOneRequest> {
    let request = SystemOneRequest {
        state: Value::String(state),
        images,
        questions: BTreeMap::from([(QUESTION_ID.to_owned(), question)]),
    };
    request.validate()?;
    Ok(request)
}

fn noul_question(instructions: String, when_true: String, when_false: String) -> SystemOneQuestion {
    let description = |value: String| (!value.trim().is_empty()).then_some(Value::String(value));
    let criteria = NoulCriteria {
        when_true: description(when_true),
        when_false: description(when_false),
    };
    SystemOneQuestion::Noul {
        instructions: Value::String(instructions),
        criteria: (criteria.when_true.is_some() || criteria.when_false.is_some())
            .then_some(criteria),
    }
}

fn single_answer(
    request: &SystemOneRequest,
    mut response: SystemOneResponse,
) -> Result<SystemOneAnswer> {
    response.validate_for(request)?;
    if let Some(SystemOneAnswer::Score { legend, .. }) = response.answers.get(QUESTION_ID)
        && legend.values().any(|value| !value.is_string())
    {
        return Err(anyhow!(
            "SystemOne Score returned a non-text level description"
        ));
    }
    response
        .answers
        .remove(QUESTION_ID)
        .ok_or_else(|| anyhow!("SystemOne response is missing its answer"))
}

fn noul_branch(probability: f64, threshold: f64) -> Result<&'static str> {
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(anyhow!("SystemOne Noul threshold must be between 0 and 1"));
    }
    if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
        return Err(anyhow!(
            "SystemOne Noul probability must be between 0 and 1"
        ));
    }
    Ok(if probability >= threshold {
        "true"
    } else {
        "false"
    })
}

#[cfg(feature = "execute")]
async fn run_decision(context: &mut ExecutionContext, kind: DecisionKind) -> Result<()> {
    let threshold = if matches!(kind, DecisionKind::Noul) {
        context.deactivate_exec_pin("true").await?;
        context.deactivate_exec_pin("false").await?;
        let threshold: f64 = context.evaluate_pin("threshold").await?;
        noul_branch(0.0, threshold)?;
        Some(threshold)
    } else {
        context.deactivate_exec_pin("done").await?;
        None
    };
    let state: String = context.evaluate_pin("state").await?;
    let instructions: String = context.evaluate_pin("instructions").await?;
    let images: Vec<NodeImage> = context.evaluate_pin("images").await?;
    let mut image_urls = Vec::with_capacity(images.len());
    for image in images {
        let image_ref = image.get_image(context).await?;
        let image = image_ref.lock().await;
        image_urls.push(
            flow_like_types::utils::data_url::image_to_data_url(
                &image,
                flow_like_types::image::ImageFormat::Png,
            )
            .await?,
        );
    }
    let question = match kind {
        DecisionKind::Noul => noul_question(
            instructions,
            context.evaluate_pin("true_description").await?,
            context.evaluate_pin("false_description").await?,
        ),
        DecisionKind::Choice => {
            let criteria: BTreeMap<String, String> = context.evaluate_pin("criteria").await?;
            SystemOneQuestion::Choice {
                instructions: Value::String(instructions),
                criteria: criteria
                    .into_iter()
                    .map(|(key, value)| (key, Value::String(value)))
                    .collect(),
            }
        }
        DecisionKind::Score => {
            let criteria: Vec<String> = context.evaluate_pin("criteria").await?;
            SystemOneQuestion::Score {
                instructions: Value::String(instructions),
                criteria: criteria.into_iter().map(Value::String).collect(),
            }
        }
    };
    let request = single_request(state, image_urls, question)?;
    let response = invoke(context, &request).await?;
    match single_answer(&request, response)? {
        SystemOneAnswer::Noul { noul } => {
            context.set_pin_value("probability", json!(noul)).await?;
            let branch = noul_branch(
                noul,
                threshold.ok_or_else(|| anyhow!("Missing Noul threshold"))?,
            )?;
            context.activate_exec_pin(branch).await?;
        }
        SystemOneAnswer::Choice {
            choice,
            probabilities,
            confidence,
        } => {
            context.set_pin_value("choice", json!(choice)).await?;
            context
                .set_pin_value("probabilities", json!(probabilities))
                .await?;
            context
                .set_pin_value("confidence", json!(confidence))
                .await?;
        }
        SystemOneAnswer::Score {
            score,
            legend,
            probabilities,
            confidence,
        } => {
            context.set_pin_value("score", json!(score)).await?;
            context.set_pin_value("legend", json!(legend)).await?;
            context
                .set_pin_value("probabilities", json!(probabilities))
                .await?;
            context
                .set_pin_value("confidence", json!(confidence))
                .await?;
        }
    }
    if !matches!(kind, DecisionKind::Noul) {
        context.activate_exec_pin("done").await?;
    }
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct SystemOneNoul {}
#[crate::register_node]
#[derive(Default)]
pub struct SystemOneChoice {}
#[crate::register_node]
#[derive(Default)]
pub struct SystemOneScore {}

macro_rules! decision_node_logic {
    ($node:ident, $kind:ident) => {
        impl $node {
            pub fn new() -> Self {
                Self {}
            }
        }
        #[async_trait]
        impl NodeLogic for $node {
            fn get_node(&self) -> Node {
                decision_node(DecisionKind::$kind)
            }
            async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
                flow_like_catalog_core::run_with_execute_gate!(context, {
                    run_decision(context, DecisionKind::$kind).await
                })
            }

            async fn on_update(&self, node: &mut Node, _board: &Board) {
                if matches!(DecisionKind::$kind, DecisionKind::Noul) {
                    return;
                }
                // Catalog upgrades preserve placed pin indices, so move Done explicitly.
                let mut outputs: Vec<_> = node
                    .pins
                    .values_mut()
                    .filter(|pin| pin.pin_type == PinType::Output)
                    .collect();
                outputs.sort_by_key(|pin| (pin.name != "done", pin.index));
                for (index, pin) in outputs.into_iter().enumerate() {
                    pin.index = index as u16 + 1;
                }
            }
        }
    };
}
decision_node_logic!(SystemOneNoul, Noul);
decision_node_logic!(SystemOneChoice, Choice);
decision_node_logic!(SystemOneScore, Score);

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::board::cleanup::sync_node_schema::sync_node_with_catalog;
    use flow_like_types::json as serde_json;

    #[test]
    fn systemone_noul_routes_below_equal_and_above_its_threshold() {
        assert_eq!(noul_branch(0.49, 0.5).unwrap(), "false");
        assert_eq!(noul_branch(0.5, 0.5).unwrap(), "true");
        assert_eq!(noul_branch(0.51, 0.5).unwrap(), "true");
        assert_eq!(noul_branch(0.0, 0.0).unwrap(), "true");
        assert_eq!(noul_branch(1.0, 1.0).unwrap(), "true");
        for invalid in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            assert!(noul_branch(0.5, invalid).is_err());
            assert!(noul_branch(invalid, 0.5).is_err());
        }
    }

    #[test]
    fn systemone_direct_requests_preserve_state_descriptions_and_level_order() {
        let state = r#"{"ticket":"Duplicate charge","amount":49}"#.to_owned();
        let request = single_request(
            state.clone(),
            vec![],
            noul_question("Refund?".into(), "Duplicate charge".into(), "".into()),
        )
        .unwrap();
        let value = json!(request);
        assert_eq!(value["state"], json!(state));
        assert_eq!(
            value["questions"][QUESTION_ID]["criteria"],
            json!({"true":"Duplicate charge"})
        );
        let request = single_request(
            "text".into(),
            vec![],
            noul_question("Urgent?".into(), "".into(), " ".into()),
        )
        .unwrap();
        assert!(
            json!(request)["questions"][QUESTION_ID]
                .get("criteria")
                .is_none()
        );
        let request = single_request(
            state,
            vec![],
            SystemOneQuestion::Score {
                instructions: json!("Urgency?"),
                criteria: vec![json!("Calm"), json!("Urgent"), json!("Critical")],
            },
        )
        .unwrap();
        assert_eq!(
            json!(request)["questions"][QUESTION_ID]["criteria"],
            json!(["Calm", "Urgent", "Critical"])
        );
        assert!(
            single_request(
                "text".into(),
                vec![],
                SystemOneQuestion::Choice {
                    instructions: json!("Route?"),
                    criteria: BTreeMap::new()
                }
            )
            .is_err()
        );
        assert!(
            single_request(
                "text".into(),
                vec![],
                SystemOneQuestion::Score {
                    instructions: json!("Rate?"),
                    criteria: vec![json!("one")]
                }
            )
            .is_err()
        );
    }

    #[test]
    fn systemone_direct_answers_reject_missing_mismatched_and_invalid_decisions() {
        let request = single_request(
            "text".into(),
            vec![],
            SystemOneQuestion::Choice {
                instructions: json!("Route?"),
                criteria: BTreeMap::from([
                    ("billing".into(), json!("Payments")),
                    ("shipping".into(), json!("Delivery")),
                ]),
            },
        )
        .unwrap();
        let valid = json!({"model":"test","answers":{"answer":{"type":"choice","choice":"billing","probabilities":{"billing":0.8,"shipping":0.2},"confidence":0.6}}});
        match single_answer(&request, serde_json::from_value(valid.clone()).unwrap()).unwrap() {
            SystemOneAnswer::Choice {
                choice,
                probabilities,
                confidence,
            } => {
                assert_eq!(choice, "billing");
                assert_eq!(probabilities["shipping"], 0.2);
                assert_eq!(confidence, 0.6);
            }
            _ => panic!("Expected choice"),
        }
        for answers in [
            json!({}),
            json!({"answer":{"type":"noul","noul":0.8}}),
            json!({"answer":{"type":"choice","choice":"other","probabilities":{"billing":0.8,"shipping":0.2},"confidence":0.6}}),
        ] {
            let mut invalid = valid.clone();
            invalid["answers"] = answers;
            assert!(single_answer(&request, serde_json::from_value(invalid).unwrap()).is_err());
        }
    }

    #[test]
    fn systemone_nodes_accept_model_bits_and_typed_decision_inputs() {
        let nodes = [
            InvokeSystemOne::new().get_node(),
            SystemOneNoul::new().get_node(),
            SystemOneChoice::new().get_node(),
            SystemOneScore::new().get_node(),
        ];
        for node in &nodes {
            assert_eq!(node.version, Some(2));
            let model = node.pins.values().find(|pin| pin.name == "model").unwrap();
            assert_eq!(model.data_type, VariableType::Struct);
            assert_eq!(
                model.schema,
                flow_like::flow::pin::Pin::schema_string_for::<Bit>()
            );
            assert_eq!(model.options.as_ref().unwrap().enforce_schema, Some(true));
        }
        let mut branches: Vec<_> = nodes[1]
            .pins
            .values()
            .filter(|pin| {
                pin.pin_type == flow_like::flow::pin::PinType::Output
                    && pin.data_type == VariableType::Execution
            })
            .map(|pin| pin.name.as_str())
            .collect();
        branches.sort();
        assert_eq!(branches, ["false", "true"]);
        for node in &nodes[1..] {
            assert_eq!(node.version, Some(2));
            let state = node.pins.values().find(|pin| pin.name == "state").unwrap();
            assert_eq!(state.data_type, VariableType::String);
            assert_eq!(state.value_type, ValueType::Normal);
            let images = node.pins.values().find(|pin| pin.name == "images").unwrap();
            assert_eq!(images.data_type, VariableType::Struct);
            assert_eq!(images.value_type, ValueType::Array);
            assert_eq!(
                images.schema,
                flow_like::flow::pin::Pin::schema_string_for::<NodeImage>()
            );
            assert_eq!(images.options.as_ref().unwrap().enforce_schema, Some(true));
        }
        for node in [&nodes[0], &nodes[2], &nodes[3]] {
            let first_output = node
                .pins
                .values()
                .filter(|pin| pin.pin_type == flow_like::flow::pin::PinType::Output)
                .min_by_key(|pin| pin.index)
                .unwrap();
            assert_eq!(first_output.name, "done");
            assert_eq!(first_output.data_type, VariableType::Execution);
        }
        for (node, pin_name, kind, value_type) in [
            (
                &nodes[1],
                "probability",
                VariableType::Float,
                ValueType::Normal,
            ),
            (
                &nodes[2],
                "criteria",
                VariableType::String,
                ValueType::HashMap,
            ),
            (
                &nodes[2],
                "probabilities",
                VariableType::Float,
                ValueType::HashMap,
            ),
            (
                &nodes[3],
                "criteria",
                VariableType::String,
                ValueType::Array,
            ),
            (
                &nodes[3],
                "legend",
                VariableType::String,
                ValueType::HashMap,
            ),
        ] {
            let pin = node.pins.values().find(|pin| pin.name == pin_name).unwrap();
            assert_eq!(pin.data_type, kind);
            assert_eq!(pin.value_type, value_type);
        }
    }

    #[tokio::test]
    async fn systemone_upgrades_move_done_first_and_preserve_output_connections() {
        let board = Board::new_detached(None, "systemone-test".into());
        let nodes: Vec<Box<dyn NodeLogic>> = vec![
            Box::new(SystemOneChoice::new()),
            Box::new(SystemOneScore::new()),
        ];
        for logic in nodes {
            let catalog = logic.get_node();
            let mut placed = catalog.clone();
            placed.version = Some(1);
            let done_index = if placed.name == "ai_systemone_choice" {
                2
            } else {
                3
            };
            for pin in placed.pins.values_mut() {
                match pin.name.as_str() {
                    "state" => pin.data_type = VariableType::Generic,
                    "model" | "images" => {
                        pin.data_type = VariableType::String;
                        pin.schema = None;
                        pin.options = None;
                    }
                    _ => {}
                }
                if pin.pin_type != PinType::Output {
                    continue;
                }
                if pin.name == "done" {
                    pin.index = done_index;
                    pin.connected_to.insert("next-node-exec".into());
                } else if pin.index <= done_index {
                    pin.index -= 1;
                }
            }
            let before = placed.pins.clone();

            sync_node_with_catalog(&mut placed, &catalog);
            logic.on_update(&mut placed, &board).await;

            for name in ["model", "state", "images"] {
                let expected = catalog.get_pin_by_name(name).unwrap();
                let upgraded = placed.get_pin_by_name(name).unwrap();
                assert_eq!(upgraded.data_type, expected.data_type);
                assert_eq!(upgraded.schema, expected.schema);
            }
            for pin in placed
                .pins
                .values()
                .filter(|pin| pin.pin_type == PinType::Output)
            {
                let expected = catalog.get_pin_by_name(&pin.name).unwrap();
                assert_eq!(pin.index, expected.index);
                assert_eq!(pin.connected_to, before[&pin.id].connected_to);
                assert_eq!(pin.depends_on, before[&pin.id].depends_on);
            }
            assert_eq!(placed.version, Some(2));
        }
    }
}

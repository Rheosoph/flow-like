use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

fn variadic_node(id: &str, label: &str, description: &str) -> Node {
    let mut node = Node::new(id, label, description, "Utils/Bool");
    node.add_icon("/flow/icons/bool.svg");
    node.set_scores(pure_scores());

    node.add_input_pin("boolean", "Boolean", "Input Boolean", VariableType::Boolean)
        .set_default_value(Some(json!(false)));
    node.add_input_pin("boolean", "Boolean", "Input Boolean", VariableType::Boolean)
        .set_default_value(Some(json!(false)));
    node.add_output_pin("result", "Result", description, VariableType::Boolean);

    node
}

async fn booleans(context: &mut ExecutionContext) -> flow_like_types::Result<Vec<bool>> {
    let pins = context.get_pins_by_name("boolean").await?;

    let mut values = Vec::with_capacity(pins.len());
    for pin in pins {
        values.push(context.evaluate_pin_ref::<bool>(pin).await?);
    }
    Ok(values)
}

#[crate::register_node]
#[derive(Default)]
pub struct BoolNandNode {}

impl BoolNandNode {
    pub fn new() -> Self {
        BoolNandNode {}
    }
}

#[async_trait]
impl NodeLogic for BoolNandNode {
    fn get_node(&self) -> Node {
        let mut node = variadic_node("bool_nand", "Nand", "True unless every input is true");
        node.set_flowscript_name("bool", "nand");
        node.set_receiver("boolean");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = booleans(context).await?;
        context
            .set_pin_value("result", json!(!values.iter().all(|value| *value)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BoolNorNode {}

impl BoolNorNode {
    pub fn new() -> Self {
        BoolNorNode {}
    }
}

#[async_trait]
impl NodeLogic for BoolNorNode {
    fn get_node(&self) -> Node {
        let mut node = variadic_node("bool_nor", "Nor", "True only when every input is false");
        node.set_flowscript_name("bool", "nor");
        node.set_receiver("boolean");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let values = booleans(context).await?;
        context
            .set_pin_value("result", json!(!values.iter().any(|value| *value)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BoolImpliesNode {}

impl BoolImpliesNode {
    pub fn new() -> Self {
        BoolImpliesNode {}
    }
}

#[async_trait]
impl NodeLogic for BoolImpliesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "bool_implies",
            "Implies",
            "False only when the premise is true and the conclusion is false",
            "Utils/Bool",
        );
        node.set_flowscript_name("bool", "implies");
        node.set_receiver("premise");
        node.add_icon("/flow/icons/bool.svg");
        node.set_scores(pure_scores());

        node.add_input_pin(
            "premise",
            "Premise",
            "The condition that is assumed",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "conclusion",
            "Conclusion",
            "What has to hold when the premise is true",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "result",
            "Result",
            "True when the implication holds",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let premise: bool = context.evaluate_pin("premise").await?;
        let conclusion: bool = context.evaluate_pin("conclusion").await?;
        context
            .set_pin_value("result", json!(!premise || conclusion))
            .await?;
        Ok(())
    }
}

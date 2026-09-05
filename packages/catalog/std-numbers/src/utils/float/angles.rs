use super::unary::{binary_node, binary_run, unary_node, unary_run};
use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct FloatToDegreesNode {}

impl FloatToDegreesNode {
    pub fn new() -> Self {
        FloatToDegreesNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatToDegreesNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_to_degrees",
            "To Degrees",
            "Converts radians into degrees",
            "Math/Float/Trigonometry",
        );
        node.set_flowscript_name("float", "toDegrees");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "To Degrees", f64::to_degrees).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatToRadiansNode {}

impl FloatToRadiansNode {
    pub fn new() -> Self {
        FloatToRadiansNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatToRadiansNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_to_radians",
            "To Radians",
            "Converts degrees into radians",
            "Math/Float/Trigonometry",
        );
        node.set_flowscript_name("float", "toRadians");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "To Radians", f64::to_radians).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatRecipNode {}

impl FloatRecipNode {
    pub fn new() -> Self {
        FloatRecipNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatRecipNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_recip",
            "Reciprocal",
            "One divided by a float",
            "Math/Float",
        );
        node.set_flowscript_name("float", "recip");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Reciprocal", f64::recip).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatHypotNode {}

impl FloatHypotNode {
    pub fn new() -> Self {
        FloatHypotNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatHypotNode {
    fn get_node(&self) -> Node {
        let mut node = binary_node(
            "float_hypot",
            "Hypotenuse",
            "Length of the hypotenuse of a right-angled triangle",
            "Math/Float",
        );
        node.set_flowscript_name("float", "hypot");
        node.set_receiver("float1");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        binary_run(context, "Hypotenuse", f64::hypot).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatCopySignNode {}

impl FloatCopySignNode {
    pub fn new() -> Self {
        FloatCopySignNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatCopySignNode {
    fn get_node(&self) -> Node {
        let mut node = binary_node(
            "float_copysign",
            "Copy Sign",
            "Takes the magnitude of the first float and the sign of the second",
            "Math/Float",
        );
        node.set_flowscript_name("float", "copysign");
        node.set_receiver("float1");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        binary_run(context, "Copy Sign", f64::copysign).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatMulAddNode {}

impl FloatMulAddNode {
    pub fn new() -> Self {
        FloatMulAddNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatMulAddNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_mul_add",
            "Multiply Add",
            "Multiplies two floats and adds a third in one rounding step",
            "Math/Float",
        );
        node.set_flowscript_name("float", "mulAdd");
        node.set_receiver("float");
        node.add_icon("/flow/icons/sigma.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("float", "Float", "Input Float", VariableType::Float);
        node.add_input_pin(
            "factor",
            "Factor",
            "Multiplied with the input",
            VariableType::Float,
        )
        .set_default_value(Some(json!(1.0)));
        node.add_input_pin(
            "addend",
            "Addend",
            "Added to the product",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));

        node.add_output_pin(
            "result",
            "Result",
            "Input multiplied by the factor plus the addend",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let float: f64 = context.evaluate_pin("float").await?;
        let factor: f64 = context.evaluate_pin("factor").await?;
        let addend: f64 = context.evaluate_pin("addend").await?;

        context
            .set_pin_value("result", json!(float.mul_add(factor, addend)))
            .await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatLerpNode {}

impl FloatLerpNode {
    pub fn new() -> Self {
        FloatLerpNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatLerpNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_lerp",
            "Lerp",
            "Interpolates linearly between two floats",
            "Math/Float",
        );
        node.set_flowscript_name("float", "lerp");
        node.set_receiver("from");
        node.add_icon("/flow/icons/sigma.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("from", "From", "Value at t = 0", VariableType::Float)
            .set_default_value(Some(json!(0.0)));
        node.add_input_pin("to", "To", "Value at t = 1", VariableType::Float)
            .set_default_value(Some(json!(1.0)));
        node.add_input_pin("t", "T", "Interpolation factor", VariableType::Float)
            .set_default_value(Some(json!(0.5)));
        node.add_input_pin(
            "clamp",
            "Clamp",
            "Clamp the factor into the range 0 to 1",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "result",
            "Result",
            "The interpolated value",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let from: f64 = context.evaluate_pin("from").await?;
        let to: f64 = context.evaluate_pin("to").await?;
        let t: f64 = context.evaluate_pin("t").await?;
        let clamp: bool = context.evaluate_pin("clamp").await?;

        let t = if clamp { t.clamp(0.0, 1.0) } else { t };
        context
            .set_pin_value("result", json!(from + (to - from) * t))
            .await?;
        Ok(())
    }
}

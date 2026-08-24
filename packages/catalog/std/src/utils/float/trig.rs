use super::unary::{binary_node, binary_run, unary_node, unary_run};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
};
use flow_like_types::async_trait;

const CATEGORY: &str = "Math/Float/Trigonometry";

#[crate::register_node]
#[derive(Default)]
pub struct FloatSinNode {}

impl FloatSinNode {
    pub fn new() -> Self {
        FloatSinNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatSinNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node("float_sin", "Sin", "Sine of an angle in radians", CATEGORY);
        node.set_flowscript_name("float", "sin");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Sin", f64::sin).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatCosNode {}

impl FloatCosNode {
    pub fn new() -> Self {
        FloatCosNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatCosNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_cos",
            "Cos",
            "Cosine of an angle in radians",
            CATEGORY,
        );
        node.set_flowscript_name("float", "cos");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Cos", f64::cos).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatTanNode {}

impl FloatTanNode {
    pub fn new() -> Self {
        FloatTanNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatTanNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_tan",
            "Tan",
            "Tangent of an angle in radians",
            CATEGORY,
        );
        node.set_flowscript_name("float", "tan");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Tan", f64::tan).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatAsinNode {}

impl FloatAsinNode {
    pub fn new() -> Self {
        FloatAsinNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatAsinNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_asin",
            "Asin",
            "Arc sine in radians, input must be between -1 and 1",
            CATEGORY,
        );
        node.set_flowscript_name("float", "asin");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Asin", f64::asin).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatAcosNode {}

impl FloatAcosNode {
    pub fn new() -> Self {
        FloatAcosNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatAcosNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_acos",
            "Acos",
            "Arc cosine in radians, input must be between -1 and 1",
            CATEGORY,
        );
        node.set_flowscript_name("float", "acos");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Acos", f64::acos).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatAtanNode {}

impl FloatAtanNode {
    pub fn new() -> Self {
        FloatAtanNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatAtanNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node("float_atan", "Atan", "Arc tangent in radians", CATEGORY);
        node.set_flowscript_name("float", "atan");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Atan", f64::atan).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatAtan2Node {}

impl FloatAtan2Node {
    pub fn new() -> Self {
        FloatAtan2Node {}
    }
}

#[async_trait]
impl NodeLogic for FloatAtan2Node {
    fn get_node(&self) -> Node {
        let mut node = binary_node(
            "float_atan2",
            "Atan2",
            "Angle in radians between the positive x axis and the point (x, y)",
            CATEGORY,
        );
        node.set_flowscript_name("float", "atan2");
        node.set_receiver("float1");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        binary_run(context, "Atan2", |y, x| y.atan2(x)).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatSinhNode {}

impl FloatSinhNode {
    pub fn new() -> Self {
        FloatSinhNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatSinhNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node("float_sinh", "Sinh", "Hyperbolic sine", CATEGORY);
        node.set_flowscript_name("float", "sinh");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Sinh", f64::sinh).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatCoshNode {}

impl FloatCoshNode {
    pub fn new() -> Self {
        FloatCoshNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatCoshNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node("float_cosh", "Cosh", "Hyperbolic cosine", CATEGORY);
        node.set_flowscript_name("float", "cosh");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Cosh", f64::cosh).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatTanhNode {}

impl FloatTanhNode {
    pub fn new() -> Self {
        FloatTanhNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatTanhNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node("float_tanh", "Tanh", "Hyperbolic tangent", CATEGORY);
        node.set_flowscript_name("float", "tanh");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Tanh", f64::tanh).await
    }
}

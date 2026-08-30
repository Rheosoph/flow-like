use super::unary::{unary_node, unary_run};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct FloatLnNode {}

impl FloatLnNode {
    pub fn new() -> Self {
        FloatLnNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatLnNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_ln",
            "Natural Log",
            "Natural logarithm of a float",
            "Math/Float",
        );
        node.set_flowscript_name("float", "ln");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Natural Log", f64::ln).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatLog10Node {}

impl FloatLog10Node {
    pub fn new() -> Self {
        FloatLog10Node {}
    }
}

#[async_trait]
impl NodeLogic for FloatLog10Node {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_log10",
            "Log 10",
            "Base 10 logarithm of a float",
            "Math/Float",
        );
        node.set_flowscript_name("float", "log10");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Log 10", f64::log10).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatLog2Node {}

impl FloatLog2Node {
    pub fn new() -> Self {
        FloatLog2Node {}
    }
}

#[async_trait]
impl NodeLogic for FloatLog2Node {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_log2",
            "Log 2",
            "Base 2 logarithm of a float",
            "Math/Float",
        );
        node.set_flowscript_name("float", "log2");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Log 2", f64::log2).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatLogNode {}

impl FloatLogNode {
    pub fn new() -> Self {
        FloatLogNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatLogNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_log",
            "Log",
            "Logarithm of a float to a custom base",
            "Math/Float",
        );
        node.set_flowscript_name("float", "log");
        node.set_receiver("float");
        node.add_input_pin("base", "Base", "Logarithm base", VariableType::Float)
            .set_default_value(Some(json!(10.0)));

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let base: f64 = context.evaluate_pin("base").await?;
        unary_run(context, "Log", |value| value.log(base)).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatExpNode {}

impl FloatExpNode {
    pub fn new() -> Self {
        FloatExpNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatExpNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_exp",
            "Exp",
            "Raises e to the power of a float",
            "Math/Float",
        );
        node.set_flowscript_name("float", "exp");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Exp", f64::exp).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatExp2Node {}

impl FloatExp2Node {
    pub fn new() -> Self {
        FloatExp2Node {}
    }
}

#[async_trait]
impl NodeLogic for FloatExp2Node {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_exp2",
            "Exp 2",
            "Raises two to the power of a float",
            "Math/Float",
        );
        node.set_flowscript_name("float", "exp2");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Exp 2", f64::exp2).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct FloatSqrtNode {}

impl FloatSqrtNode {
    pub fn new() -> Self {
        FloatSqrtNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatSqrtNode {
    fn get_node(&self) -> Node {
        let mut node = unary_node(
            "float_sqrt",
            "Square Root",
            "Square root of a float",
            "Math/Float",
        );
        node.set_flowscript_name("float", "sqrt");
        node.set_receiver("float");
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        unary_run(context, "Square Root", f64::sqrt).await
    }
}

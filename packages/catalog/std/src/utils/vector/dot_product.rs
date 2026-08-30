use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::json::json;
#[cfg(feature = "execute")]
use nalgebra::DVector;

#[crate::register_node]
#[derive(Default)]
pub struct FloatVectorDotProductNode {}

impl FloatVectorDotProductNode {
    pub fn new() -> Self {
        FloatVectorDotProductNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatVectorDotProductNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_vector_dot_product",
            "Dot Product",
            "Calculates the dot product of two float vectors",
            "Utils/Math/Vector",
        );
        node.set_flowscript_name("math.vector", "dot");
        node.set_receiver("vector1");
        node.add_icon("/flow/icons/grip.svg");

        node.add_input_pin(
            "vector1",
            "Vector 1",
            "First float vector",
            VariableType::Float,
        )
        .set_value_type(ValueType::Array);
        node.add_input_pin(
            "vector2",
            "Vector 2",
            "Second float vector",
            VariableType::Float,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "result",
            "Result",
            "Dot product of the two vectors",
            VariableType::Float,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let vector1: Vec<f64> = context.evaluate_pin("vector1").await?;
        let vector2: Vec<f64> = context.evaluate_pin("vector2").await?;

        let v1 = DVector::from_vec(vector1);
        let v2 = DVector::from_vec(vector2);

        if v1.len() != v2.len() {
            return Err(flow_like_types::anyhow!(
                "Vectors must have the same length"
            ));
        }

        let dot_product = v1.dot(&v2);

        context.set_pin_value("result", json!(dot_product)).await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This feature requires the 'execute' feature"
        ))
    }
}

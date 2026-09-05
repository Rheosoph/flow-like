#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
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
pub struct FloatVectorCrossProductNode {}

impl FloatVectorCrossProductNode {
    pub fn new() -> Self {
        FloatVectorCrossProductNode {}
    }
}

#[async_trait]
impl NodeLogic for FloatVectorCrossProductNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "float_vector_cross_product",
            "Cross Product",
            "Calculates the cross product of two float vectors",
            "Utils/Math/Vector",
        );
        node.set_flowscript_name("math.vector", "cross");
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
            "result_vector",
            "Result Vector",
            "Cross product of the two vectors",
            VariableType::Float,
        )
        .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let vector1: Vec<f64> = context.evaluate_pin("vector1").await?;
        let vector2: Vec<f64> = context.evaluate_pin("vector2").await?;

        let v1 = DVector::from_vec(vector1);
        let v2 = DVector::from_vec(vector2);

        if v1.len() != v2.len() || v1.len() < 2 || v1.len() > 3 {
            context.log_message(
                "Vectors must have the same length and be 2D or 3D",
                LogLevel::Error,
            );
            return Err(flow_like_types::anyhow!(
                "Vectors must have the same length and be 2D or 3D"
            ));
        }

        let result_vector = if v1.len() == 2 {
            let cross_product = v1[0] * v2[1] - v1[1] * v2[0];
            vec![cross_product]
        } else {
            let cross_product = v1.cross(&v2);
            cross_product.iter().cloned().collect::<Vec<_>>()
        };

        context
            .set_pin_value("result_vector", json!(result_vector))
            .await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This feature requires the 'execute' feature"
        ))
    }
}

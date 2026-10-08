use crate::onnx::NodeOnnxSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Result, anyhow, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const MAX_TENSOR_ELEMENTS: usize = 16_777_216;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "dtype", content = "values", rename_all = "snake_case")]
pub enum TensorValues {
    Float32(Vec<f32>),
    Float64(Vec<f64>),
    Int64(Vec<i64>),
    Int32(Vec<i32>),
    Bool(Vec<bool>),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NamedTensor {
    pub name: String,
    pub shape: Vec<usize>,
    pub data: TensorValues,
}

impl NamedTensor {
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() || self.shape.len() > 16 {
            return Err(anyhow!("A tensor needs a name and at most 16 dimensions"));
        }
        let count = self
            .shape
            .iter()
            .try_fold(1usize, |n, d| n.checked_mul(*d))
            .ok_or_else(|| anyhow!("Tensor shape overflows"))?;
        let (len, finite) = match &self.data {
            TensorValues::Float32(v) => (v.len(), v.iter().all(|x| x.is_finite())),
            TensorValues::Float64(v) => (v.len(), v.iter().all(|x| x.is_finite())),
            TensorValues::Int64(v) => (v.len(), true),
            TensorValues::Int32(v) => (v.len(), true),
            TensorValues::Bool(v) => (v.len(), true),
        };
        if count != len || count > MAX_TENSOR_ELEMENTS || !finite {
            return Err(anyhow!(
                "Tensor '{}' has invalid shape, size, or non-finite values",
                self.name
            ));
        }
        Ok(())
    }

    #[cfg(feature = "execute")]
    fn into_value(self) -> Result<flow_like_model_provider::ml::ort::value::DynValue> {
        use flow_like_model_provider::ml::ort::value::Tensor;
        self.validate()?;
        Ok(match self.data {
            TensorValues::Float32(v) => Tensor::from_array((self.shape, v))?.into_dyn(),
            TensorValues::Float64(v) => Tensor::from_array((self.shape, v))?.into_dyn(),
            TensorValues::Int64(v) => Tensor::from_array((self.shape, v))?.into_dyn(),
            TensorValues::Int32(v) => Tensor::from_array((self.shape, v))?.into_dyn(),
            TensorValues::Bool(v) => Tensor::from_array((self.shape, v))?.into_dyn(),
        })
    }
}

#[cfg(feature = "execute")]
pub fn infer_tensors(
    session: &mut flow_like_model_provider::ml::ort::session::Session,
    tensors: Vec<NamedTensor>,
) -> Result<Vec<NamedTensor>> {
    use flow_like_model_provider::ml::ort::session::SessionInputValue;
    use std::{borrow::Cow, collections::HashSet};
    if tensors.is_empty() || tensors.len() > 128 {
        return Err(anyhow!("Supply between 1 and 128 named input tensors"));
    }
    let mut names = HashSet::new();
    let mut inputs: Vec<(Cow<'static, str>, SessionInputValue<'static>)> = Vec::new();
    for tensor in tensors {
        if !names.insert(tensor.name.clone()) {
            return Err(anyhow!("Duplicate input tensor '{}'", tensor.name));
        }
        let name = tensor.name.clone();
        inputs.push((Cow::Owned(name), tensor.into_value()?.into()));
    }
    let outputs = session.run(inputs)?;
    let mut result = Vec::new();
    for (name, value) in outputs.iter() {
        macro_rules! extract {
            ($ty:ty, $variant:ident) => {
                if let Ok((shape, values)) = value.try_extract_tensor::<$ty>() {
                    let shape = shape
                        .iter()
                        .map(|d| usize::try_from(*d))
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    if values.len() > MAX_TENSOR_ELEMENTS {
                        return Err(anyhow!(
                            "Output tensor '{}' exceeds the element limit",
                            name
                        ));
                    }
                    let tensor = NamedTensor {
                        name: name.to_string(),
                        shape,
                        data: TensorValues::$variant(values.to_vec()),
                    };
                    tensor.validate()?;
                    result.push(tensor);
                    continue;
                }
            };
        }
        extract!(f32, Float32);
        extract!(f64, Float64);
        extract!(i64, Int64);
        extract!(i32, Int32);
        extract!(bool, Bool);
        return Err(anyhow!(
            "Output '{}' uses an unsupported tensor type: {:?}",
            name,
            value.dtype()
        ));
    }
    Ok(result)
}

#[crate::register_node]
#[derive(Default)]
pub struct TensorInferenceNode;

#[async_trait]
impl NodeLogic for TensorInferenceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "onnx_tensor_inference",
            "ONNX Tensor Inference",
            "Run named numeric or boolean tensors through an ONNX model",
            "AI/ML/ONNX",
        );
        node.set_flowscript_name("onnx", "inferTensors");
        node.add_input_pin("exec_in", "Input", "Run inference", VariableType::Execution);
        node.add_input_pin(
            "model",
            "Model",
            "Loaded ONNX session",
            VariableType::Struct,
        )
        .set_schema::<NodeOnnxSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "inputs",
            "Inputs",
            "Named inputs with shape, type and values",
            VariableType::Struct,
        )
        .set_schema::<NamedTensor>()
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "exec_out",
            "Done",
            "Inference completed",
            VariableType::Execution,
        );
        node.add_output_pin(
            "outputs",
            "Outputs",
            "Named output tensors",
            VariableType::Struct,
        )
        .set_schema::<NamedTensor>()
        .set_value_type(ValueType::Array);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let model: NodeOnnxSession = context.evaluate_pin("model").await?;
            let inputs: Vec<NamedTensor> = context.evaluate_pin("inputs").await?;
            let session = model.get_session(context).await?;
            let outputs = infer_tensors(&mut session.lock().await.session, inputs)?;
            context.set_pin_value("outputs", json!(outputs)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "execute"))]
        {
            let _ = context;
            Err(anyhow!("ONNX execution requires the execute feature"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_shape_finite_values_and_scalar() {
        let mut tensor = NamedTensor {
            name: "x".into(),
            shape: vec![2],
            data: TensorValues::Float32(vec![1.0, 2.0]),
        };
        assert!(tensor.validate().is_ok());
        tensor.shape = vec![3];
        assert!(tensor.validate().is_err());
        tensor.shape = vec![];
        tensor.data = TensorValues::Float32(vec![1.0]);
        assert!(tensor.validate().is_ok());
        tensor.data = TensorValues::Float32(vec![f32::NAN]);
        assert!(tensor.validate().is_err());
        tensor.shape = vec![usize::MAX, 2];
        assert!(tensor.validate().is_err());
    }
}

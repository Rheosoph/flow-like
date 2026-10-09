use crate::{Error, Objective, Recipe, Result, TensorData, model::Model};
use burn::tensor::{Device, Tensor, TensorData as BurnData};
use burn_onnx::export::{OnnxExporter, OnnxModel};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct OnnxExportReport {
    pub opset: u32,
    pub input_shape: Vec<usize>,
    pub output_semantics: String,
}
// Burn 0.22's exporter lowers these primitives but does not lower its fused Softmax op.
fn softmax<const D: usize>(input: Tensor<D>, axis: usize) -> Tensor<D> {
    let maximum = input.clone().max_dim_with_indices(axis).0;
    let exponential = (input - maximum).exp();
    exponential.clone() / exponential.sum_dim(axis)
}

impl Model {
    pub(crate) fn export_onnx(
        &self,
        input: &TensorData,
        device: &Device,
        recipe: &Recipe,
    ) -> Result<(OnnxModel, OnnxExportReport)> {
        let exporter = OnnxExporter::new();
        let classify = recipe.objective() == Some(Objective::Classification);
        let probabilities = |x: Tensor<2>| if classify { softmax(x, 1) } else { x };
        let result = match self {
            Self::DinoV2(_) => return Err(Error::Record("DINOv2 attention is not supported by the ONNX exporter".into())),
            Self::RtdetrV2(_) => return Err(Error::Record("RT-DETRv2 deformable attention is not supported by the ONNX exporter".into())),
            Self::RfDetr(_) => return Err(Error::Record("RF-DETR windowed attention is not supported by the ONNX exporter".into())),
            Self::DfineNano(_) => return Err(Error::Record("D-FINE distribution refinement is not supported by the ONNX exporter".into())),
            Self::MaskRcnn(_) => return Err(Error::Record("Native Mask R-CNN proposal selection and ROIAlign are not supported by the ONNX exporter".into())),
            Self::Mlp(m) => exporter.export(m, tensor::<2>(input, device), |m, x| {
                probabilities(m.forward(x))
            }),
            Self::Recurrent(m) => exporter.export(m, tensor::<3>(input, device), |m, x| {
                probabilities(m.forward(x))
            }),
            Self::Cnn1d(m) => exporter.export(m, tensor::<3>(input, device), |m, x| {
                probabilities(m.forward(x))
            }),
            Self::Tcn(m) => exporter.export(m, tensor::<3>(input, device), |m, x| {
                probabilities(m.forward(x))
            }),
            Self::DenseAutoencoder(m) => {
                exporter.export(m, tensor::<2>(input, device), |m, x| m.forward(x))
            }
            Self::ConvAutoencoder(m) => {
                exporter.export(m, tensor::<3>(input, device), |m, x| m.forward(x))
            }
            Self::LstmAutoencoder(m) => {
                exporter.export(m, tensor::<3>(input, device), |m, x| m.forward(x))
            }
            Self::CnnLstm(m) => exporter.export(m, tensor::<5>(input, device), |m, x| {
                probabilities(m.forward(x))
            }),
            Self::ImageSensorFusion(m) => exporter.export(m, tensor::<2>(input, device), |m, x| {
                probabilities(m.forward(x))
            }),
            Self::ResNet18(m) => exporter.export(m, tensor::<4>(input, device), |m, x| {
                softmax(m.forward(x), 1)
            }),
            Self::MobileNetV2(m) => exporter.export(m, tensor::<4>(input, device), |m, x| {
                softmax(m.forward(x), 1)
            }),
            Self::EfficientNet(m) => exporter.export(m, tensor::<4>(input, device), |m, x| {
                softmax(m.forward(x), 1)
            }),
            Self::YoloX(m) => exporter.export(m, tensor::<4>(input, device), |m, x| {
                m.forward(x).raw_logits()
            }),
            Self::UNet(m) => exporter.export(m, tensor::<4>(input, device), |m, x| {
                softmax(m.forward(x), 1)
            }),
        }
        .map_err(|e| Error::Record(format!("ONNX export is unavailable for this graph: {e}")))?;
        let output_semantics = if recipe.is_detection() {
            "yolox_raw_logits_b_a_5_plus_classes; grid_decode_and_nms_required"
        } else if classify {
            "class_probabilities"
        } else if recipe.objective().is_none() {
            "reconstruction"
        } else {
            "regression_values"
        }
        .to_string();
        Ok((
            result,
            OnnxExportReport {
                opset: 18,
                input_shape: input.shape.clone(),
                output_semantics,
            },
        ))
    }
}
fn tensor<const D: usize>(input: &TensorData, device: &Device) -> Tensor<D> {
    Tensor::from_data(
        BurnData::new(input.values.clone(), input.shape.clone()),
        device,
    )
}

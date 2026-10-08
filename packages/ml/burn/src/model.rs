use crate::DetectionOptions;
use crate::fusion::{CnnLstm, ImageSensorFusion};
use crate::mask_rcnn::{MaskRcnn, MaskRcnnOutput};
use crate::mobile::{EfficientNet, MobileNetV2};
use crate::temporal::{
    Cnn1d, ConvAutoencoder, DenseAutoencoder, LstmAutoencoder, Mlp, Recurrent, Tcn,
};
use crate::vision::{ResNet18, UNet};
use crate::yolox::{YoloOutput, YoloX};
use crate::{
    Error, Objective, PredictionBatch, Recipe, Result, Targets, TensorData, TensorDataset,
};
use burn::{
    module::Module,
    nn::loss::{CrossEntropyLossConfig, MseLoss, Reduction},
    tensor::{Device, Int, Tensor, TensorData as BurnData, activation::softmax},
};

#[derive(Module, Debug)]
pub(crate) enum Model {
    Mlp(Mlp),
    Recurrent(Recurrent),
    Cnn1d(Cnn1d),
    Tcn(Tcn),
    DenseAutoencoder(DenseAutoencoder),
    ConvAutoencoder(ConvAutoencoder),
    LstmAutoencoder(LstmAutoencoder),
    CnnLstm(CnnLstm),
    ImageSensorFusion(ImageSensorFusion),
    ResNet18(ResNet18),
    MobileNetV2(MobileNetV2),
    EfficientNet(EfficientNet),
    YoloX(YoloX),
    MaskRcnn(MaskRcnn),
    UNet(UNet),
}
impl Model {
    pub(crate) fn new(recipe: &Recipe, device: &Device) -> Result<Self> {
        Ok(match recipe {
            Recipe::MaskRcnn { config } => Self::MaskRcnn(MaskRcnn::new(config, device)?),
            Recipe::Mlp {
                input_features,
                hidden,
                outputs,
                ..
            } => Self::Mlp(Mlp::new(*input_features, *hidden, *outputs, device)),
            Recipe::Lstm {
                input_features,
                hidden,
                outputs,
                ..
            } => Self::Recurrent(Recurrent::new(
                *input_features,
                *hidden,
                *outputs,
                true,
                device,
            )),
            Recipe::Gru {
                input_features,
                hidden,
                outputs,
                ..
            } => Self::Recurrent(Recurrent::new(
                *input_features,
                *hidden,
                *outputs,
                false,
                device,
            )),
            Recipe::Cnn1d {
                input_features,
                hidden,
                outputs,
                ..
            } => Self::Cnn1d(Cnn1d::new(*input_features, *hidden, *outputs, device)),
            Recipe::Tcn {
                input_features,
                hidden,
                levels,
                outputs,
                ..
            } => Self::Tcn(Tcn::new(
                *input_features,
                *hidden,
                *levels,
                *outputs,
                device,
            )),
            Recipe::DenseAutoencoder {
                input_features,
                hidden,
                latent,
            } => Self::DenseAutoencoder(DenseAutoencoder::new(
                *input_features,
                *hidden,
                *latent,
                device,
            )),
            Recipe::Conv1dAutoencoder {
                input_features,
                hidden,
                latent,
            } => Self::ConvAutoencoder(ConvAutoencoder::new(
                *input_features,
                *hidden,
                *latent,
                device,
            )),
            Recipe::LstmAutoencoder {
                input_features,
                hidden,
                latent,
            } => Self::LstmAutoencoder(LstmAutoencoder::new(
                *input_features,
                *hidden,
                *latent,
                device,
            )),
            Recipe::CnnLstm {
                input_channels,
                cnn_channels,
                hidden,
                outputs,
                ..
            } => Self::CnnLstm(CnnLstm::new(
                *input_channels,
                *cnn_channels,
                *hidden,
                *outputs,
                device,
            )),
            Recipe::ImageSensorFusion {
                input_channels,
                height,
                width,
                sensor_features,
                cnn_channels,
                hidden,
                outputs,
                ..
            } => Self::ImageSensorFusion(ImageSensorFusion::new(
                *input_channels,
                *height,
                *width,
                *sensor_features,
                *cnn_channels,
                *hidden,
                *outputs,
                device,
            )),
            Recipe::ResNet18 {
                input_channels,
                classes,
                base_channels,
            } => Self::ResNet18(ResNet18::new(
                *input_channels,
                *classes,
                *base_channels,
                device,
            )),
            Recipe::MobileNetV2 {
                input_channels,
                classes,
                width_multiplier,
            } => Self::MobileNetV2(MobileNetV2::new(
                *input_channels,
                *classes,
                *width_multiplier,
                device,
            )),
            Recipe::EfficientNet {
                input_channels,
                classes,
                width_multiplier,
            } => Self::EfficientNet(EfficientNet::new(
                *input_channels,
                *classes,
                *width_multiplier,
                device,
            )),
            Recipe::YoloX {
                input_channels,
                classes,
                width_multiplier,
                depth_multiplier,
            } => Self::YoloX(YoloX::new(
                *input_channels,
                *classes,
                *width_multiplier,
                *depth_multiplier,
                device,
            )),
            Recipe::UNet {
                input_channels,
                classes,
                base_channels,
                depth,
            } => Self::UNet(UNet::new(
                *input_channels,
                *classes,
                *base_channels,
                *depth,
                device,
            )),
        })
    }
    pub(crate) fn forward(&self, input: &TensorData, device: &Device) -> Result<Output> {
        Ok(match self {
            Self::MaskRcnn(m) => Output::MaskRcnn(m.forward(tensor(input, device), None)?),
            Self::Mlp(m) => Output::Matrix(m.forward(tensor(input, device))),
            Self::Recurrent(m) => Output::Matrix(m.forward(tensor(input, device))),
            Self::Cnn1d(m) => Output::Matrix(m.forward(tensor(input, device))),
            Self::Tcn(m) => Output::Matrix(m.forward(tensor(input, device))),
            Self::DenseAutoencoder(m) => Output::Matrix(m.forward(tensor(input, device))),
            Self::ConvAutoencoder(m) => Output::Sequence(m.forward(tensor(input, device))),
            Self::LstmAutoencoder(m) => Output::Sequence(m.forward(tensor(input, device))),
            Self::CnnLstm(m) => Output::Matrix(m.forward(tensor(input, device))),
            Self::ImageSensorFusion(m) => Output::Matrix(m.forward(tensor(input, device))),
            Self::ResNet18(m) => Output::Matrix(m.forward(tensor(input, device))),
            Self::MobileNetV2(m) => Output::Matrix(m.forward(tensor(input, device))),
            Self::EfficientNet(m) => Output::Matrix(m.forward(tensor(input, device))),
            Self::YoloX(m) => Output::Detection(m.forward(tensor(input, device))),
            Self::UNet(m) => Output::Image(m.forward(tensor(input, device))),
        })
    }
    pub(crate) fn loss(&self, data: &TensorDataset, device: &Device) -> Result<Tensor<1>> {
        if let (Self::MaskRcnn(model), Targets::Instances { values }) = (self, &data.targets) {
            return model
                .forward(tensor(&data.inputs, device), Some(values))?
                .loss(values);
        }
        Ok(match (self.forward(&data.inputs, device)?, &data.targets) {
            (Output::Matrix(logits), Targets::Classes { values }) => {
                CrossEntropyLossConfig::new().init(device).forward(
                    logits,
                    Tensor::<1, Int>::from_data(
                        BurnData::new(values.clone(), [values.len()]),
                        device,
                    ),
                )
            }
            (Output::Matrix(output), Targets::Dense { tensor: target }) => {
                MseLoss::new().forward(output, tensor(target, device), Reduction::Mean)
            }
            (Output::Matrix(output), Targets::Reconstruction) => {
                MseLoss::new().forward(output, tensor(&data.inputs, device), Reduction::Mean)
            }
            (Output::Sequence(output), Targets::Reconstruction) => {
                MseLoss::new().forward(output, tensor(&data.inputs, device), Reduction::Mean)
            }
            (Output::Image(output), Targets::Segmentation { values, .. }) => {
                let [b, c, h, w] = output.dims();
                let logits = output.permute([0, 2, 3, 1]).reshape([b * h * w, c]);
                CrossEntropyLossConfig::new().init(device).forward(
                    logits,
                    Tensor::<1, Int>::from_data(
                        BurnData::new(values.clone(), [values.len()]),
                        device,
                    ),
                )
            }
            (Output::Detection(output), Targets::Boxes { values }) => output.loss(values),
            _ => unreachable!("Dataset and recipe validated before training"),
        })
    }
}
fn tensor<const D: usize>(input: &TensorData, device: &Device) -> Tensor<D> {
    Tensor::from_data(
        BurnData::new(input.values.clone(), input.shape.clone()),
        device,
    )
}
pub(crate) enum Output {
    Matrix(Tensor<2>),
    Sequence(Tensor<3>),
    Image(Tensor<4>),
    Detection(YoloOutput),
    MaskRcnn(MaskRcnnOutput),
}
impl Output {
    pub(crate) fn prediction(
        self,
        recipe: &Recipe,
        input: &TensorData,
        options: &DetectionOptions,
    ) -> Result<PredictionBatch> {
        if let Self::MaskRcnn(output) = self {
            let instances = output.prediction(options)?.instances;
            let scores = instances
                .iter()
                .map(|sample| {
                    sample
                        .iter()
                        .map(|item| item.detection.confidence)
                        .fold(0.0f32, f32::max)
                })
                .collect();
            let detections = instances
                .iter()
                .map(|sample| sample.iter().map(|item| item.detection.clone()).collect())
                .collect();
            return Ok(PredictionBatch {
                output: TensorData {
                    shape: vec![instances.len(), 1],
                    values: scores,
                },
                classes: None,
                reconstruction_error: None,
                detections: Some(detections),
                instances: Some(instances),
            });
        }
        if let Self::Detection(output) = self {
            let (output, detections) = output.prediction(options)?;
            return Ok(PredictionBatch {
                output,
                classes: None,
                reconstruction_error: None,
                detections: Some(detections),
                instances: None,
            });
        }
        let (output, classes) = match self {
            Self::Matrix(x) if recipe.objective() == Some(Objective::Classification) => {
                let probs = softmax(x, 1);
                let classes = probs
                    .clone()
                    .argmax(1)
                    .into_data()
                    .convert::<i64>()
                    .try_to_vec::<i64>()
                    .map_err(|e| Error::Record(e.to_string()))?;
                (to_data(probs)?, Some(classes))
            }
            Self::Image(x) => {
                let probs = softmax(x, 1);
                let classes = probs
                    .clone()
                    .argmax(1)
                    .into_data()
                    .convert::<i64>()
                    .try_to_vec::<i64>()
                    .map_err(|e| Error::Record(e.to_string()))?;
                (to_data(probs)?, Some(classes))
            }
            Self::Matrix(x) => (to_data(x)?, None),
            Self::Sequence(x) => (to_data(x)?, None),
            Self::Detection(_) => unreachable!("Detection handled above"),
            Self::MaskRcnn(_) => unreachable!("Instance detection handled above"),
        };
        let reconstruction_error = if recipe.objective().is_none() {
            let stride = input.values.len() / input.shape[0];
            Some(
                input
                    .values
                    .chunks_exact(stride)
                    .zip(output.values.chunks_exact(stride))
                    .map(|(a, b)| {
                        a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum::<f32>() / stride as f32
                    })
                    .collect(),
            )
        } else {
            None
        };
        Ok(PredictionBatch {
            output,
            classes,
            reconstruction_error,
            detections: None,
            instances: None,
        })
    }
}
fn to_data<const D: usize>(x: Tensor<D>) -> Result<TensorData> {
    let shape = x.dims().to_vec();
    let values = x
        .into_data()
        .try_to_vec::<f32>()
        .map_err(|e| Error::Record(e.to_string()))?;
    let data = TensorData { shape, values };
    data.validate()?;
    Ok(data)
}

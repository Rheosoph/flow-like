// Native port of RT-DETRv2, Copyright (c) 2023 lyuwenyu.
// Licensed under Apache-2.0; see licenses/RT-DETR.
use crate::{
    Error, Result, RtdetrV2Backbone, RtdetrV2Config,
    detr_layers::{ConvNorm, DecoderLayer, Mlp, inverse_sigmoid, linear, norm},
    pretrained::SafeTensorReader,
    rtdetr_backbone::{HybridEncoder, PResNet},
};
use burn::{
    module::{Module, Param},
    nn::{LayerNorm, Linear},
    tensor::{Device, Tensor, TensorData, activation::sigmoid},
};

#[derive(Clone, Debug)]
pub struct DetrPrediction {
    /// Independent foreground class logits, without a background class.
    pub logits: Tensor<3>,
    /// Normalized center-x, center-y, width, height boxes.
    pub boxes: Tensor<3>,
}
#[derive(Clone, Debug)]
pub struct DetrOutput {
    pub prediction: DetrPrediction,
    pub auxiliary: Vec<DetrPrediction>,
}
impl DetrOutput {
    pub(crate) fn loss(&self, targets: &[Vec<crate::BoundingBox>]) -> Result<Tensor<1>> {
        let config = crate::DetrLossConfig::default();
        let mut loss = crate::detr_loss(
            self.prediction.logits.clone(),
            self.prediction.boxes.clone(),
            targets,
            &config,
        )?;
        for auxiliary in &self.auxiliary {
            loss = loss
                + crate::detr_loss(
                    auxiliary.logits.clone(),
                    auxiliary.boxes.clone(),
                    targets,
                    &config,
                )?;
        }
        Ok(loss)
    }
}
impl DetrPrediction {
    /// DETR predicts a set of objects. Rank query/class pairs without NMS.
    pub(crate) fn prediction(
        self,
        options: &crate::DetectionOptions,
    ) -> Result<crate::PredictionBatch> {
        options.validate()?;
        let [batch, queries, classes] = self.logits.dims();
        if self.boxes.dims() != [batch, queries, 4] {
            return Err(Error::Invalid("DETR box and logit shapes differ".into()));
        }
        let scores = sigmoid(self.logits)
            .to_data()
            .try_to_vec::<f32>()
            .map_err(|e| Error::Record(e.to_string()))?;
        let boxes = self
            .boxes
            .to_data()
            .try_to_vec::<f32>()
            .map_err(|e| Error::Record(e.to_string()))?;
        if scores.iter().chain(&boxes).any(|v| !v.is_finite()) {
            return Err(Error::Invalid(
                "DETR produced non-finite predictions".into(),
            ));
        }
        let mut detections = Vec::with_capacity(batch);
        let mut maxima = Vec::with_capacity(batch);
        for sample in 0..batch {
            let mut ranked: Vec<_> = scores
                [sample * queries * classes..(sample + 1) * queries * classes]
                .iter()
                .copied()
                .enumerate()
                .filter(|(_, score)| *score >= options.confidence_threshold)
                .collect();
            ranked.sort_by(|(ia, a), (ib, b)| b.total_cmp(a).then(ia.cmp(ib)));
            let selected: Vec<_> = ranked
                .into_iter()
                .filter_map(|(index, confidence)| {
                    let offset = (sample * queries + index / classes) * 4;
                    let [cx, cy, width, height] = boxes[offset..offset + 4].try_into().unwrap();
                    let detection = crate::Detection {
                        class_id: index % classes,
                        confidence,
                        x_min: (cx - width / 2.0f32).clamp(0.0, 1.0),
                        y_min: (cy - height / 2.0f32).clamp(0.0, 1.0),
                        x_max: (cx + width / 2.0f32).clamp(0.0, 1.0),
                        y_max: (cy + height / 2.0f32).clamp(0.0, 1.0),
                    };
                    (detection.x_max > detection.x_min && detection.y_max > detection.y_min)
                        .then_some(detection)
                })
                .take(options.max_detections)
                .collect();
            maxima.push(
                selected
                    .first()
                    .map_or(0.0, |detection| detection.confidence),
            );
            detections.push(selected);
        }
        Ok(crate::PredictionBatch {
            output: crate::TensorData {
                shape: vec![batch, 1],
                values: maxima,
            },
            classes: None,
            reconstruction_error: None,
            detections: Some(detections),
            instances: None,
        })
    }
}

#[derive(Module, Debug)]
pub struct RtdetrV2 {
    backbone: PResNet,
    encoder: HybridEncoder,
    input_projections: Vec<ConvNorm>,
    encoder_projection: Linear,
    encoder_norm: LayerNorm,
    encoder_score: Linear,
    encoder_boxes: Mlp,
    query_position: Mlp,
    decoder: Vec<DecoderLayer>,
    scores: Vec<Linear>,
    boxes: Vec<Mlp>,
    #[module(skip)]
    config: RtdetrV2Config,
}
impl RtdetrV2 {
    pub fn new(config: &RtdetrV2Config, device: &Device) -> Result<Self> {
        config.validate()?;
        let bottleneck = config.backbone == RtdetrV2Backbone::ResNet50;
        let channels = 256;
        let inputs = if bottleneck {
            [512, 1024, 2048]
        } else {
            [128, 256, 512]
        };
        Ok(Self {
            backbone: PResNet::new(bottleneck, device),
            encoder: HybridEncoder::new(
                inputs,
                channels,
                8,
                if bottleneck { 1.0 } else { 0.5 },
                device,
            ),
            input_projections: (0..3)
                .map(|_| ConvNorm::new(channels, channels, 1, 1, device))
                .collect(),
            encoder_projection: linear(channels, channels, device),
            encoder_norm: norm(channels, device),
            encoder_score: score_head(channels, config.classes, device),
            encoder_boxes: Mlp::new(channels, channels, 4, 3, device).zero_last(),
            query_position: Mlp::new(4, channels * 2, channels, 2, device),
            decoder: (0..config.layers())
                .map(|_| DecoderLayer::new(channels, 8, vec![4, 4, 4], 1024, device))
                .collect(),
            scores: (0..config.layers())
                .map(|_| score_head(channels, config.classes, device))
                .collect(),
            boxes: (0..config.layers())
                .map(|_| Mlp::new(channels, channels, 4, 3, device).zero_last())
                .collect(),
            config: config.clone(),
        })
    }
    pub fn forward(&self, input: Tensor<4>) -> Result<DetrOutput> {
        self.config.validate_input_shape(&input.dims())?;
        let features = self.encoder.forward(self.backbone.forward(input));
        let features: Vec<_> = features
            .into_iter()
            .zip(&self.input_projections)
            .map(|(feature, layer)| layer.forward(feature))
            .collect();
        let shapes: Vec<_> = features
            .iter()
            .map(|feature| [feature.dims()[2], feature.dims()[3]])
            .collect();
        let memory = Tensor::cat(
            features
                .into_iter()
                .map(|feature| {
                    let [batch, channels, height, width] = feature.dims();
                    feature
                        .reshape([batch, channels, height * width])
                        .swap_dims(1, 2)
                })
                .collect(),
            1,
        );
        let [batch, _, channels] = memory.dims();
        let (anchors, valid) = anchors(&shapes, &memory.device());
        let projected = self
            .encoder_norm
            .forward(self.encoder_projection.forward(memory.clone() * valid));
        let logits = self.encoder_score.forward(projected.clone());
        let coordinates = self.encoder_boxes.forward(projected.clone()) + anchors;
        let indices = logits
            .clone()
            .max_dim(2)
            .squeeze_dim::<2>(2)
            .topk_with_indices(self.config.queries, 1)
            .1
            .unsqueeze_dim::<3>(2);
        let coordinates =
            coordinates.gather(1, indices.clone().expand([batch, self.config.queries, 4]));
        let mut target = projected
            .gather(
                1,
                indices
                    .clone()
                    .expand([batch, self.config.queries, channels]),
            )
            .detach();
        let mut references = sigmoid(coordinates.clone().detach());
        let mut predictions = vec![DetrPrediction {
            logits: logits.gather(
                1,
                indices.expand([batch, self.config.queries, self.config.classes]),
            ),
            boxes: sigmoid(coordinates),
        }];
        let mut previous = references.clone();
        for (i, layer) in self.decoder.iter().enumerate() {
            target = layer.forward(
                target,
                self.query_position.forward(references.clone()),
                references.clone(),
                memory.clone(),
                &shapes,
            );
            let offset = self.boxes[i].forward(target.clone());
            let refined = sigmoid(offset.clone() + inverse_sigmoid(references));
            let output_boxes = if i == 0 {
                refined.clone()
            } else {
                sigmoid(offset + inverse_sigmoid(previous))
            };
            predictions.push(DetrPrediction {
                logits: self.scores[i].forward(target.clone()),
                boxes: output_boxes,
            });
            previous = refined.clone();
            references = refined.detach();
        }
        Ok(DetrOutput {
            prediction: predictions.pop().unwrap(),
            auxiliary: predictions,
        })
    }
    pub(crate) fn reset_classifier(mut self, classes: usize, device: &Device) -> Self {
        self.encoder_score = score_head(256, classes, device);
        self.scores = (0..self.config.layers())
            .map(|_| score_head(256, classes, device))
            .collect();
        self.config.classes = classes;
        self
    }
    pub(crate) fn configure_fine_tuning(mut self, freeze_backbone: bool) -> Self {
        self = self.unfreeze();
        if freeze_backbone {
            self.backbone = self.backbone.freeze();
        }
        self
    }
    pub(crate) fn import_safetensors(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.backbone = self.backbone.import(reader, device)?;
        self.encoder = self.encoder.import(reader, device)?;
        self.input_projections = self
            .input_projections
            .into_iter()
            .enumerate()
            .map(|(i, layer)| layer.import(&format!("decoder.input_proj.{i}"), reader, device))
            .collect::<Result<_>>()?;
        self.encoder_projection =
            reader.linear(self.encoder_projection, "decoder.enc_output.proj", device)?;
        self.encoder_norm =
            reader.layer_norm(self.encoder_norm, "decoder.enc_output.norm", device)?;
        self.encoder_score = reader.linear(self.encoder_score, "decoder.enc_score_head", device)?;
        self.encoder_boxes = self
            .encoder_boxes
            .import("decoder.enc_bbox_head", reader, device)?;
        self.query_position =
            self.query_position
                .import("decoder.query_pos_head", reader, device)?;
        self.decoder = self
            .decoder
            .into_iter()
            .enumerate()
            .map(|(i, layer)| layer.import(&format!("decoder.decoder.layers.{i}"), reader, device))
            .collect::<Result<_>>()?;
        self.scores = self
            .scores
            .into_iter()
            .enumerate()
            .map(|(i, layer)| reader.linear(layer, &format!("decoder.dec_score_head.{i}"), device))
            .collect::<Result<_>>()?;
        self.boxes = self
            .boxes
            .into_iter()
            .enumerate()
            .map(|(i, layer)| layer.import(&format!("decoder.dec_bbox_head.{i}"), reader, device))
            .collect::<Result<_>>()?;
        // Denoising is a training augmentation; fine-tuning here uses matching queries only.
        if reader.has("decoder.denoising_class_embed.weight") {
            let _ = reader.tensor::<2>(
                "decoder.denoising_class_embed.weight",
                [self.config.classes + 1, 256],
                device,
            )?;
        }
        validate_anchor_buffers(reader, &[[80, 80], [40, 40], [20, 20]], device)?;
        Ok(self)
    }
}

fn score_head(channels: usize, classes: usize, device: &Device) -> Linear {
    let mut layer = linear(channels, classes, device);
    layer.bias = Some(Param::from_tensor(Tensor::full(
        [classes],
        (0.01f32 / 0.99).ln(),
        device,
    )));
    layer
}
pub(crate) fn anchors(shapes: &[[usize; 2]], device: &Device) -> (Tensor<3>, Tensor<3>) {
    let count = shapes.iter().map(|[h, w]| h * w).sum::<usize>();
    let mut boxes = Vec::with_capacity(count * 4);
    let mut valid = Vec::with_capacity(count);
    for (level, &[height, width]) in shapes.iter().enumerate() {
        for y in 0..height {
            for x in 0..width {
                let wh = 0.05 * 2f32.powi(level as i32);
                let anchor = [
                    (x as f32 + 0.5) / width as f32,
                    (y as f32 + 0.5) / height as f32,
                    wh,
                    wh,
                ];
                let is_valid = anchor.iter().all(|&v| v > 0.01 && v < 0.99);
                valid.push(if is_valid { 1.0f32 } else { 0.0 });
                boxes.extend(anchor.map(|v| {
                    if is_valid {
                        (v / (1.0 - v)).ln()
                    } else {
                        f32::INFINITY
                    }
                }));
            }
        }
    }
    (
        Tensor::from_data(TensorData::new(boxes, [1, count, 4]), device),
        Tensor::from_data(TensorData::new(valid, [1, count, 1]), device),
    )
}

/// Upstream evaluation buffers use 640px images. Validate before replacing them at runtime.
pub(crate) fn validate_anchor_buffers(
    reader: &mut SafeTensorReader<'_>,
    shapes: &[[usize; 2]],
    device: &Device,
) -> Result<()> {
    let present = [
        reader.has("decoder.anchors"),
        reader.has("decoder.valid_mask"),
    ];
    if present == [false, false] {
        return Ok(());
    }
    if present != [true, true] {
        return Err(Error::Invalid(
            "Detector checkpoint contains incomplete anchor buffers".into(),
        ));
    }
    let count = shapes.iter().map(|[h, w]| h * w).sum::<usize>();
    let (dtype, shape, bytes) = reader.raw("decoder.anchors")?;
    if dtype != safetensors::Dtype::F32 || shape != [1, count, 4] {
        return Err(Error::Invalid(
            "Detector checkpoint anchor buffer does not match its 640px evaluation shape".into(),
        ));
    }
    let (expected, mask) = anchors(shapes, device);
    let expected = expected
        .to_data()
        .try_to_vec::<f32>()
        .map_err(|e| Error::Record(e.to_string()))?;
    for (index, (bytes, expected)) in bytes.chunks_exact(4).zip(expected).enumerate() {
        let actual = f32::from_le_bytes(bytes.try_into().unwrap());
        // Released checkpoints contain rounded cached anchors (up to 3e-4 in logit space).
        // These derived buffers are checked here and regenerated for each actual input size.
        if actual.is_nan() || (actual != expected && (actual - expected).abs() > 1e-3) {
            return Err(Error::Invalid(format!(
                "Detector checkpoint anchor {index} is {actual}, expected {expected}"
            )));
        }
    }
    let (dtype, shape, bytes) = reader.raw("decoder.valid_mask")?;
    let expected = mask
        .to_data()
        .try_to_vec::<f32>()
        .map_err(|e| Error::Record(e.to_string()))?;
    if dtype != safetensors::Dtype::BOOL
        || shape != [1, count, 1]
        || bytes
            .iter()
            .zip(expected)
            .any(|(actual, expected)| *actual != expected as u8)
    {
        return Err(Error::Invalid(
            "Detector checkpoint valid-anchor mask does not match the architecture".into(),
        ));
    }
    Ok(())
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Requires an official RT-DETRv2 R18 checkpoint at FLOW_LIKE_RTDETR_WEIGHTS"]
    fn official_rtdetr_checkpoint_import_and_forward() {
        let _guard = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let path = std::env::var("FLOW_LIKE_RTDETR_WEIGHTS").unwrap();
        let metadata = crate::PretrainedWeightsMetadata {
            source: "https://github.com/lyuwenyu/RT-DETR".into(),
            license: Some("Apache-2.0".into()),
            weights_license: None,
            revision: None,
            expected_sha256: None,
        };
        let (bytes, _, _) = crate::pretrained::read_import_weights(
            std::path::Path::new(&path),
            &metadata,
            1024 * 1024 * 1024,
        )
        .unwrap();
        let mut reader = SafeTensorReader::new(&bytes).unwrap();
        let device = crate::backend::device(&crate::BackendChoice::Cpu, false).unwrap();
        let model = RtdetrV2::new(&RtdetrV2Config::default(), &device)
            .unwrap()
            .import_safetensors(&mut reader, &device)
            .unwrap()
            .valid();
        reader.finish().unwrap();
        // The same fully convolutional checkpoint accepts smaller validated images.
        let input = Tensor::from_data(
            TensorData::new(
                (0..3 * 128 * 128)
                    .map(|index| (index % 251) as f32 / 250.0)
                    .collect::<Vec<_>>(),
                [1, 3, 128, 128],
            ),
            &device,
        );
        let output = model.forward(input).unwrap();
        assert_eq!(output.prediction.logits.dims(), [1, 300, 80]);
        let logits = output
            .prediction
            .logits
            .to_data()
            .try_to_vec::<f32>()
            .unwrap();
        let boxes = output
            .prediction
            .boxes
            .to_data()
            .try_to_vec::<f32>()
            .unwrap();
        // Official graph at source revision 29320b6, F32 CPU, same deterministic input.
        for (&actual, expected) in logits.iter().zip([
            -3.7485268f32,
            -4.9234886,
            -5.035775,
            -6.404114,
            -4.649565,
            -5.3761215,
            -4.76769,
            -5.2843113,
        ]) {
            assert!(
                (actual - expected).abs() < 1e-3,
                "logit {actual} != {expected}"
            );
        }
        for (&actual, expected) in boxes.iter().zip([
            0.48875308f32,
            0.43327788,
            0.45806688,
            0.32825494,
            0.5580443,
            0.5486739,
            0.3112401,
            0.13382667,
        ]) {
            assert!(
                (actual - expected).abs() < 1e-3,
                "box {actual} != {expected}"
            );
        }
        if let Ok(reference) = std::env::var("FLOW_LIKE_RTDETR_REFERENCE") {
            let reference: serde_json::Value =
                serde_json::from_slice(&std::fs::read(reference).unwrap()).unwrap();
            for (name, values) in [("logits", &logits), ("boxes", &boxes)] {
                let expected = reference[name].as_array().unwrap();
                assert_eq!(values.len(), expected.len());
                let maximum = values
                    .iter()
                    .zip(expected)
                    .map(|(actual, expected)| (*actual as f64 - expected.as_f64().unwrap()).abs())
                    .fold(0.0f64, f64::max);
                assert!(maximum < 1e-3, "RT-DETRv2 {name} max error {maximum}");
                eprintln!("RT-DETRv2 {name} max error {maximum}");
            }
        }
        let predictions = output
            .prediction
            .prediction(&crate::DetectionOptions::default())
            .unwrap();
        assert!(predictions.output.values.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn detr_keeps_overlapping_queries_and_ranks_classes() {
        let _guard = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, false).unwrap();
        let prediction = DetrPrediction {
            logits: Tensor::from_data([[[2.0f32, -4.0], [3.0, 1.0]]], &device),
            boxes: Tensor::from_data([[[0.5f32, 0.5, 0.5, 0.5], [0.5, 0.5, 0.5, 0.5]]], &device),
        }
        .prediction(&crate::DetectionOptions {
            confidence_threshold: 0.5,
            max_detections: 3,
            ..Default::default()
        })
        .unwrap();
        let detections = &prediction.detections.unwrap()[0];
        assert_eq!(detections.len(), 3);
        assert_eq!(
            detections.iter().map(|d| d.class_id).collect::<Vec<_>>(),
            [0, 0, 1]
        );
        assert!(detections[0].confidence > detections[1].confidence);
        assert_eq!(detections[0].x_min, 0.25);
        assert_eq!(detections[0].x_max, 0.75);
    }

    #[test]
    fn rtdetr_r18_forward_has_auxiliary_predictions_and_replaced_head() {
        let _guard = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, false).unwrap();
        let config = RtdetrV2Config {
            classes: 2,
            queries: 4,
            ..Default::default()
        };
        let model = RtdetrV2::new(&config, &device).unwrap().valid();
        let input = Tensor::ones([1, 3, 64, 64], &device) * 0.25;
        let output = model.forward(input.clone()).unwrap();
        assert_eq!(output.prediction.logits.dims(), [1, 4, 2]);
        assert_eq!(output.prediction.boxes.dims(), [1, 4, 4]);
        assert_eq!(output.auxiliary.len(), 3);
        let boxes = output
            .prediction
            .boxes
            .to_data()
            .try_to_vec::<f32>()
            .unwrap();
        assert!(
            boxes
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        );
        let replaced = model
            .reset_classifier(3, &device)
            .configure_fine_tuning(true)
            .valid();
        let second = replaced.forward(input).unwrap();
        assert_eq!(second.prediction.logits.dims(), [1, 4, 3]);
        let second_boxes = second
            .prediction
            .boxes
            .to_data()
            .try_to_vec::<f32>()
            .unwrap();
        assert!(second_boxes.iter().all(|v| v.is_finite()));
    }
}

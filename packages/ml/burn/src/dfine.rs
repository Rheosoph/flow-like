// Native D-FINE Nano, Copyright (c) 2024 The D-FINE Authors.
// Upstream architecture is licensed under Apache-2.0; see licenses/D-FINE.
use crate::{
    DetrOutput, DetrPrediction, DfineConfig, Error, Result,
    detr_layers::{Attention, Mlp, inverse_sigmoid, linear, norm},
    dfine_backbone::{DfineNanoEncoder, HgNetV2B0},
    pretrained::SafeTensorReader,
    rtdetr::{anchors, validate_anchor_buffers},
};
use burn::{
    module::{Module, Param, RunningState},
    nn::{Initializer, LayerNorm, Linear, LinearConfig},
    tensor::{
        Device, Tensor, TensorData,
        activation::{relu, sigmoid, softmax},
        ops::GridSampleOptions,
    },
};

const CHANNELS: usize = 128;
const HEADS: usize = 8;
const POINTS: usize = 6;
const BINS: usize = 33;
const SCALE: f64 = 4.0;

#[derive(Module, Debug)]
struct CrossAttention {
    offsets: Linear,
    weights: Linear,
    point_scale: RunningState<Tensor<1>>,
}

impl CrossAttention {
    fn new(device: &Device) -> Self {
        let mut offsets = LinearConfig::new(CHANNELS, HEADS * POINTS * 2 * 2)
            .with_initializer(Initializer::Zeros)
            .init(device);
        let mut bias = Vec::new();
        for head in 0..HEADS {
            let (y, x) = (head as f32 * std::f32::consts::TAU / HEADS as f32).sin_cos();
            let maximum = x.abs().max(y.abs());
            for _ in 0..2 {
                for point in 1..=POINTS {
                    bias.extend([x / maximum * point as f32, y / maximum * point as f32]);
                }
            }
        }
        offsets.bias = Some(Param::from_tensor(Tensor::from_data(
            TensorData::new(bias, [HEADS * POINTS * 2 * 2]),
            device,
        )));
        Self {
            offsets,
            weights: LinearConfig::new(CHANNELS, HEADS * POINTS * 2)
                .with_initializer(Initializer::Zeros)
                .init(device),
            point_scale: RunningState::new(Tensor::full([POINTS * 2], 1.0 / POINTS as f32, device)),
        }
    }

    fn forward(
        &self,
        query: Tensor<3>,
        reference: Tensor<3>,
        memory: Tensor<3>,
        shapes: &[[usize; 2]],
    ) -> Tensor<3> {
        let [batch, queries, _] = query.dims();
        let tokens = memory.dims()[1];
        let values = memory.reshape([batch, tokens, HEADS, CHANNELS / HEADS]);
        let offsets =
            self.offsets
                .forward(query.clone())
                .reshape([batch, queries, HEADS, POINTS * 2, 2]);
        let weights = softmax(
            self.weights
                .forward(query)
                .reshape([batch, queries, HEADS, POINTS * 2]),
            3,
        )
        .permute([0, 2, 1, 3])
        .reshape([batch * HEADS, 1, queries, POINTS * 2]);
        let xy = reference
            .clone()
            .slice([0..batch, 0..queries, 0..2])
            .reshape([batch, queries, 1, 1, 2]);
        let wh = reference
            .slice([0..batch, 0..queries, 2..4])
            .reshape([batch, queries, 1, 1, 2]);
        let mut samples = Vec::new();
        let mut start = 0;
        for (level, &[height, width]) in shapes.iter().enumerate() {
            let grid = xy.clone()
                + offsets.clone().slice([
                    0..batch,
                    0..queries,
                    0..HEADS,
                    level * POINTS..(level + 1) * POINTS,
                    0..2,
                ]) * wh.clone()
                    * self
                        .point_scale
                        .value()
                        .slice([level * POINTS..(level + 1) * POINTS])
                        .reshape([1, 1, 1, POINTS, 1])
                    * 0.5;
            let grid = (grid * 2.0 - 1.0).permute([0, 2, 1, 3, 4]).reshape([
                batch * HEADS,
                queries,
                POINTS,
                2,
            ]);
            let value = values
                .clone()
                .slice([
                    0..batch,
                    start..start + height * width,
                    0..HEADS,
                    0..CHANNELS / HEADS,
                ])
                .permute([0, 2, 3, 1])
                .reshape([batch * HEADS, CHANNELS / HEADS, height, width]);
            samples.push(value.grid_sample_2d(grid, GridSampleOptions::default()));
            start += height * width;
        }
        (Tensor::cat(samples, 3) * weights)
            .sum_dim(3)
            .reshape([batch, CHANNELS, queries])
            .swap_dims(1, 2)
    }

    fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.offsets =
            reader.linear(self.offsets, &format!("{prefix}.sampling_offsets"), device)?;
        self.weights =
            reader.linear(self.weights, &format!("{prefix}.attention_weights"), device)?;
        let points =
            reader.tensor::<1>(&format!("{prefix}.num_points_scale"), [POINTS * 2], device)?;
        if points
            .to_data()
            .iter::<f32>()
            .any(|value| (value - 1.0 / POINTS as f32).abs() > 1e-4)
        {
            return Err(Error::Invalid(
                "D-FINE point scales do not match the Nano architecture".into(),
            ));
        }
        // Official EMA checkpoints contain a small drift in this non-trainable buffer.
        self.point_scale = RunningState::new(points);
        Ok(self)
    }
}

#[derive(Module, Debug)]
struct Decoder {
    attention: Attention,
    cross: CrossAttention,
    norm1: LayerNorm,
    gate: Linear,
    gate_norm: LayerNorm,
    first: Linear,
    second: Linear,
    norm3: LayerNorm,
}

impl Decoder {
    fn new(device: &Device) -> Self {
        Self {
            attention: Attention::new(CHANNELS, HEADS, device),
            cross: CrossAttention::new(device),
            norm1: norm(CHANNELS, device),
            gate: LinearConfig::new(CHANNELS * 2, CHANNELS * 2)
                .with_initializer(Initializer::Zeros)
                .init(device),
            gate_norm: norm(CHANNELS, device),
            first: linear(CHANNELS, 512, device),
            second: linear(512, CHANNELS, device),
            norm3: norm(CHANNELS, device),
        }
    }
    fn forward(
        &self,
        target: Tensor<3>,
        position: Tensor<3>,
        reference: Tensor<3>,
        memory: Tensor<3>,
        shapes: &[[usize; 2]],
    ) -> Tensor<3> {
        let [batch, queries, _] = target.dims();
        let positioned = target.clone() + position.clone();
        let target = self.norm1.forward(
            target.clone()
                + self
                    .attention
                    .forward(positioned.clone(), positioned, target),
        );
        let cross = self
            .cross
            .forward(target.clone() + position, reference, memory, shapes);
        let gates = sigmoid(
            self.gate
                .forward(Tensor::cat(vec![target.clone(), cross.clone()], 2)),
        );
        let target = self.gate_norm.forward(
            gates.clone().slice([0..batch, 0..queries, 0..CHANNELS]) * target
                + gates.slice([0..batch, 0..queries, CHANNELS..CHANNELS * 2]) * cross,
        );
        let update = self
            .second
            .forward(relu(self.first.forward(target.clone())));
        self.norm3
            .forward((target + update).clamp(-65504.0, 65504.0))
    }
    fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.attention = self
            .attention
            .import(&format!("{prefix}.self_attn"), reader, device)?;
        self.cross = self
            .cross
            .import(&format!("{prefix}.cross_attn"), reader, device)?;
        self.norm1 = reader.layer_norm(self.norm1, &format!("{prefix}.norm1"), device)?;
        self.gate = reader.linear(self.gate, &format!("{prefix}.gateway.gate"), device)?;
        self.gate_norm =
            reader.layer_norm(self.gate_norm, &format!("{prefix}.gateway.norm"), device)?;
        self.first = reader.linear(self.first, &format!("{prefix}.linear1"), device)?;
        self.second = reader.linear(self.second, &format!("{prefix}.linear2"), device)?;
        self.norm3 = reader.layer_norm(self.norm3, &format!("{prefix}.norm3"), device)?;
        Ok(self)
    }
}

#[derive(Module, Debug)]
pub struct DfineNano {
    backbone: HgNetV2B0,
    encoder: DfineNanoEncoder,
    encoder_projection: Linear,
    encoder_norm: LayerNorm,
    encoder_score: Linear,
    encoder_boxes: Mlp,
    query_position: Mlp,
    pre_boxes: Mlp,
    decoder: Vec<Decoder>,
    scores: Vec<Linear>,
    distributions: Vec<Mlp>,
    quality: Vec<Mlp>,
    #[module(skip)]
    config: DfineConfig,
}

impl DfineNano {
    pub fn new(config: &DfineConfig, device: &Device) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            backbone: HgNetV2B0::new(device),
            encoder: DfineNanoEncoder::new(device),
            encoder_projection: linear(CHANNELS, CHANNELS, device),
            encoder_norm: norm(CHANNELS, device),
            encoder_score: score_head(config.classes, device),
            encoder_boxes: Mlp::new(CHANNELS, CHANNELS, 4, 3, device).zero_last(),
            query_position: Mlp::new(4, CHANNELS * 2, CHANNELS, 2, device),
            pre_boxes: Mlp::new(CHANNELS, CHANNELS, 4, 3, device).zero_last(),
            decoder: (0..3).map(|_| Decoder::new(device)).collect(),
            scores: (0..3).map(|_| score_head(config.classes, device)).collect(),
            distributions: (0..3)
                .map(|_| Mlp::new(CHANNELS, CHANNELS, 4 * BINS, 3, device).zero_last())
                .collect(),
            quality: (0..3)
                .map(|_| Mlp::new(20, 64, 1, 2, device).zero_last())
                .collect(),
            config: config.clone(),
        })
    }

    pub fn forward(&self, input: Tensor<4>) -> Result<DetrOutput> {
        self.config.validate_input_shape(&input.dims())?;
        let features = self.encoder.forward(self.backbone.forward(input));
        let shapes = features
            .iter()
            .map(|feature| [feature.dims()[2], feature.dims()[3]])
            .collect::<Vec<_>>();
        let memory = Tensor::cat(
            features
                .into_iter()
                .map(|feature| feature.flatten::<3>(2, 3).swap_dims(1, 2))
                .collect(),
            1,
        );
        let [batch, _, _] = memory.dims();
        let (anchors, valid) = anchors(&shapes, &memory.device());
        let memory = memory * valid;
        let projected = self
            .encoder_norm
            .forward(self.encoder_projection.forward(memory.clone()));
        let logits = self.encoder_score.forward(projected.clone());
        let indices = logits
            .clone()
            .max_dim(2)
            .squeeze_dim::<2>(2)
            .topk_with_indices(self.config.queries, 1)
            .1
            .unsqueeze_dim::<3>(2);
        let selected = projected.gather(
            1,
            indices
                .clone()
                .expand([batch, self.config.queries, CHANNELS]),
        );
        let selected_anchors = anchors
            .expand([batch, shapes.iter().map(|[h, w]| h * w).sum(), 4])
            .gather(1, indices.clone().expand([batch, self.config.queries, 4]));
        let coordinates = self.encoder_boxes.forward(selected.clone()) + selected_anchors;
        let mut target = selected.detach();
        let mut references = sigmoid(coordinates.clone().detach());
        let mut predictions = vec![DetrPrediction {
            logits: logits.gather(
                1,
                indices.expand([batch, self.config.queries, self.config.classes]),
            ),
            boxes: sigmoid(coordinates),
        }];
        let mut initial = references.clone();
        let mut previous_output = target.zeros_like();
        let mut corners = Tensor::zeros([batch, self.config.queries, 4 * BINS], &memory.device());
        for index in 0..3 {
            let position = self
                .query_position
                .forward(references.clone())
                .clamp(-10.0, 10.0);
            target = self.decoder[index].forward(
                target,
                position,
                references.clone(),
                memory.clone(),
                &shapes,
            );
            if index == 0 {
                let boxes =
                    sigmoid(self.pre_boxes.forward(target.clone()) + inverse_sigmoid(references));
                initial = boxes.clone().detach();
                predictions.push(DetrPrediction {
                    logits: self.scores[0].forward(target.clone()),
                    boxes,
                });
            }
            corners = self.distributions[index].forward(target.clone() + previous_output) + corners;
            let probabilities = softmax(
                corners
                    .clone()
                    .reshape([batch, self.config.queries, 4, BINS]),
                3,
            );
            let boxes = decode_distribution(initial.clone(), probabilities.clone());
            let top = probabilities.topk(4, 3);
            let statistics = Tensor::cat(vec![top.clone(), top.mean_dim(3)], 3).reshape([
                batch,
                self.config.queries,
                20,
            ]);
            let logits = self.scores[index].forward(target.clone())
                + self.quality[index].forward(statistics);
            references = boxes.clone().detach();
            previous_output = target.clone().detach();
            predictions.push(DetrPrediction { logits, boxes });
        }
        Ok(DetrOutput {
            prediction: predictions.pop().unwrap(),
            auxiliary: predictions,
        })
    }

    pub(crate) fn reset_classifier(mut self, classes: usize, device: &Device) -> Self {
        self.encoder_score = score_head(classes, device);
        self.scores = (0..3).map(|_| score_head(classes, device)).collect();
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
        self.pre_boxes = self
            .pre_boxes
            .import("decoder.pre_bbox_head", reader, device)?;
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
        self.distributions = self
            .distributions
            .into_iter()
            .enumerate()
            .map(|(i, layer)| layer.import(&format!("decoder.dec_bbox_head.{i}"), reader, device))
            .collect::<Result<_>>()?;
        self.quality = self
            .quality
            .into_iter()
            .enumerate()
            .map(|(i, layer)| {
                layer.import(
                    &format!("decoder.decoder.lqe_layers.{i}.reg_conf"),
                    reader,
                    device,
                )
            })
            .collect::<Result<_>>()?;
        for (name, value) in [
            ("decoder.up", 0.5f32),
            ("decoder.reg_scale", SCALE as f32),
            ("decoder.decoder.up", 0.5),
            ("decoder.decoder.reg_scale", SCALE as f32),
        ] {
            if reader.has(name) {
                let (dtype, shape, bytes) = reader.raw(name)?;
                let actual = match dtype {
                    safetensors::Dtype::F32 if bytes.len() == 4 => {
                        f32::from_le_bytes(bytes.try_into().unwrap())
                    }
                    safetensors::Dtype::I64 if bytes.len() == 8 => {
                        i64::from_le_bytes(bytes.try_into().unwrap()) as f32
                    }
                    _ => f32::NAN,
                };
                if shape != [1] || actual != value {
                    return Err(Error::Invalid(format!(
                        "Unsupported D-FINE distribution parameter {name}"
                    )));
                }
            }
        }
        if reader.has("decoder.denoising_class_embed.weight") {
            let _ = reader.tensor::<2>(
                "decoder.denoising_class_embed.weight",
                [self.config.classes + 1, CHANNELS],
                device,
            )?;
        }
        validate_anchor_buffers(reader, &[[40, 40], [20, 20]], device)?;
        Ok(self)
    }
}

fn score_head(classes: usize, device: &Device) -> Linear {
    let mut layer = linear(CHANNELS, classes, device);
    layer.bias = Some(Param::from_tensor(Tensor::full(
        [classes],
        (0.01f32 / 0.99).ln(),
        device,
    )));
    layer
}

fn distribution_weights() -> Vec<f32> {
    let step = (1.0 + 0.5 * SCALE).powf(2.0 / (BINS - 3) as f64);
    let mut weights = vec![-SCALE as f32];
    weights.extend(
        (1..BINS / 2)
            .rev()
            .map(|index| (1.0 - step.powi(index as i32)) as f32),
    );
    weights.push(0.0);
    weights.extend((1..BINS / 2).map(|index| (step.powi(index as i32) - 1.0) as f32));
    weights.push(SCALE as f32);
    weights
}

fn decode_distribution(reference: Tensor<3>, probability: Tensor<4>) -> Tensor<3> {
    let [batch, queries, _] = reference.dims();
    let project = Tensor::from_data(
        TensorData::new(distribution_weights(), [1, 1, 1, BINS]),
        &reference.device(),
    );
    let distance = (probability * project)
        .sum_dim(3)
        .reshape([batch, queries, 4]);
    let center = reference.clone().slice([0..batch, 0..queries, 0..2]);
    let size = reference.slice([0..batch, 0..queries, 2..4]);
    let lower = center.clone()
        - (distance.clone().slice([0..batch, 0..queries, 0..2]) + SCALE * 0.5) * size.clone()
            / SCALE;
    let upper =
        center + (distance.slice([0..batch, 0..queries, 2..4]) + SCALE * 0.5) * size / SCALE;
    Tensor::cat(
        vec![(lower.clone() + upper.clone()) * 0.5, upper - lower],
        2,
    )
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;

    #[test]
    fn symmetric_distribution_preserves_reference_boxes_and_refinement_has_gradients() {
        let _guard = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, true).unwrap();
        let weights = distribution_weights();
        assert_eq!(weights.len(), BINS);
        assert_eq!(weights[16], 0.0);
        assert_eq!(weights[0], -4.0);
        assert_eq!(weights[32], 4.0);
        let reference = Tensor::from_data([[[0.4f32, 0.6, 0.2, 0.3]]], &device);
        let logits = Tensor::<4>::zeros([1, 1, 4, BINS], &device).require_grad();
        let boxes = decode_distribution(reference.clone(), softmax(logits.clone(), 3));
        let error = (boxes.clone() - reference).abs().max().into_scalar::<f32>();
        assert!(error < 1e-6);
        let gradients = boxes.square().sum().backward();
        let values = logits
            .grad(&gradients)
            .unwrap()
            .to_data()
            .iter::<f32>()
            .collect::<Vec<_>>();
        assert!(values.iter().all(|v| v.is_finite()));
        assert!(values.iter().any(|v| v.abs() > 1e-6));
    }

    #[test]
    fn nano_forward_and_replaced_head_keep_all_auxiliary_predictions() {
        let _guard = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, false).unwrap();
        let model = DfineNano::new(
            &DfineConfig {
                classes: 2,
                queries: 4,
            },
            &device,
        )
        .unwrap()
        .valid();
        let input = Tensor::<4>::ones([1, 3, 64, 64], &device);
        let output = model.forward(input.clone()).unwrap();
        assert_eq!(output.prediction.logits.dims(), [1, 4, 2]);
        assert_eq!(output.prediction.boxes.dims(), [1, 4, 4]);
        assert_eq!(output.auxiliary.len(), 4);
        assert!(
            output
                .prediction
                .boxes
                .to_data()
                .iter::<f32>()
                .all(|value| value.is_finite())
        );
        let model = model.reset_classifier(3, &device);
        assert_eq!(
            model.forward(input).unwrap().prediction.logits.dims(),
            [1, 4, 3]
        );
    }

    #[test]
    fn nano_matching_loss_trains_decoder_classifier_and_distribution_heads() {
        let _guard = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, true).unwrap();
        let model = DfineNano::new(
            &DfineConfig {
                classes: 2,
                queries: 4,
            },
            &device,
        )
        .unwrap()
        .train();
        let output = model
            .forward(Tensor::ones([2, 3, 64, 64], &device))
            .unwrap();
        let targets = vec![
            vec![crate::BoundingBox {
                class_id: 0,
                x_min: 0.2,
                y_min: 0.3,
                x_max: 0.6,
                y_max: 0.7,
            }];
            2
        ];
        let loss = output.loss(&targets).unwrap();
        assert!(loss.clone().into_scalar::<f32>().is_finite());
        let gradients = loss.backward();
        for parameter in [
            model.scores[2].weight.val(),
            model.distributions[2].layers[2].weight.val(),
            model.decoder[2].first.weight.val(),
        ] {
            let values = parameter
                .grad(&gradients)
                .unwrap()
                .to_data()
                .iter::<f32>()
                .collect::<Vec<_>>();
            assert!(values.iter().all(|value| value.is_finite()));
            assert!(values.iter().any(|value| value.abs() > 1e-7));
        }
    }

    #[test]
    #[ignore = "Requires the official checkpoint provided by FLOW_LIKE_DFINE_WEIGHTS"]
    fn official_nano_checkpoint_imports_and_runs_natively() {
        let _guard = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let checkpoint = std::env::var("FLOW_LIKE_DFINE_WEIGHTS").unwrap();
        let metadata = crate::PretrainedWeightsMetadata {
            source:
                "https://github.com/Peterande/storage/releases/download/dfinev1.0/dfine_n_coco.pth"
                    .into(),
            license: Some("Apache-2.0".into()),
            weights_license: None,
            revision: None,
            expected_sha256: None,
        };
        let artifact = tempfile::tempdir().unwrap();
        let info =
            crate::import_dfine_nano_weights(&checkpoint, artifact.path(), &metadata).unwrap();
        assert!(
            info.provenance
                .format
                .starts_with("dfine_nano:pytorch_zip:")
        );
        let predictor = crate::Predictor::load(artifact.path(), crate::BackendChoice::Cpu).unwrap();
        assert_eq!(predictor.recipe(), &info.recipe);
        let device = crate::backend::device(&crate::BackendChoice::Cpu, false).unwrap();
        let (bytes, _, _) = crate::pretrained::read_import_weights(
            std::path::Path::new(&checkpoint),
            &metadata,
            256 * 1024 * 1024,
        )
        .unwrap();
        let mut reader = SafeTensorReader::new(&bytes).unwrap();
        let model = DfineNano::new(&DfineConfig::default(), &device)
            .unwrap()
            .import_safetensors(&mut reader, &device)
            .unwrap()
            .valid();
        reader.finish().unwrap();
        let input = Tensor::from_data(
            TensorData::new(
                (0..3 * 320 * 320)
                    .map(|index| (index % 251) as f32 / 250.0)
                    .collect(),
                [1, 3, 320, 320],
            ),
            &device,
        );
        let prediction = model.forward(input).unwrap().prediction;
        let logits = prediction
            .logits
            .to_data()
            .iter::<f32>()
            .collect::<Vec<_>>();
        let boxes = prediction.boxes.to_data().iter::<f32>().collect::<Vec<_>>();
        let reference = std::env::var("FLOW_LIKE_DFINE_REFERENCE").ok().map(|path| {
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(path).unwrap()).unwrap()
        });
        let expected_logits: Vec<f32> = reference
            .as_ref()
            .map(|value| serde_json::from_value(value["logits"].clone()).unwrap())
            .unwrap_or_else(|| {
                vec![
                    -3.29314, -4.1900396, -3.9867053, -4.3435555, -2.397341, -3.7961297,
                    -2.4473875, -3.557666,
                ]
            });
        let expected_boxes: Vec<f32> = reference
            .as_ref()
            .map(|value| serde_json::from_value(value["boxes"].clone()).unwrap())
            .unwrap_or_else(|| {
                vec![
                    0.540658, 0.509612, 0.6306656, 0.6006757, 0.51099765, 0.50063443, 0.93743753,
                    0.6949888,
                ]
            });
        for (kind, actual, expected) in [
            ("logits", logits, expected_logits),
            ("boxes", boxes, expected_boxes),
        ] {
            if reference.is_some() {
                assert_eq!(actual.len(), expected.len());
            }
            let error = actual
                .iter()
                .zip(&expected)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(actual.iter().all(|value| value.is_finite()));
            assert!(
                error < 1e-3,
                "D-FINE {kind} parity error {error}; actual first values {:?}",
                &actual[..8]
            );
        }
    }
}
